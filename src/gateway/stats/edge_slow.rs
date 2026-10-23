//! Consecutive slow edge generations, used to raise the next request's difficulty.

use crate::gateway::routing::LONG_GEN_EDGE_TPS_MIN;

/// Completions shorter than this are ignored. The generation window is at least
/// one second, so a handful of tokens always looks slower than the threshold.
pub const EDGE_SLOW_MIN_COMPLETION_TOKENS: u32 = 16;

/// One streak level expires this long after the latest slow sample.
pub const EDGE_SLOW_DECAY_SECS: u64 = 180;

#[derive(Debug, Default)]
pub struct EdgeSlowStreak {
    streak: u32,
    last_slow_unix: u64,
}

impl EdgeSlowStreak {
    pub fn observe(&mut self, tps: f64, completion_tokens: u32, now: u64) {
        if completion_tokens < EDGE_SLOW_MIN_COMPLETION_TOKENS {
            return;
        }
        if tps < LONG_GEN_EDGE_TPS_MIN {
            self.streak = self.streak.saturating_add(1);
            self.last_slow_unix = now;
        } else {
            self.streak = 0;
            self.last_slow_unix = 0;
        }
    }

    /// Streak at `now`, reduced by one level per [`EDGE_SLOW_DECAY_SECS`].
    pub fn effective(&self, now: u64) -> u32 {
        if self.streak == 0 || self.last_slow_unix == 0 {
            return 0;
        }
        let elapsed = now.saturating_sub(self.last_slow_unix);
        let decay_steps = (elapsed / EDGE_SLOW_DECAY_SECS) as u32;
        self.streak.saturating_sub(decay_steps)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slow_samples_increment_and_fast_sample_resets() {
        let mut streak = EdgeSlowStreak::default();
        streak.observe(10.0, 40, 1_000);
        streak.observe(12.0, 40, 1_010);
        assert_eq!(streak.effective(1_010), 2);
        streak.observe(40.0, 40, 1_020);
        assert_eq!(streak.effective(1_020), 0);
    }

    #[test]
    fn short_completion_does_not_change_streak() {
        let mut streak = EdgeSlowStreak::default();
        streak.observe(10.0, 40, 1_000);
        streak.observe(1.0, 8, 1_005);
        streak.observe(80.0, 4, 1_006);
        assert_eq!(streak.effective(1_006), 1);
    }

    #[test]
    fn effective_streak_decays_one_level_per_window() {
        let mut streak = EdgeSlowStreak::default();
        streak.observe(10.0, 40, 1_000);
        streak.observe(10.0, 40, 1_000);
        streak.observe(10.0, 40, 1_000);
        assert_eq!(streak.effective(1_000), 3);
        assert_eq!(streak.effective(1_000 + EDGE_SLOW_DECAY_SECS), 2);
        assert_eq!(streak.effective(1_000 + EDGE_SLOW_DECAY_SECS * 3), 0);
    }
}
