# annex
Rust based extension to codex-rs


# mods to codex-rs to integrate this package:

## Cargo.toml changes:

```toml
[workspace]
members = ["codex-ext", /* existing crates ... */]
```

## Bootstrap this into Core:

```rust
// core/src/services.rs (new)
use std::sync::Arc;
use codex_ext::{
  layered_config::ConfigManager,
  hooks::{HookRegistry, AuditLogHook},
  mcp_runtime::McpRuntime,
  slash::{SlashRegistry, AllowCommand, McpAddCommand, ConfigSetCommand},
};

pub struct Services {
    pub cfg: Arc<ConfigManager>,
    pub hooks: Arc<HookRegistry>,
    pub mcp: Arc<McpRuntime>,
    pub slash: Arc<SlashRegistry>,
}

impl Services {
    pub async fn init(workspace_root: std::path::PathBuf) -> anyhow::Result<Self> {
        let cfg = Arc::new(ConfigManager::load(&workspace_root)?);
        let hooks = Arc::new(HookRegistry::default());
        hooks.register(std::sync::Arc::new(AuditLogHook)).await;
        let mcp = Arc::new(McpRuntime::new(cfg.clone()));
        mcp.reconcile().await?;
        let mut slash = SlashRegistry::new();
        slash.register(std::sync::Arc::new(AllowCommand { cfg: cfg.clone() }));
        slash.register(std::sync::Arc::new(McpAddCommand { cfg: cfg.clone() }));
        slash.register(std::sync::Arc::new(ConfigSetCommand { cfg: cfg.clone() }));
        Ok(Self { cfg, hooks, mcp, slash: Arc::new(slash) })
    }
}
```

## Adjust Spawn:

```rust
// core/src/spawn.rs
// Pseudocode inserted where Command is constructed/executed.

use codex_ext::hooks::{HookContext, HookEvent, HookDecision};
use std::collections::BTreeMap;

// Build context once per session:
let ctx = HookContext {
    cwd: std::env::current_dir().unwrap_or_default(),
    env: std::env::vars().map(|(k,v)|(k,v)).collect::<BTreeMap<_,_>>(),
    session_id: format!("{}", uuid::Uuid::new_v4()),
};

// BEFORE spawn:
if let HookDecision::Deny { reason } = services.hooks.emit(&ctx, &HookEvent::PreExec {
    cmd: cmd_string.clone(),
    argv: argv_vec.clone(),
}).await? {
    // Respect denial
    return Err(anyhow::anyhow!("Execution denied by hook: {}", reason));
}

// ... run the process ...

// AFTER completion:
let _ = services.hooks.emit(&ctx, &HookEvent::PostExec {
    cmd: cmd_string,
    argv: argv_vec,
    status: exit_code,
    stdout_len,
    stderr_len,
}).await;
```

## Adjust MCP behavior

```rust
// ??
// On startup:
services.mcp.reconcile().await?;

// Subscribe to config changes (hot‑reload):
let mut rx = services.cfg.subscribe();
tokio::spawn({
    let mcp = services.mcp.clone();
    async move {
        while rx.recv().await.is_ok() {
            let _ = mcp.reconcile().await;
        }
    }
});

// Around request/response calls:
use codex_ext::hooks::{HookEvent, HookDecision};
if let HookDecision::Deny{reason} = services.hooks.emit(&ctx, &HookEvent::PreMcp{
    server: server_name.clone(),
    method: method_name.clone(),
    payload: serde_json::json!(/* your payload here */),
}).await? {
    return Err(anyhow::anyhow!("MCP call denied: {}", reason));
}
// ... perform MCP call ...
let _ = services.hooks.emit(&ctx, &HookEvent::PostMcp{ server: server_name, method: method_name, payload: serde_json::json!(result) }).await;
```

## TUI Mod:

```
// tui/src/tui.rs
// When user submits a line that starts with `/`, dispatch:
if input_line.starts_with('/') {
    match services.slash.dispatch(&input_line).await {
        Ok(msg) => ui.flash_info(msg),
        Err(e)  => ui.flash_error(format!("{}", e)),
    }
    input_line.clear();
    continue;
}
```