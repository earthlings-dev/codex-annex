use anyhow::{Context, Result};
use directories::ProjectDirs;
use notify::{recommended_watcher, Event, RecursiveMode, Watcher};
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, fs, path::{Path, PathBuf}, sync::Arc};
use tokio::sync::broadcast;

/// Config is merged: system -> user -> workspace -> runtime (ephemeral)
#[derive(Clone, Debug, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct Config {
    pub models: ModelsConfig,
    pub sandbox: SandboxConfig,
    pub shell: ShellConfig,
    pub ui: UiConfig,
    pub history: HistoryConfig,
    pub todo: TodoConfig,
    pub compact: CompactConfig,
    pub sessions: SessionsConfig,
    pub hooks: HooksConfig,       // default registry knobs (limits etc.)
    pub slash: SlashConfigMeta,   // path/location info for slash YAMLs
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct ModelsConfig {
    /// Default chat/completion model (used unless overridden).
    pub default: ModelTarget,
    /// Map of function-role -> model target; e.g. "title", "session_name", "compact", "meta_prompt", "task_status"
    pub overrides: BTreeMap<String, ModelTarget>,
    /// Named profiles you can reference in tasks (per-task model label)
    pub profiles: BTreeMap<String, ModelTarget>,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct ModelTarget {
    pub name: String,                  // e.g. "gpt-4o-mini", "gemini-1.5-flash", "claude-3.7-sonnet"
    pub base_url: Option<String>,      // e.g. "https://api.openai.com/v1"
    pub api_key_env: Option<String>,   // e.g. "OPENAI_API_KEY" (looked up at call time)
    pub extra_headers: BTreeMap<String, String>, // per-provider headers if needed
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

impl ModelRole {
    pub fn key(&self) -> &'static str {
        match self {
            Self::Chat => "chat",
            Self::Title => "title",
            Self::SessionName => "session_name",
            Self::Compact => "compact",
            Self::MetaPrompt => "meta_prompt",
            Self::TaskStatus => "task_status",
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct SandboxConfig {
    pub mode: Option<String>,              // "danger_full_access"|"read_only"|"workspace_write"
    pub network_access: Option<bool>,
    pub writable_roots: Vec<PathBuf>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalMode { OnRequest, OnFailure, UnlessTrusted, Never }
impl Default for ApprovalMode { fn default() -> Self { Self::OnRequest } }

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct ShellConfig {
    pub approval: ApprovalMode,
    pub allowlist_roots: Vec<String>,
    pub denylist_roots: Vec<String>,
    pub env_inherit: Option<String>,       // "none"|"core"|"all"
    pub env_exclude_patterns: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct UiConfig {
    pub command_palette: bool,
    pub status_bar: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct HistoryConfig {
    pub persist: Option<String>, // "none"|"session"|"all"
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct TodoConfig {
    pub dir: Option<PathBuf>, // default: .codex/todos/
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct CompactConfig {
    pub auto_enable: bool,
    pub auto_min_interval_secs: u64,
    pub auto_on_task_end: bool,
    pub max_context_chars: usize,
    pub max_files: usize,
    pub include_globs_default: Vec<String>,
}
impl Default for CompactConfig {
    fn default() -> Self {
        Self {
            auto_enable: true,
            auto_min_interval_secs: 120,
            auto_on_task_end: true,
            max_context_chars: 40_000,
            max_files: 24,
            include_globs_default: vec!["**/*.rs".into(),"**/*.md".into(),"**/*.toml".into(),"**/*.yaml".into(),"**/*.yml".into()],
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct SessionsConfig {
    pub dir: Option<PathBuf>,     // default: ~/.local/share/codex/sessions
    pub auto_purge_days: Option<u32>,
    pub resume_on_launch: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct HooksConfig {
    pub recursion_limit: Option<u32>, // default in code
    pub dirs: Vec<PathBuf>,           // additional lookup dirs for hooks/*.yaml
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct SlashConfigMeta {
    pub dirs: Vec<PathBuf>, // lookup dirs for slash/*.yaml
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Scope { System, User, Workspace, Runtime }

#[derive(Clone)]
pub struct ConfigManager {
    inner: Arc<RwLock<Config>>,
    runtime_overlay: Arc<RwLock<Config>>,
    tx: broadcast::Sender<Config>,
    _watcher: Arc<RwLock<Option<notify::RecommendedWatcher>>>,
    system_dir: PathBuf,
    user_dir: PathBuf,
    workspace_dir: PathBuf,
}

impl ConfigManager {
    pub fn load(workspace_root: impl AsRef<Path>) -> Result<Self> {
        let proj = ProjectDirs::from("com", "openai", "codex").context("ProjectDirs")?;
        let system_dir = if cfg!(target_os="windows") { PathBuf::from(r"C:\ProgramData\Codex") } else { PathBuf::from("/etc/codex") };
        let user_dir = proj.config_dir().to_path_buf();
        let workspace_dir = workspace_root.as_ref().join(".codex");
        let me = Self {
            inner: Arc::new(RwLock::new(Config::default())),
            runtime_overlay: Arc::new(RwLock::new(Config::default())),
            tx: broadcast::channel(64).0,
            _watcher: Arc::new(RwLock::new(None)),
            system_dir, user_dir, workspace_dir,
        };
        let mut m = me;
        m.reload_all()?;
        m.start_watch()?;
        Ok(m)
    }

    fn read_yaml_dir(dir: &Path) -> Config {
        // Merge all *.yaml in directory (lexicographic order)
        let mut cfg = Config::default();
        let Ok(rd) = fs::read_dir(dir) else { return cfg; };
        let mut files: Vec<PathBuf> = rd.filter_map(|e| e.ok().map(|x| x.path()))
                                        .filter(|p| p.extension().is_some_and(|e| e=="yaml"||e=="yml"))
                                        .collect();
        files.sort();
        for f in files {
            if let Ok(text) = fs::read_to_string(&f) {
                if let Ok(part) = serde_yml::from_str::<Config>(&text) {
                    merge(&mut cfg, &part);
                }
            }
        }
        cfg
    }

    pub fn reload_all(&mut self) -> Result<()> {
        let mut merged = Config::default();
        merge(&mut merged, &Self::read_yaml_dir(&self.system_dir));
        merge(&mut merged, &Self::read_yaml_dir(&self.user_dir));
        merge(&mut merged, &Self::read_yaml_dir(&self.workspace_dir));
        // runtime overlay (in-memory)
        let rt = self.runtime_overlay.read().clone();
        merge(&mut merged, &rt);
        *self.inner.write() = merged.clone();
        let _ = self.tx.send(merged);
        Ok(())
    }

    fn start_watch(&mut self) -> Result<()> {
        let system = self.system_dir.clone();
        let user = self.user_dir.clone();
        let workspace = self.workspace_dir.clone();
        let tx = self.tx.clone();
        let inner = self.inner.clone();
        let runtime = self.runtime_overlay.clone();
        let mut watcher = recommended_watcher(move |res: Result<Event, _>| {
            if res.is_err() { return; }
            let mut cfg = Config::default();
            merge(&mut cfg, &ConfigManager::read_yaml_dir(&system));
            merge(&mut cfg, &ConfigManager::read_yaml_dir(&user));
            merge(&mut cfg, &ConfigManager::read_yaml_dir(&workspace));
            merge(&mut cfg, &runtime.read().clone());
            *inner.write() = cfg.clone();
            let _ = tx.send(cfg);
        })?;
        for d in [&self.system_dir, &self.user_dir, &self.workspace_dir] {
            fs::create_dir_all(d)?;
            watcher.watch(d, RecursiveMode::NonRecursive)?;
        }
        *self._watcher.write() = Some(watcher);
        Ok(())
    }

    pub fn get(&self) -> Config { self.inner.read().clone() }
    pub fn subscribe(&self) -> broadcast::Receiver<Config> { self.tx.subscribe() }

    /// In-memory overlay (not persisted).
    pub fn apply_runtime_overlay(&self, patch: Config) -> Result<()> {
        {
            let mut rt = self.runtime_overlay.write();
            merge(&mut *rt, &patch);
        }
        self.reload_all()
    }

    /// Persist a patch into one scope directory as `<name>.yaml` (default: `99-runtime.yaml` for Workspace writes).
    pub fn write_patch_file(&self, scope: Scope, name: &str, patch: &Config) -> Result<PathBuf> {
        use std::io::Write;
        let dir = match scope {
            Scope::System => &self.system_dir,
            Scope::User => &self.user_dir,
            Scope::Workspace => &self.workspace_dir,
            Scope::Runtime => anyhow::bail!("runtime scope is not persisted"),
        };
        fs::create_dir_all(dir)?;
        let path = dir.join(name);
        let text = serde_yml::to_string(patch)?;
        let mut f = fs::File::create(&path)?;
        f.write_all(text.as_bytes())?;
        self.reload_all()?;
        Ok(path)
    }

    /// Choose model target for a role.
    pub fn pick_model(&self, role: ModelRole) -> ModelTarget {
        let cfg = self.get();
        if let Some(mt) = cfg.models.overrides.get(role.key()) { return mt.clone(); }
        cfg.models.default.clone()
    }
}

fn merge(a: &mut Config, b: &Config) {
    // helper to overlay strings if present
    macro_rules! ov { ($dst:expr, $src:expr) => { if !$src.is_empty() { $dst = $src.clone(); } } }

    // models
    if !b.models.default.name.is_empty() { a.models.default = b.models.default.clone(); }
    for (k,v) in &b.models.overrides { a.models.overrides.insert(k.clone(), v.clone()); }
    for (k,v) in &b.models.profiles { a.models.profiles.insert(k.clone(), v.clone()); }

    // sandbox
    if b.sandbox.mode.is_some() { a.sandbox.mode = b.sandbox.mode.clone(); }
    if b.sandbox.network_access.is_some() { a.sandbox.network_access = b.sandbox.network_access; }
    if !b.sandbox.writable_roots.is_empty() { a.sandbox.writable_roots = b.sandbox.writable_roots.clone(); }

    // shell
    a.shell.approval = b.shell.approval.clone();
    if !b.shell.allowlist_roots.is_empty() { a.shell.allowlist_roots = b.shell.allowlist_roots.clone(); }
    if !b.shell.denylist_roots.is_empty() { a.shell.denylist_roots = b.shell.denylist_roots.clone(); }
    if b.shell.env_inherit.is_some() { a.shell.env_inherit = b.shell.env_inherit.clone(); }
    if !b.shell.env_exclude_patterns.is_empty() { a.shell.env_exclude_patterns = b.shell.env_exclude_patterns.clone(); }

    // ui, history
    a.ui.command_palette |= b.ui.command_palette;
    a.ui.status_bar |= b.ui.status_bar;
    if b.history.persist.is_some() { a.history.persist = b.history.persist.clone(); }

    // todo/compact/sessions
    if b.todo.dir.is_some() { a.todo.dir = b.todo.dir.clone(); }
    if b.compact.auto_enable { a.compact.auto_enable = true; }
    if b.compact.auto_min_interval_secs != 0 { a.compact.auto_min_interval_secs = b.compact.auto_min_interval_secs; }
    if b.compact.auto_on_task_end { a.compact.auto_on_task_end = true; }
    if b.compact.max_context_chars != 0 { a.compact.max_context_chars = b.compact.max_context_chars; }
    if b.compact.max_files != 0 { a.compact.max_files = b.compact.max_files; }
    if !b.compact.include_globs_default.is_empty() { a.compact.include_globs_default = b.compact.include_globs_default.clone(); }
    if b.sessions.dir.is_some() { a.sessions.dir = b.sessions.dir.clone(); }
    if b.sessions.auto_purge_days.is_some() { a.sessions.auto_purge_days = b.sessions.auto_purge_days; }
    a.sessions.resume_on_launch |= b.sessions.resume_on_launch;

    // hooks/slash
    if b.hooks.recursion_limit.is_some() { a.hooks.recursion_limit = b.hooks.recursion_limit; }
    if !b.hooks.dirs.is_empty() { a.hooks.dirs = b.hooks.dirs.clone(); }
    if !b.slash.dirs.is_empty() { a.slash.dirs = b.slash.dirs.clone(); }
}