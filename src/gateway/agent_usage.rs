use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

const BUDGET_WINDOW_SECS: u64 = 5 * 3600;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct AgentCloudUsageData {
    window_key: u64,
    tokens: u64,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct AgentCloudUsageFile {
    agents: HashMap<String, AgentCloudUsageData>,
}

pub struct AgentCloudUsageStore {
    path: PathBuf,
    inner: Mutex<AgentCloudUsageFile>,
    dirty: Mutex<bool>,
}

impl AgentCloudUsageStore {
    pub fn open(data_dir: &std::path::Path) -> anyhow::Result<std::sync::Arc<Self>> {
        let path = data_dir.join("agent_cloud_usage.json");
        let file: AgentCloudUsageFile = if path.exists() {
            let text = std::fs::read_to_string(&path)
                .unwrap_or_default();
            serde_json::from_str(&text).unwrap_or_default()
        } else {
            AgentCloudUsageFile::default()
        };
        Ok(std::sync::Arc::new(Self {
            path,
            inner: Mutex::new(file),
            dirty: Mutex::new(false),
        }))
    }

    pub fn check_budget(&self, agent_id: &str, limit: Option<u64>, estimated_new: u32) -> bool {
        let Some(limit) = limit else {
            return true;
        };
        if limit == 0 {
            return true;
        }
        let key = window_key();
        let mut guard = self.inner.lock().expect("agent_usage mutex");
        let usage = guard
            .agents
            .entry(agent_id.to_string())
            .or_insert_with(|| AgentCloudUsageData {
                window_key: key,
                tokens: 0,
            });
        if usage.window_key != key {
            usage.window_key = key;
            usage.tokens = 0;
        }
        usage.tokens.saturating_add(estimated_new as u64) < limit
    }

    pub fn record_tokens(&self, agent_id: &str, tokens: u64) {
        let key = window_key();
        let mut guard = self.inner.lock().expect("agent_usage mutex");
        let usage = guard
            .agents
            .entry(agent_id.to_string())
            .or_insert_with(|| AgentCloudUsageData {
                window_key: key,
                tokens: 0,
            });
        if usage.window_key != key {
            usage.window_key = key;
            usage.tokens = 0;
        }
        usage.tokens = usage.tokens.saturating_add(tokens);
        *self.dirty.lock().expect("agent_usage dirty") = true;
    }

    pub fn flush_if_dirty(&self) -> anyhow::Result<()> {
        let dirty = *self.dirty.lock().expect("agent_usage dirty");
        if !dirty {
            return Ok(());
        }
        let guard = self.inner.lock().expect("agent_usage mutex");
        let json = serde_json::to_string_pretty(&*guard)?;
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let tmp = self.path.with_extension("json.tmp");
        std::fs::write(&tmp, json)?;
        std::fs::rename(&tmp, &self.path)?;
        *self.dirty.lock().expect("agent_usage dirty") = false;
        Ok(())
    }

    pub fn flush(&self) -> anyhow::Result<()> {
        *self.dirty.lock().expect("agent_usage dirty") = true;
        self.flush_if_dirty()
    }

    pub fn spawn_flush_task(self: &std::sync::Arc<Self>) {
        let store = self.clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(std::time::Duration::from_secs(5));
            interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            loop {
                interval.tick().await;
                if let Err(e) = store.flush_if_dirty() {
                    tracing::warn!(error = %e, "agent_usage flush failed");
                }
            }
        });
    }
}

fn window_key() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() / BUDGET_WINDOW_SECS)
        .unwrap_or(0)
}
