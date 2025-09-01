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