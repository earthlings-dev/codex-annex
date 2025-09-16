// annex/src/compact.rs

use anyhow::{Context, Result};
use git2::Repository;
use globset::{Glob, GlobSet, GlobSetBuilder};
use ignore::WalkBuilder;
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::{collections::{BTreeMap, BTreeSet}, fs, path::{Path, PathBuf}, time::{Duration, SystemTime}};

use crate::{layered_config::ConfigManager, todo::{TodoStore, TodoStatus}};

#[derive(Clone, Copy, Debug)]
pub enum AutoCompactStage {
    MidTask,    // in the middle of a task
    EndOfTask,  // right after task end
}

#[derive(Clone)]
pub struct Compactor {
    pub cfg: std::sync::Arc<ConfigManager>,
    pub workspace_root: PathBuf,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CompactResult {
    pub chosen_files: Vec<PathBuf>,
    pub focus_prompt: String,
}

/// Build a GlobSet from patterns.
fn build_globset(patterns: &[String]) -> Result<GlobSet> {
    let mut b = GlobSetBuilder::new();
    for p in patterns {
        b.add(Glob::new(p).with_context(|| format!("invalid glob: {}", p))?);
    }
    Ok(b.build()?)
}

fn is_probably_text(path: &Path) -> bool {
    // quick heuristics by extension; extend as needed
    matches!(path.extension().and_then(|s| s.to_str()).unwrap_or("").to_ascii_lowercase().as_str(),
        "rs"|"md"|"toml"|"json"|"yml"|"yaml"|"ts"|"tsx"|"js"|"py"|"go"|"java"|"kt"|"c"|"h"|"cpp"|"hpp"|"txt"|"sh"|"bash"|"zsh"|"fish"|"cfg"|"ini")
}

fn now() -> SystemTime { SystemTime::now() }

fn recent_mtime_score(path: &Path) -> u32 {
    fs::metadata(path).ok()
        .and_then(|m| m.modified().ok())
        .and_then(|t| now().duration_since(t).ok())
        .map(|d| (1_000_000u64.saturating_sub(d.as_secs())).min(1_000_000) as u32)
        .unwrap_or(0)
}

/// Extract file-like tokens from the last N lines of `.codex/audit.log`.
fn extract_recent_exec_files(workspace: &Path, n_lines: usize) -> BTreeSet<PathBuf> {
    let mut out = BTreeSet::new();
    let log = workspace.join(".codex/audit.log");
    let Ok(data) = fs::read_to_string(&log) else { return out; };
    let lines: Vec<&str> = data.lines().rev().take(n_lines).collect();
    // Simple file-ish regex: capture paths with / or \ and an extension
    let re = Regex::new(r#"([A-Za-z0-9_\-./\\]+?\.[A-Za-z0-9]{1,8})"#).unwrap();
    for line in lines {
        for cap in re.captures_iter(line) {
            let p = cap.get(1).unwrap().as_str();
            let pb = workspace.join(p);
            if pb.exists() { out.insert(pb); }
        }
    }
    out
}

/// Score candidate files; higher is better.
fn score_files(
    candidates: &BTreeSet<PathBuf>,
    changed: &BTreeSet<PathBuf>,
    todo_refs: &BTreeSet<PathBuf>,
    exec_refs: &BTreeSet<PathBuf>,
) -> BTreeMap<PathBuf, u64> {
    let mut scores = BTreeMap::<PathBuf, u64>::new();
    for p in candidates {
        let mut s = 0u64;
        if changed.contains(p) { s += 5000; }
        if todo_refs.contains(p) { s += 3000; }
        if exec_refs.contains(p) { s += 2000; }
        s += recent_mtime_score(p) as u64 / 10;
        scores.insert(p.clone(), s);
    }
    scores
}

fn git_changed_files(repo: &Repository, root: &Path) -> BTreeSet<PathBuf> {
    let mut out = BTreeSet::new();
    // Index (staged)
    if let Ok(idx) = repo.index() {
        for e in idx.iter() {
            if let Some(path) = std::str::from_utf8(&e.path).ok() {
                out.insert(root.join(path));
            }
        }
    }
    // Workdir (unstaged) via status
    if let Ok(statuses) = repo.statuses(None) {
        for e in statuses.iter() {
            if let Some(path) = e.path() {
                out.insert(root.join(path));
            }
        }
    }
    out
}

fn todo_file_set(store: &TodoStore, root: &Path) -> BTreeSet<PathBuf> {
    let mut out = BTreeSet::new();
    for it in &store.items {
        if matches!(it.status, TodoStatus::Open | TodoStatus::InProgress) {
            for f in &it.files { out.insert(root.join(f)); }
        }
    }
    out
}

fn default_includes(cfg: &crate::layered_config::Config) -> Vec<String> {
    if cfg.compact.include_globs_default.is_empty() {
        vec!["**/*".into()]
    } else { cfg.compact.include_globs_default.clone() }
}

impl Compactor {
    pub fn new(cfg: std::sync::Arc<ConfigManager>, workspace_root: PathBuf) -> Self {
        Self { cfg, workspace_root }
    }

    /// Manual compact: user-provided focus + include globs, returns the chosen files and the final summarization prompt you should feed to the model.
    pub fn manual_compact(&self, user_focus: Option<String>, include_globs: Vec<String>, conversation_tail: &str) -> Result<CompactResult> {
        let cfg = self.cfg.get();
        let includes = if include_globs.is_empty() { default_includes(&cfg) } else { include_globs };
        let gs = build_globset(&includes)?;

        // Collect matching files with ignore/.gitignore honored
        let mut candidates = BTreeSet::<PathBuf>::new();
        for r in WalkBuilder::new(&self.workspace_root).hidden(false).follow_links(false).git_ignore(true).build() {
            let de = match r { Ok(d) => d, Err(_) => continue };
            let p = de.path();
            if !p.is_file() { continue; }
            let rel = p.strip_prefix(&self.workspace_root).unwrap_or(p);
            if gs.is_match(rel) && is_probably_text(p) { candidates.insert(p.to_path_buf()); }
        }

        // Limit to configured max_files
        let chosen: Vec<PathBuf> = candidates.into_iter().take(cfg.compact.max_files).collect();

        // Build summarization prompt (manual: user_focus leads)
        let mut focus = String::new();
        if let Some(f) = user_focus { focus.push_str(&format!("User focus:\n{}\n\n", f)); }
        focus.push_str("Conversation context (tail):\n");
        focus.push_str(conversation_tail);
        focus.push_str("\n\nSummarize concisely with explicit references to the listed files where relevant. Output sections: What changed, Why, Open TODOs, Next steps.");

        Ok(CompactResult { chosen_files: chosen, focus_prompt: focus })
    }

    /// Auto compact: stage-aware. We first ask the **model** to produce a focused summarization prompt,
    /// then we use it to request the compact summary. This function returns the chosen files + generated focus prompt.
    ///
    /// `gen_meta_prompt`: takes (stage, todo_snapshot_json, activity_json) -> meta-prompt string via model.
    pub async fn auto_compact<FMeta>(&self, stage: AutoCompactStage, gen_meta_prompt: FMeta) -> Result<CompactResult>
    where
        FMeta: Fn(AutoCompactStage, String, String) -> std::pin::Pin<Box<dyn std::future::Future<Output=Result<String>> + Send>> + Send,
    {
        let cfg = self.cfg.get();

        // Load TODOs
        let todo_path = cfg.todo.path.clone().unwrap_or(self.workspace_root.join(".codex").join("todo.json"));
        let todos = crate::todo::TodoStore::load(&todo_path).unwrap_or_default();
        let todo_json = serde_json::to_string(&todos).unwrap_or("{}".into());

        // Recent activity (audit log & basic counters)
        let recent_exec_files = extract_recent_exec_files(&self.workspace_root, 500);
        let activity = serde_json::json!({
            "recent_exec_files": recent_exec_files,
            "time": chrono::Utc::now().to_rfc3339(),
            "stage": format!("{:?}", stage)
        }).to_string();

        // Ask the model to produce a **focused meta-prompt** for summarizing
        let focus_prompt = gen_meta_prompt(stage, todo_json, activity).await?;

        // Candidate assembly
        let repo = Repository::discover(&self.workspace_root).ok();
        let changed = repo
            .as_ref()
            .map(|r| git_changed_files(r, &self.workspace_root))
            .unwrap_or_default();

        // Default includes + ignore rules
        let gs = build_globset(&default_includes(&cfg))?;
        let mut candidates = BTreeSet::<PathBuf>::new();
        for r in WalkBuilder::new(&self.workspace_root).hidden(false).follow_links(false).git_ignore(true).build() {
            let de = match r { Ok(d) => d, Err(_) => continue };
            let p = de.path();
            if !p.is_file() { continue; }
            let rel = p.strip_prefix(&self.workspace_root).unwrap_or(p);
            if gs.is_match(rel) && is_probably_text(p) { candidates.insert(p.to_path_buf()); }
        }

        let todo_refs = todo_file_set(&todos, &self.workspace_root);
        let scores = score_files(&candidates, &changed, &todo_refs, &recent_exec_files);
        let mut ranked: Vec<(PathBuf, u64)> = scores.into_iter().collect();
        ranked.sort_by(|a,b| b.1.cmp(&a.1));
        let chosen: Vec<PathBuf> = ranked.into_iter().map(|(p,_)| p).take(cfg.compact.max_files).collect();

        Ok(CompactResult { chosen_files: chosen, focus_prompt })
    }

    /// Should we trigger auto-compact now? Simple interval + optional stage gate.
    pub fn should_autotrigger(&self, last_compact: Option<std::time::SystemTime>, stage: AutoCompactStage) -> bool {
        let cfg = self.cfg.get();
        if !cfg.compact.auto_enable { return false; }
        if matches!(stage, AutoCompactStage::EndOfTask) && !cfg.compact.auto_on_task_end { return false; }
        if let Some(t) = last_compact {
            if let Ok(elapsed) = t.elapsed() {
                return elapsed >= Duration::from_secs(cfg.compact.auto_min_interval_secs);
            }
        }
        true
    }
}
