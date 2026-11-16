//! `sessions` 仓储（§8.2）：会话事实 + CAS。
//!
//! `version == 0` 表示“新会话”，走 INSERT；非 0 走 `UPDATE ... WHERE version = ?`，
//! 影响 0 行即冲突——由调用方决定丢弃还是重试（§8.4）。

use rusqlite::params;
use tr_contracts::SessionFacts;

use crate::store::{Db, StoreError};

/// 保存结果。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SaveOutcome {
    Saved { version: u64 },
    Conflict,
}

/// 读取会话；不存在返回 `None`（调用方按“无会话”降级）。
pub fn load(
    db: &Db,
    session_key: &str,
    branch_id: &str,
) -> Result<Option<SessionFacts>, StoreError> {
    let connection = db.read();
    let mut statement = connection.prepare(
        "SELECT facts_json, version FROM sessions WHERE session_key = ?1 AND branch_id = ?2",
    )?;
    let mut rows = statement.query(params![session_key, branch_id])?;
    let Some(row) = rows.next()? else {
        return Ok(None);
    };
    let json: String = row.get(0)?;
    let version: i64 = row.get(1)?;
    let mut facts: SessionFacts = serde_json::from_str(&json)?;
    facts.version = version as u64;
    Ok(Some(facts))
}

/// 保存会话事实，带 CAS。
pub fn save(db: &Db, facts: &SessionFacts) -> Result<SaveOutcome, StoreError> {
    let connection = db.write();
    save_with(&connection, facts)
}

/// 事务内版本（`commit` 使用）。
pub(crate) fn save_with(
    connection: &rusqlite::Connection,
    facts: &SessionFacts,
) -> Result<SaveOutcome, StoreError> {
    let json = serde_json::to_string(facts)?;

    if facts.version == 0 {
        let inserted = connection.execute(
            "INSERT INTO sessions (session_key, branch_id, facts_json, version, created_at, updated_at)
             VALUES (?1, ?2, ?3, 1, ?4, ?4)
             ON CONFLICT(session_key, branch_id) DO NOTHING",
            params![
                facts.session_key,
                facts.branch_id,
                json,
                facts.updated_at as i64
            ],
        )?;
        if inserted == 1 {
            Ok(SaveOutcome::Saved { version: 1 })
        } else {
            Ok(SaveOutcome::Conflict)
        }
    } else {
        let expected = facts.version as i64;
        let updated = connection.execute(
            "UPDATE sessions
                SET facts_json = ?3, version = ?4, updated_at = ?5
              WHERE session_key = ?1 AND branch_id = ?2 AND version = ?6",
            params![
                facts.session_key,
                facts.branch_id,
                json,
                expected + 1,
                facts.updated_at as i64,
                expected
            ],
        )?;
        if updated == 1 {
            Ok(SaveOutcome::Saved {
                version: (expected + 1) as u64,
            })
        } else {
            Ok(SaveOutcome::Conflict)
        }
    }
}
