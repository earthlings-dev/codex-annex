// annex/src/hooks.rs — TOML rules engine with plugin handlers

use anyhow::{anyhow, Context, Result};
use async_trait::async_trait;
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, fs, path::{Path, PathBuf}, sync::Arc};
use tokio::process::Command;

use crate::layered_config::{ConfigManager, ModelRole};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct HookContext { pub cwd: PathBuf, pub session_id: String, pub env: BTreeMap<String,String> }

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag="type", rename_all="snake_case")]
pub enum HookEvent {
    PreToolUse { tool: String, args: serde_json::Value },
    PostToolUse { tool: String, result: serde_json::Value },
    PreExec { cmd: String, argv: Vec<String> },
    PostExec { cmd: String, argv: Vec<String>, status: i32, stdout_len: usize, stderr_len: usize },
    PreMcp { server: String, method: String, payload: serde_json::Value },
    PostMcp { server: String, method: String, payload: serde_json::Value },
    TaskStart { task_name: String },
    TaskProgress { task_name: String, status_line: String },
    TaskEnd { task_name: String, success: bool },
    Git { kind: GitEvent },
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum GitEvent { PreCommit, PostCommit, PrePush, PostPush }

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum HookDecision { Continue, Deny { reason: String } }

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag="kind", rename_all="snake_case")]
pub enum HookAction {
    Exec { cmd: String, args: Vec<String> },
    Prompt { model_profile: Option<String>, instruction: String, max_tokens: Option<u32> },
    Plugin { handler: String, #[serde(default)] config: serde_json::Value },
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct HookRule {
    pub name: String,
    pub when: Vec<String>,   // e.g., ["pre_exec","post_exec","task_end"]
    pub actions: Vec<HookAction>,
    #[serde(default)]
    pub deny_on_fail: bool,
    #[serde(default="default_true")]
    pub enabled: bool,
}
fn default_true() -> bool { true }

#[derive(Default)]
pub struct HookRegistry {
    rules: Vec<HookRule>,
    depth: Arc<Mutex<usize>>,
    recursion_limit: usize,
    cfg: Arc<ConfigManager>,
    plugins: BTreeMap<String, Arc<dyn HookActionHandler>>, // by handler name
}

impl HookRegistry {
    pub fn load_from_dirs(cfg: Arc<ConfigManager>, dirs: &[PathBuf]) -> Result<Self> {
        let mut rules = vec![];
        for d in dirs {
            if !d.exists() { continue; }
            for e in fs::read_dir(d)? {
                let p = e?.path();
                if p.extension().is_some_and(|x| x=="toml") {
                    let text = fs::read_to_string(&p)?;
                    // Accept either [[rule]] or a top-level array of rules
                    let parsed: Result<HookRulesFile> = toml::from_str(&text).context("parse hooks toml");
                    match parsed {
                        Ok(mut f) => {
                            if let Some(mut v) = f.rule.take() { rules.append(&mut v); }
                            if let Some(mut v) = f.rules.take() { rules.append(&mut v); }
                        }
                        Err(_) => {
                            // Try Vec<HookRule>
                            if let Ok(mut v) = toml::from_str::<Vec<HookRule>>(&text) { rules.append(&mut v); }
                            else { return Err(anyhow!("invalid hook file: {}", p.display())); }
                        }
                    }
                }
            }
        }
        let recursion_limit = cfg.get().hooks.recursion_limit.unwrap_or(3) as usize;
        // Register built-in plugin(s)
        let mut me = Self { rules, depth: Arc::new(Mutex::new(0)), recursion_limit, cfg, plugins: BTreeMap::new() };
        me.register_plugin("audit_log", Arc::new(AuditLogPlugin));
        Ok(me)
    }

    pub fn register_plugin(&mut self, name: &str, handler: Arc<dyn HookActionHandler>) { self.plugins.insert(name.into(), handler); }

    pub async fn emit(&self, ctx: &HookContext, event: &HookEvent) -> Result<HookDecision> {
        {
            let mut d = self.depth.lock();
            if *d >= self.recursion_limit { return Ok(HookDecision::Continue); }
            *d += 1;
        }
        let res = self.emit_inner(ctx, event).await;
        *self.depth.lock() -= 1;
        res
    }

    async fn emit_inner(&self, ctx: &HookContext, event: &HookEvent) -> Result<HookDecision> {
        for r in &self.rules {
            if !r.enabled { continue; }
            if !rule_matches(r, event) { continue; }
            for a in &r.actions {
                let res: Result<()> = match a {
                    HookAction::Exec { cmd, args } => {
                        let status = Command::new(cmd).args(args).current_dir(&ctx.cwd).status().await?;
                        if !status.success() { anyhow::bail!("exec failed: {}", r.name); }
                        Ok(())
                    }
                    HookAction::Prompt { model_profile, instruction: _instruction, max_tokens: _ } => {
                        // Select model only; actual send is host-responsibility
                        let _model = if let Some(p) = model_profile {
                            self.cfg.get().models.profiles.get(p).cloned()
                                .unwrap_or(self.cfg.pick_model(ModelRole::Chat))
                        } else { self.cfg.pick_model(ModelRole::Chat) };
                        Ok(())
                    }
                    HookAction::Plugin { handler, config } => {
                        let h = self.plugins.get(handler).ok_or_else(|| anyhow!("unknown plugin handler: {}", handler))?;
                        h.run(ctx, event, config).await
                    }
                };
                if let Err(e) = res {
                    if r.deny_on_fail { return Ok(HookDecision::Deny { reason: e.to_string() }); }
                }
            }
        }
        Ok(HookDecision::Continue)
    }
}

#[derive(Default, Deserialize)]
struct HookRulesFile { rule: Option<Vec<HookRule>>, rules: Option<Vec<HookRule>> }

fn rule_matches(rule: &HookRule, ev: &HookEvent) -> bool {
    let ty = match ev {
        HookEvent::PreToolUse{..} => "pre_tool_use",
        HookEvent::PostToolUse{..} => "post_tool_use",
        HookEvent::PreExec{..} => "pre_exec",
        HookEvent::PostExec{..} => "post_exec",
        HookEvent::PreMcp{..} => "pre_mcp",
        HookEvent::PostMcp{..} => "post_mcp",
        HookEvent::TaskStart{..} => "task_start",
        HookEvent::TaskProgress{..} => "task_progress",
        HookEvent::TaskEnd{..} => "task_end",
        HookEvent::Git{..} => "git",
    };
    rule.when.iter().any(|w| w == ty)
}

#[async_trait]
pub trait HookActionHandler: Send + Sync {
    async fn run(&self, ctx: &HookContext, ev: &HookEvent, config: &serde_json::Value) -> Result<()>;
}

/// Built-in plugin: append a compact event line to .codex/audit.log
struct AuditLogPlugin;
#[async_trait]
impl HookActionHandler for AuditLogPlugin {
    async fn run(&self, ctx: &HookContext, ev: &HookEvent, _config: &serde_json::Value) -> Result<()> {
        use std::io::Write; use std::fs::{self, OpenOptions};
        let log = ctx.cwd.join(".codex").join("audit.log");
        if let Some(dir) = log.parent() { fs::create_dir_all(dir)?; }
        let mut f = OpenOptions::new().create(true).append(true).open(&log)?;
        writeln!(f, "{} {:?}", chrono::Utc::now().to_rfc3339(), ev)?;
        Ok(())
    }
}
