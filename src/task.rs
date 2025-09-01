// annex/src/task.rs

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::hooks::{HookContext, HookEvent, HookRegistry, HookDecision};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum TaskStep {
    Chat { prompt: String, agent: Option<String> },
    Exec { cmd: String, args: Vec<String> },
    McpCall { server: String, method: String, payload: serde_json::Value },
    Git { action: String, args: Vec<String> },
    /// Spawn a sub-agent (profile) for a nested set of steps; shares session id, isolated policy.
    SubAgent { agent: String, steps: Vec<TaskStep> },
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TaskSpec {
    pub name: String,
    pub steps: Vec<TaskStep>,
}

pub struct TaskRunner<'a> {
    pub spec: &'a TaskSpec,
    pub hooks: &'a HookRegistry,
    pub ctx: HookContext,
    // Bridge closures into your existing layers:
    pub do_chat: Box<dyn Fn(&str, Option<&str>) -> std::pin::Pin<Box<dyn std::future::Future<Output=Result<()>> + Send>> + Send + Sync>,
    pub do_exec: Box<dyn Fn(&str, &[String]) -> std::pin::Pin<Box<dyn std::future::Future<Output=Result<(i32, usize, usize)>> + Send>> + Send + Sync>,
    pub do_mcp:  Box<dyn Fn(&str, &str, &serde_json::Value) -> std::pin::Pin<Box<dyn std::future::Future<Output=Result<serde_json::Value>> + Send>> + Send + Sync>,
    /// Optionally switch effective agent profile for nested steps.
    pub with_agent: Box<dyn Fn(&str) -> std::pin::Pin<Box<dyn std::future::Future<Output=Result<()>> + Send>> + Send + Sync>,
}

impl<'a> TaskRunner<'a> {
    pub async fn run(&self) -> Result<()> {
        self.hooks.emit(&self.ctx, &HookEvent::TaskStart { task_name: self.spec.name.clone() }).await.ok();
        let mut ok = true;
        for step in &self.spec.steps {
            match step {
                TaskStep::Chat { prompt, agent } => {
                    let _ = self.hooks.emit(&self.ctx, &HookEvent::PreToolUse{ tool: "chat".into(), args: serde_json::json!({"agent":agent,"prompt":prompt}) }).await?;
                    (self.do_chat)(prompt, agent.as_deref()).await?;
                    let _ = self.hooks.emit(&self.ctx, &HookEvent::PostToolUse{ tool: "chat".into(), result: serde_json::json!({}) }).await?;
                }
                TaskStep::Exec { cmd, args } => {
                    if let HookDecision::Deny{reason} = self.hooks.emit(&self.ctx, &HookEvent::PreExec{ cmd: cmd.clone(), argv: args.clone() }).await? {
                        anyhow::bail!("denied by hook: {}", reason);
                    }
                    let (status, out_len, err_len) = (self.do_exec)(cmd, args).await?;
                    let _ = self.hooks.emit(&self.ctx, &HookEvent::PostExec{ cmd: cmd.clone(), argv: args.clone(), status, stdout_len: out_len, stderr_len: err_len }).await?;
                    if status != 0 { ok = false; }
                }
                TaskStep::McpCall { server, method, payload } => {
                    if let HookDecision::Deny{reason} = self.hooks.emit(&self.ctx, &HookEvent::PreMcp{ server: server.clone(), method: method.clone(), payload: payload.clone() }).await? {
                        anyhow::bail!("denied by hook: {}", reason);
                    }
                    let resp = (self.do_mcp)(server, method, payload).await?;
                    let _ = self.hooks.emit(&self.ctx, &HookEvent::PostMcp{ server: server.clone(), method: method.clone(), payload: resp }).await?;
                }
                TaskStep::Git { action, args } => {
                    let (status, _, _) = (self.do_exec)("git", args).await?;
                    if status != 0 { ok = false; }
                }
                TaskStep::SubAgent { agent, steps } => {
                    // Switch profile, run nested steps, then revert.
                    (self.with_agent)(agent).await?;
                    let nested = TaskSpec { name: format!("{}::{}", self.spec.name, agent), steps: steps.clone() };
                    let nested_runner = TaskRunner {
                        spec: &nested, hooks: self.hooks, ctx: self.ctx.clone(),
                        do_chat: self.do_chat.clone(), do_exec: self.do_exec.clone(), do_mcp: self.do_mcp.clone(),
                        with_agent: self.with_agent.clone(),
                    };
                    nested_runner.run().await?;
                }
            }
        }
        self.hooks.emit(&self.ctx, &HookEvent::TaskEnd { task_name: self.spec.name.clone(), success: ok }).await.ok();
        if ok { Ok(()) } else { Err(anyhow::anyhow!("one or more steps failed")) }
    }
}