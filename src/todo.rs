// annex/src/todo.rs
// this is an in-progress file that needs to be merged & needed portions that are gaps converted to the yaml implementation, & unneeded portions (from the non-yaml implementation) removed

// annex/src/todo_yaml.rs

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

// annex/src/todo.rs content below

use anyhow::{Context, Result};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::{fs, path::{Path, PathBuf}};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TodoStatus { Open, InProgress, Done }

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TodoItem {
    pub id: String,
    pub title: String,
    pub description: Option<String>,
    pub files: Vec<PathBuf>,   // referenced files/globs
    pub tags: Vec<String>,
    pub status: TodoStatus,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Default, Clone, Debug, Serialize, Deserialize)]
pub struct TodoStore {
    pub items: Vec<TodoItem>,
}

impl TodoStore {
    pub fn load(path: &Path) -> Result<Self> {
        if !path.exists() { return Ok(Self::default()); }
        let data = fs::read_to_string(path)?;
        let s: Self = serde_json::from_str(&data).context("parse todo store")?;
        Ok(s)
    }
    pub fn save(&self, path: &Path) -> Result<()> {
        if let Some(dir) = path.parent() { fs::create_dir_all(dir)?; }
        fs::write(path, serde_json::to_string_pretty(self)?)?;
        Ok(())
    }
    pub fn add(&mut self, title: String, description: Option<String>, files: Vec<PathBuf>, tags: Vec<String>) -> &TodoItem {
        let now = Utc::now().to_rfc3339();
        self.items.push(TodoItem {
            id: uuid::Uuid::new_v4().to_string(),
            title, description, files, tags,
            status: TodoStatus::Open,
            created_at: now.clone(), updated_at: now,
        });
        self.items.last().unwrap()
    }
    pub fn set_status(&mut self, id: &str, status: TodoStatus) -> Result<()> {
        let now = Utc::now().to_rfc3339();
        let it = self.items.iter_mut().find(|x| x.id == id).context("todo not found")?;
        it.status = status;
        it.updated_at = now;
        Ok(())
    }
    pub fn remove(&mut self, id: &str) -> Result<()> {
        let before = self.items.len();
        self.items.retain(|x| x.id != id);
        if self.items.len() == before { anyhow::bail!("todo not found"); }
        Ok(())
    }
}