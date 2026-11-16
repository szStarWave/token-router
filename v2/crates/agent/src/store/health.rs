//! `circuit_breakers` 仓储（§6.6）：候选健康度。
//!
//! 写健康度是 `upstream` 的职责；这里只负责持久化。`Failover` 策略读到的
//! `CandidateHealth` 必须来自这里或进程内缓存，不能来自上游探测（v2 不做主动探测）。

use std::collections::BTreeMap;

use rusqlite::{OptionalExtension, params};
use tr_contracts::{BreakerState, CandidateHealth};

use crate::store::{Db, StoreError};

/// half-open 探针占用窗口：窗口内只允许一个请求探测，避免雪崩式试探。
pub const HALF_OPEN_PROBE_WINDOW_SECS: u64 = 30;

/// 认领一次 half-open 探针（P0a-10）。
///
/// - 从未打开（`open_until = None`）→ 放行；
/// - 冷却未结束 → 拒绝；
/// - 冷却已结束且窗口内无人认领 → 原子写入 `half_open_since` 并放行；
/// - 窗口内已有探针 → 拒绝，等它结束。
pub fn acquire_half_open_probe(
    db: &Db,
    candidate_id: &str,
    now_unix: u64,
) -> Result<bool, StoreError> {
    let connection = db.write();
    let state: Option<(Option<i64>, Option<i64>)> = connection
        .query_row(
            "SELECT open_until, half_open_since FROM circuit_breakers WHERE candidate_id = ?1",
            params![candidate_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?;
    let Some((open_until, half_open_since)) = state else {
        return Ok(true);
    };
    let Some(open_until) = open_until else {
        return Ok(true);
    };
    let open_until = open_until.max(0) as u64;
    if open_until > now_unix {
        return Ok(false);
    }

    let window_start = now_unix.saturating_sub(HALF_OPEN_PROBE_WINDOW_SECS);
    if half_open_since.is_some_and(|since| since.max(0) as u64 > window_start) {
        return Ok(false);
    }

    let updated = connection.execute(
        "UPDATE circuit_breakers SET half_open_since = ?2
          WHERE candidate_id = ?1
            AND (half_open_since IS NULL OR half_open_since <= ?3)",
        params![candidate_id, now_unix as i64, window_start as i64],
    )?;
    Ok(updated == 1)
}

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
