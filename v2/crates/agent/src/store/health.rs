//! `circuit_breakers` 仓储（§6.6）：候选健康度。
//!
//! 写健康度是 `upstream` 的职责；这里只负责持久化。`Failover` 策略读到的
//! `CandidateHealth` 必须来自这里或进程内缓存，不能来自上游探测（v2 不做主动探测）。

use std::collections::BTreeMap;

use rusqlite::params;
use tr_contracts::{BreakerState, CandidateHealth};

use crate::store::{Db, StoreError};

/// 读取全部熔断状态。
pub fn load(db: &Db) -> Result<CandidateHealth, StoreError> {
    let connection = db.read();
    let mut statement = connection.prepare(
        "SELECT candidate_id, consecutive_failures, opened_count, open_until,
                half_open_since, last_error_class, last_failure_at, updated_at
           FROM circuit_breakers",
    )?;
    let mut rows = statement.query([])?;

    let mut candidates = BTreeMap::new();
    while let Some(row) = rows.next()? {
        let candidate_id: String = row.get(0)?;
        let last_error_class: Option<String> = row.get(5)?;
        candidates.insert(
            candidate_id,
            BreakerState {
                consecutive_failures: row.get::<_, i64>(1)? as u32,
                opened_count: row.get::<_, i64>(2)? as u32,
                open_until: row.get::<_, Option<i64>>(3)?.map(|value| value as u64),
                half_open_since: row.get::<_, Option<i64>>(4)?.map(|value| value as u64),
                last_error_class: last_error_class.and_then(|value| value.parse().ok()),
                last_failure_at: row.get::<_, Option<i64>>(6)?.map(|value| value as u64),
                updated_at: row.get::<_, i64>(7)? as u64,
            },
        );
    }
    Ok(CandidateHealth { candidates })
}

/// 写入单个候选的熔断状态（整体覆盖）。
pub fn upsert(db: &Db, candidate_id: &str, state: &BreakerState) -> Result<(), StoreError> {
    let connection = db.write();
    upsert_with(&connection, candidate_id, state)
}

/// 事务内版本（`commit` 使用）。
pub(crate) fn upsert_with(
    connection: &rusqlite::Connection,
    candidate_id: &str,
    state: &BreakerState,
) -> Result<(), StoreError> {
    connection.execute(
        "INSERT INTO circuit_breakers (
            candidate_id, consecutive_failures, opened_count, open_until,
            half_open_since, last_error_class, last_failure_at, updated_at
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
         ON CONFLICT(candidate_id) DO UPDATE SET
            consecutive_failures = excluded.consecutive_failures,
            opened_count         = excluded.opened_count,
            open_until           = excluded.open_until,
            half_open_since      = excluded.half_open_since,
            last_error_class     = excluded.last_error_class,
            last_failure_at      = excluded.last_failure_at,
            updated_at           = excluded.updated_at",
        params![
            candidate_id,
            i64::from(state.consecutive_failures),
            i64::from(state.opened_count),
            state.open_until.map(|value| value as i64),
            state.half_open_since.map(|value| value as i64),
            state.last_error_class.map(|class| class.as_str()),
            state.last_failure_at.map(|value| value as i64),
            state.updated_at as i64
        ],
    )?;
    Ok(())
}
