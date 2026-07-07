use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::Duration;

use serde::Serialize;

use super::data::{self, ClassifierData, Label, LabelCounts};
use super::features::FeatureVector;
use super::model::{fuse_difficulty, label_from_outcome, predict, seed_heuristic_priors};
use crate::gateway::experience::RequestOutcome;
use crate::gateway::routing::{RouteTier, WorkStrategy};

#[derive(Debug, Clone)]
pub struct ClassifierSettings {
    pub enabled: bool,
    pub min_samples: u64,
    pub prior_alpha: f32,
    pub decay_half_life_hours: f64,
    pub prior_from_heuristic: bool,
    pub min_feature_count: f64,
}

impl Default for ClassifierSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            min_samples: 100,
            prior_alpha: 1.0,
            decay_half_life_hours: 168.0,
            prior_from_heuristic: true,
            min_feature_count: 0.5,
        }
    }
}

pub struct ClassifierStore {
    inner: Mutex<ClassifierData>,
    path: PathBuf,
    dirty: AtomicBool,
    settings: Mutex<ClassifierSettings>,
}

impl ClassifierStore {
    pub fn open(data_dir: &Path, settings: ClassifierSettings) -> anyhow::Result<std::sync::Arc<Self>> {
        let path = data_dir.join("classifier.json");
        let mut data = data::load(&path)?;
        if settings.prior_from_heuristic && data.prior.total() == 0 && data.features.is_empty() {
            seed_heuristic_priors(&mut data);
            data.touch();
        }
        Ok(std::sync::Arc::new(Self {
            inner: Mutex::new(data),
            path,
            dirty: AtomicBool::new(false),
            settings: Mutex::new(settings),
        }))
    }

    #[cfg(test)]
    pub fn new_in_memory(settings: ClassifierSettings) -> std::sync::Arc<Self> {
        let mut data = ClassifierData::default();
        if settings.prior_from_heuristic {
            seed_heuristic_priors(&mut data);
        }
        std::sync::Arc::new(Self {
            inner: Mutex::new(data),
            path: PathBuf::from("/tmp/flowy-test-classifier.json"),
            dirty: AtomicBool::new(false),
            settings: Mutex::new(settings),
        })
    }

    pub fn update_settings(&self, settings: ClassifierSettings) {
        *self.settings.lock().expect("classifier settings mutex") = settings;
    }

    fn settings(&self) -> ClassifierSettings {
        self.settings
            .lock()
            .expect("classifier settings mutex")
            .clone()
    }

    pub fn classifier_file(&self) -> &Path {
        &self.path
    }

    pub fn predict_and_fuse(
        &self,
        features: &FeatureVector,
        d_heuristic: f32,
    ) -> ClassifierPrediction {
        let settings = self.settings();
        if !settings.enabled {
            return ClassifierPrediction {
                difficulty: d_heuristic,
                edge_ok_probability: None,
                bayes_weight: 0.0,
                warmed_up: false,
            };
        }

        let data = self.inner.lock().expect("classifier mutex");
        let total = data.total_samples();
        let p_edge = predict(&data, features, settings.prior_alpha);
        let (d_final, w) = fuse_difficulty(d_heuristic, p_edge, total, settings.min_samples);
        let warmed_up = total >= settings.min_samples;

        ClassifierPrediction {
            difficulty: d_final,
            edge_ok_probability: Some(p_edge),
            bayes_weight: w,
            warmed_up,
        }
    }

    pub fn record(
        &self,
        features: &FeatureVector,
        outcome: RequestOutcome,
        route: RouteTier,
        work_strategy: WorkStrategy,
        tool_error_streak: u32,
    ) {
        let settings = self.settings();
        if !settings.enabled {
            return;
        }
        if !super::model::should_record_outcome(outcome, route, work_strategy == WorkStrategy::Verify)
        {
            return;
        }
        let Some(label) = label_from_outcome(outcome, tool_error_streak) else {
            return;
        };
        self.with_mut(|data| {
            match label {
                Label::EdgeOk => data.prior.edge_ok += 1,
                Label::CloudNeeded => data.prior.cloud_needed += 1,
            }
            for key in &features.keys {
                let entry = data
                    .features
                    .entry(key.clone())
                    .or_default();
                match label {
                    Label::EdgeOk => entry.edge_ok += 1,
                    Label::CloudNeeded => entry.cloud_needed += 1,
                }
            }
            data.meta.total_updates += 1;
        });
    }

    pub fn decay_and_retrain(&self) -> anyhow::Result<()> {
        let settings = self.settings();
        if !settings.enabled || settings.decay_half_life_hours <= 0.0 {
            return Ok(());
        }

        let now = data::now_unix();
        self.with_mut(|data| {
            let last = data.last_retrain_at_unix.unwrap_or(data.last_updated_at_unix.unwrap_or(now));
            let elapsed_secs = now.saturating_sub(last) as f64;
            let half_life_secs = settings.decay_half_life_hours * 3600.0;
            if elapsed_secs < half_life_secs / 2.0 {
                return;
            }

            let factor = 0.5_f64.powf(elapsed_secs / half_life_secs);
            decay_counts(&mut data.prior, factor);

            let min_count = settings.min_feature_count;
            let mut merge_keys = Vec::new();
            for (key, counts) in data.features.iter_mut() {
                decay_counts(counts, factor);
                if counts.total() as f64 <= min_count {
                    merge_keys.push(key.clone());
                }
            }

            for key in merge_keys {
                if let Some(counts) = data.features.remove(&key) {
                    data.prior.edge_ok = data.prior.edge_ok.saturating_add(counts.edge_ok);
                    data.prior.cloud_needed = data
                        .prior
                        .cloud_needed
                        .saturating_add(counts.cloud_needed);
                }
            }

            data.last_retrain_at_unix = Some(now);
            data.meta.decay_generation += 1;
            tracing::info!(
                factor = factor,
                generation = data.meta.decay_generation,
                "classifier decay retrain applied"
            );
        });
        Ok(())
    }

    fn with_mut(&self, f: impl FnOnce(&mut ClassifierData)) {
        let mut guard = self.inner.lock().expect("classifier mutex");
        f(&mut guard);
        guard.touch();
        self.dirty.store(true, Ordering::Release);
    }

    pub fn flush_if_dirty(&self) -> anyhow::Result<()> {
        if !self.dirty.swap(false, Ordering::AcqRel) {
            return Ok(());
        }
        let data = self.inner.lock().expect("classifier mutex").clone();
        data::save(&self.path, &data)
    }

    pub fn flush(&self) -> anyhow::Result<()> {
        self.dirty.store(true, Ordering::Release);
        self.flush_if_dirty()
    }

    pub fn snapshot(&self) -> ClassifierSnapshot {
        let settings = self.settings();
        let data = self.inner.lock().expect("classifier mutex").clone();
        let mut features: Vec<FeatureSnapshot> = data
            .features
            .iter()
            .map(|(name, counts)| {
                let total = counts.total();
                let cloud_rate = if total > 0 {
                    counts.cloud_needed as f64 / total as f64
                } else {
                    0.0
                };
                FeatureSnapshot {
                    feature: name.clone(),
                    edge_ok: counts.edge_ok,
                    cloud_needed: counts.cloud_needed,
                    cloud_rate,
                }
            })
            .collect();
        features.sort_by(|a, b| {
            b.cloud_rate
                .partial_cmp(&a.cloud_rate)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| a.feature.cmp(&b.feature))
        });
        features.truncate(20);

        ClassifierSnapshot {
            enabled: settings.enabled,
            classifier_file: self.path.display().to_string(),
            last_updated_at_unix: data.last_updated_at_unix,
            last_retrain_at_unix: data.last_retrain_at_unix,
            total_samples: data.total_samples(),
            total_updates: data.meta.total_updates,
            decay_generation: data.meta.decay_generation,
            prior: data.prior.clone(),
            min_samples: settings.min_samples,
            prior_alpha: settings.prior_alpha,
            decay_half_life_hours: settings.decay_half_life_hours,
            top_cloud_features: features,
        }
    }

    pub fn spawn_flush_task(self: &std::sync::Arc<Self>) {
        let store = self.clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(5));
            interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            loop {
                interval.tick().await;
                if let Err(e) = store.flush_if_dirty() {
                    tracing::warn!(error = %e, "classifier flush failed");
                }
            }
        });
    }

    pub fn spawn_decay_task(self: &std::sync::Arc<Self>) {
        let store = self.clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(3600));
            interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            loop {
                interval.tick().await;
                if let Err(e) = store.decay_and_retrain() {
                    tracing::warn!(error = %e, "classifier decay retrain failed");
                }
            }
        });
    }
}

fn decay_counts(counts: &mut LabelCounts, factor: f64) {
    counts.edge_ok = ((counts.edge_ok as f64) * factor).round() as u64;
    counts.cloud_needed = ((counts.cloud_needed as f64) * factor).round() as u64;
}

#[derive(Debug, Clone)]
pub struct ClassifierPrediction {
    pub difficulty: f32,
    pub edge_ok_probability: Option<f32>,
    pub bayes_weight: f32,
    pub warmed_up: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct ClassifierSnapshot {
    pub enabled: bool,
    pub classifier_file: String,
    pub last_updated_at_unix: Option<u64>,
    pub last_retrain_at_unix: Option<u64>,
    pub total_samples: u64,
    pub total_updates: u64,
    pub decay_generation: u64,
    pub prior: LabelCounts,
    pub min_samples: u64,
    pub prior_alpha: f32,
    pub decay_half_life_hours: f64,
    pub top_cloud_features: Vec<FeatureSnapshot>,
}

#[derive(Debug, Clone, Serialize)]
pub struct FeatureSnapshot {
    pub feature: String,
    pub edge_ok: u64,
    pub cloud_needed: u64,
    pub cloud_rate: f64,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gateway::experience::RequestOutcome;

    #[test]
    fn record_increments_counts() {
        let store = ClassifierStore::new_in_memory(ClassifierSettings {
            prior_from_heuristic: false,
            ..Default::default()
        });
        let features = FeatureVector {
            keys: vec!["step_kind:direct_chat".into()],
        };
        store.record(
            &features,
            RequestOutcome {
                edge_ok: true,
                cascade_fallback: false,
                upstream_error: false,
            },
            RouteTier::Edge,
            WorkStrategy::None,
            0,
        );
        let snap = store.snapshot();
        assert_eq!(snap.total_updates, 1);
        assert_eq!(snap.prior.edge_ok, 1);
    }

    #[test]
    fn decay_reduces_counts() {
        let store = ClassifierStore::new_in_memory(ClassifierSettings {
            prior_from_heuristic: false,
            decay_half_life_hours: 1.0,
            min_feature_count: 0.0,
            ..Default::default()
        });
        let features = FeatureVector {
            keys: vec!["flag:multimodal".into()],
        };
        for _ in 0..10 {
            store.record(
                &features,
                RequestOutcome {
                    edge_ok: true,
                    cascade_fallback: false,
                    upstream_error: false,
                },
                RouteTier::Edge,
                WorkStrategy::None,
                0,
            );
        }
        {
            let mut data = store.inner.lock().unwrap();
            data.last_retrain_at_unix = Some(data::now_unix().saturating_sub(7200));
        }
        store.decay_and_retrain().unwrap();
        let snap = store.snapshot();
        assert!(snap.prior.edge_ok < 10);
    }
}
