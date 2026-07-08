use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::Duration;

use super::data::{self, ModelCapability, MultimodalData, TierSupport};
use super::fingerprint::upstream_fingerprint;
use crate::gateway::config::AppConfig;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MultimodalRouteHint {
    CachedEdge,
    CachedCloud,
    CachedEdgeFallback,
    Probe,
}

pub struct MultimodalStore {
    inner: Mutex<MultimodalData>,
    path: PathBuf,
    dirty: AtomicBool,
}

impl MultimodalStore {
    pub fn open(data_dir: &Path) -> anyhow::Result<std::sync::Arc<Self>> {
        let path = data_dir.join("multimodal.json");
        let data = data::load(&path)?;
        Ok(std::sync::Arc::new(Self {
            inner: Mutex::new(data),
            path,
            dirty: AtomicBool::new(false),
        }))
    }

    #[cfg(test)]
    pub fn new_in_memory() -> std::sync::Arc<Self> {
        std::sync::Arc::new(Self {
            inner: Mutex::new(MultimodalData::default()),
            path: PathBuf::from("/tmp/flowy-test-multimodal.json"),
            dirty: AtomicBool::new(false),
        })
    }

    pub fn route_hint(&self, config: &AppConfig, model: &str) -> MultimodalRouteHint {
        self.sync_fingerprint(config);
        let data = self.inner.lock().expect("multimodal mutex");
        let cap = data.by_model.get(model).cloned().unwrap_or_default();
        route_hint_from_capability(&cap)
    }

    pub fn record_edge(&self, config: &AppConfig, model: &str, supported: bool) {
        self.sync_fingerprint(config);
        self.with_mut(|data| {
            let entry = data.model_entry(model);
            entry.edge = if supported {
                TierSupport::Supported
            } else {
                TierSupport::Unsupported
            };
            entry.probed_at_unix = Some(data::now_unix());
        });
    }

    pub fn record_cloud(&self, config: &AppConfig, model: &str, supported: bool) {
        self.sync_fingerprint(config);
        self.with_mut(|data| {
            let entry = data.model_entry(model);
            entry.cloud = if supported {
                TierSupport::Supported
            } else {
                TierSupport::Unsupported
            };
            entry.probed_at_unix = Some(data::now_unix());
        });
    }

    pub fn flush_if_dirty(&self) -> anyhow::Result<()> {
        if !self.dirty.swap(false, Ordering::AcqRel) {
            return Ok(());
        }
        let data = self.inner.lock().expect("multimodal mutex").clone();
        data::save(&self.path, &data)
    }

    pub fn flush(&self) -> anyhow::Result<()> {
        self.dirty.store(true, Ordering::Release);
        self.flush_if_dirty()
    }

    pub fn spawn_flush_task(self: &std::sync::Arc<Self>) {
        let store = self.clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(5));
            interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            loop {
                interval.tick().await;
                if let Err(e) = store.flush_if_dirty() {
                    tracing::warn!(error = %e, "multimodal capability flush failed");
                }
            }
        });
    }

    fn sync_fingerprint(&self, config: &AppConfig) {
        let fp = upstream_fingerprint(config);
        self.with_mut(|data| {
            if data.upstream_fingerprint != fp {
                data.upstream_fingerprint = fp;
                data.by_model.clear();
            }
        });
    }

    fn with_mut(&self, f: impl FnOnce(&mut MultimodalData)) {
        let mut guard = self.inner.lock().expect("multimodal mutex");
        f(&mut guard);
        guard.touch();
        self.dirty.store(true, Ordering::Release);
    }
}

fn route_hint_from_capability(cap: &ModelCapability) -> MultimodalRouteHint {
    if cap.edge == TierSupport::Supported {
        return MultimodalRouteHint::CachedEdge;
    }
    if cap.edge == TierSupport::Unsupported && cap.cloud == TierSupport::Supported {
        return MultimodalRouteHint::CachedCloud;
    }
    if cap.edge == TierSupport::Unsupported
        && cap.cloud == TierSupport::Unsupported
        && cap.probed_at_unix.is_some()
    {
        return MultimodalRouteHint::CachedEdgeFallback;
    }
    MultimodalRouteHint::Probe
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gateway::config::AppConfig;
    use crate::config::{ConfigFile, UpstreamEndpoint};

    fn test_config(edge_url: &str, cloud_url: &str) -> AppConfig {
        let mut file = ConfigFile::default();
        file.upstream.edge = Some(UpstreamEndpoint {
            base_url: edge_url.into(),
            api_key: None,
            model: None,
        });
        file.upstream.cloud = Some(UpstreamEndpoint {
            base_url: cloud_url.into(),
            api_key: None,
            model: None,
        });
        AppConfig::from_file(file, std::path::PathBuf::from("/tmp/flowy-test"))
            .unwrap()
    }

    #[test]
    fn fingerprint_change_clears_cache() {
        let store = MultimodalStore::new_in_memory();
        let cfg_a = test_config("http://127.0.0.1:11434/v1", "https://api.example.com/v1");
        store.record_edge(&cfg_a, "m1", true);
        assert_eq!(
            store.route_hint(&cfg_a, "m1"),
            MultimodalRouteHint::CachedEdge
        );

        let cfg_b = test_config("http://127.0.0.1:11435/v1", "https://api.example.com/v1");
        assert_eq!(store.route_hint(&cfg_b, "m1"), MultimodalRouteHint::Probe);
    }

    #[test]
    fn cached_routes_follow_probe_results() {
        let store = MultimodalStore::new_in_memory();
        let cfg = test_config("http://127.0.0.1:11434/v1", "https://api.example.com/v1");

        assert_eq!(store.route_hint(&cfg, "vision"), MultimodalRouteHint::Probe);

        store.record_edge(&cfg, "vision", false);
        assert_eq!(store.route_hint(&cfg, "vision"), MultimodalRouteHint::Probe);

        store.record_cloud(&cfg, "vision", true);
        assert_eq!(
            store.route_hint(&cfg, "vision"),
            MultimodalRouteHint::CachedCloud
        );

        store.record_edge(&cfg, "vision2", false);
        store.record_cloud(&cfg, "vision2", false);
        assert_eq!(
            store.route_hint(&cfg, "vision2"),
            MultimodalRouteHint::CachedEdgeFallback
        );
    }
}
