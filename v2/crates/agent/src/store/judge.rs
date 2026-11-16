//! `judge_calls` / `shadow_records` 仓储（§7、§11.3）。
//!
//! 判官审计只存结构化字段：模型、耗时、token、结果类别、失败码；
//! **不存 prompt 正文**，避免把用户内容落进审计表。

use rusqlite::params;
use serde_json::Value;
use uuid::Uuid;

use crate::store::{Db, StoreError};

/// 一次判官调用审计。
#[derive(Debug, Clone)]
pub struct JudgeCallRecord {
    pub request_id: Uuid,
    pub question_id: String,
    pub service_id: String,
    pub revision: u64,
    pub protocol_digest: Option<String>,
    pub effective_source: String,
    pub outcome: String,
    pub failure_code: Option<String>,
    pub latency_ms: u32,
    pub prompt_tokens: Option<u32>,
    pub completion_tokens: Option<u32>,
    pub cost_micros: Option<i64>,
    pub created_at: u64,
}

/// 写一条判官调用。
pub fn insert_call(db: &Db, record: &JudgeCallRecord) -> Result<(), StoreError> {
    let connection = db.write();
    connection.execute(
        "INSERT INTO judge_calls (
            request_id, question_id, service_id, revision, protocol_digest,
            effective_source, outcome, failure_code, latency_ms,
            prompt_tokens, completion_tokens, cost_micros, created_at
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
        params![
            record.request_id.to_string(),
            record.question_id,
            record.service_id,
            record.revision as i64,
            record.protocol_digest,
            record.effective_source,
            record.outcome,
            record.failure_code,
            i64::from(record.latency_ms),
            record.prompt_tokens.map(i64::from),
            record.completion_tokens.map(i64::from),
            record.cost_micros,
            record.created_at as i64
        ],
    )?;
    Ok(())
}

/// 判官调用行数（测试/诊断）。
pub fn count_calls(db: &Db) -> Result<i64, StoreError> {
    let connection = db.read();
    Ok(connection.query_row("SELECT COUNT(*) FROM judge_calls", [], |row| row.get(0))?)
}

/// 判官调用累计成本（micros），用于对账 `routing_cost`。
pub fn total_call_cost_micros(db: &Db) -> Result<i64, StoreError> {
    let connection = db.read();
    Ok(connection.query_row(
        "SELECT COALESCE(SUM(cost_micros), 0) FROM judge_calls",
        [],
        |row| row.get(0),
    )?)
}

/// 影子记录（判官建议 vs 内核结论）。
#[derive(Debug, Clone)]
pub struct ShadowRecord {
    pub request_id: Uuid,
    pub selection_model: String,
    pub kernel_choice: String,
    pub judge_question_id: Option<String>,
    pub judge_value: Option<Value>,
    pub agreement: Option<bool>,
    pub effective_source: Option<String>,
    pub judge_latency_ms: Option<u32>,
    pub judge_cost_micros: Option<i64>,
    pub snapshot_digest: String,
    pub created_at: u64,
}

/// 写一条影子记录。
pub fn insert_shadow(db: &Db, record: &ShadowRecord) -> Result<(), StoreError> {
    let judge_value_json = record
        .judge_value
        .as_ref()
        .map(serde_json::to_string)
        .transpose()?;
    let connection = db.write();
    connection.execute(
        "INSERT INTO shadow_records (
            request_id, selection_model, kernel_choice, judge_question_id,
            judge_value_json, agreement, effective_source, judge_latency_ms,
            judge_cost_micros, snapshot_digest, created_at
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
        params![
            record.request_id.to_string(),
            record.selection_model,
            record.kernel_choice,
            record.judge_question_id,
            judge_value_json,
            record.agreement.map(i64::from),
            record.effective_source,
            record.judge_latency_ms.map(i64::from),
            record.judge_cost_micros,
            record.snapshot_digest,
            record.created_at as i64
        ],
    )?;
    Ok(())
}

/// 影子记录行数（测试/诊断）。
pub fn count_shadow(db: &Db) -> Result<i64, StoreError> {
    let connection = db.read();
    Ok(connection.query_row("SELECT COUNT(*) FROM shadow_records", [], |row| row.get(0))?)
}

/// shadow 报告（`cli shadow-report`）。
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ShadowSummary {
    pub total: i64,
    pub agreed: i64,
    pub disagreed: i64,
    pub unknown: i64,
    pub avg_latency_ms: Option<f64>,
}

impl ShadowSummary {
    /// 有判定的样本里的一致率；无样本返回 `None`。
    pub fn agreement_rate(&self) -> Option<f64> {
        let judged = self.agreed + self.disagreed;
        (judged > 0).then(|| self.agreed as f64 / judged as f64)
    }
}

/// 聚合 shadow 记录。
pub fn shadow_summary(db: &Db) -> Result<ShadowSummary, StoreError> {
    let connection = db.read();
    let (total, agreed, disagreed, unknown, avg): (i64, i64, i64, i64, Option<f64>) = connection
        .query_row(
            "SELECT COUNT(*),
                    COALESCE(SUM(CASE agreement WHEN 1 THEN 1 ELSE 0 END), 0),
                    COALESCE(SUM(CASE agreement WHEN 0 THEN 1 ELSE 0 END), 0),
                    COALESCE(SUM(CASE WHEN agreement IS NULL THEN 1 ELSE 0 END), 0),
                    AVG(judge_latency_ms)
               FROM shadow_records",
            [],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                ))
            },
        )?;
    Ok(ShadowSummary {
        total,
        agreed,
        disagreed,
        unknown,
        avg_latency_ms: avg,
    })
}
