// annex/src/main.rs

use clap::{Parser, ValueEnum};
use std::{net::SocketAddr};
use tracing::info;

use rmcp::{
    model::{CallToolResult, Content, ServerCapabilities, ServerInfo},
    tool, tool_handler, tool_router,
    ServerHandler,
};

#[derive(Clone)]
struct AnnexMcp {
    // later: inject your hook registries, task runners, etc.
}

#[tool_router]
impl AnnexMcp {
    fn new() -> Self { Self {} }

    #[tool(description = "Liveness check; returns 'pong'.")]
    async fn ping(&self) -> Result<CallToolResult, rmcp::Error> {
        Ok(CallToolResult::success(vec![Content::text("pong")]))
    }
}

#[tool_handler]
impl ServerHandler for AnnexMcp {
    fn get_info(&self) -> ServerInfo {
        ServerInfo {
            name: Some("annex-mcp".into()),
            version: Some(env!("CARGO_PKG_VERSION").into()),
            instructions: Some("annex MCP adapter".into()),
            capabilities: ServerCapabilities::builder().enable_tools().build(),
            ..Default::default()
        }
    }
}

#[derive(ValueEnum, Clone, Debug)]
enum Transport { Stdio, Sse, StreamableHttp }

#[derive(Parser)]
#[command(name="annex-mcp", version, about="MCP server for annex")]
struct Args {
    /// Choose the transport: stdio | sse | streamable-http
    #[arg(long, value_enum, default_value_t = Transport::Stdio)]
    transport: Transport,
    /// Bind address for SSE / Streamable HTTP (ignored for stdio)
    #[arg(long, default_value = "127.0.0.1:8848")]
    addr: String,
    /// HTTP endpoint path for Streamable HTTP (default '/mcp')
    #[arg(long, default_value = "/mcp")]
    http_path: String,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(std::env::var("RUST_LOG").unwrap_or_else(|_| "info".into()))
        .init();

    let args = Args::parse();
    match args.transport {
        Transport::Stdio => run_stdio().await?,
        Transport::Sse => run_sse(args.addr.parse()?).await?,
        Transport::StreamableHttp => run_streamable_http(args.addr.parse()?, args.http_path).await?,
    }
    Ok(())
}

async fn run_stdio() -> Result<(), Box<dyn std::error::Error>> {
    use rmcp::service::serve_server;
    use rmcp::transport::io::stdio;

    info!("Starting MCP server over STDIO …");
    let running = serve_server(AnnexMcp::new(), stdio()).await?;
    running.waiting().await?;
    Ok(())
}

#[cfg(feature = "sse")]
async fn run_sse(bind: SocketAddr) -> Result<(), Box<dyn std::error::Error>> {
    use rmcp::service::serve_server;
    use rmcp::transport::sse_server::SseServer;

    info!("Starting MCP server over SSE at http://{bind}/mcp (see spec).");
    let mut sse = SseServer::serve(bind).await?;
    // Each SSE connection yields a transport; serve it with a fresh handler
    while let Some(transport) = sse.next_transport().await {
        tokio::spawn(async move {
            let _ = serve_server(AnnexMcp::new(), transport).await;
        });
    }
    Ok(())
}

#[cfg(not(feature = "sse"))]
async fn run_sse(_: SocketAddr) -> Result<(), Box<dyn std::error::Error>> {
    Err("annex-mcp built without 'sse' feature".into())
}

#[cfg(feature = "streamable_http")]
async fn run_streamable_http(bind: SocketAddr, path: String) -> Result<(), Box<dyn std::error::Error>> {
    use std::{convert::Infallible, sync::Arc};
    use hyper::{server::conn::http1, service::service_fn};
    use rmcp::transport::streamable_http_server::tower::{
        LocalSessionManager, StreamableHttpServerConfig, StreamableHttpService,
    };

    info!("Starting MCP server over Streamable HTTP at http://{bind}{path}");
    let config = StreamableHttpServerConfig::default().with_path(path);
    let svc = StreamableHttpService::new(
        || Ok(AnnexMcp::new()),
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

#[cfg(not(feature = "streamable_http"))]
async fn run_streamable_http(_: SocketAddr, _: String) -> Result<(), Box<dyn std::error::Error>> {
    Err("annex-mcp built without 'streamable_http' feature".into())
}