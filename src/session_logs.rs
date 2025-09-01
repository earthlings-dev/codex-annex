// annex/src/session_logs.rs

use anyhow::Result;
use chrono::{Utc, Datelike};
use serde::{Deserialize, Serialize};
use std::{fs, path::{Path, PathBuf}};
use crate::yaml_config::ConfigManager;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag="type", rename_all="snake_case")]
pub enum SessionEvent {
    UserMsg { content: String },
    ModelMsg { model: String, content: String },
    Exec { cmd: String, argv: Vec<String>, status: i32, cwd: String },
    FileRef { path: String, reason: String },
    Meta { key: String, value: serde_json::Value },
}

#[derive(Clone)]
pub struct SessionLogWriter {
    root_dir: PathBuf,
    session_id: String,
    day_dir: PathBuf,
    file: PathBuf,
}

impl SessionLogWriter {
    pub fn new(cfg: &ConfigManager, session_id: impl Into<String>) -> Result<Self> {
        let session_id = session_id.into();
        let base = cfg.get().sessions.dir.clone()
            .unwrap_or(directories::ProjectDirs::from("com","openai","codex").unwrap().data_dir().join("sessions"));
        let now = Utc::now();
        let day = format!("{:04}-{:02}-{:02}", now.year(), now.month(), now.day());
        let day_dir = base.join(&day).join(&session_id);
        fs::create_dir_all(&day_dir)?;
        let file = day_dir.join("session.yaml");
        if !file.exists() {
            fs::write(&file, "---\n# Codex session log\nentries:\n")?;
        }
        Ok(Self { root_dir: base, session_id, day_dir, file })
    }

    pub fn append(&self, ev: &SessionEvent) -> Result<()> {
        // append one YAML document per event to keep streaming simple
        let mut s = String::new();
        s.push_str("- ");
        s.push_str(&serde_yml::to_string(ev)?); // serializes to YAML, we prefix "- " to add to entries list
        // sanitize leading "- " duplication
        let s = s.replace("---\n", "");
        append_to_file(&self.file, &s)
    }

    pub fn purge_old(&self, keep_days: u32) -> Result<()> {
        use std::time::{SystemTime, Duration};
        let now = SystemTime::now();
        for e in fs::read_dir(&self.root_dir)? {
            let d = e?.path();
            if !d.is_dir() { continue; }
            let md = fs::metadata(&d)?; 
            if let Ok(modified) = md.modified() {
                if now.duration_since(modified).unwrap_or(Duration::ZERO) > Duration::from_secs(86400 * keep_days as u64) {
                    let _ = fs::remove_dir_all(&d);
                }
            }
        }
        Ok(())
    }

    pub fn path(&self) -> &Path { &self.file }
}

fn append_to_file(path: &Path, text: &str) -> Result<()> {
    use std::io::Write;
    let mut f = fs::OpenOptions::new().create(true).append(true).open(path)?;
    f.write_all(text.as_bytes())?;
    Ok(())
}