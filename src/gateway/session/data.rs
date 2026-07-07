use std::path::Path;

use anyhow::Context;
use serde::{Deserialize, Serialize};

use crate::gateway::routing::{RouteTier, StepKind};

pub const SESSION_VERSION: u32 = 2;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SessionData {
    pub version: u32,
    pub last_tok_in: u32,
    #[serde(default)]
    pub cloud_cache_anchor_unix: Option<u64>,
    #[serde(default)]
    pub cloud_cache_peak_linear: f32,
    /// Legacy field; migrated on load into anchor/peak when still active.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cloud_sticky_until_unix: Option<u64>,
    #[serde(default)]
    pub last_assistant_failed: bool,
    #[serde(default)]
    pub last_route: Option<String>,
    #[serde(default)]
    pub last_fallback: Option<bool>,
    #[serde(default)]
    pub last_step_kind: Option<String>,
    #[serde(default)]
    pub last_updated_unix: u64,
}

impl SessionData {
    pub fn cloud_cache_active_at(&self, _now: u64) -> bool {
        self.cloud_cache_anchor_unix.is_some_and(|_| {
            self.cloud_cache_peak_linear > f32::EPSILON
        })
    }

    pub fn clear_cloud_cache(&mut self) {
        self.cloud_cache_anchor_unix = None;
        self.cloud_cache_peak_linear = 0.0;
        self.cloud_sticky_until_unix = None;
    }

    pub fn refresh_cloud_cache(&mut self, now: u64, peak: f32) {
        self.cloud_cache_anchor_unix = Some(now);
        self.cloud_cache_peak_linear = peak;
        self.cloud_sticky_until_unix = None;
    }

    pub fn migrate_legacy_sticky(&mut self, now: u64, default_peak: f32) {
        if let Some(until) = self.cloud_sticky_until_unix {
            if now < until {
                self.cloud_cache_anchor_unix = Some(now);
                self.cloud_cache_peak_linear = default_peak;
            }
            self.cloud_sticky_until_unix = None;
        }
    }

    pub fn last_activity_unix(&self, fallback_mtime: Option<u64>) -> u64 {
        if self.last_updated_unix > 0 {
            self.last_updated_unix
        } else {
            fallback_mtime.unwrap_or(0)
        }
    }

    pub fn is_expired(&self, retention_days: u64, fallback_mtime: Option<u64>, now: u64) -> bool {
        if retention_days == 0 {
            return false;
        }
        if self.cloud_cache_active_at(now) {
            return false;
        }
        let activity = self.last_activity_unix(fallback_mtime);
        if activity == 0 {
            return false;
        }
        activity.saturating_add(retention_days.saturating_mul(86_400)) < now
    }
}

pub fn load(path: &Path) -> anyhow::Result<SessionData> {
    if !path.exists() {
        return Ok(SessionData::default());
    }
    let text = std::fs::read_to_string(path)
        .with_context(|| format!("read session {}", path.display()))?;
    match serde_json::from_str::<SessionData>(&text) {
        Ok(mut data) => {
            data.migrate_legacy_sticky(now_unix(), 0.18);
            Ok(data)
        }
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
