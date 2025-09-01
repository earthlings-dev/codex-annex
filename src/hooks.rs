// annex/src/hooks.rs

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