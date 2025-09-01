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
// core/src/services.rs
use std::sync::Arc;
use codex_ext::{
  ConfigManager, HookRegistry, SlashRegistry,
  session_log::SessionLogWriter,
  yaml_config::{Scope, ModelRole},
  hooks_yaml::HookContext,
  taskset::{TaskSetRunner, TaskSetPlan, UiEvent},
  todo_yaml::TodoStore,
  compact::{Compactor, AutoCompactStage},
};

pub struct Services {
    pub cfg: Arc<ConfigManager>,
    pub hooks: Arc<HookRegistry>,
    pub slash: Arc<SlashRegistry>,
}

impl Services {
    pub async fn init(workspace_root: std::path::PathBuf) -> anyhow::Result<Self> {
        let cfg = Arc::new(ConfigManager::load(&workspace_root)?);

        // Hooks + Slash from YAML dirs (system/user/workspace)
        let mut hook_dirs = vec![workspace_root.join(".codex").join("hooks")];
        hook_dirs.extend(cfg.get().hooks.dirs.clone());
        let hooks = Arc::new(HookRegistry::load_from_dirs(cfg.clone(), &hook_dirs)?);

        let mut slash_dirs = vec![workspace_root.join(".codex").join("slash")];
        slash_dirs.extend(cfg.get().slash.dirs.clone());
        let slash = Arc::new(SlashRegistry::load_from_dirs(cfg.clone(), &slash_dirs)?);

        // Session logs: setup + optional purge
        let log = SessionLogWriter::new(&cfg, "SESSION-UUID")?; // you’ll generate per run
        if let Some(days) = cfg.get().sessions.auto_purge_days { log.purge_old(days)?; }

        Ok(Self { cfg, hooks, slash })
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

# Configs

## File Layout

.codex/
  10-models.yaml                      # model routing (base_url, overrides, profiles)
  20-shell.yaml
  30-compact.yaml
  40-sessions.yaml                    # dir, auto_purge_days, resume_on_launch
  hooks/
    10-audit.yaml
    20-summarize.yaml
  slash/
    commands.yaml
  tasks/
    2025-08-31/SESSION-UUID/
      set-01.yaml                     # TaskSetSpec
      set-02.yaml
  todos/
    2025-08-31/SESSION-UUID/
      001-<todoid>.yaml               # TodoItem (session/date/task_number)
  sessions/
    2025-08-31/SESSION-UUID/session.yaml  # rolling YAML log (messages + metadata)

## Main Config 

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

## Model Configs

```yaml
# .codex/10-models.yaml
models:
  default:
    name: gpt-4o-mini
    base_url: https://api.openai.com/v1
    api_key_env: OPENAI_API_KEY
  overrides:
    title:
      name: gpt-4o-mini
    session_name:
      name: gpt-4o-mini
    compact:
      name: gpt-4o-mini
    meta_prompt:
      name: gpt-4o-mini
  profiles:
    heavy:
      name: gpt-4.1
    fast:
      name: gpt-4o-mini
    google:
      name: gemini-1.5-flash
      base_url: https://generativelanguage.googleapis.com
      api_key_env: GOOGLE_API_KEY
    anthropic:
      name: claude-3.7-sonnet
      base_url: https://api.anthropic.com
      api_key_env: ANTHROPIC_API_KEY
```

## Example Slash Commands


```
# .codex/slash/commands.yaml
allow:
  kind: builtin
  name: allowlist.add
  args: {}
todo:
  kind: alias
  expands_to: "/todo $ARGS"
compact:
  kind: alias
  expands_to: "/compact $ARGS"
quick-title:
  kind: macro
  lines:
    - "/config-set models.overrides.title.name gpt-4o-mini"
    - "/run title $ARGS"
``` 

## Example Hooks (workspace)

**.codex/hooks/\*.yaml**

```yaml
- name: audit-log
  enabled: true
  when: [post_exec, task_end]
  actions:
    - action: exec
      cmd: bash
      args: ["-lc", "echo \"$(date -Is) $CMD\" >> .codex/audit.log"]
```

```yaml
- name: summarize-task
  enabled: true
  when: [task_end]
  deny_on_fail: false
  actions:
    - action: prompt
      model_profile: heavy
      instruction: |
        Generate a one-line status that explains what the task achieved and any blockers.
``` 