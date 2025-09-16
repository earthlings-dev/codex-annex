// annex/src/layered_config.rs

use anyhow::{Context, Result};
use directories::ProjectDirs;
use notify::{recommended_watcher, Event, RecursiveMode, Watcher};
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, fs, path::{Path, PathBuf}, sync::Arc};
use tokio::sync::broadcast;

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct Config {
    // Legacy minimal model knob (kept for backward compat with early drafts)
    pub model: ModelConfig,
    // New routing & profiles (source of truth)
    pub models: ModelsConfig,
    pub sandbox: SandboxConfig,
    pub shell: ShellConfig,
    pub mcp: McpConfig,
    pub ui: UiConfig,
    pub history: HistoryConfig,
    pub todo: TodoConfig,
    pub compact: CompactConfig,
    pub sessions: SessionsConfig,
    pub hooks: HooksConfig,
    pub slash: SlashConfigMeta,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalMode { OnRequest, OnFailure, UnlessTrusted, Never }
impl Default for ApprovalMode { fn default() -> Self { Self::OnRequest } }

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct ModelConfig {
    pub name: Option<String>,
    pub reasoning_effort: Option<String>,
    pub reasoning_summary: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct ModelsConfig {
    /// Default chat/completion model (used unless overridden)
    pub default: ModelTarget,
    /// Map of function-role -> model target; e.g. "title", "session_name", "compact", "meta_prompt", "task_status"
    pub overrides: BTreeMap<String, ModelTarget>,
    /// Named profiles you can reference in tasks (per-task model label)
    pub profiles: BTreeMap<String, ModelTarget>,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct ModelTarget {
    pub name: String,                        // e.g. "gpt-4o-mini", "gemini-1.5-pro"
    pub base_url: Option<String>,            // e.g. "https://api.openai.com/v1"
    /// Name of env var carrying an API key (if provider uses a key)
    pub api_key_env: Option<String>,         // e.g. OPENAI_API_KEY
    /// Name of env var carrying an API token (if provider uses bearer tokens)
    pub api_token_env: Option<String>,       // e.g. ANTHROPIC_API_KEY or custom token
    pub extra_headers: BTreeMap<String, String>,
}

#[derive(Clone, Copy, Debug)]
pub enum ModelRole {
    Chat,
    Title,
    SessionName,
    Compact,
    MetaPrompt,
    TaskStatus,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct SandboxConfig {
    pub mode: Option<String>,              // "danger_full_access" | "read_only" | "workspace_write"
    pub network_access: Option<bool>,
    pub writable_roots: Vec<PathBuf>,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct ShellConfig {
    pub approval: ApprovalMode,
    pub allowlist_roots: Vec<String>,      // e.g., ["git","rg","ls","cat","cargo"]
    pub denylist_roots: Vec<String>,
    pub environment_inherit: Option<String>,  // "none" | "core" | "all"
    pub env_exclude_patterns: Vec<String>,    // ["*KEY*","*TOKEN*"]
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct UiConfig {
    pub command_palette: bool,
    pub status_bar: bool,
    pub screen_reader: bool,
    pub kitty_protocol: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct HistoryConfig {
    pub persist: Option<String>, // "none" | "session" | "all"
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct TodoConfig {
    pub path: Option<PathBuf>,   // defaults to .codex/todo.json
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct CompactConfig {
    /// Enable auto-compact background heuristic.
    pub auto_enable: bool,
    /// Minimum seconds between auto-compacts.
    pub auto_min_interval_secs: u64,
    /// Trigger auto-compact at task end.
    pub auto_on_task_end: bool,
    /// Heuristic thresholds.
    pub max_context_chars: usize,   // soft target for summary input assembly
    pub max_files: usize,           // cap included file list
    pub include_globs_default: Vec<String>, // baseline patterns for manual/auto
}
impl Default for CompactConfig {
    fn default() -> Self {
        Self {
            auto_enable: true,
            auto_min_interval_secs: 120,
            auto_on_task_end: true,
            max_context_chars: 40_000,
            max_files: 24,
            include_globs_default: vec!["**/*.rs".into(),"**/*.md".into(),"**/*.toml".into()],
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct SessionsConfig {
    pub dir: Option<PathBuf>,              // default: ~/.local/share/codex/sessions
    pub auto_purge_days: Option<u32>,
    pub resume_on_launch: bool,
    /// "json" | "jsonl" | "both" (default)
    pub write_mode: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct HooksConfig {
    pub recursion_limit: Option<u32>,
    /// Additional lookup dirs for hooks/*.toml
    pub dirs: Vec<PathBuf>,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct SlashConfigMeta {
    /// Lookup dirs for slash/*.toml
    pub dirs: Vec<PathBuf>,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct McpConfig {
    pub servers: BTreeMap<String, McpServer>,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct McpServer {
    pub enabled: bool,
    pub transport: String,                 // "stdio" | "tcp"
    pub command: Option<PathBuf>,          // for stdio
    pub args: Vec<String>,
    pub env: BTreeMap<String, String>,
    pub host: Option<String>,              // for tcp
    pub port: Option<u16>,
    pub scope: Option<String>,             // "system" | "user" | "workspace" (for UI)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Scope { System, User, Workspace, Runtime }

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct PartialConfig(pub Config);

fn merge(a: &mut Config, b: &Config) {
    let overlay = |dst: &mut Option<String>, src: &Option<String>| { if src.is_some() { *dst = src.clone(); } };
    overlay(&mut a.model.name, &b.model.name);
    overlay(&mut a.model.reasoning_effort, &b.model.reasoning_effort);
    overlay(&mut a.model.reasoning_summary, &b.model.reasoning_summary);

    // models routing
    if !b.models.default.name.is_empty() { a.models.default = b.models.default.clone(); }
    for (k, v) in &b.models.overrides { a.models.overrides.insert(k.clone(), v.clone()); }
    for (k, v) in &b.models.profiles { a.models.profiles.insert(k.clone(), v.clone()); }

    overlay(&mut a.sandbox.mode, &b.sandbox.mode);
    if let Some(v) = b.sandbox.network_access { a.sandbox.network_access = Some(v); }
    if !b.sandbox.writable_roots.is_empty() { a.sandbox.writable_roots = b.sandbox.writable_roots.clone(); }

    a.shell.approval = b.shell.approval.clone();
    if !b.shell.allowlist_roots.is_empty() { a.shell.allowlist_roots = b.shell.allowlist_roots.clone(); }
    if !b.shell.denylist_roots.is_empty() { a.shell.denylist_roots = b.shell.denylist_roots.clone(); }
    overlay(&mut a.shell.environment_inherit, &b.shell.environment_inherit);
    if !b.shell.env_exclude_patterns.is_empty() { a.shell.env_exclude_patterns = b.shell.env_exclude_patterns.clone(); }

    a.ui.command_palette |= b.ui.command_palette;
    a.ui.status_bar |= b.ui.status_bar;
    a.ui.screen_reader |= b.ui.screen_reader;
    a.ui.kitty_protocol |= b.ui.kitty_protocol;

    overlay(&mut a.history.persist, &b.history.persist);

    if b.todo.path.is_some() { a.todo.path = b.todo.path.clone(); }

    // compact
    a.compact.auto_enable |= b.compact.auto_enable;
    if b.compact.auto_min_interval_secs != 0 { a.compact.auto_min_interval_secs = b.compact.auto_min_interval_secs; }
    a.compact.auto_on_task_end |= b.compact.auto_on_task_end;
    if b.compact.max_context_chars != 0 { a.compact.max_context_chars = b.compact.max_context_chars; }
    if b.compact.max_files != 0 { a.compact.max_files = b.compact.max_files; }
    if !b.compact.include_globs_default.is_empty() { a.compact.include_globs_default = b.compact.include_globs_default.clone(); }

    // sessions
    if b.sessions.dir.is_some() { a.sessions.dir = b.sessions.dir.clone(); }
    if b.sessions.auto_purge_days.is_some() { a.sessions.auto_purge_days = b.sessions.auto_purge_days; }
    a.sessions.resume_on_launch |= b.sessions.resume_on_launch;
    overlay(&mut a.sessions.write_mode, &b.sessions.write_mode);

    // hooks
    if b.hooks.recursion_limit.is_some() { a.hooks.recursion_limit = b.hooks.recursion_limit; }
    if !b.hooks.dirs.is_empty() { a.hooks.dirs = b.hooks.dirs.clone(); }

    // slash
    if !b.slash.dirs.is_empty() { a.slash.dirs = b.slash.dirs.clone(); }

    // MCP servers
    for (k, v) in &b.mcp.servers { a.mcp.servers.insert(k.clone(), v.clone()); }
}

fn config_paths(workspace_root: &Path) -> Result<(PathBuf, PathBuf, PathBuf)> {
    let proj = ProjectDirs::from("com", "openai", "codex").context("ProjectDirs not available")?;
    let user = proj.config_dir().join("config.toml");
    let system = if cfg!(target_os = "windows") {
        PathBuf::from(r"C:\ProgramData\Codex\config.toml")
    } else {
        PathBuf::from("/etc/codex/config.toml")
    };
    let workspace = workspace_root.join(".codex").join("config.toml");
    Ok((system, user, workspace))
}

#[derive(Clone)]
pub struct ConfigManager {
    inner: Arc<RwLock<Config>>,
    tx: broadcast::Sender<Config>,
    _watcher: Arc<RwLock<Option<notify::RecommendedWatcher>>>,
    system_path: PathBuf,
    user_path: PathBuf,
    workspace_path: PathBuf,
    runtime_overlay: Arc<RwLock<Config>>,
}

impl ConfigManager {
    pub fn load(workspace_root: impl AsRef<Path>) -> Result<Self> {
        let (system_path, user_path, workspace_path) = config_paths(workspace_root.as_ref())?;
        let cm = Self {
            inner: Arc::new(RwLock::new(Config::default())),
            tx: broadcast::channel(64).0,
            _watcher: Arc::new(RwLock::new(None)),
            system_path, user_path, workspace_path,
            runtime_overlay: Arc::new(RwLock::new(Config::default())),
        };
        let me = cm;
        me.reload_all()?;
        me.start_watch()?;
        Ok(me)
    }

    fn read_file(path: &Path) -> Option<Config> {
        let text = fs::read_to_string(path).ok()?;
        let p: PartialConfig = toml::from_str(&text).ok()?;
        Some(p.0)
    }

    pub fn reload_all(&self) -> Result<()> {
        let mut merged = Config::default();
        if let Some(sys) = Self::read_file(&self.system_path) { merge(&mut merged, &sys); }
        if let Some(usr) = Self::read_file(&self.user_path) { merge(&mut merged, &usr); }
        if let Some(ws)  = Self::read_file(&self.workspace_path) { merge(&mut merged, &ws); }
        let rt = self.runtime_overlay.read().clone();
        merge(&mut merged, &rt);
        *self.inner.write() = merged.clone();
        let _ = self.tx.send(merged);
        Ok(())
    }

    fn start_watch(&self) -> Result<()> {
        let system = self.system_path.clone();
        let user = self.user_path.clone();
        let workspace = self.workspace_path.clone();
        let tx = self.tx.clone();
        let inner = self.inner.clone();
        let runtime_overlay = self.runtime_overlay.clone();

        let mut watcher = recommended_watcher(move |res: Result<Event, _>| {
            if res.is_err() { return; }
            let mut merged = Config::default();
            if let Some(sys) = ConfigManager::read_file(&system) { merge(&mut merged, &sys); }
            if let Some(usr) = ConfigManager::read_file(&user) { merge(&mut merged, &usr); }
            if let Some(ws)  = ConfigManager::read_file(&workspace) { merge(&mut merged, &ws); }
            let rt = runtime_overlay.read().clone();
            merge(&mut merged, &rt);
            *inner.write() = merged.clone();
            let _ = tx.send(merged);
        })?;
        for p in [&self.system_path, &self.user_path, &self.workspace_path] {
            if let Some(dir) = p.parent() { watcher.watch(dir, RecursiveMode::NonRecursive)?; }
        }
        *self._watcher.write() = Some(watcher);
        Ok(())
    }

    pub fn get(&self) -> Config { self.inner.read().clone() }
    pub fn subscribe(&self) -> broadcast::Receiver<Config> { self.tx.subscribe() }

    pub fn apply_runtime_overlay(&self, patch: Config) -> Result<()> {
        {
            let mut rt = self.runtime_overlay.write();
            merge(&mut *rt, &patch);
        }
        self.reload_all()
    }

    pub fn write_patch(&self, scope: Scope, patch: &Config) -> Result<()> {
        use std::io::Write;
        let path = match scope {
            Scope::System   => &self.system_path,
            Scope::User     => &self.user_path,
            Scope::Workspace=> &self.workspace_path,
            Scope::Runtime  => anyhow::bail!("Runtime scope is ephemeral; cannot persist"),
        };
        if let Some(dir) = path.parent() { fs::create_dir_all(dir)?; }
        let current = Self::read_file(path).unwrap_or_default();
        let mut merged = current.clone();
        merge(&mut merged, patch);
        let text = toml::to_string_pretty(&PartialConfig(merged)).context("serialize toml")?;
        let mut f = fs::File::create(path)?;
        f.write_all(text.as_bytes())?;
        Ok(())
    }

    /// Pick a model target given a function role. Falls back to default chat model.
    pub fn pick_model(&self, role: ModelRole) -> ModelTarget {
        let cfg = self.get();
        let key = match role {
            ModelRole::Chat => None,
            ModelRole::Title => Some("title"),
            ModelRole::SessionName => Some("session_name"),
            ModelRole::Compact => Some("compact"),
            ModelRole::MetaPrompt => Some("meta_prompt"),
            ModelRole::TaskStatus => Some("task_status"),
        };
        if let Some(k) = key {
            if let Some(t) = cfg.models.overrides.get(k) { return t.clone(); }
        }
        cfg.models.default.clone()
    }

    /// Helper: resolve API credentials from environment for a target.
    /// Returns (api_key, api_token) as discovered (both optional).
    pub fn resolve_credentials(&self, target: &ModelTarget) -> (Option<String>, Option<String>) {
        let key = target.api_key_env.as_ref().and_then(|k| std::env::var(k).ok());
        let tok = target.api_token_env.as_ref().and_then(|k| std::env::var(k).ok());
        (key, tok)
    }
}
