// annex/src/session_logs.rs

use anyhow::Result;
use chrono::{Datelike, Utc};
use serde::{Deserialize, Serialize};
use std::{fs, io::Write, path::{Path, PathBuf}};

use crate::layered_config::ConfigManager;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SessionEvent {
    UserMsg { content: String },
    ModelMsg { model: String, content: String },
    Exec { cmd: String, argv: Vec<String>, status: i32, cwd: String },
    FileRef { path: String, reason: String },
    Meta { key: String, value: serde_json::Value },
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct OutEvent {
    ts: String,
    #[serde(flatten)]
    ev: SessionEvent,
}

#[derive(Clone)]
pub struct SessionLogWriter {
    root_dir: PathBuf,
    _session_id: String,
    day_dir: PathBuf,
    json_file: PathBuf,
    jsonl_file: PathBuf,
    write_mode: WriteMode,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum WriteMode { Json, Jsonl, Both }

impl SessionLogWriter {
    pub fn new(cfg: &ConfigManager, session_id: impl Into<String>) -> Result<Self> {
        let session_id = session_id.into();
        let base = cfg.get().sessions.dir.clone()
            .unwrap_or(directories::ProjectDirs::from("com", "openai", "codex").unwrap().data_dir().join("sessions"));
        let now = Utc::now();
        let day = format!("{:04}-{:02}-{:02}", now.year(), now.month(), now.day());
        let day_dir = base.join(&day).join(&session_id);
        fs::create_dir_all(&day_dir)?;
        let json_file = day_dir.join("session.json");
        if !json_file.exists() {
            fs::write(&json_file, "[]")?;
        }
        let jsonl_file = day_dir.join("session.jsonl");
        if !jsonl_file.exists() {
            fs::File::create(&jsonl_file)?; // empty file
        }
        let mode = match cfg.get().sessions.write_mode.as_deref() {
            Some("json") => WriteMode::Json,
            Some("jsonl") => WriteMode::Jsonl,
            _ => WriteMode::Both,
        };
        Ok(Self { root_dir: base, _session_id: session_id, day_dir, json_file, jsonl_file, write_mode: mode })
    }

    pub fn append(&self, ev: &SessionEvent) -> Result<()> {
        let ts = Utc::now().to_rfc3339();
        let out = OutEvent { ts, ev: ev.clone() };
        let redacted = redact_json(serde_json::to_value(out)?)?;
        match self.write_mode {
            WriteMode::Json => self.append_json(&redacted)?,
            WriteMode::Jsonl => self.append_jsonl(&redacted)?,
            WriteMode::Both => { self.append_json(&redacted)?; self.append_jsonl(&redacted)?; }
        }
        Ok(())
    }

    fn append_json(&self, value: &serde_json::Value) -> Result<()> {
        // Read, push, write back. For small session logs this is fine.
        let data = fs::read_to_string(&self.json_file).unwrap_or_else(|_| "[]".into());
        let mut arr: Vec<serde_json::Value> = serde_json::from_str(&data).unwrap_or_default();
        arr.push(value.clone());
        let text = serde_json::to_string_pretty(&arr)?;
        fs::write(&self.json_file, text)?;
        Ok(())
    }

    fn append_jsonl(&self, value: &serde_json::Value) -> Result<()> {
        let mut f = fs::OpenOptions::new().create(true).append(true).open(&self.jsonl_file)?;
        writeln!(f, "{}", serde_json::to_string(value)?)?;
        Ok(())
    }

    pub fn purge_old(&self, keep_days: u32) -> Result<()> {
        use std::time::{Duration, SystemTime};
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

    pub fn json_path(&self) -> &Path { &self.json_file }
    pub fn jsonl_path(&self) -> &Path { &self.jsonl_file }
}

fn redact_json(mut v: serde_json::Value) -> Result<serde_json::Value> {
    fn redact_str(s: &str) -> String {
        let patterns = ["KEY", "TOKEN", "SECRET", "PASSWORD"];
        if patterns.iter().any(|p| s.to_ascii_uppercase().contains(p)) {
            "[REDACTED]".into()
        } else { s.into() }
    }
    match v {
        serde_json::Value::String(ref mut s) => {
            let r = redact_str(s);
            *s = r;
        }
        serde_json::Value::Array(ref mut arr) => {
            for x in arr.iter_mut() { *x = redact_json(std::mem::take(x)).unwrap_or_else(|_| serde_json::Value::Null); }
        }
        serde_json::Value::Object(ref mut map) => {
            for (_k, x) in map.iter_mut() { *x = redact_json(std::mem::take(x)).unwrap_or_else(|_| serde_json::Value::Null); }
        }
        _ => {}
    }
    Ok(v)
}
