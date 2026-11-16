//! `ledger` 查询扩展（P4-013/014/015/016）：管理面只读查询。
//!
//! 语义（技术方案 §4.8）：
//! - 分页游标 `(created_at, id)`，base64 编码；不用 SQL offset；
//! - 时间窗左闭右开，默认最近 24h，最大 90 天；
//! - 统计只从 ledger 聚合；`reason` 过滤用 `json_each` 精确匹配；
//! - 查询必须走 `idx_ledger_created`。

use rusqlite::{OptionalExtension, params};
use tr_contracts::{
    CostUsage, JudgeSummary, LatencySummary, RequestDetail, RequestSummary, SeriesGroup,
    SeriesPoint, SeriesResponse, StatsSummary, StatsTokenUsage, TokenUsage,
};

use crate::store::{Db, StoreError};

/// 最大查询窗口（毫秒）：90 天。
pub const MAX_WINDOW_MS: i64 = 90 * 24 * 3600 * 1000;

/// 游标编码：`<created_at>-<id>` 的 base64（URL 安全）。
pub fn encode_cursor(created_at: i64, id: i64) -> String {
    base64url(format!("{created_at}-{id}").as_bytes())
}

/// 游标解码；非法返回 None。
pub fn decode_cursor(cursor: &str) -> Option<(i64, i64)> {
    let text = base64url_decode(cursor)?;
    let raw = String::from_utf8(text).ok()?;
    let (created, id) = raw.split_once('-')?;
    Some((created.parse().ok()?, id.parse().ok()?))
}

fn base64url(bytes: &[u8]) -> String {
    const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let mut out = String::new();
    let mut buffer: u32 = 0;
    let mut bits = 0u32;
    for &byte in bytes {
        buffer = (buffer << 8) | u32::from(byte);
        bits += 8;
        while bits >= 6 {
            bits -= 6;
            let index = ((buffer >> bits) & 0x3f) as usize;
            out.push(ALPHABET[index] as char);
        }
    }
    if bits > 0 {
        let index = ((buffer << (6 - bits)) & 0x3f) as usize;
        out.push(ALPHABET[index] as char);
    }
    out
}

fn base64url_decode(text: &str) -> Option<Vec<u8>> {
    let mut table = [255u8; 256];
    const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    for (index, &byte) in ALPHABET.iter().enumerate() {
        table[byte as usize] = index as u8;
    }
    let mut out = Vec::new();
    let mut buffer: u32 = 0;
    let mut bits = 0u32;
    for byte in text.bytes() {
        let value = table[byte as usize];
        if value == 255 {
            return None;
        }
        buffer = (buffer << 6) | u32::from(value);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push(((buffer >> bits) & 0xff) as u8);
        }
    }
    Some(out)
}

/// 请求列表过滤条件。
#[derive(Debug, Default, Clone)]
pub struct LedgerQuery {
    pub from: Option<i64>,
    pub to: Option<i64>,
    pub session_key: Option<String>,
    pub model: Option<String>,
    pub outcome: Option<String>,
    pub protocol: Option<String>,
    pub reason_code: Option<String>,
    pub limit: u32,
    pub cursor: Option<(i64, i64)>,
}

/// 账本列表（契约 `RequestSummary`）。
///
/// 单位约定：DB 里 `created_at` 是 Unix **秒**；契约 API 是 Unix **毫秒**。
/// 查询入口把毫秒窗口折算成秒；返回前把 `created_at` 放大成毫秒。
pub fn list_requests(db: &Db, query: &LedgerQuery) -> Result<Vec<RequestSummary>, StoreError> {
    let to_ms = query.to.unwrap_or_else(now_millis);
    let from_ms = query.from.unwrap_or(to_ms - 24 * 3600 * 1000);
    if to_ms - from_ms > MAX_WINDOW_MS {
        return Err(StoreError::Invalid("window too large".to_string()));
    }
    // 毫秒 → 秒（DB 列单位）。
    let to = to_ms / 1000 + 1; // 扩 1 秒：created_at 是整秒，保证最后一秒被 <to 覆盖。
    let from = from_ms / 1000;

    let mut clauses: Vec<String> = vec![
        "created_at >= ?1".to_string(),
        "created_at < ?2".to_string(),
    ];
    let mut args: Vec<Box<dyn rusqlite::types::ToSql>> = vec![Box::new(from), Box::new(to)];
    if let Some(session) = &query.session_key {
        args.push(Box::new(session.clone()));
        clauses.push(format!("session_key = ?{}", args.len()));
    }
    if let Some(model) = &query.model {
        args.push(Box::new(model.clone()));
        clauses.push(format!(
            "(dispatch_model = ?{n} OR selection_model = ?{n})",
            n = args.len()
        ));
    }
    if let Some(outcome) = &query.outcome {
        args.push(Box::new(outcome.clone()));
        clauses.push(format!("outcome = ?{}", args.len()));
    }
    if let Some(protocol) = &query.protocol {
        args.push(Box::new(protocol.clone()));
        clauses.push(format!("ingress_protocol = ?{}", args.len()));
    }
    if let Some(reason) = &query.reason_code {
        args.push(Box::new(reason.clone()));
        clauses.push(format!(
            "EXISTS (SELECT 1 FROM json_each(reason_codes_json) WHERE json_each.value = ?{})",
            args.len()
        ));
    }
    // 游标：严格更旧（(created_at, id) 双键序）。
    if let Some((cursor_created, cursor_id)) = query.cursor {
        args.push(Box::new(cursor_created));
        args.push(Box::new(cursor_id));
        let n = args.len();
        clauses.push(format!("(created_at, id) < (?{a}, ?{b})", a = n - 1, b = n));
    }
    let sql = format!(
        "SELECT id, request_id, created_at, session_key, branch_id, ingress_protocol,
                selection_model, dispatch_model, dispatch_source, outcome, http_status,
                ttft_ms, duration_ms, input_tokens, cache_read_tokens, output_tokens,
                answer_cost_micros, routing_cost_micros, reason_codes_json, plan_id
           FROM ledger
          WHERE {}
          ORDER BY created_at DESC, id DESC
          LIMIT {}",
        clauses.join(" AND "),
        query.limit.clamp(1, 200),
    );

    let connection = db.read();
    let mut statement = connection.prepare(&sql)?;
    let reference: Vec<&dyn rusqlite::types::ToSql> = args.iter().map(|arg| arg.as_ref()).collect();
    let rows = statement.query_map(reference.as_slice(), row_summary)?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(StoreError::from)
}

/// 单请求详情（契约 `RequestDetail`）。
pub fn request_detail(db: &Db, request_id: &str) -> Result<Option<RequestDetail>, StoreError> {
    let connection = db.read();
    let mut statement = connection.prepare(
        "SELECT id, request_id, created_at, session_key, branch_id, ingress_protocol,
                selection_model, selection_level, dispatch_model, dispatch_provider,
                dispatch_level, dispatch_source, plan_mode, preference_json, failover_json,
                attempts_json, switches, outcome, http_status, ttft_ms, duration_ms,
                input_tokens, cache_read_tokens, cache_write_tokens, output_tokens,
                answer_cost_micros, routing_cost_micros, reason_codes_json, plan_id,
                task_id, session_key_source, collision_suspect, facts_completeness,
                profile_revision, config_digest, judge_json, contributions_json, authority_json
           FROM ledger WHERE request_id = ?1",
    )?;
    let row = statement
        .query_row(params![request_id], |row| {
            let summary = row_summary(row)?;
            let preference_json: String = row.get(13)?;
            let failover_json: String = row.get(14)?;
            let attempts_json: String = row.get(15)?;
            let _reason_codes: Vec<String> = parse_reason_codes(&row.get::<_, String>(18)?);
            let task_id: Option<String> = row.get(29)?;
            let session_key_source: String = row.get(30)?;
            let collision_suspect: i64 = row.get(31)?;
            let facts_completeness: String = row.get(32)?;
            let profile_revision: Option<String> = row.get(33)?;
            let config_digest: Option<String> = row.get(34)?;
            let judge_json: Option<String> = row.get(35)?;
            let contributions_json: String = row.get(36)?;
            let authority_json: Option<String> = row.get(37)?;

            // attempts 脱敏投影：candidate/outcome/latency/error_class。
            let attempts = serde_json::from_str::<serde_json::Value>(&attempts_json)
                .ok()
                .and_then(|value| value.as_array().cloned())
                .unwrap_or_default()
                .into_iter()
                .map(|attempt| {
                    serde_json::json!({
                        "candidate_id": attempt.get("candidate_id").cloned().unwrap_or_default(),
                        "outcome": attempt.get("outcome").cloned().unwrap_or_default(),
                        "latency_ms": attempt.get("duration_ms").cloned().unwrap_or_default(),
                        "error_class": attempt.get("error_class").cloned().unwrap_or_default(),
                    })
                })
                .collect::<Vec<_>>();

            Ok(RequestDetail {
                summary,
                task_id,
                session_key_source,
                collision_suspect: collision_suspect != 0,
                selection_level: String::new(),
                dispatch_provider: None,
                dispatch_level: None,
                plan_mode: String::new(),
                preference: serde_json::from_str(&preference_json)
                    .unwrap_or(serde_json::Value::Null),
                failover: serde_json::from_str(&failover_json).unwrap_or(serde_json::Value::Null),
                attempts: attempts
                    .into_iter()
                    .filter_map(|value| serde_json::from_value(value).ok())
                    .collect(),
                switches: 0,
                cache_write_tokens: row.get::<_, Option<i64>>(23)?.unwrap_or(0),
                facts_completeness,
                profile_revision,
                config_digest,
                judge: judge_json
                    .as_deref()
                    .and_then(|text| serde_json::from_str(text).ok()),
                contributions: serde_json::from_str(&contributions_json)
                    .unwrap_or(serde_json::Value::Null),
                authority: authority_json
                    .as_deref()
                    .and_then(|text| serde_json::from_str(text).ok()),
            })
        })
        .optional()?;
    Ok(row)
}

#[allow(clippy::type_complexity)]
fn row_summary(row: &rusqlite::Row<'_>) -> Result<RequestSummary, rusqlite::Error> {
    let reason_codes = parse_reason_codes(&row.get::<_, String>(18)?);
    // DB 秒 → 契约毫秒。
    let created_at_secs: i64 = row.get(2)?;
    Ok(RequestSummary {
        request_id: row.get(1)?,
        created_at: created_at_secs.saturating_mul(1000),
        session_key: row.get(3)?,
        branch_id: row.get(4)?,
        ingress_protocol: row.get(5)?,
        selection_model: row.get(6)?,
        dispatch_model: row.get(7)?,
        dispatch_source: row.get(8)?,
        outcome: row.get(9)?,
        http_status: row.get(10)?,
        ttft_ms: row.get(11)?,
        duration_ms: row.get(12)?,
        tokens: TokenUsage {
            input: row.get::<_, Option<i64>>(13)?.unwrap_or(0),
            cache_read: row.get::<_, Option<i64>>(14)?.unwrap_or(0),
            output: row.get::<_, Option<i64>>(15)?.unwrap_or(0),
        },
        cost_micros: CostUsage {
            answer: row.get::<_, Option<i64>>(16)?.unwrap_or(0),
            routing: row.get::<_, Option<i64>>(17)?.unwrap_or(0),
        },
        reason_codes,
        plan_id: row.get(19)?,
    })
}

fn parse_reason_codes(json: &str) -> Vec<String> {
    serde_json::from_str::<Vec<String>>(json).unwrap_or_default()
}

/// 时间窗统计汇总（契约 `StatsSummary`）。只从 ledger 聚合。
///
/// `from`/`to` 是 Unix 毫秒（契约）；内部折算成 DB 的秒。
pub fn stats_summary(db: &Db, from_ms: i64, to_ms: i64) -> Result<StatsSummary, StoreError> {
    if to_ms - from_ms > MAX_WINDOW_MS {
        return Err(StoreError::Invalid("window too large".to_string()));
    }
    let (from, to) = (from_ms / 1000, to_ms / 1000 + 1); // to 扩 1 秒：created_at 是整秒，保证最后一秒被 <to 覆盖
    let connection = db.read();
    let mut statement = connection.prepare(
        "SELECT COUNT(*),
                SUM(CASE WHEN outcome = 'success' THEN 1 ELSE 0 END),
                SUM(CASE WHEN outcome = 'upstream_error' THEN 1 ELSE 0 END),
                SUM(CASE WHEN outcome = 'quality_fail' THEN 1 ELSE 0 END),
                SUM(CASE WHEN outcome = 'cancelled' THEN 1 ELSE 0 END),
                SUM(CASE WHEN outcome = 'rejected' THEN 1 ELSE 0 END),
                COALESCE(SUM(switches), 0),
                COALESCE(SUM(input_tokens), 0), COALESCE(SUM(output_tokens), 0),
                COALESCE(SUM(cache_read_tokens), 0), COALESCE(SUM(cache_write_tokens), 0),
                COALESCE(SUM(answer_cost_micros), 0), COALESCE(SUM(routing_cost_micros), 0),
                -- p95 分位（近似：按 ttft 排序取第 95 百分位行）
                (SELECT ttft_ms FROM ledger WHERE created_at >= ?1 AND created_at < ?2 AND ttft_ms IS NOT NULL
                  ORDER BY ttft_ms LIMIT 1 OFFSET ((
                    SELECT COUNT(*) FROM ledger WHERE created_at >= ?1 AND created_at < ?2 AND ttft_ms IS NOT NULL
                  ) * 95 / 100)),
                (SELECT duration_ms FROM ledger WHERE created_at >= ?1 AND created_at < ?2
                  ORDER BY duration_ms LIMIT 1 OFFSET ((
                    SELECT COUNT(*) FROM ledger WHERE created_at >= ?1 AND created_at < ?2
                  ) * 95 / 100))
           FROM ledger WHERE created_at >= ?1 AND created_at < ?2",
    )?;
    let requests = statement.query_row(params![from, to], |row| {
        Ok((
            row.get::<_, i64>(0)?,
            row.get::<_, Option<i64>>(1)?.unwrap_or(0),
            row.get::<_, Option<i64>>(2)?.unwrap_or(0),
            row.get::<_, Option<i64>>(3)?.unwrap_or(0),
            row.get::<_, Option<i64>>(4)?.unwrap_or(0),
            row.get::<_, Option<i64>>(5)?.unwrap_or(0),
            row.get::<_, i64>(6)?,
            row.get::<_, i64>(7)?,
            row.get::<_, i64>(8)?,
            row.get::<_, i64>(9)?,
            row.get::<_, i64>(10)?,
            row.get::<_, i64>(11)?,
            row.get::<_, i64>(12)?,
            row.get::<_, Option<i64>>(13)?,
            row.get::<_, Option<i64>>(14)?,
        ))
    });
    let requests = match requests {
        Ok(value) => value,
        Err(rusqlite::Error::QueryReturnedNoRows) => {
            return Ok(StatsSummary {
                requests: 0,
                successes: 0,
                success_rate: 0.0,
                upstream_errors: 0,
                quality_fails: 0,
                cancelled: 0,
                rejected: 0,
                switches: 0,
                tokens: StatsTokenUsage::default(),
                cost_micros: CostUsage::default(),
                latency: LatencySummary::default(),
                judge: JudgeSummary::default(),
            });
        }
        Err(error) => return Err(error.into()),
    };
    let total = requests.0;
    let successes = requests.1;
    // 判官维度从 judge_calls 聚合。
    let (judge_calls, judge_failed, judge_cost) = judge_stats(&connection, from, to)?;
    Ok(StatsSummary {
        requests: total,
        successes,
        success_rate: if total > 0 {
            f64::from(successes as u32) / total as f64
        } else {
            0.0
        },
        upstream_errors: requests.2,
        quality_fails: requests.3,
        cancelled: requests.4,
        rejected: requests.5,
        switches: requests.6,
        tokens: StatsTokenUsage {
            input: requests.7,
            output: requests.8,
            cache_read: requests.9,
            cache_write: requests.10,
        },
        cost_micros: CostUsage {
            answer: requests.11,
            routing: requests.12,
        },
        latency: LatencySummary {
            ttft_p50: None,
            ttft_p95: requests.13,
            duration_p95: requests.14,
        },
        judge: JudgeSummary {
            calls: judge_calls,
            failed: judge_failed,
            cost_micros: judge_cost,
        },
    })
}

fn judge_stats(
    connection: &rusqlite::Connection,
    from: i64,
    to: i64,
) -> Result<(i64, i64, i64), StoreError> {
    connection
        .query_row(
            "SELECT COUNT(*),
                    SUM(CASE WHEN outcome = 'failed' THEN 1 ELSE 0 END),
                    COALESCE(SUM(cost_micros), 0)
               FROM judge_calls WHERE created_at >= ?1 AND created_at < ?2",
            params![from, to],
            |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, Option<i64>>(1)?.unwrap_or(0),
                    row.get::<_, i64>(2)?,
                ))
            },
        )
        .map_err(StoreError::from)
}

/// 时间序列（契约 `SeriesResponse`）：bucket hour/day + group_by + metric。
///
/// `from`/`to` 是 Unix 毫秒（契约）；内部折算成 DB 的秒，bucket 用毫秒。
pub fn stats_series(
    db: &Db,
    from_ms: i64,
    to_ms: i64,
    bucket: &str,
    group_by: &str,
    metric: &str,
) -> Result<SeriesResponse, StoreError> {
    if to_ms - from_ms > MAX_WINDOW_MS {
        return Err(StoreError::Invalid("window too large".to_string()));
    }
    let (from, to) = (from_ms / 1000, to_ms / 1000 + 1); // to 扩 1 秒：created_at 是整秒，保证最后一秒被 <to 覆盖
    let bucket_ms = match bucket {
        "hour" => 3600 * 1000i64,
        "day" => 24 * 3600 * 1000i64,
        _ => return Err(StoreError::Invalid(format!("invalid bucket `{bucket}`"))),
    };
    let group_column = match group_by {
        "virtual_model" => "selection_model",
        "dispatch_model" => "dispatch_model",
        "outcome" => "outcome",
        "protocol" => "ingress_protocol",
        _ => {
            return Err(StoreError::Invalid(format!(
                "invalid group_by `{group_by}`"
            )));
        }
    };
    let metric_expr = match metric {
        "requests" => "COUNT(*)".to_string(),
        "successes" => "SUM(CASE WHEN outcome = 'success' THEN 1 ELSE 0 END)".to_string(),
        "cost" => "COALESCE(SUM(answer_cost_micros + routing_cost_micros), 0)".to_string(),
        "input_tokens" => "COALESCE(SUM(input_tokens), 0)".to_string(),
        "output_tokens" => "COALESCE(SUM(output_tokens), 0)".to_string(),
        _ => return Err(StoreError::Invalid(format!("invalid metric `{metric}`"))),
    };

    // bucket 在 SQL 里按**秒**对齐（created_at 是秒），返回时放大成毫秒。
    let bucket_secs = bucket_ms / 1000;
    let sql = format!(
        "SELECT (?1 + (created_at - ?1) / ?4 * ?4) AS bucket_start_secs,
                {column},
                {metric}
           FROM ledger
          WHERE created_at >= ?1 AND created_at < ?2
          GROUP BY bucket_start_secs, {column}
          ORDER BY bucket_start_secs",
        column = group_column,
        metric = metric_expr,
    );
    let connection = db.read();
    let mut statement = connection.prepare(&sql)?;
    let rows = statement.query_map(params![from, to, from, bucket_secs], |row| {
        Ok((
            row.get::<_, i64>(0)?,
            row.get::<_, Option<String>>(1)?
                .unwrap_or_else(|| "unknown".to_string()),
            row.get::<_, i64>(2)?,
        ))
    })?;

    // 按 key 分组、按 bucket 对齐补零（bucket 边界用毫秒）。
    let mut groups: std::collections::BTreeMap<String, Vec<(i64, i64)>> = Default::default();
    for row in rows {
        let (bucket_start_secs, key, value) = row?;
        groups
            .entry(key)
            .or_default()
            .push((bucket_start_secs.saturating_mul(1000), value));
    }
    let buckets = (from_ms..to_ms).step_by(bucket_ms as usize);
    let series = groups
        .into_iter()
        .map(|(key, mut points)| {
            points.sort();
            let map: std::collections::BTreeMap<i64, i64> = points.iter().copied().collect();
            SeriesGroup {
                key,
                points: buckets
                    .clone()
                    .map(|t| SeriesPoint {
                        t,
                        value: map.get(&t).copied().unwrap_or(0),
                    })
                    .collect(),
            }
        })
        .collect();
    Ok(SeriesResponse {
        bucket: bucket.to_string(),
        from: from_ms,
        to: to_ms,
        group_by: group_by.to_string(),
        metric: metric.to_string(),
        series,
    })
}

/// 模型维度汇总（契约 `StatsSummary` 的模型切片）。
pub fn stats_by_model(
    db: &Db,
    from_ms: i64,
    to_ms: i64,
) -> Result<Vec<serde_json::Value>, StoreError> {
    if to_ms - from_ms > MAX_WINDOW_MS {
        return Err(StoreError::Invalid("window too large".to_string()));
    }
    let (from, to) = (from_ms / 1000, to_ms / 1000 + 1); // to 扩 1 秒：created_at 是整秒，保证最后一秒被 <to 覆盖
    let connection = db.read();
    let mut statement = connection.prepare(
        "SELECT COALESCE(dispatch_model, selection_model),
                COUNT(*),
                SUM(CASE WHEN outcome = 'success' THEN 1 ELSE 0 END),
                COALESCE(SUM(input_tokens), 0), COALESCE(SUM(output_tokens), 0),
                COALESCE(SUM(answer_cost_micros), 0)
           FROM ledger WHERE created_at >= ?1 AND created_at < ?2
          GROUP BY 1 ORDER BY 2 DESC",
    )?;
    let rows = statement.query_map(params![from, to], |row| {
        Ok(serde_json::json!({
            "model": row.get::<_, Option<String>>(0)?.unwrap_or_else(|| "unknown".to_string()),
            "requests": row.get::<_, i64>(1)?,
            "successes": row.get::<_, Option<i64>>(2)?.unwrap_or(0),
            "input_tokens": row.get::<_, i64>(3)?,
            "output_tokens": row.get::<_, i64>(4)?,
            "cost_micros": row.get::<_, i64>(5)?,
        }))
    })?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(StoreError::from)
}

fn now_millis() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::Db;
    use tr_contracts::{
        AttemptRecord, Cost, DispatchSource, IngressProtocol, LedgerRecord, OutcomeKind, PlanMode,
        ReasonCode, RequestAuthority, SessionKeySource,
    };
    use uuid::Uuid;

    fn record(
        request_id: &str,
        created_at: u64,
        outcome: OutcomeKind,
        model: &str,
    ) -> LedgerRecord {
        LedgerRecord {
            request_id: Uuid::parse_str(request_id).unwrap(),
            plan_id: format!("pl-{request_id}"),
            session_key: "s-1".to_string(),
            branch_id: "main".to_string(),
            task_id: String::new(),
            session_key_source: SessionKeySource::Content,
            collision_suspect: false,
            ingress_protocol: IngressProtocol::ChatCompletions,
            authority: RequestAuthority::default(),
            selected_virtual_model: Some("auto".into()),
            selected_thinking_level: Some("auto".into()),
            request_reason: Default::default(),
            plan_mode: PlanMode::Cascade,
            preference: vec![],
            failover: vec![],
            served_slot: None,
            served_physical_model: Some(model.into()),
            served_provider: Some("openai".into()),
            served_thinking_level: None,
            dispatch_source: DispatchSource::Strategy,
            switches: 0,
            attempts: vec![AttemptRecord {
                candidate_id: "c-1".into(),
                error_class: None,
                http_status: Some(200),
                committed: true,
                duration_ms: 100,
                ttft_ms: Some(50),
                usage: None,
            }],
            judge: None,
            contributions: vec![],
            input_tokens: Some(10),
            output_tokens: Some(5),
            cache_read_tokens: None,
            cache_write_tokens: None,
            cache_hit_tokens: None,
            answer_cost: Some(Cost {
                micros: 100,
                currency: "USD".into(),
            }),
            routing_cost: None,
            cost_equiv: None,
            facts_completeness: tr_contracts::FactsCompleteness::Complete,
            reason_codes: vec![ReasonCode::new(
                tr_contracts::ReasonCodeKind::EscalationLatched,
            )],
            outcome,
            http_status: Some(200),
            profile_revision: None,
            config_digest: None,
            ttft_ms: Some(50),
            duration_ms: 100,
            created_at,
        }
    }

    fn seed() -> Db {
        let db = Db::open_in_memory().unwrap();
        // DB created_at 单位是 Unix **秒**。
        let base: u64 = 1_763_000_000;
        crate::store::ledger::insert(
            &db,
            &record(
                "00000000-0000-0000-0000-000000000001",
                base,
                OutcomeKind::Success,
                "glm-flash",
            ),
        )
        .unwrap();
        crate::store::ledger::insert(
            &db,
            &record(
                "00000000-0000-0000-0000-000000000002",
                base + 1,
                OutcomeKind::UpstreamError,
                "glm-pro",
            ),
        )
        .unwrap();
        db
    }

    #[test]
    fn list_requests_filters_and_paginates() {
        let db = seed();
        // 查询窗口单位是毫秒（契约）；base 秒 → 毫秒。
        let base_ms: i64 = 1_763_000_000 * 1000;
        let window = |query: LedgerQuery| LedgerQuery {
            from: Some(base_ms - 1000),
            to: Some(base_ms + 10_000),
            ..query
        };
        let rows = list_requests(
            &db,
            &window(LedgerQuery {
                limit: 10,
                ..Default::default()
            }),
        )
        .unwrap();
        assert_eq!(rows.len(), 2);
        // created_at DESC：第二条在前。
        assert_eq!(rows[0].request_id, "00000000-0000-0000-0000-000000000002");

        let rows = list_requests(
            &db,
            &window(LedgerQuery {
                limit: 1,
                ..Default::default()
            }),
        )
        .unwrap();
        assert_eq!(rows.len(), 1);

        // outcome 过滤。
        let rows = list_requests(
            &db,
            &window(LedgerQuery {
                outcome: Some("success".to_string()),
                limit: 10,
                ..Default::default()
            }),
        )
        .unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].outcome, "success");
    }

    #[test]
    fn cursor_roundtrip() {
        let cursor = encode_cursor(1_763_000_000_000, 42);
        assert_eq!(decode_cursor(&cursor), Some((1_763_000_000_000, 42)));
        assert_eq!(decode_cursor("!!!"), None);
    }

    #[test]
    fn stats_summary_matches_rows() {
        let db = seed();
        let base_ms: i64 = 1_763_000_000 * 1000;
        let summary = stats_summary(&db, base_ms - 1000, base_ms + 10_000).unwrap();
        assert_eq!(summary.requests, 2);
        assert_eq!(summary.successes, 1);
        assert_eq!(summary.upstream_errors, 1);
        assert_eq!(summary.cost_micros.answer, 200);
    }

    #[test]
    fn series_buckets_align() {
        let db = seed();
        let base_ms: i64 = 1_763_000_000 * 1000;
        let series = stats_series(
            &db,
            base_ms,
            base_ms + 3_600_000,
            "hour",
            "dispatch_model",
            "requests",
        )
        .unwrap();
        assert_eq!(series.series.len(), 2);
        for group in &series.series {
            assert_eq!(group.points.len(), 1);
        }
    }

    #[test]
    fn window_too_large_rejected() {
        let db = seed();
        let base_ms: i64 = 1_763_000_000 * 1000;
        let result = stats_summary(&db, base_ms, base_ms + MAX_WINDOW_MS + 1);
        assert!(result.is_err());
    }
}

#[cfg(test)]
mod handler_equiv_tests {
    use super::*;
    use crate::store::Db;
    use tr_contracts::{
        Cost, DispatchSource, IngressProtocol, LedgerRecord, OutcomeKind, PlanMode,
        RequestAuthority, SessionKeySource,
    };
    use uuid::Uuid;

    fn now_secs() -> u64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0)
    }

    #[test]
    fn handler_default_window_finds_recent_row() {
        let db = Db::open_in_memory().unwrap();
        let record = LedgerRecord {
            request_id: Uuid::new_v4(),
            plan_id: "pl-1".into(),
            session_key: "s".into(),
            branch_id: "main".into(),
            task_id: String::new(),
            session_key_source: SessionKeySource::Content,
            collision_suspect: false,
            ingress_protocol: IngressProtocol::ChatCompletions,
            authority: RequestAuthority::default(),
            selected_virtual_model: Some("auto".into()),
            selected_thinking_level: Some("auto".into()),
            request_reason: Default::default(),
            plan_mode: PlanMode::Cascade,
            preference: vec![],
            failover: vec![],
            served_slot: None,
            served_physical_model: Some("m".into()),
            served_provider: Some("openai".into()),
            served_thinking_level: None,
            dispatch_source: DispatchSource::Strategy,
            switches: 0,
            attempts: vec![],
            judge: None,
            contributions: vec![],
            input_tokens: Some(1),
            output_tokens: Some(1),
            cache_read_tokens: None,
            cache_write_tokens: None,
            cache_hit_tokens: None,
            answer_cost: Some(Cost {
                micros: 5,
                currency: "USD".into(),
            }),
            routing_cost: None,
            cost_equiv: None,
            facts_completeness: tr_contracts::FactsCompleteness::Complete,
            reason_codes: vec![],
            outcome: OutcomeKind::Success,
            http_status: Some(200),
            profile_revision: None,
            config_digest: None,
            ttft_ms: Some(10),
            duration_ms: 20,
            created_at: now_secs(),
        };
        crate::store::ledger::insert(&db, &record).unwrap();
        // handler default window
        let now_ms = now_secs() as i64 * 1000;
        let from_ms = now_ms - 24 * 3600 * 1000;
        let summary = stats_summary(&db, from_ms, now_ms).unwrap();
        assert_eq!(summary.requests, 1, "default window should find recent row");
    }
}
