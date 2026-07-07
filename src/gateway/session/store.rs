
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;

use super::data::{self, SessionData};
use crate::gateway::experience::RequestOutcome;
use crate::gateway::routing::RouteDecision;
use crate::gateway::routing::StepKind;
use crate::gateway::served_outcome::{CloudCacheSettings, ServedOutcome};

#[derive(Debug, Clone, Copy)]
pub struct CloudCacheState {
    pub anchor_unix: Option<u64>,
    pub peak_linear: f32,
}

pub struct SessionStore {
    sessions_dir: PathBuf,
    persist_enabled: bool,
    inner: Mutex<HashMap<String, SessionEntry>>,
    dirty_keys: Mutex<Vec<String>>,
}

struct SessionEntry {
    data: SessionData,
}

impl SessionStore {
    pub fn open(sessions_dir: PathBuf, persist_enabled: bool) -> anyhow::Result<std::sync::Arc<Self>> {
        if persist_enabled {
            std::fs::create_dir_all(&sessions_dir)?;
        }
        Ok(std::sync::Arc::new(Self {
            sessions_dir,
            persist_enabled,
            inner: Mutex::new(HashMap::new()),
            dirty_keys: Mutex::new(Vec::new()),
        }))
    }

    #[cfg(test)]
    pub fn new_in_memory() -> std::sync::Arc<Self> {
        std::sync::Arc::new(Self {
            sessions_dir: PathBuf::from("/tmp/flowy-test-sessions"),
            persist_enabled: false,
            inner: Mutex::new(HashMap::new()),
            dirty_keys: Mutex::new(Vec::new()),
        })
    }

    pub fn sessions_dir(&self) -> &Path {
        &self.sessions_dir
    }

    pub fn get_last_tok_in(&self, conversation_key: &str) -> Option<u32> {
        let data = self.get_or_load(conversation_key);
        if data.last_tok_in == 0 {
            None
        } else {
            Some(data.last_tok_in)
        }
    }

    pub fn cloud_cache_state(&self, conversation_key: &str) -> CloudCacheState {
        let data = self.get_or_load(conversation_key);
        CloudCacheState {
            anchor_unix: data.cloud_cache_anchor_unix,
            peak_linear: data.cloud_cache_peak_linear,
        }
    }

    #[cfg(test)]
    pub fn cloud_cache_anchor(&self, conversation_key: &str) -> Option<u64> {
        self.cloud_cache_state(conversation_key).anchor_unix
    }

    pub fn record_tokens(&self, conversation_key: &str, tok_in: u32) {
        self.with_mut(conversation_key, |data| {
            data.last_tok_in = tok_in;
        });
    }

    pub fn apply_served_outcome(
        &self,
        conversation_key: &str,
        decision: &RouteDecision,
        served: &ServedOutcome,
        settings: &CloudCacheSettings,
        assistant_failed_signal: bool,
    ) {
        let outcome = served.outcome;
        self.with_mut(conversation_key, |data| {
            data.last_route = Some(data::route_name(decision.route).to_string());
            data.last_fallback = Some(outcome.cascade_fallback);
            data.last_step_kind = Some(data::step_kind_name(decision.step_kind));
            data.last_assistant_failed = assistant_failed_signal;

            let edge_served_ok = served.served_tier == "edge"
                && outcome.edge_ok
                && !outcome.cascade_fallback
                && !outcome.upstream_error;

            if edge_served_ok {
                data.clear_cloud_cache();
                return;
            }

            let now = now_unix();
            if served.served_tier == "cloud" {
                let mut peak = settings.boost_max;
                if served.cached_tokens > 0 && served.prompt_tokens > 0 {
                    let ratio =
                        served.cached_tokens as f32 / served.prompt_tokens as f32;
                    peak = (peak * (1.0 + ratio * 0.25)).min(settings.boost_max);
                }
                data.refresh_cloud_cache(now, peak);
            } else if decision.force_cloud_sticky
                || outcome.should_set_cloud_sticky(decision.step_kind)
            {
                data.refresh_cloud_cache(now, settings.boost_max);
            }
        });
    }

    /// Legacy path for tests that still call the old signature.
    #[cfg(test)]
    pub fn apply_outcome(
        &self,
        conversation_key: &str,
        decision: &RouteDecision,
        outcome: RequestOutcome,
        _ttl: u64,
        assistant_failed_signal: bool,
    ) {
        let served_tier = if outcome.cascade_fallback {
            "cloud".to_string()
        } else if matches!(decision.route, crate::gateway::routing::RouteTier::Cloud) {
            "cloud".to_string()
        } else if outcome.edge_ok {
            "edge".to_string()
        } else {
            "edge".to_string()
        };
        let served = ServedOutcome {
            outcome,
            served_tier,
            served_model: "test".to_string(),
            cached_tokens: 0,
            prompt_tokens: decision.tokens_in_estimate,
        };
        let settings = CloudCacheSettings {
            boost_max: 0.18,
            decay_half_life_secs: 600,
            route_cache_enabled: false,
        };
        self.apply_served_outcome(
            conversation_key,
            decision,
            &served,
            &settings,
            assistant_failed_signal,
        );
    }

    fn get_or_load(&self, conversation_key: &str) -> SessionData {
        let mut guard = self.inner.lock().expect("session mutex");
        if let Some(entry) = guard.get(conversation_key) {
            return entry.data.clone();
        }
        let data = if self.persist_enabled {
            let path = self.session_path(conversation_key);
            data::load(&path).unwrap_or_default()
        } else {
            SessionData::default()
        };
        guard.insert(
            conversation_key.to_string(),
            SessionEntry { data: data.clone() },
        );
        data
    }

    fn with_mut(&self, conversation_key: &str, f: impl FnOnce(&mut SessionData)) {
        {
            let mut guard = self.inner.lock().expect("session mutex");
            if !guard.contains_key(conversation_key) {
                let data = if self.persist_enabled {
                    let path = self.session_path(conversation_key);
                    data::load(&path).unwrap_or_default()
                } else {
                    SessionData::default()
                };
                guard.insert(
                    conversation_key.to_string(),
                    SessionEntry { data },
                );
            }
            if let Some(entry) = guard.get_mut(conversation_key) {
                f(&mut entry.data);
                entry.data.version = data::SESSION_VERSION;
                entry.data.last_updated_unix = now_unix();
            }
        }
        if self.persist_enabled {
            if let Ok(mut dirty) = self.dirty_keys.lock() {
                if !dirty.iter().any(|k| k == conversation_key) {
                    dirty.push(conversation_key.to_string());
                }
            }
        }
    }

    fn session_path(&self, conversation_key: &str) -> PathBuf {
        let safe: String = conversation_key
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                    c
                } else {
                    '_'
                }
            })
            .collect();
        self.sessions_dir.join(format!("{safe}.json"))
    }

    pub fn flush_if_dirty(&self) -> anyhow::Result<()> {
        if !self.persist_enabled {
            return Ok(());
        }
        let keys: Vec<String> = self
            .dirty_keys
            .lock()
            .map(|mut d| std::mem::take(&mut *d))
            .unwrap_or_default();
        if keys.is_empty() {
            return Ok(());
        }
        let guard = self.inner.lock().expect("session mutex");
        for key in keys {
            if let Some(entry) = guard.get(&key) {
                data::save(&self.session_path(&key), &entry.data)?;
            }
        }
        Ok(())
    }

    pub fn flush(&self) -> anyhow::Result<()> {
        if !self.persist_enabled {
            return Ok(());
        }
        let keys: Vec<String> = self
            .inner
            .lock()
            .expect("session mutex")
            .keys()
            .cloned()
            .collect();
        for key in keys {
            let guard = self.inner.lock().expect("session mutex");
            if let Some(entry) = guard.get(&key) {
                data::save(&self.session_path(&key), &entry.data)?;
            }
        }
        if let Ok(mut dirty) = self.dirty_keys.lock() {
            dirty.clear();
        }
        Ok(())
    }

    pub fn spawn_flush_task(self: &std::sync::Arc<Self>) {
        let store = self.clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(5));
            interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            loop {
                interval.tick().await;
                if let Err(e) = store.flush_if_dirty() {
                    tracing::warn!(error = %e, "session flush failed");
                }
            }
        });
    }

    pub fn spawn_cleanup_task(
        self: &std::sync::Arc<Self>,
        retention_days: u64,
        cleanup_interval_secs: u64,
    ) {
        let store = self.clone();
        let interval_secs = cleanup_interval_secs.max(60);
        tokio::spawn(async move {
            if let Err(e) = store.cleanup(retention_days) {
                tracing::warn!(error = %e, "session cleanup failed");
            }
            let mut interval = tokio::time::interval(Duration::from_secs(interval_secs));
            interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            loop {
                interval.tick().await;
                if let Err(e) = store.cleanup(retention_days) {
                    tracing::warn!(error = %e, "session cleanup failed");
                }
            }
        });
    }

    pub fn cleanup(&self, retention_days: u64) -> anyhow::Result<CleanupStats> {
        if !self.persist_enabled {
            return Ok(CleanupStats::default());
        }

        let skip_paths = self.dirty_session_paths();
        let now = now_unix();
        let mut stats = CleanupStats::default();

        let entries = match std::fs::read_dir(&self.sessions_dir) {
            Ok(entries) => entries,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(stats),
            Err(e) => return Err(e.into()),
        };

        for entry in entries {
            let entry = entry?;
            let path = entry.path();
            if !path.is_file() {
                continue;
            }

            let file_name = path
                .file_name()
                .and_then(|s| s.to_str())
                .unwrap_or_default();

            if file_name.ends_with(".json.tmp") {
                if remove_session_file(&path, skip_paths.contains(&path)) {
                    stats.removed_invalid += 1;
                }
                continue;
            }

            if !file_name.ends_with(".json") {
                continue;
            }

            if skip_paths.contains(&path) {
                stats.kept += 1;
                continue;
            }

            let text = match std::fs::read_to_string(&path) {
                Ok(text) => text,
                Err(e) => {
                    tracing::warn!(path = %path.display(), error = %e, "session cleanup read failed");
                    stats.kept += 1;
                    continue;
                }
            };

            match serde_json::from_str::<SessionData>(&text) {
                Err(e) => {
                    tracing::warn!(
                        path = %path.display(),
                        error = %e,
                        "removing invalid session file"
                    );
                    if remove_session_file(&path, false) {
                        self.evict_path(&path);
                        stats.removed_invalid += 1;
                    }
                }
                Ok(data) => {
                    let mtime = file_mtime_unix(&path);
                    if data.is_expired(retention_days, mtime, now) {
                        if remove_session_file(&path, false) {
                            self.evict_path(&path);
                            stats.removed_expired += 1;
                        }
                    } else {
                        stats.kept += 1;
                    }
                }
            }
        }

        if stats.removed_invalid > 0 || stats.removed_expired > 0 {
            tracing::info!(
                removed_invalid = stats.removed_invalid,
                removed_expired = stats.removed_expired,
                kept = stats.kept,
                retention_days,
                "session cleanup"
            );
        }

        Ok(stats)
    }

    fn dirty_session_paths(&self) -> std::collections::HashSet<PathBuf> {
        let keys = self
            .dirty_keys
            .lock()
            .map(|d| d.clone())
            .unwrap_or_default();
        keys.into_iter()
            .map(|key| self.session_path(&key))
            .collect()
    }

    fn evict_path(&self, path: &Path) {
        let mut guard = self.inner.lock().expect("session mutex");
        guard.retain(|key, _| self.session_path(key) != path);
    }
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct CleanupStats {
    pub removed_invalid: u32,
    pub removed_expired: u32,
    pub kept: u32,
}

fn remove_session_file(path: &Path, skipped: bool) -> bool {
    if skipped {
        return false;
    }
    match std::fs::remove_file(path) {
        Ok(()) => true,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => false,
        Err(e) => {
            tracing::warn!(path = %path.display(), error = %e, "session cleanup delete failed");
            false
        }
    }
}

fn file_mtime_unix(path: &Path) -> Option<u64> {
    let modified = std::fs::metadata(path).ok()?.modified().ok()?;
    modified
        .duration_since(std::time::UNIX_EPOCH)
        .ok()
        .map(|d| d.as_secs())
}

fn now_unix() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}
