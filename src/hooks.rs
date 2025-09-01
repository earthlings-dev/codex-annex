// annex/src/hooks.rs
// this is an in-progress file that needs to be merged & needed portions that are gaps converted to the yaml implementation, & unneeded portions (from the non-yaml implementation) removed

// annex/src/hooks_yaml.rs content below

use anyhow::{anyhow, Context, Result};
use async_trait::async_trait;
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use std::{collections::{BTreeMap, HashMap}, fs, path::{Path, PathBuf}, sync::Arc};
use tokio::process::Command;

use crate::yaml_config::{ConfigManager, ModelRole};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct HookContext { pub cwd: PathBuf, pub session_id: String, pub env: BTreeMap<String,String> }

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag="type", rename_all="snake_case")]
pub enum HookEvent {
    PreToolUse { tool: String, args: serde_json::Value },
    PostToolUse { tool: String, result: serde_json::Value },
    PreExec { cmd: String, argv: Vec<String> },
    PostExec { cmd: String, argv: Vec<String>, status: i32 },
    PreMcp { server: String, method: String, payload: serde_json::Value },
    PostMcp { server: String, method: String, payload: serde_json::Value },
    TaskStart { task_name: String },
    TaskProgress { task_name: String, status_line: String },
    TaskEnd { task_name: String, success: bool },
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum HookDecision { Continue, Deny { reason: String } }

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag="action", rename_all="snake_case")]
pub enum HookAction {
    Exec { cmd: String, args: Vec<String> },
    Prompt { model_profile: Option<String>, instruction: String, max_tokens: Option<u32> },
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct HookRule {
    pub name: String,
    pub when: Vec<String>,   // e.g., ["pre_exec","post_exec","task_end"]
    pub actions: Vec<HookAction>,
    pub deny_on_fail: bool,
    pub enabled: bool,
}

#[derive(Default)]
pub struct HookRegistry {
    rules: Vec<HookRule>,
    depth: Arc<Mutex<usize>>,
    recursion_limit: usize,
    cfg: Arc<ConfigManager>,
}

impl HookRegistry {
    pub fn load_from_dirs(cfg: Arc<ConfigManager>, dirs: &[PathBuf]) -> Result<Self> {
        let mut rules = vec![];
        for d in dirs {
            if !d.exists() { continue; }
            for e in fs::read_dir(d)? {
                let p = e?.path();
                if p.extension().is_some_and(|x| x=="yaml"||x=="yml") {
                    let text = fs::read_to_string(&p)?;
                    let mut v: Vec<HookRule> = serde_yml::from_str(&text)
                        .with_context(|| format!("parse hook yaml: {}", p.display()))?;
                    rules.append(&mut v);
                }
            }
        }
        let recursion_limit = cfg.get().hooks.recursion_limit.unwrap_or(3) as usize;
        Ok(Self { rules, depth: Arc::new(Mutex::new(0)), recursion_limit, cfg })
    }

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
        let mut last = HookDecision::Continue;
        for r in &self.rules {
            if !r.enabled { continue; }
            if !rule_matches(r, event) { continue; }
            for a in &r.actions {
                match a {
                    HookAction::Exec { cmd, args } => {
                        let status = Command::new(cmd).args(args).current_dir(&ctx.cwd).status().await?;
                        if !status.success() && r.deny_on_fail {
                            return Ok(HookDecision::Deny { reason: format!("hook {} exec failed", r.name) });
                        }
                    }
                    HookAction::Prompt { model_profile, instruction, max_tokens: _ } => {
                        // delegate to your existing chat call; here we only pick the model
                        let model = if let Some(p) = model_profile {
                            self.cfg.get().models.profiles.get(p).cloned()
                                .unwrap_or(self.cfg.pick_model(ModelRole::Chat))
                        } else {
                            self.cfg.pick_model(ModelRole::Chat)
                        };
                        // You will call your chat layer with (model, system=instruction, user="")
                        // This is a placeholder to show selection:
                        let _mt = model; let _instr = instruction;
                        // e.g., chat(model, system_prompt=instruction, user_prompt="")
                    }
                }
            }
            last = HookDecision::Continue;
        }
        Ok(last)
    }
}

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
    };
    rule.when.iter().any(|w| w == ty)
}

// **above to be merged with below**

// annex/src/hooks.rs content below

use anyhow::Result;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, path::PathBuf, sync::Arc};
use tokio::sync::RwLock;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct HookContext {
    pub cwd: PathBuf,
    pub env: BTreeMap<String, String>,
    pub session_id: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum HookEvent {
    PreToolUse { tool: String, args: serde_json::Value },
    PostToolUse { tool: String, result: serde_json::Value },
    PreExec { cmd: String, argv: Vec<String> },
    PostExec { cmd: String, argv: Vec<String>, status: i32, stdout_len: usize, stderr_len: usize },
    PreMcp { server: String, method: String, payload: serde_json::Value },
    PostMcp { server: String, method: String, payload: serde_json::Value },
    TaskStart { task_name: String },
    TaskEnd { task_name: String, success: bool },
    Git { kind: GitEvent },
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum GitEvent { PreCommit, PostCommit, PrePush, PostPush }

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum HookDecision {
    Continue,
    Deny { reason: String },
    ModifyEnv { set: BTreeMap<String, String>, remove: Vec<String> },
}

#[async_trait]
pub trait Hook: Send + Sync {
    fn name(&self) -> &'static str;
    async fn on_event(&self, ctx: &HookContext, event: &HookEvent) -> Result<HookDecision>;
}

#[derive(Default)]
pub struct HookRegistry {
    hooks: RwLock<Vec<Arc<dyn Hook>>>,
}
impl HookRegistry {
    pub fn new() -> Self { Self { hooks: RwLock::new(Vec::new()) } }
    pub async fn register(&self, hook: Arc<dyn Hook>) { self.hooks.write().await.push(hook); }

    pub async fn emit(&self, ctx: &HookContext, event: &HookEvent) -> Result<HookDecision> {
        let hooks = self.hooks.read().await.clone();
        let mut merged_env_sets = BTreeMap::<String,String>::new();
        let mut merged_env_removes = vec![];
        for h in hooks {
            match h.on_event(ctx, event).await? {
                HookDecision::Continue => {}
                HookDecision::Deny { reason } => return Ok(HookDecision::Deny { reason }),
                HookDecision::ModifyEnv { set, remove } => {
                    merged_env_sets.extend(set);
                    merged_env_removes.extend(remove);
                }
            }
        }
        if merged_env_sets.is_empty() && merged_env_removes.is_empty() {
            Ok(HookDecision::Continue)
        } else {
            Ok(HookDecision::ModifyEnv { set: merged_env_sets, remove: merged_env_removes })
        }
    }
}

/// Minimal example: append exec transcripts to `.codex/audit.log`
/// Used by the compactor’s "recent file detection" heuristic.
pub struct AuditLogHook;

#[async_trait]
impl Hook for AuditLogHook {
    fn name(&self) -> &'static str { "audit_log" }
    async fn on_event(&self, ctx: &HookContext, event: &HookEvent) -> Result<HookDecision> {
        use std::fs::{self, OpenOptions};
        use std::io::Write;
        let log_dir = ctx.cwd.join(".codex");
        fs::create_dir_all(&log_dir)?;
        let mut f = OpenOptions::new().create(true).append(true).open(log_dir.join("audit.log"))?;
        writeln!(f, "[{}] {:?}", chrono::Utc::now().to_rfc3339(), event)?;
        Ok(HookDecision::Continue)
    }
}