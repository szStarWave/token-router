//! `judge_calls` / `shadow_records` 仓储（§7、§11.3）。
//!
//! 判官审计以结构化字段为主：模型、耗时、token、结果类别、失败码；
//! migration 16 起可选记录**调用详情**（任务文本、问题提示词、选项、答案、失败消息），
//! 由 `agent::judge_detail` 生成摘要，不存完整 HTTP 请求体。

use rusqlite::params;
use serde_json::Value;
use uuid::Uuid;

use crate::store::{Db, StoreError};

/// 判官调用的详细输入/输出（JSON 字符串；旧行/未记录时为 `None`）。
#[derive(Debug, Clone, Default, PartialEq)]
pub struct JudgeCallDetail {
    /// `{task, prompt, choices}`：任务与提示词截断，选项为 `{id,label}`。
    pub request_json: Option<String>,
    /// `{kind, value, confidence, reason}`。
    pub response_json: Option<String>,
    /// 失败详情（失败码之外的消息）。
    pub failure_message: Option<String>,
}

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
    /// 详细调用记录（可选）。
    pub detail: JudgeCallDetail,
}

/// 写一条判官调用。
pub fn insert_call(db: &Db, record: &JudgeCallRecord) -> Result<(), StoreError> {
    let connection = db.write();
    connection.execute(
        "INSERT INTO judge_calls (
            request_id, question_id, service_id, revision, protocol_digest,
            effective_source, outcome, failure_code, latency_ms,
            prompt_tokens, completion_tokens, cost_micros, created_at,
            request_json, response_json, failure_message
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16)",
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
            record.created_at as i64,
            record.detail.request_json,
            record.detail.response_json,
            record.detail.failure_message
        ],
    )?;
    Ok(())
}

/// 判官调用列表筛选（契约 `JudgeCallSummary`）。
///
/// `session_key` 需要 join 账本（`judge_calls` 本身不带会话维度）：账本里没有对应行
/// 的判官调用（请求在记账前就失败了）只在按 request_id 查时可见。
#[derive(Debug, Clone, Default)]
pub struct JudgeCallFilter {
    pub request_id: Option<String>,
    pub session_key: Option<String>,
    pub from_ms: Option<i64>,
    pub to_ms: Option<i64>,
    pub limit: u32,
}

/// 判官调用列表（契约 `JudgeCallSummary`）。DB 里 `created_at` 是秒，输出毫秒。
pub fn list_calls(
    db: &Db,
    filter: &JudgeCallFilter,
) -> Result<Vec<tr_contracts::JudgeCallSummary>, StoreError> {
    let from_secs = filter.from_ms.unwrap_or(0) / 1000;
    let to_secs = filter
        .to_ms
        .map(|ms| ms / 1000 + 1)
        .unwrap_or(i64::MAX / 1000 - 1);
    let limit = filter.limit.clamp(1, 500);

    const COLUMNS: &str =
        "jc.request_id, jc.question_id, jc.service_id, jc.outcome, jc.failure_code,
                           jc.latency_ms, jc.prompt_tokens, jc.completion_tokens, jc.cost_micros,
                           jc.created_at, jc.effective_source, jc.request_json, jc.response_json,
                           jc.failure_message";

    let connection = db.read();
    let mut out = Vec::new();
    if let Some(session_key) = &filter.session_key {
        let mut statement = connection.prepare(&format!(
            "SELECT {COLUMNS}
               FROM judge_calls jc JOIN ledger l ON l.request_id = jc.request_id
              WHERE l.session_key = ?1 AND jc.created_at >= ?2 AND jc.created_at < ?3
              ORDER BY jc.created_at DESC, jc.id DESC
              LIMIT ?4"
        ))?;
        let rows = statement.query_map(
            params![session_key, from_secs, to_secs, limit],
            judge_call_row,
        )?;
        for row in rows {
            out.push(row?);
        }
    } else {
        let mut statement = connection.prepare(&format!(
            "SELECT {COLUMNS}
               FROM judge_calls jc
              WHERE (?1 IS NULL OR jc.request_id = ?1)
                AND jc.created_at >= ?2 AND jc.created_at < ?3
              ORDER BY jc.created_at DESC, jc.id DESC
              LIMIT ?4"
        ))?;
        let rows = statement.query_map(
            params![filter.request_id, from_secs, to_secs, limit],
            judge_call_row,
        )?;
        for row in rows {
            out.push(row?);
        }
    }
    Ok(out)
}

fn judge_call_row(
    row: &rusqlite::Row<'_>,
) -> Result<tr_contracts::JudgeCallSummary, rusqlite::Error> {
    let created_at_secs: i64 = row.get(9)?;
    let parse =
        |text: Option<String>| text.and_then(|text| serde_json::from_str::<Value>(&text).ok());
    Ok(tr_contracts::JudgeCallSummary {
        request_id: row.get(0)?,
        question_id: row.get(1)?,
        service_id: row.get(2)?,
        outcome: row.get(3)?,
        failure_code: row.get(4)?,
        latency_ms: row.get::<_, Option<i64>>(5)?.unwrap_or(0),
        prompt_tokens: row.get(6)?,
        completion_tokens: row.get(7)?,
        cost_micros: row.get(8)?,
        effective_source: row.get::<_, Option<String>>(10)?.unwrap_or_default(),
        request: parse(row.get(11)?),
        response: parse(row.get(12)?),
        failure_message: row.get(13)?,
        created_at: created_at_secs.saturating_mul(1000),
    })
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
    /// P95 延迟（毫秒）；样本为空时为 `None`。
    pub p95_latency_ms: Option<f64>,
    /// 影子期间判官成本合计（micros，来自 `routing_cost`），用于预算消耗评估。
    pub total_cost_micros: i64,
}

impl ShadowSummary {
    /// 有判定的样本里的一致率；无样本返回 `None`。
    pub fn agreement_rate(&self) -> Option<f64> {
        let judged = self.agreed + self.disagreed;
        (judged > 0).then(|| self.agreed as f64 / judged as f64)
    }
}

fn percentile_95(latencies: &mut [i64]) -> Option<f64> {
    if latencies.is_empty() {
        return None;
    }
    latencies.sort_unstable();
    // 最近秩法：ceil(0.95 * n) - 1。
    let rank = ((latencies.len() as f64) * 0.95).ceil() as usize;
    let index = rank.saturating_sub(1).min(latencies.len() - 1);
    Some(latencies[index] as f64)
}

/// 聚合 shadow 记录。
pub fn shadow_summary(db: &Db) -> Result<ShadowSummary, StoreError> {
    let connection = db.read();
    let (total, agreed, disagreed, unknown, avg, cost): (i64, i64, i64, i64, Option<f64>, i64) =
        connection.query_row(
            "SELECT COUNT(*),
                COALESCE(SUM(CASE agreement WHEN 1 THEN 1 ELSE 0 END), 0),
                COALESCE(SUM(CASE agreement WHEN 0 THEN 1 ELSE 0 END), 0),
                COALESCE(SUM(CASE WHEN agreement IS NULL THEN 1 ELSE 0 END), 0),
                AVG(judge_latency_ms),
                COALESCE(SUM(judge_cost_micros), 0)
           FROM shadow_records",
            [],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                    row.get(5)?,
                ))
            },
        )?;

    let mut latencies: Vec<i64> = {
        let mut statement = connection.prepare(
            "SELECT judge_latency_ms FROM shadow_records WHERE judge_latency_ms IS NOT NULL",
        )?;
        statement
            .query_map([], |row| row.get(0))?
            .collect::<Result<_, _>>()?
    };
    let p95 = percentile_95(&mut latencies);

    Ok(ShadowSummary {
        total,
        agreed,
        disagreed,
        unknown,
        avg_latency_ms: avg,
        p95_latency_ms: p95,
        total_cost_micros: cost,
    })
}
