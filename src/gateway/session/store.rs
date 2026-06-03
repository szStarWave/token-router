use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;

use super::data::{self, SessionData};
use crate::gateway::experience::RequestOutcome;
use crate::gateway::routing::RouteDecision;

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

    pub fn cloud_sticky_until(&self, conversation_key: &str) -> Option<u64> {
        let data = self.get_or_load(conversation_key);
        if data.cloud_sticky_active() {
            data.cloud_sticky_until_unix
        } else {
            None
        }
    }

    pub fn record_tokens(&self, conversation_key: &str, tok_in: u32) {
        self.with_mut(conversation_key, |data| {
            data.last_tok_in = tok_in;
        });
    }

    pub fn apply_outcome(
        &self,
        conversation_key: &str,
        decision: &RouteDecision,
        outcome: RequestOutcome,
        cloud_sticky_ttl_secs: u64,
        assistant_failed_signal: bool,
    ) {
        self.with_mut(conversation_key, |data| {
            data.last_route = Some(data::route_name(decision.route).to_string());
            data.last_fallback = Some(outcome.cascade_fallback);
            data.last_step_kind = Some(data::step_kind_name(decision.step_kind));
            data.last_assistant_failed = assistant_failed_signal;

            if outcome.edge_ok && !outcome.cascade_fallback && !outcome.upstream_error {
                data.cloud_sticky_until_unix = None;
            } else if decision.force_cloud_sticky
                || outcome.should_set_cloud_sticky(decision.step_kind)
            {
                data.cloud_sticky_until_unix =
                    Some(now_unix().saturating_add(cloud_sticky_ttl_secs));
            }
        });
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
}

fn now_unix() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}
