use std::sync::Mutex;

use super::adaptive::{EffectiveRouting, compute_effective_routing};
use crate::gateway::config::AppConfig;

pub struct AdaptiveTuner {
    state: Mutex<EffectiveRouting>,
}

impl AdaptiveTuner {
    pub fn new(initial: EffectiveRouting) -> Self {
        Self {
            state: Mutex::new(initial),
        }
    }

    pub fn refresh(&self, config: &AppConfig) -> EffectiveRouting {
        let effective = compute_effective_routing(config);
        *self.state.lock().expect("adaptive tuner mutex") = effective.clone();
        effective
    }

    pub fn snapshot(&self) -> EffectiveRouting {
        self.state
            .lock()
            .expect("adaptive tuner mutex")
            .clone()
    }

    /// Recompute routing knobs from config after a hot config change.
    pub fn recompute(&self, config: &AppConfig) {
        *self.state.lock().expect("adaptive tuner mutex") =
            compute_effective_routing(config);
    }
}
