// annex/src/slash.rs — directory TOML files with alias/macro/builtins

use anyhow::{anyhow, Result};
use serde::Deserialize;
use std::{collections::BTreeMap, fs, path::PathBuf, sync::Arc};

use crate::{
    layered_config::{Config, ConfigManager, Scope},
    compact::Compactor,
    todo::{TodoStore, TodoStatus},
};

#[derive(Clone)]
pub struct SlashRegistry {
    aliases: BTreeMap<String, String>,
    macros: BTreeMap<String, Vec<String>>,
    builtins: BTreeMap<String, BTreeMap<String, String>>, // name -> args
    cfg: Arc<ConfigManager>,
    workspace_root: PathBuf,
}

#[derive(Default, Deserialize)]
struct SlashTomlFile {
    #[serde(default)]
    alias: BTreeMap<String, String>,
    #[serde(default, rename = "macro")]
    macros: Vec<SlashMacro>,
    #[serde(default)]
    builtin: BTreeMap<String, BTreeMap<String, String>>,
}

#[derive(Clone, Debug, Deserialize)]
struct SlashMacro { name: String, lines: Vec<String> }

impl SlashRegistry {
    pub fn load_from_dirs_with_workspace(cfg: Arc<ConfigManager>, workspace_root: PathBuf, dirs: &[PathBuf]) -> Result<Self> {
        let mut aliases = BTreeMap::new();
        let mut macros: BTreeMap<String, Vec<String>> = BTreeMap::new();
        let mut builtins: BTreeMap<String, BTreeMap<String, String>> = BTreeMap::new();
        for d in dirs {
            if !d.exists() { continue; }
            for e in fs::read_dir(d)? {
                let p = e?.path();
                if p.extension().is_some_and(|x| x=="toml") {
                    let text = fs::read_to_string(&p)?;
                    let f: SlashTomlFile = toml::from_str(&text)?;
                    aliases.extend(f.alias);
                    for m in f.macros { macros.insert(m.name, m.lines); }
                    for (k, v) in f.builtin { builtins.insert(k, v); }
                }
            }
        }
        Ok(Self { aliases, macros, builtins, cfg, workspace_root })
    }

    // Backwards-compatible helper: default workspace is current dir
    pub fn load_from_dirs(cfg: Arc<ConfigManager>, dirs: &[PathBuf]) -> Result<Self> {
        let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        Self::load_from_dirs_with_workspace(cfg, cwd, dirs)
    }

    pub async fn dispatch(&self, input: &str) -> Result<String> {
        if !input.starts_with('/') { return Err(anyhow!("not a slash command")); }
        let (name, rest) = input[1..].split_once(' ').map(|(a,b)| (a,b)).unwrap_or((&input[1..], ""));
        if let Some(expands) = self.aliases.get(name) {
            return Ok(expands.replace("$ARGS", rest));
        }
        if let Some(lines) = self.macros.get(name) {
            return Ok(lines.join("\n"));
        }
        if let Some(args) = self.builtins.get(name) {
            return self.dispatch_builtin(name, rest.trim(), args).await;
        }
        Err(anyhow!("unknown slash: {}", name))
    }

    async fn dispatch_builtin(&self, name: &str, argstr: &str, args: &BTreeMap<String, String>) -> Result<String> {
        match name {
            "config-set" => {
                // expects: path value
                let parts: Vec<&str> = argstr.split_whitespace().collect();
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
            "allow" => {
                let root = argstr.trim();
                if root.is_empty() { return Err(anyhow!("usage: /allow <root-binary>")); }
                let mut patch = Config::default();
                patch.shell.allowlist_roots = vec![root.to_string()];
                self.cfg.write_patch(Scope::Workspace, &patch)?;
                Ok(format!("added to allowlist (workspace): {}", root))
            }
            "mcp-add" => {
                // JSON: {"name":"X","stdio":{...}} or {"name":"X","tcp":{...}}
                let v: serde_json::Value = serde_json::from_str(argstr)?;
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
                } else { return Err(anyhow!("expect stdio or tcp")); }
                let mut patch = Config::default();
                patch.mcp.servers.insert(name.into(), m);
                self.cfg.write_patch(Scope::Workspace, &patch)?;
                Ok("MCP server added (workspace)".into())
            }
            "todo" => {
                // /todo add {json} | list | done <id> | rm <id>
                let cfg = self.cfg.get();
                let path = cfg.todo.path.clone().unwrap_or(self.workspace_root.join(".codex").join("todo.json"));
                let mut store = TodoStore::load(&path)?;
                let parts: Vec<&str> = argstr.split_whitespace().collect();
                match parts.get(0).copied().unwrap_or("") {
                    "add" => {
                        let v: serde_json::Value = serde_json::from_str(parts[1..].join(" ").trim())?;
                        let title = v.get("title").and_then(|x| x.as_str()).ok_or_else(|| anyhow!("title required"))?;
                        let desc = v.get("description").and_then(|x| x.as_str()).map(|s| s.to_string());
                        let files: Vec<PathBuf> = v.get("files").and_then(|x| x.as_array()).unwrap_or(&vec![])
                            .iter().filter_map(|x| x.as_str().map(|s| self.workspace_root.join(s))).collect();
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
                        store.set_status(id, TodoStatus::Done)?; store.save(&path)?;
                        Ok(format!("todo {} marked done", id))
                    }
                    "rm" => {
                        let id = parts.get(1).ok_or_else(|| anyhow!("usage: /todo rm <id>"))?;
                        store.remove(id)?; store.save(&path)?;
                        Ok(format!("todo {} removed", id))
                    }
                    _ => Err(anyhow!("usage: /todo [add|list|done|rm] …")),
                }
            }
            "compact" => {
                // args JSON: {"focus":"…","include":["glob1"],"conversation_tail":"…"}
                let v: serde_json::Value = serde_json::from_str(argstr.trim())?;
                let focus = v.get("focus").and_then(|x| x.as_str()).map(|s| s.to_string());
                let includes: Vec<String> = v.get("include").and_then(|x| x.as_array()).unwrap_or(&vec![])
                    .iter().filter_map(|x| x.as_str().map(|s| s.to_string())).collect();
                let tail = v.get("conversation_tail").and_then(|x| x.as_str()).unwrap_or("");
                let comp = Compactor::new(self.cfg.clone(), self.workspace_root.clone());
                let res = comp.manual_compact(focus, includes, tail)?;
                Ok(serde_json::to_string_pretty(&res)?)
            }
            "autocompact" => {
                let mut patch = Config::default();
                match argstr.trim() {
                    "on" => { patch.compact.auto_enable = true; }
                    "off" => { patch.compact.auto_enable = false; }
                    _ => return Err(anyhow!("usage: /autocompact on|off")),
                }
                self.cfg.apply_runtime_overlay(patch)?;
                Ok(format!("auto-compact {}", argstr.trim()))
            }
            _ => Ok(format!("builtin:{} {}", name, serde_json::to_string(args)?)),
        }
    }
}
