//! `ledger` 仓储（§9.2）：逐请求一行。
//!
//! 失败请求也要写（§8.4）：`served_*` 为空、`outcome = upstream_error`，
//! 不能因为“没成功”就丢掉审计事实。

use rusqlite::params;
use tr_contracts::{Cost, LedgerRecord, ReasonCode};

use crate::store::{Db, StoreError};

/// 追加一条账本。
pub fn insert(db: &Db, record: &LedgerRecord) -> Result<(), StoreError> {
    let connection = db.write();
    insert_with(&connection, record)
}

/// 事务内版本（`commit` 使用）。
pub(crate) fn insert_with(
    connection: &rusqlite::Connection,
    record: &LedgerRecord,
) -> Result<(), StoreError> {
    let preference_json = serde_json::to_string(&record.preference)?;
    let failover_json = serde_json::to_string(&record.failover)?;
    let attempts_json = serde_json::to_string(&record.attempts)?;
    let reason_codes_json = serde_json::to_string(&record.reason_codes)?;

    connection.execute(
        "INSERT INTO ledger (
            request_id, plan_id, session_key, branch_id, task_id,
            session_key_source, collision_suspect, ingress_protocol,
            selection_model, selection_level,
            dispatch_model, dispatch_provider, dispatch_level, dispatch_source,
            plan_mode, preference_json, failover_json, attempts_json, switches,
            reason_codes_json, outcome, http_status,
            input_tokens, cache_read_tokens, cache_write_tokens, output_tokens,
            answer_cost_micros, routing_cost_micros, facts_completeness,
            profile_revision, config_digest, ttft_ms, duration_ms, created_at
         ) VALUES (
            ?1, ?2, ?3, ?4, ?5,
            ?6, ?7, ?8,
            ?9, ?10,
            ?11, ?12, ?13, ?14,
            ?15, ?16, ?17, ?18, ?19,
            ?20, ?21, ?22,
            ?23, ?24, ?25, ?26,
            ?27, ?28, ?29,
            ?30, ?31, ?32, ?33, ?34
         )",
        params![
            record.request_id.to_string(),
            record.plan_id,
            record.session_key,
            record.branch_id,
            record.task_id,
            record.session_key_source.as_str(),
            i64::from(record.collision_suspect),
            record.ingress_protocol.as_str(),
            record.selected_virtual_model,
            record.selected_thinking_level,
            record.served_physical_model,
            record.served_provider,
            record.served_thinking_level,
            record.dispatch_source.as_str(),
            record.plan_mode.as_str(),
            preference_json,
            failover_json,
            attempts_json,
            i64::from(record.switches),
            reason_codes_json,
            record.outcome.as_str(),
            record.http_status.map(i64::from),
            record.input_tokens.map(i64::from),
            record.cache_read_tokens.map(|value| value as i64),
            record.cache_write_tokens.map(|value| value as i64),
            record.output_tokens.map(i64::from),
            record.answer_cost.as_ref().map(|cost| cost.micros),
            record.routing_cost.as_ref().map(|cost| cost.micros),
            record.facts_completeness.as_str(),
            record.profile_revision,
            record.config_digest,
            record.ttft_ms.map(|value| value as i64),
            record.duration_ms as i64,
            record.created_at as i64
        ],
    )?;
    Ok(())
}

/// 账本总行数（测试与 `doctor` 使用）。
pub fn count(db: &Db) -> Result<i64, StoreError> {
    let connection = db.read();
    let count = connection.query_row("SELECT COUNT(*) FROM ledger", [], |row| row.get(0))?;
    Ok(count)
}

/// 追加理由码（会话 CAS 冲突后补记 `STATE_CAS_CONFLICT`）。
pub fn replace_reason_codes(
    db: &Db,
    request_id: &str,
    codes: &[ReasonCode],
) -> Result<(), StoreError> {
    let json = serde_json::to_string(codes)?;
    let connection = db.write();
    connection.execute(
        "UPDATE ledger SET reason_codes_json = ?2 WHERE request_id = ?1",
        params![request_id, json],
    )?;
    Ok(())
}

/// 读取单条账本的等价成本（用于测试与对账）。
pub fn cost_equiv_of(db: &Db, request_id: &str) -> Result<Option<Cost>, StoreError> {
    let connection = db.read();
    let mut statement = connection.prepare(
        "SELECT answer_cost_micros, routing_cost_micros FROM ledger WHERE request_id = ?1",
    )?;
    let mut rows = statement.query(params![request_id])?;
    let Some(row) = rows.next()? else {
        return Ok(None);
    };
    let answer: Option<i64> = row.get(0)?;
    let routing: Option<i64> = row.get(1)?;
    let micros = answer.unwrap_or(0) + routing.unwrap_or(0);
    Ok(Some(Cost {
        micros,
        currency: String::new(),
    }))
}
