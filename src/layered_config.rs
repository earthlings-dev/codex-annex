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
    pub model: ModelConfig,
    pub sandbox: SandboxConfig,
    pub shell: ShellConfig,
    pub mcp: McpConfig,
    pub ui: UiConfig,
    pub history: HistoryConfig,
    pub todo: TodoConfig,
    pub compact: CompactConfig,
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
        let mut me = cm;
        me.reload_all()?;
        me.start_watch()?;
        Ok(me)
    }

    fn read_file(path: &Path) -> Option<Config> {
        let text = fs::read_to_string(path).ok()?;
        let p: PartialConfig = toml::from_str(&text).ok()?;
        Some(p.0)
    }

    pub fn reload_all(&mut self) -> Result<()> {
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

    fn start_watch(&mut self) -> Result<()> {
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
}