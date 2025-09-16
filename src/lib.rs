// annex/src/lib.rs

pub mod layered_config;     // layered TOML config + model routing
pub mod session_logs;       // JSON / JSONL session logs (+ purge and resume)
pub mod hooks;              // TOML-defined hooks (exec/prompt/plugin) + recursion limit
pub mod slash;              // TOML-defined slash commands/macros/builtins
pub mod taskset;            // Task Sets: parallel/seq, live status, per-task model
pub mod todo;               // TODO store in JSON
pub mod compact;            // manual/auto compaction
#[cfg(feature = "acp")]
pub mod acp_server;         // ACP server skeleton bridging to codex task/todo/hooks

// re-exports
pub use layered_config::{ConfigManager, Config, Scope, ModelRole, ModelTarget};
pub use session_logs::{SessionLogWriter, SessionEvent};
pub use hooks::{HookRegistry, HookDecision, HookEvent, HookContext};
pub use slash::SlashRegistry;
pub use taskset::{TaskSetRunner, TaskSpec, TaskStep, TaskSetSpec, TaskSetPlan, TaskStatus};
pub use todo::{TodoStore, TodoItem, TodoStatus};
pub use compact::{Compactor, AutoCompactStage};
