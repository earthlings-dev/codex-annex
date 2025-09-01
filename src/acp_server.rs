// annex/src/acp_server.rs

use anyhow::Result;
use std::sync::Arc;
use agent_client_protocol as acp; // from zed-industries repo
use crate::{layered_config::ConfigManager, taskset::TaskSetPlan, hooks::HookRegistry, todo::TodoStore};

/// Starts an ACP server on stdio so Zed (or other ACP clients) can spawn Codex-rs as an agent.
/// NOTE: ACP is evolving; pin the git rev for stability. See schema in the repo.
pub async fn run_stdio(cfg: Arc<ConfigManager>, hooks: Arc<HookRegistry>) -> Result<()> {
    // Rough outline; bind handlers required by ACP crate:
    // - initialize / shutdown
    // - capabilities (edits, prompts, MCP tools)
    // - run_task_set (custom extension)
    // - apply_edits / review diffs
    // The actual method names/types come from the ACP crate/schema; connect them to codex services.
    let _ = (cfg, hooks);
    // TODO: Wire acp::Server::new(stdin, stdout).on_* handlers to your TaskSetRunner and MCP bridge.
    Ok(())
}
