use std::path::PathBuf;
use std::sync::{Arc, RwLock};

use crate::config::setup::{UpstreamSetupUpdate, UpstreamSetupView, view_from_config, view_from_config_for_agent};
use crate::config::{load_from_path, save, ConfigFile};
use crate::gateway::config::AppConfig;

#[derive(Clone)]
pub struct ConfigManager {
    path: PathBuf,
    inner: Arc<RwLock<AppConfig>>,
}

impl ConfigManager {
    pub fn new(config: AppConfig) -> Arc<Self> {
        let path = config.config_path.clone();
        Arc::new(Self {
            path,
            inner: Arc::new(RwLock::new(config)),
        })
    }

    pub fn path(&self) -> &PathBuf {
        &self.path
    }

    pub fn get(&self) -> AppConfig {
        self.inner
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    pub fn setup_view(&self) -> anyhow::Result<UpstreamSetupView> {
        self.setup_view_for_agent(None)
    }

    pub fn setup_view_for_agent(&self, agent_id: Option<&str>) -> anyhow::Result<UpstreamSetupView> {
        let (file, _) = load_from_path(&self.path)?;
        Ok(match agent_id {
            Some(id) => view_from_config_for_agent(&file, id),
            None => view_from_config(&file),
        })
    }

    pub fn apply_setup(&self, patch: &UpstreamSetupUpdate) -> anyhow::Result<UpstreamSetupView> {
        let (view, _) = self.apply_setup_with_config(patch)?;
        Ok(view)
    }

    pub fn apply_setup_with_config(
        &self,
        patch: &UpstreamSetupUpdate,
    ) -> anyhow::Result<(UpstreamSetupView, AppConfig)> {
        let (mut file, _) = load_from_path(&self.path)?;
        crate::config::setup::apply_setup_patch(&mut file, patch)
            .map_err(|e| anyhow::anyhow!(e))?;
        save(&self.path, &file)?;
        let config = self.reload_from_file(&file)?;
        let view = match patch.agent_id.as_deref() {
            Some(id) if !id.is_empty() => view_from_config_for_agent(&file, id),
            _ => view_from_config(&file),
        };
        Ok((view, config))
    }

    pub fn write_default_setup(&self) -> anyhow::Result<UpstreamSetupView> {
        let (mut file, _) = load_from_path(&self.path)?;
        crate::config::setup::apply_default_upstream(&mut file);
        save(&self.path, &file)?;
        self.reload_from_file(&file)?;
        Ok(view_from_config(&file))
    }

    fn reload_from_file(&self, file: &ConfigFile) -> anyhow::Result<AppConfig> {
        let updated = AppConfig::from_file(file.clone(), self.path.clone())?;
        *self
            .inner
            .write()
            .unwrap_or_else(|e| e.into_inner()) = updated.clone();
        Ok(updated)
    }
}
