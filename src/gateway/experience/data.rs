use std::collections::HashMap;
use std::path::Path;

use anyhow::Context;
use serde::{Deserialize, Serialize};

use crate::gateway::routing::StepKind;

pub const EXPERIENCE_VERSION: u32 = 1;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct StepExperience {
    pub edge_ok: u64,
    pub cascade_fallback: u64,
    pub upstream_error: u64,
    #[serde(default)]
    pub tool_failure: u64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ExperienceData {
    pub version: u32,
    #[serde(default)]
    pub last_updated_at_unix: Option<u64>,
    #[serde(default)]
    pub by_step: HashMap<String, StepExperience>,
}

impl ExperienceData {
    pub fn touch(&mut self) {
        self.version = EXPERIENCE_VERSION;
        self.last_updated_at_unix = Some(now_unix());
    }

    pub fn step_entry(&mut self, step_kind: StepKind) -> &mut StepExperience {
        let key = step_kind_key(step_kind);
        self.by_step.entry(key).or_default()
    }
}

pub fn load(path: &Path) -> anyhow::Result<ExperienceData> {
    if !path.exists() {
        return Ok(ExperienceData::default());
    }
    let text = std::fs::read_to_string(path)
        .with_context(|| format!("read experience {}", path.display()))?;
    match serde_json::from_str(&text) {
        Ok(data) => Ok(data),
        Err(e) => {
            tracing::warn!(
                path = %path.display(),
                error = %e,
                "invalid experience file, starting fresh"
            );
            Ok(ExperienceData::default())
        }
    }
}

pub fn save(path: &Path, data: &ExperienceData) -> anyhow::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("create experience dir {}", parent.display()))?;
    }
    let tmp = path.with_extension("json.tmp");
    let json = serde_json::to_string_pretty(data)?;
    std::fs::write(&tmp, json).with_context(|| format!("write experience {}", tmp.display()))?;
    std::fs::rename(&tmp, path).with_context(|| format!("rename experience {}", path.display()))?;
    Ok(())
}

pub fn step_kind_key(k: StepKind) -> String {
    format!("{:?}", k).to_ascii_lowercase()
}

fn now_unix() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}
