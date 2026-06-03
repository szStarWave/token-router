use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::Duration;

use serde::Serialize;

use super::data::{self, ExperienceData, StepExperience};
use super::outcome::RequestOutcome;
use crate::gateway::routing::StepKind;

#[derive(Debug, Clone)]
pub struct ExperienceSettings {
    pub enabled: bool,
    pub learning_rate: f32,
    pub max_bias: f32,
    pub target_fallback: f32,
}

impl Default for ExperienceSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            learning_rate: 0.08,
            max_bias: 0.12,
            target_fallback: 0.15,
        }
    }
}

pub struct ExperienceStore {
    inner: Mutex<ExperienceData>,
    path: PathBuf,
    dirty: AtomicBool,
    settings: Mutex<ExperienceSettings>,
}

pub const MIN_TRUST_SAMPLES: u64 = 3;

impl ExperienceStore {
    pub fn open(data_dir: &Path, settings: ExperienceSettings) -> anyhow::Result<std::sync::Arc<Self>> {
        let path = data_dir.join("experience.json");
        let data = data::load(&path)?;
        Ok(std::sync::Arc::new(Self {
            inner: Mutex::new(data),
            path,
            dirty: AtomicBool::new(false),
            settings: Mutex::new(settings),
        }))
    }

    #[cfg(test)]
    pub fn new_in_memory(settings: ExperienceSettings) -> std::sync::Arc<Self> {
        std::sync::Arc::new(Self {
            inner: Mutex::new(ExperienceData::default()),
            path: PathBuf::from("/tmp/flowy-test-experience.json"),
            dirty: AtomicBool::new(false),
            settings: Mutex::new(settings),
        })
    }

    pub fn update_settings(&self, settings: ExperienceSettings) {
        *self.settings.lock().expect("experience settings mutex") = settings;
    }

    fn settings(&self) -> ExperienceSettings {
        self.settings
            .lock()
            .expect("experience settings mutex")
            .clone()
    }

    pub fn experience_file(&self) -> &Path {
        &self.path
    }

    pub fn bias_for(&self, step_kind: StepKind) -> f32 {
        let settings = self.settings();
        if !settings.enabled {
            return 0.0;
        }
        let data = self.inner.lock().expect("experience mutex");
        let key = data::step_kind_key(step_kind);
        let Some(entry) = data.by_step.get(&key) else {
            return 0.0;
        };
        compute_bias(entry, &settings)
    }

    /// Enough cloud-verified edge successes to route work steps directly to edge.
    pub fn edge_trusted(&self, step_kind: StepKind) -> bool {
        let settings = self.settings();
        if !settings.enabled {
            return false;
        }
        let data = self.inner.lock().expect("experience mutex");
        let key = data::step_kind_key(step_kind);
        let Some(entry) = data.by_step.get(&key) else {
            return false;
        };
        is_edge_trusted(entry, &settings)
    }

    pub fn record_outcome(&self, step_kind: StepKind, outcome: RequestOutcome) {
        if !self.settings().enabled {
            return;
        }
        self.with_mut(|data| {
            let entry = data.step_entry(step_kind);
            if outcome.edge_ok {
                entry.edge_ok += 1;
            }
            if outcome.cascade_fallback {
                entry.cascade_fallback += 1;
            }
            if outcome.upstream_error {
                entry.upstream_error += 1;
            }
        });
    }

    fn with_mut(&self, f: impl FnOnce(&mut ExperienceData)) {
        let mut guard = self.inner.lock().expect("experience mutex");
        f(&mut guard);
        guard.touch();
        self.dirty.store(true, Ordering::Release);
    }

    pub fn flush_if_dirty(&self) -> anyhow::Result<()> {
        if !self.dirty.swap(false, Ordering::AcqRel) {
            return Ok(());
        }
        let data = self.inner.lock().expect("experience mutex").clone();
        data::save(&self.path, &data)
    }

    pub fn flush(&self) -> anyhow::Result<()> {
        self.dirty.store(true, Ordering::Release);
        self.flush_if_dirty()
    }

    pub fn snapshot(&self) -> ExperienceSnapshot {
        let settings = self.settings();
        let data = self.inner.lock().expect("experience mutex").clone();
        let mut steps: Vec<StepSnapshot> = data
            .by_step
            .iter()
            .map(|(name, entry)| step_snapshot(name, entry, &settings))
            .collect();
        steps.sort_by(|a, b| {
            b.total_outcomes
                .cmp(&a.total_outcomes)
                .then_with(|| a.step_kind.cmp(&b.step_kind))
        });

        let mut totals = ExperienceTotals {
            step_kinds: steps.len() as u64,
            ..ExperienceTotals::default()
        };
        for step in &steps {
            totals.edge_ok += step.edge_ok;
            totals.cascade_fallback += step.cascade_fallback;
            totals.upstream_error += step.upstream_error;
            totals.verified_total += step.verified_total;
            totals.total_outcomes += step.total_outcomes;
            if step.edge_trusted {
                totals.trusted_steps += 1;
            }
        }
        totals.fallback_rate = if totals.verified_total > 0 {
            totals.cascade_fallback as f64 / totals.verified_total as f64
        } else {
            0.0
        };
        totals.edge_success_rate = if totals.verified_total > 0 {
            totals.edge_ok as f64 / totals.verified_total as f64
        } else {
            0.0
        };

        ExperienceSnapshot {
            enabled: settings.enabled,
            experience_file: self.path.display().to_string(),
            last_updated_at_unix: data.last_updated_at_unix,
            settings: ExperienceSettingsSnapshot {
                learning_rate: settings.learning_rate,
                max_bias: settings.max_bias,
                target_fallback: settings.target_fallback,
                min_trust_samples: MIN_TRUST_SAMPLES,
            },
            totals,
            steps,
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
                    tracing::warn!(error = %e, "experience flush failed");
                }
            }
        });
    }
}

fn is_edge_trusted(entry: &StepExperience, settings: &ExperienceSettings) -> bool {
    let verified = entry.edge_ok + entry.cascade_fallback;
    verified >= MIN_TRUST_SAMPLES
        && (entry.cascade_fallback as f32 / verified as f32) <= settings.target_fallback
}

fn step_snapshot(
    name: &str,
    entry: &StepExperience,
    settings: &ExperienceSettings,
) -> StepSnapshot {
    let verified = entry.edge_ok + entry.cascade_fallback;
    let fallback_rate = if verified > 0 {
        entry.cascade_fallback as f64 / verified as f64
    } else {
        0.0
    };
    let edge_success_rate = if verified > 0 {
        entry.edge_ok as f64 / verified as f64
    } else {
        0.0
    };
    StepSnapshot {
        step_kind: name.to_string(),
        edge_ok: entry.edge_ok,
        cascade_fallback: entry.cascade_fallback,
        upstream_error: entry.upstream_error,
        verified_total: verified,
        total_outcomes: verified + entry.upstream_error,
        fallback_rate,
        edge_success_rate,
        bias: compute_bias(entry, settings),
        edge_trusted: settings.enabled && is_edge_trusted(entry, settings),
    }
}

fn compute_bias(entry: &StepExperience, settings: &ExperienceSettings) -> f32 {
    let total = entry.edge_ok + entry.cascade_fallback;
    if total == 0 {
        return 0.0;
    }
    let fallback_rate = entry.cascade_fallback as f32 / total as f32;
    let raw = settings.learning_rate * (fallback_rate - settings.target_fallback);
    raw.clamp(-settings.max_bias, settings.max_bias)
}

#[derive(Debug, Clone, Serialize)]
pub struct ExperienceSnapshot {
    pub enabled: bool,
    pub experience_file: String,
    pub last_updated_at_unix: Option<u64>,
    pub settings: ExperienceSettingsSnapshot,
    pub totals: ExperienceTotals,
    pub steps: Vec<StepSnapshot>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ExperienceSettingsSnapshot {
    pub learning_rate: f32,
    pub max_bias: f32,
    pub target_fallback: f32,
    pub min_trust_samples: u64,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct ExperienceTotals {
    pub step_kinds: u64,
    pub edge_ok: u64,
    pub cascade_fallback: u64,
    pub upstream_error: u64,
    pub verified_total: u64,
    pub total_outcomes: u64,
    pub fallback_rate: f64,
    pub edge_success_rate: f64,
    pub trusted_steps: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct StepSnapshot {
    pub step_kind: String,
    pub edge_ok: u64,
    pub cascade_fallback: u64,
    pub upstream_error: u64,
    pub verified_total: u64,
    pub total_outcomes: u64,
    pub fallback_rate: f64,
    pub edge_success_rate: f64,
    pub bias: f32,
    pub edge_trusted: bool,
}
