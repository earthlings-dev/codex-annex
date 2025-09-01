use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct AgentProfile {
    pub name: String,
    pub model: String,
    pub sandbox_mode: Option<String>,
    pub shell_allowlist: Vec<String>,
    pub mcp_enabled: Vec<String>,
    pub system_prompt: Option<String>,
}

#[derive(Clone, Debug, Default)]
pub struct AgentDirectory {
    pub profiles: BTreeMap<String, AgentProfile>,
}
impl AgentDirectory {
    pub fn get(&self, name: &str) -> Option<&AgentProfile> { self.profiles.get(name) }
    pub fn upsert(&mut self, p: AgentProfile) { self.profiles.insert(p.name.clone(), p); }
}