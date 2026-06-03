use std::path::Path;

use anyhow::Context;
use serde::{Deserialize, Serialize};

use crate::gateway::routing::{RouteTier, StepKind};

pub const SESSION_VERSION: u32 = 1;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SessionData {
    pub version: u32,
    pub last_tok_in: u32,
    #[serde(default)]
    pub cloud_sticky_until_unix: Option<u64>,
    #[serde(default)]
    pub last_assistant_failed: bool,
    #[serde(default)]
    pub last_route: Option<String>,
    #[serde(default)]
    pub last_fallback: Option<bool>,
    #[serde(default)]
    pub last_step_kind: Option<String>,
}

impl SessionData {
    pub fn cloud_sticky_active(&self) -> bool {
        self.cloud_sticky_until_unix
            .is_some_and(|until| now_unix() < until)
    }
}

pub fn load(path: &Path) -> anyhow::Result<SessionData> {
    if !path.exists() {
        return Ok(SessionData::default());
    }
    let text = std::fs::read_to_string(path)
        .with_context(|| format!("read session {}", path.display()))?;
    match serde_json::from_str(&text) {
        Ok(data) => Ok(data),
        Err(e) => {
            tracing::warn!(
                path = %path.display(),
                error = %e,
                "invalid session file, starting fresh"
            );
            Ok(SessionData::default())
        }
    }
}

pub fn save(path: &Path, data: &SessionData) -> anyhow::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("create session dir {}", parent.display()))?;
    }
    let tmp = path.with_extension("json.tmp");
    let json = serde_json::to_string_pretty(data)?;
    std::fs::write(&tmp, json).with_context(|| format!("write session {}", tmp.display()))?;
    std::fs::rename(&tmp, path).with_context(|| format!("rename session {}", path.display()))?;
    Ok(())
}

pub fn route_name(t: RouteTier) -> &'static str {
    match t {
        RouteTier::Edge => "edge",
        RouteTier::Cloud => "cloud",
        RouteTier::Cascade => "cascade",
    }
}

pub fn step_kind_name(k: StepKind) -> String {
    format!("{:?}", k).to_ascii_lowercase()
}

fn now_unix() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

