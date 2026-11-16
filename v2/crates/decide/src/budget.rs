//! 预算（§7、§12）：turn / minute / session 三级，防判官把用户流量挤掉。
//!
//! 预算池与审批池分离：审批延迟不受判官 deadline 约束，反之亦然。

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Instant;

/// 判官预算（配置项）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Budget {
    pub max_calls_per_turn: u32,
    pub max_calls_per_minute: u32,
    pub max_calls_per_session: u32,
    pub deadline_ms: u32,
}

impl Default for Budget {
    fn default() -> Self {
        Self {
            max_calls_per_turn: 1,
            max_calls_per_minute: 30,
            max_calls_per_session: 200,
            deadline_ms: 300,
        }
    }
}

/// 预算判定结果。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BudgetDecision {
    Allowed,
    TurnExceeded,
    MinuteExceeded,
    SessionExceeded,
}

#[derive(Debug, Default)]
struct SessionUsage {
    turn: u32,
    minute_window_start: Option<Instant>,
    minute: u32,
    session: u32,
}

/// 进程内预算跟踪器。`agent` 持有，按会话计数。
#[derive(Debug, Default)]
pub struct BudgetTracker {
    sessions: Mutex<HashMap<String, SessionUsage>>,
}

impl BudgetTracker {
    pub fn new() -> Self {
        Self::default()
    }

    /// 尝试消耗一次判官调用。`turn_key` 用于区分轮次（如 plan_id）。
    pub fn try_consume(&self, session: &str, _turn_key: &str, budget: Budget) -> BudgetDecision {
        let mut sessions = self
            .sessions
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let usage = sessions.entry(session.to_string()).or_default();

        // 轮次由 `start_turn` 显式重置，避免依赖时间/哈希猜测。
        if usage.turn >= budget.max_calls_per_turn {
            return BudgetDecision::TurnExceeded;
        }

        let now = Instant::now();
        match usage.minute_window_start {
            Some(start) if now.duration_since(start).as_secs() < 60 => {}
            _ => {
                usage.minute_window_start = Some(now);
                usage.minute = 0;
            }
        }
        if usage.minute >= budget.max_calls_per_minute {
            return BudgetDecision::MinuteExceeded;
        }
        if usage.session >= budget.max_calls_per_session {
            return BudgetDecision::SessionExceeded;
        }

        usage.turn += 1;
        usage.minute += 1;
        usage.session += 1;
        BudgetDecision::Allowed
    }

    /// 每轮开始重置 turn 计数（由编排层显式调用，避免依赖时间猜测）。
    pub fn start_turn(&self, session: &str) {
        let mut sessions = self
            .sessions
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let usage = sessions.entry(session.to_string()).or_default();
        usage.turn = 0;
    }

    /// 已用会话额度（诊断）。
    pub fn session_usage(&self, session: &str) -> u32 {
        let sessions = self
            .sessions
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        sessions.get(session).map_or(0, |usage| usage.session)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn turn_budget_blocks_second_call() {
        let tracker = BudgetTracker::new();
        let budget = Budget {
            max_calls_per_turn: 1,
            ..Budget::default()
        };
        assert_eq!(
            tracker.try_consume("s", "turn", budget),
            BudgetDecision::Allowed
        );
        assert_eq!(
            tracker.try_consume("s", "turn", budget),
            BudgetDecision::TurnExceeded
        );
    }

    #[test]
    fn start_turn_resets_allowance() {
        let tracker = BudgetTracker::new();
        let budget = Budget {
            max_calls_per_turn: 1,
            ..Budget::default()
        };
        assert_eq!(
            tracker.try_consume("s", "t1", budget),
            BudgetDecision::Allowed
        );
        tracker.start_turn("s");
        assert_eq!(
            tracker.try_consume("s", "t2", budget),
            BudgetDecision::Allowed
        );
    }

    #[test]
    fn session_budget_is_cumulative() {
        let tracker = BudgetTracker::new();
        let budget = Budget {
            max_calls_per_turn: 10,
            max_calls_per_minute: 10,
            max_calls_per_session: 2,
            ..Budget::default()
        };
        assert_eq!(
            tracker.try_consume("s", "t", budget),
            BudgetDecision::Allowed
        );
        assert_eq!(
            tracker.try_consume("s", "t", budget),
            BudgetDecision::Allowed
        );
        assert_eq!(
            tracker.try_consume("s", "t", budget),
            BudgetDecision::SessionExceeded
        );
        assert_eq!(tracker.session_usage("s"), 2);
    }
}
