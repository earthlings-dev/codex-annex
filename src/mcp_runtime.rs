use anyhow::Result;
use parking_lot::RwLock;
use std::{collections::BTreeMap, sync::Arc};

use crate::layered_config::{Config, ConfigManager, McpServer};

#[derive(Clone)]
pub struct McpRuntime {
    cfg: Arc<ConfigManager>,
    state: Arc<RwLock<BTreeMap<String, McpState>>>,
}

#[derive(Clone, Debug)]
pub enum McpState { Disconnected, Connecting, Connected, Error(String) }

impl McpRuntime {
    pub fn new(cfg: Arc<ConfigManager>) -> Self {
        Self { cfg, state: Arc::new(RwLock::new(BTreeMap::new())) }
    }
    pub fn snapshot(&self) -> BTreeMap<String, McpState> { self.state.read().clone() }
    pub async fn reconcile(&self) -> Result<()> {
        let cfg = self.cfg.get();
        for (name, server) in cfg.mcp.servers.iter() {
            if !server.enabled {
                self.state.write().insert(name.clone(), McpState::Disconnected);
            } else {
                self.state.write().insert(name.clone(), McpState::Connected);
            }
        }
        Ok(())
    }
    pub async fn set_enabled(&self, name: &str, enabled: bool) -> Result<()> {
        let mut patch = Config::default();
        if let Some(mut s) = self.cfg.get().mcp.servers.get(name).cloned() {
            s.enabled = enabled;
            patch.mcp.servers.insert(name.into(), s);
            self.cfg.write_patch(crate::layered_config::Scope::Workspace, &patch)?;
        }
        Ok(())
    }
}