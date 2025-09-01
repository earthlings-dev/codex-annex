# annex
Rust based extension to codex-rs


# mods to codex-rs to integrate this package:

## Cargo.toml changes:

```toml
[workspace]
members = ["codex-ext", /* existing crates … */]
resolver = "2"
```

## Bootstrap this into Core:

```rust
// core/src/services.rs (new)
use std::sync::Arc;
use codex_ext::{
  ConfigManager, HookRegistry, McpRuntime,
  hooks::AuditLogHook,
  slash::{SlashRegistry, AllowCommand, McpAddCommand, ConfigSetCommand, TodoCommand, CompactCommand, AutoCompactToggle},
  compact::Compactor,
};

pub struct Services {
    pub cfg: Arc<ConfigManager>,
    pub hooks: Arc<HookRegistry>,
    pub mcp: Arc<McpRuntime>,
    pub slash: Arc<SlashRegistry>,
    pub compactor: Arc<Compactor>,
}

impl Services {
    pub async fn init(workspace_root: std::path::PathBuf) -> anyhow::Result<Self> {
        let cfg = Arc::new(ConfigManager::load(&workspace_root)?);
        let hooks = Arc::new(HookRegistry::default());
        hooks.register(std::sync::Arc::new(AuditLogHook)).await;
        let mcp = Arc::new(McpRuntime::new(cfg.clone()));
        mcp.reconcile().await?;
        let mut sr = SlashRegistry::new();
        sr.register(std::sync::Arc::new(AllowCommand { cfg: cfg.clone() }));
        sr.register(std::sync::Arc::new(McpAddCommand { cfg: cfg.clone() }));
        sr.register(std::sync::Arc::new(ConfigSetCommand { cfg: cfg.clone() }));
        sr.register(std::sync::Arc::new(TodoCommand { cfg: cfg.clone(), workspace: workspace_root.clone() }));
        sr.register(std::sync::Arc::new(CompactCommand { cfg: cfg.clone(), workspace: workspace_root.clone() }));
        sr.register(std::sync::Arc::new(AutoCompactToggle { cfg: cfg.clone() }));
        let compactor = Arc::new(Compactor::new(cfg.clone(), workspace_root.clone()));
        Ok(Self { cfg, hooks, mcp, slash: Arc::new(sr), compactor })
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

## Auto-Compact Behavior at Task End

```rust
if services.compactor.should_autotrigger(last_compact, codex_ext::compact::AutoCompactStage::EndOfTask) {
    // 1) Ask model to *generate a meta-prompt*:
    let meta = services.compactor.auto_compact(
        codex_ext::compact::AutoCompactStage::EndOfTask,
        |stage, todo_json, activity_json| {
            Box::pin(async move {
                // Call your existing chat send with a small system prompt; return String
                // Example meta-prompt (YOU will actually call the model in your pipeline):
                let meta = format!(
                    "You are preparing a compact summary. Stage={:?}. \
                     Use TODOs={} and activity={} to decide focus (completed vs pending, key diffs, blockers). \
                     Respond ONLY with a concise focus instruction for summarization.",
                    stage, todo_json, activity_json
                );
                Ok(meta)
            })
        }
    ).await?;
    // 2) Feed `res.focus_prompt` + attach `res.chosen_files` contents to the model request for final summary.
    last_compact = Some(std::time::SystemTime::now());
}
```

# Example configs:

```toml
# .codex/config.toml
[ui]
command_palette = true
status_bar = true

[model]
name = "gpt-omni-mini"   # whatever your backend supports

[shell]
allowlist_roots = ["git","rg","ls","cat","cargo"]
environment_inherit = "core"
env_exclude_patterns = ["*KEY*","*TOKEN*"]

[mcp.servers.build_indexer]
enabled = true
transport = "stdio"
command = "/usr/local/bin/build-indexer"
args = ["--fast"]
scope = "workspace"
```