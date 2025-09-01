use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, fs, path::PathBuf, sync::Arc};

use crate::yaml_config::{ConfigManager, Scope};

#[derive(Clone)]
pub struct SlashRegistry {
    pub commands: BTreeMap<String, SlashDef>,
    cfg: Arc<ConfigManager>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag="kind", rename_all="snake_case")]
pub enum SlashDef {
    Alias { expands_to: String },
    Macro { lines: Vec<String> },
    Builtin { name: String, args: BTreeMap<String,String> },
}

impl SlashRegistry {
    pub fn load_from_dirs(cfg: Arc<ConfigManager>, dirs: &[PathBuf]) -> Result<Self> {
        let mut commands = BTreeMap::new();
        for d in dirs {
            if !d.exists() { continue; }
            for e in fs::read_dir(d)? {
                let p = e?.path();
                if p.extension().is_some_and(|x| x=="yaml"||x=="yml") {
                    let text = fs::read_to_string(&p)?;
                    let items: BTreeMap<String, SlashDef> = serde_yml::from_str(&text)?;
                    commands.extend(items);
                }
            }
        }
        Ok(Self { commands, cfg })
    }

    pub async fn dispatch(&self, input: &str) -> Result<String> {
        if !input.starts_with('/') { return Err(anyhow!("not a slash command")); }
        let (name, rest) = input[1..].split_once(' ').map(|(a,b)| (a,b)).unwrap_or((&input[1..], ""));
        let def = self.commands.get(name).ok_or_else(|| anyhow!("unknown slash: {}", name))?;
        match def {
            SlashDef::Alias { expands_to } => Ok(expands_to.replace("$ARGS", rest)),
            SlashDef::Macro { lines } => Ok(lines.join("\n")),
            SlashDef::Builtin { name, args } => {
                // You can map to functions e.g. "allowlist.add", "mcp.add", "config.set"
                Ok(format!("builtin:{} {}", name, serde_json::to_string(args)?))
            }
        }
    }
}