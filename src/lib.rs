// annex/src/lib.rs

pub mod yaml_config;       // multi-scope YAML config + model routing
pub mod session_log;       // YAML session logs (+ purge and resume)
pub mod hooks_yaml;        // YAML-defined hooks (exec & prompt) + recursion limit
pub mod slash_yaml;        // YAML-defined slash commands/macros (builds SlashRegistry)
pub mod taskset;           // Task Sets: parallel/seq, live status, per-task model
pub mod todo_yaml;         // TODO store in YAML with session/date/task-number
pub mod compact;           // (from earlier) manual/auto compaction (unchanged API)
pub mod acp_server;        // ACP server skeleton bridging to codex task/todo/hooks

// re-exports
pub use yaml_config::{ConfigManager, Config, Scope, ModelRole, ModelTarget};
pub use session_log::{SessionLogWriter, SessionEvent};
pub use hooks_yaml::{HookRegistry, HookDecision, HookEvent, HookContext};
pub use slash_yaml::{SlashRegistry};
pub use taskset::{TaskSetRunner, TaskSpec, TaskStep, TaskSetSpec, TaskSetPlan, TaskStatus};
pub use todo_yaml::{TodoStore, TodoItem, TodoStatus};
pub use compact::{Compactor, AutoCompactStage};