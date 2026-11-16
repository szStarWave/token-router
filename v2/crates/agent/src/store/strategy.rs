//! `strategy_states` / `virtual_model_states` 仓储（§8.2）：学习状态与 route 状态。
//!
//! 两者作用域不同：
//! - `strategy_states` 键 = `(session, branch, task, virtual_model, strategy)`；
//! - `virtual_model_states` 键 = `(session, branch, virtual_model)`，**不是 sticky**。
//!
//! 状态 blob 只由对应策略/route 解析，内核不得窥探。

use std::collections::BTreeMap;

use rusqlite::params;
use tr_contracts::{StrategyId, StrategyState, VirtualModelState};

use crate::store::Db;
use crate::store::StoreError;
use crate::store::sessions::SaveOutcome;

/// 读取某个任务 × 虚拟模型的全部策略状态。
pub fn load_strategy_states(
    db: &Db,
    session_key: &str,
    branch_id: &str,
    task_id: &str,
    virtual_model_id: &str,
) -> Result<BTreeMap<StrategyId, StrategyState>, StoreError> {
    let connection = db.read();
    let mut statement = connection.prepare(
        "SELECT strategy_id, state_json, version, updated_at
           FROM strategy_states
          WHERE session_key = ?1 AND branch_id = ?2 AND task_id = ?3 AND virtual_model_id = ?4",
    )?;
    let mut rows = statement.query(params![session_key, branch_id, task_id, virtual_model_id])?;

    let mut states = BTreeMap::new();
    while let Some(row) = rows.next()? {
        let strategy_id: String = row.get(0)?;
        let state_json: String = row.get(1)?;
        let version: i64 = row.get(2)?;
        let updated_at: i64 = row.get(3)?;
        states.insert(
            strategy_id.clone(),
            StrategyState {
                branch_id: branch_id.to_string(),
                task_id: task_id.to_string(),
                virtual_model_id: virtual_model_id.to_string(),
                strategy_id,
                state_version: 1,
                blob: serde_json::from_str(&state_json)?,
                updated_at: updated_at as u64,
                version: version as u64,
            },
        );
    }
    Ok(states)
}

/// 保存策略状态，带 CAS。
pub fn save_strategy_state(
    db: &Db,
    session_key: &str,
    state: &StrategyState,
) -> Result<SaveOutcome, StoreError> {
    let state_json = serde_json::to_string(&state.blob)?;
    let connection = db.write();

    if state.version == 0 {
        let inserted = connection.execute(
            "INSERT INTO strategy_states (
                session_key, branch_id, task_id, virtual_model_id, strategy_id,
                state_json, version, updated_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, 1, ?7)
             ON CONFLICT(session_key, branch_id, task_id, virtual_model_id, strategy_id)
             DO NOTHING",
            params![
                session_key,
                state.branch_id,
                state.task_id,
                state.virtual_model_id,
                state.strategy_id,
                state_json,
                state.updated_at as i64
            ],
        )?;
        return Ok(if inserted == 1 {
            SaveOutcome::Saved { version: 1 }
        } else {
            SaveOutcome::Conflict
        });
    }

    let expected = state.version as i64;
    let updated = connection.execute(
        "UPDATE strategy_states
            SET state_json = ?6, version = ?7, updated_at = ?8
          WHERE session_key = ?1 AND branch_id = ?2 AND task_id = ?3
            AND virtual_model_id = ?4 AND strategy_id = ?5 AND version = ?9",
        params![
            session_key,
            state.branch_id,
            state.task_id,
            state.virtual_model_id,
            state.strategy_id,
            state_json,
            expected + 1,
            state.updated_at as i64,
            expected
        ],
    )?;
    Ok(if updated == 1 {
        SaveOutcome::Saved {
            version: (expected + 1) as u64,
        }
    } else {
        SaveOutcome::Conflict
    })
}

/// 读取虚拟模型 route 状态。
pub fn load_virtual_model_state(
    db: &Db,
    session_key: &str,
    branch_id: &str,
    virtual_model_id: &str,
) -> Result<Option<VirtualModelState>, StoreError> {
    let connection = db.read();
    let mut statement = connection.prepare(
        "SELECT state_json, version FROM virtual_model_states
          WHERE session_key = ?1 AND branch_id = ?2 AND virtual_model_id = ?3",
    )?;
    let mut rows = statement.query(params![session_key, branch_id, virtual_model_id])?;
    let Some(row) = rows.next()? else {
        return Ok(None);
    };
    let state_json: String = row.get(0)?;
    let version: i64 = row.get(1)?;
    Ok(Some(VirtualModelState {
        session_key: session_key.to_string(),
        branch_id: branch_id.to_string(),
        virtual_model_id: virtual_model_id.to_string(),
        state: serde_json::from_str(&state_json)?,
        version: version as u64,
    }))
}

/// 保存虚拟模型 route 状态，带 CAS。
pub fn save_virtual_model_state(
    db: &Db,
    state: &VirtualModelState,
) -> Result<SaveOutcome, StoreError> {
    let connection = db.write();
    save_virtual_model_state_with(&connection, state)
}

/// 事务内版本（`commit` 使用）：与会话/账本同事务写入。
pub(crate) fn save_virtual_model_state_with(
    connection: &rusqlite::Connection,
    state: &VirtualModelState,
) -> Result<SaveOutcome, StoreError> {
    let state_json = serde_json::to_string(&state.state)?;

    if state.version == 0 {
        let inserted = connection.execute(
            "INSERT INTO virtual_model_states (
                session_key, branch_id, virtual_model_id, state_json, version, updated_at
             ) VALUES (?1, ?2, ?3, ?4, 1, ?5)
             ON CONFLICT(session_key, branch_id, virtual_model_id) DO NOTHING",
            params![
                state.session_key,
                state.branch_id,
                state.virtual_model_id,
                state_json,
                crate::store::migrate::now_millis()
            ],
        )?;
        return Ok(if inserted == 1 {
            SaveOutcome::Saved { version: 1 }
        } else {
            SaveOutcome::Conflict
        });
    }

    let expected = state.version as i64;
    let updated = connection.execute(
        "UPDATE virtual_model_states
            SET state_json = ?4, version = ?5, updated_at = ?6
          WHERE session_key = ?1 AND branch_id = ?2 AND virtual_model_id = ?3 AND version = ?7",
        params![
            state.session_key,
            state.branch_id,
            state.virtual_model_id,
            state_json,
            expected + 1,
            crate::store::migrate::now_millis(),
            expected
        ],
    )?;
    Ok(if updated == 1 {
        SaveOutcome::Saved {
            version: (expected + 1) as u64,
        }
    } else {
        SaveOutcome::Conflict
    })
}
