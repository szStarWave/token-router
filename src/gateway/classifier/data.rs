use std::collections::HashMap;
use std::path::Path;

use anyhow::Context;
use serde::{Deserialize, Serialize};

pub const CLASSIFIER_VERSION: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Label {
    EdgeOk,
    CloudNeeded,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct LabelCounts {
    pub edge_ok: u64,
    pub cloud_needed: u64,
}

impl LabelCounts {
    pub fn total(&self) -> u64 {
        self.edge_ok + self.cloud_needed
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ClassifierMeta {
    pub total_updates: u64,
    pub decay_generation: u64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ClassifierData {
    pub version: u32,
    #[serde(default)]
    pub last_updated_at_unix: Option<u64>,
    #[serde(default)]
    pub last_retrain_at_unix: Option<u64>,
    #[serde(default)]
    pub prior: LabelCounts,
    #[serde(default)]
    pub features: HashMap<String, LabelCounts>,
    #[serde(default)]
    pub meta: ClassifierMeta,
}

impl ClassifierData {
    pub fn touch(&mut self) {
        self.version = CLASSIFIER_VERSION;
        self.last_updated_at_unix = Some(now_unix());
    }

    pub fn total_samples(&self) -> u64 {
        self.prior.total()
    }
}

pub fn load(path: &Path) -> anyhow::Result<ClassifierData> {
    if !path.exists() {
        return Ok(ClassifierData::default());
    }
    let text = std::fs::read_to_string(path)
        .with_context(|| format!("read classifier {}", path.display()))?;
    match serde_json::from_str(&text) {
        Ok(data) => Ok(data),
        Err(e) => {
            tracing::warn!(
                path = %path.display(),
                error = %e,
                "invalid classifier file, starting fresh"
            );
            Ok(ClassifierData::default())
        }
    }
}

pub fn save(path: &Path, data: &ClassifierData) -> anyhow::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("create classifier dir {}", parent.display()))?;
    }
    let tmp = path.with_extension("json.tmp");
    let json = serde_json::to_string_pretty(data)?;
    std::fs::write(&tmp, json).with_context(|| format!("write classifier {}", tmp.display()))?;
    std::fs::rename(&tmp, path).with_context(|| format!("rename classifier {}", path.display()))?;
    Ok(())
}

pub fn now_unix() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}
