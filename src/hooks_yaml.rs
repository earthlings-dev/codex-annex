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