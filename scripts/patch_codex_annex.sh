#!/usr/bin/env bash

# annex/scripts/patch_codex_annex.sh

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SUB="$ROOT/external/openai-codex"
CR_RS="$SUB/codex-rs"
CR_CLI="$SUB/codex-cli"

die(){ echo "ERR: $*"; exit 1; }

[ -d "$CR_RS" ] || die "codex-rs not found ($CR_RS). Add codex submodule and include codex-cli in sparse set."
[ -d "$CR_CLI" ] || die "codex-cli not found ($CR_CLI). Add codex-cli to sparse set."

# --- codex-rs: features + deps ------------------------------------------------
CARGO_RS="$CR_RS/Cargo.toml"

add_block_if_missing() { # ($file, $anchor_regex, $block)
  local file="$1" rx="$2" block="$3"
  if ! grep -Eq "$rx" "$file"; then printf "%s\n" "$block" >> "$file"; fi
}

# Ensure [features]
add_block_if_missing "$CARGO_RS" '^\[features\]' "[features]"

# Feature: annex core (connects to your annex crate by path)
add_block_if_missing "$CARGO_RS" '^annex[[:space:]]*=' 'annex = ["annex-crate"]'

# Feature groups for protocols
add_block_if_missing "$CARGO_RS" '^annex-mcp[[:space:]]*='      'annex-mcp = ["rmcp/server"]'
add_block_if_missing "$CARGO_RS" '^annex-mcp-sse[[:space:]]*='  'annex-mcp-sse = ["rmcp/transport-sse-server"]'
add_block_if_missing "$CARGO_RS" '^annex-mcp-stream[[:space:]]*=' 'annex-mcp-stream = ["rmcp/transport-streamable-http-server"]'
add_block_if_missing "$CARGO_RS" '^annex-acp[[:space:]]*='      'annex-acp = ["agent-client-protocol?/default"]'
add_block_if_missing "$CARGO_RS" '^annex-a2a[[:space:]]*='      'annex-a2a = ["jsonrpsee/server", "jsonrpsee/http-server"]'
add_block_if_missing "$CARGO_RS" '^annex-agentproto[[:space:]]*=' 'annex-agentproto = ["axum?/macros"]'
add_block_if_missing "$CARGO_RS" '^annex-all[[:space:]]*='      'annex-all = ["annex","annex-mcp","annex-mcp-sse","annex-mcp-stream","annex-acp","annex-a2a","annex-agentproto"]'

# Optional dep: your annex crate (repo root)
add_block_if_missing "$CARGO_RS" '^\[dependencies\.annex-crate\]' \
'[dependencies.annex-crate]
path = "../../../"   # from codex-rs to annex root
optional = true
'

# MCP (official rmcp crate from our pinned submodule)
add_block_if_missing "$CARGO_RS" '^\[dependencies\.rmcp\]' \
'[dependencies.rmcp]
path = "../../../external/mcp-rust-sdk/crates/rmcp"
default-features = false
features = ["server", "macros"]
'

# ACP (crate from crates.io or path if you prefer)
add_block_if_missing "$CARGO_RS" '^\[dependencies\.agent-client-protocol\]' \
'[dependencies.agent-client-protocol]
version = "0.1"
optional = true
'

# A2A + Agent Protocol scaffolding
add_block_if_missing "$CARGO_RS" '^\[dependencies\.jsonrpsee\]' \
'[dependencies.jsonrpsee]
version = "0.26"
default-features = false
features = ["server","http-server"]'

add_block_if_missing "$CARGO_RS" '^\[dependencies\.axum\]' \
'[dependencies.axum]
version = "0.8"
optional = true
features = ["macros","json"]
'

add_block_if_missing "$CARGO_RS" '^\[dependencies\.tokio\]' \
'[dependencies.tokio]
version = "1.38"
features = ["rt-multi-thread","macros","io-std","signal","net"]
'

add_block_if_missing "$CARGO_RS" '^\[dependencies\.tracing\]' 'tracing = "0.1"'
add_block_if_missing "$CARGO_RS" '^\[dependencies\.tracing-subscriber\]' \
'tracing-subscriber = { version = "0.3", features = ["fmt","env-filter"] }'
add_block_if_missing "$CARGO_RS" '^\[dependencies\.serde\]' 'serde = { version = "1", features = ["derive"] }'
add_block_if_missing "$CARGO_RS" '^\[dependencies\.serde_json\]' 'serde_json = "1"'
add_block_if_missing "$CARGO_RS" '^\[dependencies\.http\]' 'http = "1.1"'
add_block_if_missing "$CARGO_RS" '^\[dependencies\.hyper\]' 'hyper = { version = "1.4", features = ["http1","server"] }'

# Add annex module files
ANNEX_DIR="$CR_RS/src/annex"
mkdir -p "$ANNEX_DIR"

cat > "$ANNEX_DIR/mod.rs" <<'RS'
pub mod mcp;
// stubs for future phases (compile when features enabled)
#[cfg(feature = "annex-acp")]
pub mod acp;
#[cfg(feature = "annex-a2a")]
pub mod a2a;
#[cfg(feature = "annex-agentproto")]
pub mod agent_protocol;
RS

cat > "$ANNEX_DIR/mcp.rs" <<'RS'
use tracing::info;
use std::net::SocketAddr;

use rmcp::{
    ServerHandler,
    model::{ServerCapabilities, ServerInfo, CallToolResult, Content},
    tool, tool_router, tool_handler,
};

#[derive(Clone)]
struct Core; // In phase 2, inject your registries/task runner

#[tool_router]
impl Core {
    fn new() -> Self { Self }

    #[tool(description = "healthcheck")]
    async fn ping(&self) -> Result<CallToolResult, rmcp::Error> {
        Ok(CallToolResult::success(vec![Content::text("pong")]))
    }
}

#[tool_handler]
impl ServerHandler for Core {
    fn get_info(&self) -> ServerInfo {
        ServerInfo {
            name: Some("codex-mcp".into()),
            version: Some(env!("CARGO_PKG_VERSION").into()),
            instructions: Some("MCP server embedded in codex".into()),
            capabilities: ServerCapabilities::builder().enable_tools().build(),
            ..Default::default()
        }
    }
}

// ---- servers ----
pub async fn serve_stdio() -> Result<(), Box<dyn std::error::Error>> {
    use rmcp::{service::serve_server, transport::io::stdio};
    info!("codex MCP (stdio) starting…");
    let running = serve_server(Core::new(), stdio()).await?;
    running.waiting().await?;
    Ok(())
}

#[cfg(feature = "annex-mcp-sse")]
pub async fn serve_sse(bind: SocketAddr) -> Result<(), Box<dyn std::error::Error>> {
    use rmcp::{service::serve_server, transport::sse_server::SseServer};
    info!("codex MCP (SSE) http://{bind}/mcp");
    let mut sse = SseServer::serve(bind).await?;
    while let Some(transport) = sse.next_transport().await {
        tokio::spawn(async move { let _ = serve_server(Core::new(), transport).await; });
    }
    Ok(())
}

#[cfg(feature = "annex-mcp-stream")]
pub async fn serve_streamable_http(bind: SocketAddr, path: &str)
    -> Result<(), Box<dyn std::error::Error>>
{
    use std::sync::Arc;
    use hyper::{server::conn::http1, service::service_fn};
    use rmcp::transport::streamable_http_server::tower::{
        LocalSessionManager, StreamableHttpServerConfig, StreamableHttpService,
    };
    info!("codex MCP (Streamable HTTP) http://{bind}{path}");
    let config = StreamableHttpServerConfig::default().with_path(path.to_owned());
    let svc = StreamableHttpService::new(
        || Ok(Core::new()),
        Arc::new(LocalSessionManager::default()),
        config,
    );
    let listener = tokio::net::TcpListener::bind(bind).await?;
    loop {
        let (stream, _) = listener.accept().await?;
        let mut s = svc.clone();
        tokio::spawn(async move {
            if let Err(err) = http1::Builder::new()
                .serve_connection(stream, service_fn(move |req| s.call(req)))
                .await
            {
                eprintln!("HTTP error: {err}");
            }
        });
    }
}

// ---- clients ----
pub async fn connect_child_stdio(cmd: &str, args: &[&str]) -> Result<(), Box<dyn std::error::Error>> {
    use rmcp::{ServiceExt, transport::{TokioChildProcess, ConfigureCommandExt}};
    use tokio::process::Command;
    let mut command = Command::new(cmd);
    for a in args { command.arg(a); }
    let client = ().serve(TokioChildProcess::new(command.configure(|_| {}))?).await?;
    // Example interaction; list roots if available:
    let _ = client.capabilities().await.ok();
    Ok(())
}
RS

# Ensure codex-rs/lib exports `annex`
LIB_RS="$CR_RS/src/lib.rs"
if [ -f "$LIB_RS" ] && ! grep -q 'pub mod annex' "$LIB_RS"; then
  printf '\npub mod annex;\n' >> "$LIB_RS"
fi

# No transitional codex-* helper bins are added. Integration occurs via codex features only.
echo "✔ codex-rs + codex-cli patch: features aligned for annex integration"
