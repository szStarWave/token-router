//! `quota_windows` 仓储（v3 P5-B）。
//!
//! 语义：
//! - 每个候选一条最近观测；没有标准限流头时降级为**本地计数估算**
//!   （`estimated = 1`，remaining 由 429 次数与重置时间推断），不伪造精确值；
//! - 429 只累计 `consecutive_429`，**不**改熔断（配额是预算，健康是熔断）；
//! - 成功响应带限流头时覆盖为精确值并清零 429 计数。

use rusqlite::{OptionalExtension, params};
use tr_contracts::{QuotaWindowInfo, RateLimitSnapshot};

use crate::store::{Db, StoreError};

/// 配额观测（候选维度）。
pub fn record(
    db: &Db,
    candidate_id: &str,
    account_id: Option<&str>,
    provider_id: Option<&str>,
    snapshot: &RateLimitSnapshot,
    now_millis: i64,
) -> Result<(), StoreError> {
    let mut connection = db.write();
    let tx = connection.transaction()?;

    let existing: Option<(Option<i64>, Option<i64>, i64)> = tx
        .query_row(
            "SELECT remaining, reset_at, consecutive_429 FROM quota_windows WHERE candidate_id = ?1",
            params![candidate_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .optional()?;

    let consecutive_429 = if snapshot.from_429 {
        existing.as_ref().map(|(_, _, count)| *count).unwrap_or(0) + 1
    } else {
        0
    };

    // 429 且没有头：按「本窗口已耗尽」估算（estimated）。
    let (remaining, estimated) = match (snapshot.remaining, snapshot.from_429) {
        (Some(remaining), _) => (Some(remaining), 0i64),
        (None, true) => (Some(0), 1i64),
        (None, false) => (
            existing.as_ref().and_then(|(remaining, _, _)| *remaining),
            1i64,
        ),
    };
    let reset_at = snapshot
        .reset_at
        .or_else(|| existing.as_ref().and_then(|(_, reset, _)| *reset));
    let limit_value = snapshot.limit.or_else(|| {
        // 无 limit 头时保留旧值。
        tx.query_row(
            "SELECT limit_value FROM quota_windows WHERE candidate_id = ?1",
            params![candidate_id],
            |row| row.get::<_, Option<i64>>(0),
        )
        .optional()
        .ok()
        .flatten()
        .flatten()
    });

    tx.execute(
        "INSERT INTO quota_windows
            (candidate_id, account_id, provider_id, remaining, limit_value, reset_at, unit,
             estimated, consecutive_429, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
         ON CONFLICT(candidate_id) DO UPDATE SET
            account_id = COALESCE(excluded.account_id, quota_windows.account_id),
            provider_id = COALESCE(excluded.provider_id, quota_windows.provider_id),
            remaining = excluded.remaining,
            limit_value = COALESCE(excluded.limit_value, quota_windows.limit_value),
            reset_at = COALESCE(excluded.reset_at, quota_windows.reset_at),
            unit = CASE WHEN excluded.unit = 'unknown' THEN quota_windows.unit ELSE excluded.unit END,
            estimated = excluded.estimated,
            consecutive_429 = excluded.consecutive_429,
            updated_at = excluded.updated_at",
        params![
            candidate_id,
            account_id,
            provider_id,
            remaining,
            limit_value,
            reset_at,
            snapshot.unit,
            estimated,
            consecutive_429,
            now_millis,
        ],
    )?;
    tx.commit()?;
    Ok(())
}

/// 编译期为候选写入账号/厂商映射（无观测值，仅用于管理面分组）。
pub fn ensure_mapping(
    db: &Db,
    candidate_id: &str,
    account_id: Option<&str>,
    provider_id: Option<&str>,
) -> Result<(), StoreError> {
    db.write().execute(
        "INSERT INTO quota_windows (candidate_id, account_id, provider_id, unit, updated_at)
         VALUES (?1, ?2, ?3, 'unknown', 0)
         ON CONFLICT(candidate_id) DO UPDATE SET
            account_id = COALESCE(excluded.account_id, quota_windows.account_id),
            provider_id = COALESCE(excluded.provider_id, quota_windows.provider_id)",
        params![candidate_id, account_id, provider_id],
    )?;
    Ok(())
}

/// 全部配额窗口（管理面视图）。
pub fn list(db: &Db) -> Result<Vec<QuotaWindowInfo>, StoreError> {
    let connection = db.read();
    let mut statement = connection.prepare(
        "SELECT candidate_id, account_id, provider_id, remaining, limit_value, reset_at, unit,
                estimated, consecutive_429, updated_at
           FROM quota_windows ORDER BY updated_at DESC",
    )?;
    let items = statement
        .query_map([], |row| {
            Ok(QuotaWindowInfo {
                candidate_id: row.get(0)?,
                account_id: row.get(1)?,
                provider_id: row.get(2)?,
                remaining: row.get(3)?,
                limit: row.get(4)?,
                reset_at: row.get(5)?,
                unit: row.get(6)?,
                estimated: row.get::<_, i64>(7)? != 0,
                consecutive_429: row.get::<_, i64>(8)? as u32,
                updated_at: row.get(9)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(items)
}

/// 单个候选的配额快照（信号注入用）。
pub fn snapshot(db: &Db, candidate_id: &str) -> Result<Option<QuotaWindowInfo>, StoreError> {
    let connection = db.read();
    connection
        .query_row(
            "SELECT candidate_id, account_id, provider_id, remaining, limit_value, reset_at, unit,
                    estimated, consecutive_429, updated_at
               FROM quota_windows WHERE candidate_id = ?1",
            params![candidate_id],
            |row| {
                Ok(QuotaWindowInfo {
                    candidate_id: row.get(0)?,
                    account_id: row.get(1)?,
                    provider_id: row.get(2)?,
                    remaining: row.get(3)?,
                    limit: row.get(4)?,
                    reset_at: row.get(5)?,
                    unit: row.get(6)?,
                    estimated: row.get::<_, i64>(7)? != 0,
                    consecutive_429: row.get::<_, i64>(8)? as u32,
                    updated_at: row.get(9)?,
                })
            },
        )
        .optional()
        .map_err(Into::into)
}

/// 手动重置（管理面）：「上游充值/换窗口后立即恢复」。
pub fn reset(db: &Db, candidate_id: &str) -> Result<bool, StoreError> {
    let changed = db.write().execute(
        "DELETE FROM quota_windows WHERE candidate_id = ?1",
        params![candidate_id],
    )?;
    Ok(changed > 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn db() -> Db {
        Db::open_in_memory().unwrap()
    }

    fn make_snapshot(
        remaining: Option<i64>,
        from_429: bool,
        reset_at: Option<i64>,
    ) -> RateLimitSnapshot {
        RateLimitSnapshot {
            remaining,
            limit: None,
            reset_at,
            window_ms: None,
            unit: "requests".to_string(),
            from_429,
        }
    }

    #[test]
    fn record_success_headers_clears_429_count() {
        let db = db();
        record(
            &db,
            "c1",
            Some("a1"),
            Some("p1"),
            &make_snapshot(None, true, Some(1000)),
            10,
        )
        .unwrap();
        record(
            &db,
            "c1",
            Some("a1"),
            Some("p1"),
            &make_snapshot(Some(500), false, Some(2000)),
            20,
        )
        .unwrap();
        let info = snapshot(&db, "c1").unwrap().unwrap();
        assert_eq!(info.remaining, Some(500));
        assert_eq!(info.consecutive_429, 0);
        assert_eq!(info.reset_at, Some(2000));
        assert!(!info.estimated);
    }

    #[test]
    fn bare_429_marks_exhausted_estimated() {
        let db = db();
        record(&db, "c1", None, None, &make_snapshot(None, true, None), 10).unwrap();
        let info = snapshot(&db, "c1").unwrap().unwrap();
        assert_eq!(info.remaining, Some(0));
        assert!(info.estimated, "无标准头时是估算而非精确值");
        assert_eq!(info.consecutive_429, 1);
    }

    #[test]
    fn reset_deletes_row() {
        let db = db();
        record(
            &db,
            "c1",
            None,
            None,
            &make_snapshot(Some(1), false, None),
            10,
        )
        .unwrap();
        assert!(reset(&db, "c1").unwrap());
        assert!(snapshot(&db, "c1").unwrap().is_none());
        assert!(list(&db).unwrap().is_empty());
    }
}
