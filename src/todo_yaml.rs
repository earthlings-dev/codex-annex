use anyhow::{Context, Result};
use chrono::{Utc, Datelike};
use serde::{Deserialize, Serialize};
use std::{fs, path::{Path, PathBuf}};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TodoStatus { Open, InProgress, Done }

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TodoItem {
    pub id: String,
    pub session_id: String,
    pub date: String,       // YYYY-MM-DD
    pub task_number: u32,   // 1..N within session
    pub title: String,
    pub description: Option<String>,
    pub files: Vec<PathBuf>,
    pub tags: Vec<String>,
    pub status: TodoStatus,
}

#[derive(Default, Clone, Debug, Serialize, Deserialize)]
pub struct TodoStore {
    pub items: Vec<TodoItem>,
}

impl TodoStore {
    pub fn load(path: &Path) -> Result<Self> {
        if !path.exists() { return Ok(Self::default()); }
        let data = fs::read_to_string(path)?;
        let s: Self = serde_yml::from_str(&data).context("parse todo store yaml")?;
        Ok(s)
    }
    pub fn save(&self, path: &Path) -> Result<()> {
        if let Some(dir) = path.parent() { fs::create_dir_all(dir)?; }
        fs::write(path, serde_yml::to_string(self)?)?;
        Ok(())
    }

    /// Adds a TODO and also writes a *file-per-item* under .codex/todos/{YYYY-MM-DD}/{session}/{task_number}-{id}.yaml
    pub fn add_and_persist(
        &mut self, root: &Path, session_id: &str, task_number: u32, title: String,
        description: Option<String>, files: Vec<PathBuf>, tags: Vec<String>
    ) -> Result<&TodoItem> {
        let today = Utc::now();
        let date = format!("{:04}-{:02}-{:02}", today.year(), today.month(), today.day());
        let id = uuid::Uuid::new_v4().to_string();
        let item = TodoItem {
            id: id.clone(), session_id: session_id.into(), date: date.clone(), task_number,
            title, description, files, tags, status: TodoStatus::Open
        };
        self.items.push(item);
        // Write per-item YAML for resumability
        let per = root.join(".codex").join("todos").join(&date).join(session_id)
                      .join(format!("{:03}-{}.yaml", task_number, id));
        if let Some(dir) = per.parent() { fs::create_dir_all(dir)?; }
        let last = self.items.last().unwrap();
        fs::write(per, serde_yml::to_string(last)?)?;
        Ok(last)
    }
}