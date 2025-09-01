use anyhow::{Context, Result};
use futures::{future::join_all};
use serde::{Deserialize, Serialize};
use std::{sync::Arc};
use tokio::sync::{mpsc, oneshot};

use crate::{
  yaml_config::{ConfigManager, ModelRole},
  hooks_yaml::{HookRegistry, HookContext, HookEvent},
};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag="type", rename_all="snake_case")]
pub enum TaskStep {
    Chat { prompt: String, model_profile: Option<String> },
    Exec { cmd: String, args: Vec<String> },
    McpCall { server: String, method: String, payload: serde_json::Value },
    Git { action: String, args: Vec<String> },
}

/// One task inside a set
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TaskSpec {
    pub id: String,
    pub name: String,
    pub model_profile: Option<String>,  // shown in UI; overrides per-step if present
    pub steps: Vec<TaskStep>,
}

/// A set of tasks; may run parallel or seq. Only after the set completes we notify main model.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TaskSetSpec {
    pub set_id: String,
    pub title: String,
    pub mode: String,  // "sequential" | "parallel"
    pub tasks: Vec<TaskSpec>,
}

/// Execution plan: 1..N sets; we confirm between sets and can refine next set.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TaskSetPlan {
    pub session_id: String,
    pub sets: Vec<TaskSetSpec>,
}

#[derive(Clone, Debug)]
pub enum TaskStatus {
    Pending,
    Running { status_line: String },
    Done { ok: bool },
}

#[derive(Clone, Debug)]
pub enum UiEvent {
    TaskSetStart { set_id: String, title: String },
    TaskStart { set_id: String, task_id: String, model_label: String },
    TaskProgress { set_id: String, task_id: String, line: String },
    TaskEnd { set_id: String, task_id: String, ok: bool },
    TaskSetEnd { set_id: String, ok: bool },
}

pub struct TaskSetRunner<'a> {
    pub cfg: Arc<ConfigManager>,
    pub hooks: Arc<HookRegistry>,
    pub ctx: HookContext,
    pub plan: &'a TaskSetPlan,

    pub ui_tx: mpsc::UnboundedSender<UiEvent>,

    // bridges into your runtime (supply at call-site):
    pub do_chat: Arc<dyn Fn(&str, &str, &str) -> TaskFut<()> + Send + Sync>, // (model_name, base_url, prompt)
    pub do_exec: Arc<dyn Fn(&str, &[String]) -> TaskFut<(i32, String)> + Send + Sync>,
    pub do_mcp:  Arc<dyn Fn(&str,&str,&serde_json::Value) -> TaskFut<serde_json::Value> + Send + Sync>,
}
type TaskFut<T> = std::pin::Pin<Box<dyn std::future::Future<Output=anyhow::Result<T>> + Send>>;

impl<'a> TaskSetRunner<'a> {
    pub async fn run(&self) -> Result<()> {
        for (i, set) in self.plan.sets.iter().enumerate() {
            let _ = self.ui_tx.send(UiEvent::TaskSetStart { set_id: set.set_id.clone(), title: set.title.clone() });
            let ok = match set.mode.as_str() {
                "parallel" => self.run_parallel(set).await?,
                _ => self.run_sequential(set).await?,
            };
            let _ = self.ui_tx.send(UiEvent::TaskSetEnd { set_id: set.set_id.clone(), ok });

            // After a set completes, **notify main model** (summarize outcomes), then confirm before next set.
            let main = self.cfg.pick_model(ModelRole::TaskStatus);
            let summary_prompt = format!("Task set '{}' finished. Summarize status of each task and propose refinements for the next set.", set.title);
            (self.do_chat)(&main.name, main.base_url.as_deref().unwrap_or_default(), &summary_prompt).await?;

            if i + 1 < self.plan.sets.len() {
                // Ask user (through your TUI) to confirm/refine next set. You can block here with an oneshot.
                // For simplicity, we simulate a continue; wire to your actual UI confirmation flow.
            }
        }
        Ok(())
    }

    async fn run_sequential(&self, set: &TaskSetSpec) -> Result<bool> {
        let mut all_ok = true;
        for t in &set.tasks {
            let ok = self.run_one(set, t).await?;
            all_ok &= ok;
        }
        Ok(all_ok)
    }

    async fn run_parallel(&self, set: &TaskSetSpec) -> Result<bool> {
        let mut futs = vec![];
        for t in &set.tasks {
            futs.push(self.run_one(set, t));
        }
        let results = join_all(futs).await;
        Ok(results.into_iter().all(|r| r.unwrap_or(false)))
    }

    async fn run_one(&self, set: &TaskSetSpec, t: &TaskSpec) -> Result<bool> {
        // choose label/model
        let model = if let Some(p) = t.model_profile.as_deref() {
            self.cfg.get().models.profiles.get(p).cloned().unwrap_or(self.cfg.pick_model(ModelRole::Chat))
        } else { self.cfg.pick_model(ModelRole::Chat) };
        let label = t.model_profile.clone().unwrap_or_else(|| "default".into());
        let _ = self.ui_tx.send(UiEvent::TaskStart { set_id: set.set_id.clone(), task_id: t.id.clone(), model_label: label.clone() });
        self.hooks.emit(&self.ctx, &HookEvent::TaskStart { task_name: t.name.clone() }).await.ok();

        let mut ok = true;
        for step in &t.steps {
            match step {
                TaskStep::Chat { prompt, model_profile } => {
                    let chosen = if let Some(p) = model_profile {
                        self.cfg.get().models.profiles.get(p).cloned().unwrap_or(model.clone())
                    } else { model.clone() };
                    (self.do_chat)(&chosen.name, chosen.base_url.as_deref().unwrap_or_default(), prompt).await?;
                    let _ = self.ui_tx.send(UiEvent::TaskProgress { set_id: set.set_id.clone(), task_id: t.id.clone(), line: "chat sent".into() });
                }
                TaskStep::Exec { cmd, args } => {
                    let (status, out_preview) = (self.do_exec)(cmd, args).await?;
                    let _ = self.ui_tx.send(UiEvent::TaskProgress { set_id: set.set_id.clone(), task_id: t.id.clone(), line: format!("exec {} -> {}", cmd, status) });
                    self.hooks.emit(&self.ctx, &HookEvent::PostExec{ cmd: cmd.clone(), argv: args.clone(), status }).await.ok();
                    if status != 0 { ok = false; }
                }
                TaskStep::McpCall { server, method, payload } => {
                    let _resp = (self.do_mcp)(server, method, payload).await?;
                    let _ = self.ui_tx.send(UiEvent::TaskProgress { set_id: set.set_id.clone(), task_id: t.id.clone(), line: format!("mcp {}.{}", server, method) });
                }
                TaskStep::Git { action: _a, args } => {
                    let (status, _) = (self.do_exec)("git", args).await?;
                    if status != 0 { ok = false; }
                }
            }
        }

        self.hooks.emit(&self.ctx, &HookEvent::TaskEnd { task_name: t.name.clone(), success: ok }).await.ok();
        let _ = self.ui_tx.send(UiEvent::TaskEnd { set_id: set.set_id.clone(), task_id: t.id.clone(), ok });
        Ok(ok)
    }
}