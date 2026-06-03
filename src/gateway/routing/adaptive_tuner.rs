use std::sync::Mutex;
use std::time::{Duration, Instant};

use super::adaptive::{EffectiveRouting, compute_effective_routing};
use crate::gateway::config::AppConfig;
use crate::gateway::experience::ExperienceStore;
use crate::gateway::stats::GatewayStats;

const REFRESH_INTERVAL: Duration = Duration::from_secs(30);
const REFRESH_EVERY_N_REQUESTS: u64 = 40;

pub struct AdaptiveTuner {
    state: Mutex<TunerState>,
}

struct TunerState {
    effective: EffectiveRouting,
    last_refresh: Instant,
    requests_since_refresh: u64,
}

impl AdaptiveTuner {
    pub fn new(initial: EffectiveRouting) -> Self {
        Self {
            state: Mutex::new(TunerState {
                effective: initial,
                last_refresh: Instant::now(),
                requests_since_refresh: 0,
            }),
        }
    }

    pub fn refresh(
        &self,
        config: &AppConfig,
        experience: &ExperienceStore,
        stats: &GatewayStats,
    ) -> EffectiveRouting {
        let mut state = self.state.lock().expect("adaptive tuner mutex");
        state.requests_since_refresh += 1;
        let due = state.last_refresh.elapsed() >= REFRESH_INTERVAL
            || state.requests_since_refresh >= REFRESH_EVERY_N_REQUESTS;
        if due {
            let exp = experience.snapshot();
            let stats_data = stats.global_data();
            state.effective =
                compute_effective_routing(&config, &exp, Some(&stats_data), &config.adaptive_routing);
            state.last_refresh = Instant::now();
            state.requests_since_refresh = 0;
        }
        state.effective.clone()
    }

    pub fn snapshot(&self) -> EffectiveRouting {
        self.state
            .lock()
            .expect("adaptive tuner mutex")
            .effective
            .clone()
    }

    /// Recompute adaptive routing immediately after a hot config change.
    pub fn recompute(
        &self,
        config: &AppConfig,
        experience: &ExperienceStore,
        stats: &GatewayStats,
    ) {
        let exp = experience.snapshot();
        let stats_data = stats.global_data();
        let effective = compute_effective_routing(
            config,
            &exp,
            Some(&stats_data),
            &config.adaptive_routing,
        );
        let mut state = self.state.lock().expect("adaptive tuner mutex");
        state.effective = effective;
        state.last_refresh = Instant::now();
        state.requests_since_refresh = 0;
    }
}
