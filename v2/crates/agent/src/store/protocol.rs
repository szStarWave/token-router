//! `protocol_state` 仓储（§10.4）：`previous_response_id` 映射。
//!
//! 只存"响应 id → 规范化输出/工具历史"，按 TTL 过期。缺失/过期时调用方必须按
//! "无历史"降级并记 `PROTOCOL_STATE_MISSING`，**不能**返回 500。

use rusqlite::params;
use serde_json::Value;

use crate::store::{Db, StoreError};

/// 保存一条协议状态。
#[allow(clippy::too_many_arguments)]
pub fn save(
    db: &Db,
    key: &str,
    session_key: &str,
    branch_id: &str,
    kind: &str,
    payload: &Value,
    ttl_secs: u64,
    now_unix: u64,
) -> Result<(), StoreError> {
    let payload_json = serde_json::to_string(payload)?;
    let connection = db.write();
    connection.execute(
        "INSERT INTO protocol_state (key, session_key, branch_id, kind, payload_json, created_at, expires_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
         ON CONFLICT(key) DO UPDATE SET
            payload_json = excluded.payload_json,
            expires_at   = excluded.expires_at",
        params![
            key,
            session_key,
            branch_id,
            kind,
            payload_json,
            now_unix as i64,
            (now_unix + ttl_secs) as i64
        ],
    )?;
    Ok(())
}

/// 读取未过期的协议状态；过期按缺失处理。
pub fn load(db: &Db, key: &str, now_unix: u64) -> Result<Option<Value>, StoreError> {
    let connection = db.read();
    let mut statement =
        connection.prepare("SELECT payload_json, expires_at FROM protocol_state WHERE key = ?1")?;
    let mut rows = statement.query(params![key])?;
    let Some(row) = rows.next()? else {
        return Ok(None);
    };
    let payload_json: String = row.get(0)?;
    let expires_at: i64 = row.get(1)?;
    if (expires_at as u64) <= now_unix {
        return Ok(None);
    }
    Ok(Some(serde_json::from_str(&payload_json)?))
}

/// 清理过期行，返回删除数量。
pub fn purge_expired(db: &Db, now_unix: u64) -> Result<usize, StoreError> {
    let connection = db.write();
    let removed = connection.execute(
        "DELETE FROM protocol_state WHERE expires_at <= ?1",
        params![now_unix as i64],
    )?;
    Ok(removed)
}
