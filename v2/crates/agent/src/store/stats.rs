//! 账本聚合（P2-10 的指标底座）。
//!
//! 指标必须与账本**同源**：这里用 SQL 聚合 ledger，避免“日志统计”和“账本”两套口径。

use crate::store::{Db, StoreError};

/// 账本聚合。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct LedgerStats {
    pub requests: i64,
    pub successes: i64,
    pub upstream_errors: i64,
    pub quality_fails: i64,
    pub cancelled: i64,
    /// 审批/策略在本地拒绝的请求（§12，`rejected`）。
    pub rejected: i64,
    pub switches: i64,
    pub answer_cost_micros: i64,
    pub routing_cost_micros: i64,
    pub judge_fallbacks: i64,
    /// 判官调用审计（与 `judge_calls` 同源，P1-14/P2-10）。
    pub judge_calls: i64,
    pub judge_budget_denied: i64,
    pub judge_failed: i64,
    pub judge_cost_micros: i64,
}

impl LedgerStats {
    pub fn success_rate(&self) -> Option<f64> {
        (self.requests > 0).then(|| self.successes as f64 / self.requests as f64)
    }

    pub fn cost_micros(&self) -> i64 {
        self.answer_cost_micros + self.routing_cost_micros
    }
}

/// 聚合全部账本（P2 不做时间窗口；需要时加 WHERE created_at）。
pub fn stats(db: &Db) -> Result<LedgerStats, StoreError> {
    let connection = db.read();
    let stats = connection.query_row(
        "SELECT
            COUNT(*),
            COALESCE(SUM(CASE outcome WHEN 'success' THEN 1 ELSE 0 END), 0),
            COALESCE(SUM(CASE outcome WHEN 'upstream_error' THEN 1 ELSE 0 END), 0),
            COALESCE(SUM(CASE outcome WHEN 'quality_fail' THEN 1 ELSE 0 END), 0),
            COALESCE(SUM(CASE outcome WHEN 'cancelled' THEN 1 ELSE 0 END), 0),
            COALESCE(SUM(switches), 0),
            COALESCE(SUM(answer_cost_micros), 0),
            COALESCE(SUM(routing_cost_micros), 0),
            COALESCE(SUM(CASE WHEN json_extract(judge_json, '$.fallback_used') THEN 1 ELSE 0 END), 0),
            COALESCE(SUM(CASE outcome WHEN 'rejected' THEN 1 ELSE 0 END), 0)
         FROM ledger",
        [],
        |row| {
            Ok(LedgerStats {
                requests: row.get(0)?,
                successes: row.get(1)?,
                upstream_errors: row.get(2)?,
                quality_fails: row.get(3)?,
                cancelled: row.get(4)?,
                switches: row.get(5)?,
                answer_cost_micros: row.get(6)?,
                routing_cost_micros: row.get(7)?,
                judge_fallbacks: row.get(8)?,
                rejected: row.get(9)?,
                ..LedgerStats::default()
            })
        },
    )?;

    let judge = connection.query_row(
        "SELECT
            COUNT(*),
            COALESCE(SUM(CASE outcome WHEN 'budget_exceeded' THEN 1 ELSE 0 END), 0),
            COALESCE(SUM(CASE outcome WHEN 'failed' THEN 1 ELSE 0 END), 0),
            COALESCE(SUM(cost_micros), 0)
         FROM judge_calls",
        [],
        |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, i64>(2)?,
                row.get::<_, i64>(3)?,
            ))
        },
    )?;
    Ok(LedgerStats {
        judge_calls: judge.0,
        judge_budget_denied: judge.1,
        judge_failed: judge.2,
        judge_cost_micros: judge.3,
        ..stats
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rate_and_cost_are_derived() {
        let stats = LedgerStats {
            requests: 4,
            successes: 3,
            answer_cost_micros: 100,
            routing_cost_micros: 50,
            ..LedgerStats::default()
        };
        assert_eq!(stats.success_rate(), Some(0.75));
        assert_eq!(stats.cost_micros(), 150);
    }

    #[test]
    fn empty_stats_has_no_rate() {
        assert_eq!(LedgerStats::default().success_rate(), None);
    }
}
