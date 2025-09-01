use anyhow::{anyhow, Result};
use regex::Regex;
use std::sync::Arc;

use crate::layered_config::{Config, ConfigManager, Scope};
use crate::mcp_runtime::McpRuntime;

#[derive(Clone)]
pub struct SlashRegistry {
    cmds: Vec<Arc<dyn SlashCommand>>,
}

impl SlashRegistry {
    pub fn new() -> Self { Self { cmds: vec![] } }
    pub fn register(&mut self, cmd: Arc<dyn SlashCommand>) { self.cmds.push(cmd); }

    /// Parse `/command args...` and run.
    pub async fn dispatch(&self, input: &str) -> Result<String> {
        let input = input.trim();
        if !input.starts_with('/') { return Err(anyhow!("not a slash command")); }
        let parts: Vec<&str> = input[1..].split_whitespace().collect();
        if parts.is_empty() { return Err(anyhow!("empty command")); }
        let name = parts[0];
        for c in &self.cmds {
            if c.name() == name { return c.run(parts[1..].join(" ")).await; }
        }
        Err(anyhow!("unknown command: {}", name))
    }
}

#[async_trait::async_trait]
pub trait SlashCommand: Send + Sync {
    fn name(&self) -> &'static str;
    async fn run(&self, args: String) -> Result<String>;
}

/*** Built‑ins ***/

// /allow <root>  -> add to shell.allowlist_roots (workspace scope by default)
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

// /mcp-add <name> stdio <cmd> [args...]
// /mcp-add <name> tcp <host> <port>
pub struct McpAddCommand { pub cfg: Arc<ConfigManager> }
#[async_trait::async_trait]
impl SlashCommand for McpAddCommand {
    fn name(&self) -> &'static str { "mcp-add" }
    async fn run(&self, args: String) -> Result<String> {
        let re_stdio = Regex::new(r#"^(\S+)\s+stdio\s+(\S+)(?:\s+(.*))?$"#)?;
        let re_tcp   = Regex::new(r#"^(\S+)\s+tcp\s+(\S+)\s+(\d+)$"#)?;
        let mut patch = Config::default();
        if let Some(caps) = re_stdio.captures(&args) {
            let (name, cmd, tail) = (&caps[1], &caps[2], caps.get(3).map(|m| m.as_str()).unwrap_or(""));
            let mut m = crate::layered_config::McpServer::default();
            m.enabled = true; m.transport = "stdio".into();
            m.command = Some(cmd.into());
            m.args = if tail.is_empty() { vec![] } else { tail.split_whitespace().map(|s| s.to_string()).collect() };
            patch.mcp.servers.insert(name.into(), m);
        } else if let Some(caps) = re_tcp.captures(&args) {
            let (name, host, port) = (&caps[1], &caps[2], caps[3].parse::<u16>()?);
            let mut m = crate::layered_config::McpServer::default();
            m.enabled = true; m.transport = "tcp".into();
            m.host = Some(host.into()); m.port = Some(port);
            patch.mcp.servers.insert(name.into(), m);
        } else {
            return Err(anyhow!("usage: /mcp-add <name> stdio <cmd> [args..] | /mcp-add <name> tcp <host> <port>"));
        }
        self.cfg.write_patch(Scope::Workspace, &patch)?;
        Ok("MCP server added (workspace)".into())
    }
}

// /config-set model.name <value>  -> runtime overlay for quick swap
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