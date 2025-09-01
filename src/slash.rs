// annex/src/slash.rs

use anyhow::{anyhow, Result};
use std::{path::PathBuf, sync::Arc};

use crate::{
    layered_config::{Config, ConfigManager, Scope},
    mcp_runtime::McpRuntime,
    todo::{TodoStore, TodoStatus},
    compact::{Compactor, AutoCompactStage},
};

#[derive(Clone)]
pub struct SlashRegistry {
    cmds: Vec<Arc<dyn SlashCommand>>,
}
impl SlashRegistry {
    pub fn new() -> Self { Self { cmds: vec![] } }
    pub fn register(&mut self, cmd: Arc<dyn SlashCommand>) { self.cmds.push(cmd); }
    pub async fn dispatch(&self, input: &str) -> Result<String> {
        let input = input.trim();
        if !input.starts_with('/') { return Err(anyhow!("not a slash command")); }
        let parts: Vec<&str> = input[1..].split_whitespace().collect();
        if parts.is_empty() { return Err(anyhow!("empty command")); }
        let name = parts[0];
        for c in &self.cmds { if c.name() == name { return c.run(parts[1..].join(" ")).await; } }
        Err(anyhow!("unknown command: {}", name))
    }
}

#[async_trait::async_trait]
pub trait SlashCommand: Send + Sync {
    fn name(&self) -> &'static str;
    async fn run(&self, args: String) -> Result<String>;
}

/*** Existing minimal commands from earlier snippet (not re-listed for brevity) ***/
pub struct AllowCommand { pub cfg: Arc<ConfigManager> }
#[async_trait::async_trait]
impl SlashCommand for AllowCommand {
    fn name(&self) -> &'static str { "allow" }
    async fn run(&self, args: String) -> Result<String> {
        let mut patch = Config::default();
        patch.shell.allowlist_roots = vec![args.trim().to_string()];
        self.cfg.write_patch(Scope::Workspace, &patch)?;
        Ok(format!("added to allowlist (workspace): {}", args.trim()))
    }
}
pub struct McpAddCommand { pub cfg: Arc<ConfigManager> }
#[async_trait::async_trait]
impl SlashCommand for McpAddCommand {
    fn name(&self) -> &'static str { "mcp-add" }
    async fn run(&self, args: String) -> Result<String> {
        let mut patch = Config::default();
        // Simple parser: JSON object {"name":"X","stdio":{"cmd": "...","args":["..."]}} or {"name":"X","tcp":{"host":"...","port":1234}}
        let v: serde_json::Value = serde_json::from_str(&args)?;
        let name = v.get("name").and_then(|x| x.as_str()).ok_or_else(|| anyhow!("missing name"))?;
        let mut m = crate::layered_config::McpServer::default();
        m.enabled = true;
        if let Some(stdio) = v.get("stdio") {
            m.transport = "stdio".into();
            m.command = stdio.get("cmd").and_then(|x| x.as_str()).map(|s| s.into());
            if let Some(a) = stdio.get("args").and_then(|x| x.as_array()) {
                m.args = a.iter().filter_map(|x| x.as_str().map(|s| s.to_string())).collect();
            }
        } else if let Some(tcp) = v.get("tcp") {
            m.transport = "tcp".into();
            m.host = tcp.get("host").and_then(|x| x.as_str()).map(|s| s.into());
            m.port = tcp.get("port").and_then(|x| x.as_u64()).map(|n| n as u16);
        } else {
            return Err(anyhow!("expect stdio or tcp"));
        }
        patch.mcp.servers.insert(name.into(), m);
        self.cfg.write_patch(Scope::Workspace, &patch)?;
        Ok("MCP server added (workspace)".into())
    }
}
pub struct ConfigSetCommand { pub cfg: Arc<ConfigManager> }
#[async_trait::async_trait]
impl SlashCommand for ConfigSetCommand {
    fn name(&self) -> &'static str { "config-set" }
    async fn run(&self, args: String) -> Result<String> {
        let parts: Vec<&str> = args.split_whitespace().collect();
        if parts.len() < 2 { return Err(anyhow!("usage: /config-set <path> <value>")); }
        let path = parts[0]; let value = parts[1..].join(" ");
        let mut patch = Config::default();
        match path {
            "model.name" => patch.model.name = Some(value),
            "history.persist" => patch.history.persist = Some(value),
            "sandbox.mode" => patch.sandbox.mode = Some(value),
            "sandbox.network_access" => patch.sandbox.network_access = Some(value.parse::<bool>()?),
            _ => return Err(anyhow!("unsupported path: {}", path)),
        }
        self.cfg.apply_runtime_overlay(patch)?;
        Ok("runtime overlay applied".into())
    }
}

/*** NEW: TODO commands ***/
pub struct TodoCommand { pub cfg: Arc<ConfigManager>, pub workspace: PathBuf }
#[async_trait::async_trait]
impl SlashCommand for TodoCommand {
    fn name(&self) -> &'static str { "todo" }
    async fn run(&self, args: String) -> Result<String> {
        let cfg = self.cfg.get();
        let path = cfg.todo.path.clone().unwrap_or(self.workspace.join(".codex").join("todo.json"));
        let mut store = TodoStore::load(&path)?;
        let parts: Vec<&str> = args.split_whitespace().collect();
        match parts.get(0).map(|s| *s).unwrap_or("") {
            "add" => {
                // /todo add {"title":"…","description":"…","files":["path1","path2"],"tags":["x"]}
                let v: serde_json::Value = serde_json::from_str(parts[1..].join(" ").trim())?;
                let title = v.get("title").and_then(|x| x.as_str()).ok_or_else(|| anyhow!("title required"))?;
                let desc = v.get("description").and_then(|x| x.as_str()).map(|s| s.to_string());
                let files: Vec<PathBuf> = v.get("files").and_then(|x| x.as_array()).unwrap_or(&vec![])
                    .iter().filter_map(|x| x.as_str().map(|s| self.workspace.join(s))).collect();
                let tags: Vec<String> = v.get("tags").and_then(|x| x.as_array()).unwrap_or(&vec![])
                    .iter().filter_map(|x| x.as_str().map(|s| s.to_string())).collect();
                let it = store.add(title.to_string(), desc, files, tags);
                store.save(&path)?;
                Ok(format!("todo added: {} ({})", it.title, it.id))
            }
            "list" => {
                let mut s = String::new();
                for it in &store.items {
                    s.push_str(&format!("- [{}] {} ({}) {:?}\n", match it.status { TodoStatus::Open=>" ", TodoStatus::InProgress=>">", TodoStatus::Done=>"x" }, it.title, it.id, it.files));
                }
                Ok(s)
            }
            "done" => {
                let id = parts.get(1).ok_or_else(|| anyhow!("usage: /todo done <id>"))?;
                store.set_status(id, TodoStatus::Done)?;
                store.save(&path)?;
                Ok(format!("todo {} marked done", id))
            }
            "rm" => {
                let id = parts.get(1).ok_or_else(|| anyhow!("usage: /todo rm <id>"))?;
                store.remove(id)?;
                store.save(&path)?;
                Ok(format!("todo {} removed", id))
            }
            _ => Err(anyhow!("usage: /todo [add|list|done|rm] …")),
        }
    }
}

/*** NEW: compact + autocompact ***/
pub struct CompactCommand { pub cfg: Arc<ConfigManager>, pub workspace: PathBuf }
#[async_trait::async_trait]
impl SlashCommand for CompactCommand {
    fn name(&self) -> &'static str { "compact" }
    async fn run(&self, args: String) -> Result<String> {
        // JSON input: {"focus":"…","include":["glob1","glob2"],"conversation_tail":"…"}
        let v: serde_json::Value = serde_json::from_str(args.trim())?;
        let focus = v.get("focus").and_then(|x| x.as_str()).map(|s| s.to_string());
        let includes: Vec<String> = v.get("include").and_then(|x| x.as_array()).unwrap_or(&vec![])
            .iter().filter_map(|x| x.as_str().map(|s| s.to_string())).collect();
        let tail = v.get("conversation_tail").and_then(|x| x.as_str()).unwrap_or("");
        let comp = crate::compact::Compactor::new(self.cfg.clone(), self.workspace.clone());
        let res = comp.manual_compact(focus, includes, tail)?;
        Ok(serde_json::to_string_pretty(&res)?)
    }
}

pub struct AutoCompactToggle { pub cfg: Arc<ConfigManager> }
#[async_trait::async_trait]
impl SlashCommand for AutoCompactToggle {
    fn name(&self) -> &'static str { "autocompact" }
    async fn run(&self, args: String) -> Result<String> {
        let mut patch = Config::default();
        match args.trim() {
            "on" => { patch.compact.auto_enable = true; }
            "off" => { patch.compact.auto_enable = false; }
            _ => return Err(anyhow!("usage: /autocompact on|off")),
        }
        self.cfg.apply_runtime_overlay(patch)?;
        Ok(format!("auto-compact {}", args.trim()))
    }
}