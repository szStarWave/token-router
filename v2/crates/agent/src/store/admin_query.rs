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
    SeriesPoint, SeriesResponse, SessionSummary, StatsSummary, StatsTokenUsage, TokenUsage,
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
    Ok(list_requests_page(db, query)?.0)
}

/// 账本列表 + 下一页游标。
///
/// 游标内含 **DB 秒**与账本行 `id`（与 `ORDER BY created_at DESC, id DESC` 同一口径）；
/// 对调用方不透明。只有取满一页时才给游标，最后一页不会诱使客户端多请求一次。
pub fn list_requests_page(
    db: &Db,
    query: &LedgerQuery,
) -> Result<(Vec<RequestSummary>, Option<String>), StoreError> {
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
    let mut out = Vec::new();
    let mut last: Option<(i64, i64)> = None;
    // 游标键是 (created_at 秒, 账本 id)；与列表 ORDER BY 同口径。
    for row in statement.query_map(reference.as_slice(), |row| {
        let row_id: i64 = row.get(0)?;
        let created_at_secs: i64 = row.get(2)?;
        let summary = row_summary(row)?;
        Ok((created_at_secs, row_id, summary))
    })? {
        let (created_at_secs, row_id, summary) = row?;
        last = Some((created_at_secs, row_id));
        out.push(summary);
    }
    let limit = query.limit.clamp(1, 200);
    let next_cursor = if out.len() as u32 == limit {
        last.map(|(created_at_secs, row_id)| encode_cursor(created_at_secs, row_id))
    } else {
        None
    };
    Ok((out, next_cursor))
}

/// 会话汇总（契约 `SessionSummary`）：服务端按 `session_key` 聚合整段会话的调用，
/// 不受请求列表分页/时间窗截断影响；判官调用按 request_id join 账本归到会话。
pub fn sessions(
    db: &Db,
    from_ms: i64,
    to_ms: i64,
    limit: u32,
) -> Result<Vec<SessionSummary>, StoreError> {
    if to_ms - from_ms > MAX_WINDOW_MS {
        return Err(StoreError::Invalid("window too large".to_string()));
    }
    let from = from_ms / 1000;
    let to = to_ms / 1000 + 1;
    let limit = limit.clamp(1, 200);

    let connection = db.read();
    let mut statement = connection.prepare(
        "SELECT session_key,
                COUNT(*),
                SUM(CASE WHEN outcome = 'success' THEN 0 ELSE 1 END),
                COALESCE(SUM(input_tokens), 0),
                COALESCE(SUM(cache_read_tokens), 0),
                COALESCE(SUM(output_tokens), 0),
                COALESCE(SUM(answer_cost_micros), 0),
                COALESCE(SUM(routing_cost_micros), 0),
                COALESCE(GROUP_CONCAT(DISTINCT COALESCE(dispatch_model, selection_model)), ''),
                MIN(created_at),
                MAX(created_at)
           FROM ledger
          WHERE created_at >= ?1 AND created_at < ?2
          GROUP BY session_key
          ORDER BY MAX(created_at) DESC
          LIMIT ?3",
    )?;
    let mut rows = statement
        .query_map(params![from, to, limit], |row| {
            Ok(SessionSummary {
                session_key: row.get(0)?,
                requests: row.get(1)?,
                failures: row.get(2)?,
                tokens: TokenUsage {
                    input: row.get(3)?,
                    cache_read: row.get(4)?,
                    output: row.get(5)?,
                },
                cost_micros: CostUsage {
                    answer: row.get(6)?,
                    routing: row.get(7)?,
                },
                judge_calls: 0,
                judge_failures: 0,
                judge_cost_micros: 0,
                models: row
                    .get::<_, String>(8)?
                    .split(',')
                    .filter(|item| !item.is_empty())
                    .map(str::to_string)
                    .collect(),
                first_at: row.get::<_, i64>(9)?.saturating_mul(1000),
                last_at: row.get::<_, i64>(10)?.saturating_mul(1000),
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;

    let mut judge_statement = connection.prepare(
        "SELECT l.session_key,
                COUNT(*),
                SUM(CASE WHEN jc.outcome NOT IN ('answered', 'selected', 'no_match') THEN 1 ELSE 0 END),
                COALESCE(SUM(jc.cost_micros), 0)
           FROM judge_calls jc JOIN ledger l ON l.request_id = jc.request_id
          WHERE l.created_at >= ?1 AND l.created_at < ?2
          GROUP BY l.session_key",
    )?;
    let judge_rows = judge_statement
        .query_map(params![from, to], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, Option<i64>>(2)?.unwrap_or(0),
                row.get::<_, i64>(3)?,
            ))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    let judge_by_session: std::collections::HashMap<_, _> = judge_rows
        .into_iter()
        .map(|(key, calls, failures, cost)| (key, (calls, failures, cost)))
        .collect();
    for row in &mut rows {
        if let Some((calls, failures, cost)) = judge_by_session.get(&row.session_key) {
            row.judge_calls = *calls;
            row.judge_failures = *failures;
            row.judge_cost_micros = *cost;
        }
    }
    Ok(rows)
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
            // 按列名读取：列表查询与详情查询的 SELECT 顺序不同（详情多出若干列），
            // 按位置读会在两处 SQL 增删列时静默错位（曾把 http_status 当成 reason_codes_json）。
            let preference_json: String = row.get("preference_json")?;
            let failover_json: String = row.get("failover_json")?;
            let attempts_json: String = row.get("attempts_json")?;
            let task_id: Option<String> = row.get("task_id")?;
            let session_key_source: String = row.get("session_key_source")?;
            let collision_suspect: i64 = row.get("collision_suspect")?;
            let facts_completeness: String = row.get("facts_completeness")?;
            let profile_revision: Option<String> = row.get("profile_revision")?;
            let config_digest: Option<String> = row.get("config_digest")?;
            let judge_json: Option<String> = row.get("judge_json")?;
            let contributions_json: String = row.get("contributions_json")?;
            let authority_json: Option<String> = row.get("authority_json")?;

            // attempts 脱敏投影：candidate/outcome/latency/error_class。
            // 账本里没有独立的 outcome 字段：没有错误类即视为成功。
            let attempts = serde_json::from_str::<serde_json::Value>(&attempts_json)
                .ok()
                .and_then(|value| value.as_array().cloned())
                .unwrap_or_default()
                .into_iter()
                .map(|attempt| {
                    let error_class = attempt.get("error_class").cloned().filter(|v| !v.is_null());
                    serde_json::json!({
                        "candidate_id": attempt.get("candidate_id").cloned().unwrap_or_default(),
                        "outcome": error_class
                            .clone()
                            .unwrap_or_else(|| serde_json::Value::String("success".into())),
                        "latency_ms": attempt.get("duration_ms").cloned().unwrap_or_default(),
                        "error_class": error_class.unwrap_or(serde_json::Value::Null),
                    })
                })
                .collect::<Vec<_>>();

            Ok(RequestDetail {
                summary,
                task_id,
                session_key_source,
                collision_suspect: collision_suspect != 0,
                selection_level: row.get("selection_level")?,
                dispatch_provider: row.get("dispatch_provider")?,
                dispatch_level: row.get("dispatch_level")?,
                plan_mode: row.get("plan_mode")?,
                preference: serde_json::from_str(&preference_json)
                    .unwrap_or(serde_json::Value::Null),
                failover: serde_json::from_str(&failover_json).unwrap_or(serde_json::Value::Null),
                attempts: attempts
                    .into_iter()
                    .filter_map(|value| serde_json::from_value(value).ok())
                    .collect(),
                // 判官调用由调用方（control handler）按 request_id 补齐。
                judge_calls: Vec::new(),
                switches: row.get("switches")?,
                cache_write_tokens: row
                    .get::<_, Option<i64>>("cache_write_tokens")?
                    .unwrap_or(0),
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
    // 按列名读（列表查询与详情查询的 SELECT 顺序不同，位置读取会错位）。
    let reason_codes = parse_reason_codes(&row.get::<_, String>("reason_codes_json")?);
    // DB 秒 → 契约毫秒。
    let created_at_secs: i64 = row.get("created_at")?;
    Ok(RequestSummary {
        request_id: row.get("request_id")?,
        created_at: created_at_secs.saturating_mul(1000),
        session_key: row.get("session_key")?,
        branch_id: row.get("branch_id")?,
        ingress_protocol: row.get("ingress_protocol")?,
        selection_model: row.get("selection_model")?,
        dispatch_model: row.get("dispatch_model")?,
        dispatch_source: row.get("dispatch_source")?,
        outcome: row.get("outcome")?,
        http_status: row.get("http_status")?,
        ttft_ms: row.get("ttft_ms")?,
        duration_ms: row.get("duration_ms")?,
        tokens: TokenUsage {
            input: row.get::<_, Option<i64>>("input_tokens")?.unwrap_or(0),
            cache_read: row.get::<_, Option<i64>>("cache_read_tokens")?.unwrap_or(0),
            output: row.get::<_, Option<i64>>("output_tokens")?.unwrap_or(0),
        },
        cost_micros: CostUsage {
            answer: row
                .get::<_, Option<i64>>("answer_cost_micros")?
                .unwrap_or(0),
            routing: row
                .get::<_, Option<i64>>("routing_cost_micros")?
                .unwrap_or(0),
        },
        reason_codes,
        plan_id: row.get("plan_id")?,
    })
}

/// 账本里存的是 `ReasonCode` 对象数组（`{"code":"…","detail":…}`）；契约投影成
/// `string[]`。兼容纯字符串数组（早期/测试数据）。
fn parse_reason_codes(json: &str) -> Vec<String> {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(json) else {
        return Vec::new();
    };
    let Some(items) = value.as_array() else {
        return Vec::new();
    };
    items
        .iter()
        .filter_map(|item| match item {
            serde_json::Value::String(text) => Some(text.clone()),
            serde_json::Value::Object(map) => map
                .get("code")
                .and_then(|code| code.as_str())
                .map(str::to_string),
            _ => None,
        })
        .collect()
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
                    SUM(CASE WHEN outcome NOT IN ('answered', 'selected', 'no_match') THEN 1 ELSE 0 END),
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

    /// 详情查询的 SELECT 比列表多出若干列；按列名读才对得上（按位置读曾把
    /// http_status 当成 reason_codes_json，报 `Invalid column type Integer`）。
    #[test]
    fn request_detail_reads_columns_by_name() {
        let db = seed();
        let detail = request_detail(&db, "00000000-0000-0000-0000-000000000001")
            .unwrap()
            .expect("detail exists");
        let summary = &detail.summary;
        assert_eq!(summary.request_id, "00000000-0000-0000-0000-000000000001");
        assert_eq!(summary.outcome, "success");
        assert_eq!(summary.http_status, Some(200));
        assert_eq!(summary.created_at, 1_763_000_000_000);
        assert_eq!(summary.tokens.input, 10);
        assert_eq!(summary.tokens.output, 5);
        assert_eq!(summary.cost_micros.answer, 100);
        assert_eq!(summary.reason_codes.len(), 1);
        assert_eq!(summary.plan_id, "pl-00000000-0000-0000-0000-000000000001");
        // 详情独有字段按列名读：thinking level 与 plan mode 不再被硬编码成空串。
        assert_eq!(detail.selection_level, "auto");
        assert_eq!(detail.plan_mode, "cascade");
        assert_eq!(detail.attempts.len(), 1);
        assert_eq!(detail.attempts[0].candidate_id, "c-1");
        assert_eq!(detail.attempts[0].outcome, "success");
        assert_eq!(detail.attempts[0].latency_ms, Some(100));
    }

    /// 会话汇总覆盖整段会话（不分页），并把判官调用按 request_id join 进来；
    /// `selected`/`no_match` 是合法答案，只有 unavailable/failed/budget 计入失败。
    #[test]
    fn sessions_aggregate_includes_judge_calls() {
        let db = seed();
        let first = "00000000-0000-0000-0000-000000000001";
        let second = "00000000-0000-0000-0000-000000000002";
        for (request_id, outcome, cost) in
            [(first, "selected", Some(7)), (second, "unavailable", None)]
        {
            crate::store::judge::insert_call(
                &db,
                &crate::store::judge::JudgeCallRecord {
                    request_id: Uuid::parse_str(request_id).unwrap(),
                    question_id: "auto".to_string(),
                    service_id: "group-judge".to_string(),
                    revision: 1,
                    protocol_digest: None,
                    effective_source: "heuristic".to_string(),
                    outcome: outcome.to_string(),
                    failure_code: None,
                    latency_ms: 12,
                    prompt_tokens: Some(10),
                    completion_tokens: Some(5),
                    cost_micros: cost,
                    created_at: 1_763_000_000,
                    detail: Default::default(),
                },
            )
            .unwrap();
        }

        let base_ms: i64 = 1_763_000_000 * 1000;
        let rows = sessions(&db, base_ms - 1000, base_ms + 10_000, 10).unwrap();
        assert_eq!(rows.len(), 1, "两个请求同会话");
        let session = &rows[0];
        assert_eq!(session.session_key, "s-1");
        assert_eq!(session.requests, 2);
        assert_eq!(session.failures, 1);
        assert_eq!(session.tokens.input, 20);
        assert_eq!(session.tokens.output, 10);
        assert_eq!(session.cost_micros.answer, 200);
        assert_eq!(session.judge_calls, 2);
        assert_eq!(session.judge_failures, 1);
        assert_eq!(session.judge_cost_micros, 7);
        assert_eq!(
            session.models,
            vec!["glm-flash".to_string(), "glm-pro".to_string()]
        );
        assert_eq!(session.first_at, base_ms);
        assert_eq!(session.last_at, base_ms + 1000);
    }

    #[test]
    fn judge_calls_filter_by_request_and_session() {
        let db = seed();
        let first = "00000000-0000-0000-0000-000000000001";
        let second = "00000000-0000-0000-0000-000000000002";
        for (request_id, service) in [(first, "group-judge"), (second, "judge")] {
            crate::store::judge::insert_call(
                &db,
                &crate::store::judge::JudgeCallRecord {
                    request_id: Uuid::parse_str(request_id).unwrap(),
                    question_id: "auto".to_string(),
                    service_id: service.to_string(),
                    revision: 1,
                    protocol_digest: None,
                    effective_source: "heuristic".to_string(),
                    outcome: "ok".to_string(),
                    failure_code: None,
                    latency_ms: 5,
                    prompt_tokens: Some(1),
                    completion_tokens: Some(1),
                    cost_micros: Some(2),
                    created_at: 1_763_000_000,
                    detail: if service == "group-judge" {
                        crate::store::judge::JudgeCallDetail {
                            request_json: Some(
                                r#"{"task":"hi","prompt":"pick","choices":[{"id":"a","label":"A"}]}"#
                                    .to_string(),
                            ),
                            response_json: Some(
                                r#"{"kind":"choice","value":"a","confidence":0.9}"#.to_string(),
                            ),
                            failure_message: None,
                        }
                    } else {
                        Default::default()
                    },
                },
            )
            .unwrap();
        }

        let by_request = crate::store::judge::list_calls(
            &db,
            &crate::store::judge::JudgeCallFilter {
                request_id: Some(first.to_string()),
                limit: 10,
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(by_request.len(), 1);
        assert_eq!(by_request[0].service_id, "group-judge");
        assert_eq!(by_request[0].created_at, 1_763_000_000_000);
        // 详细调用记录：输入摘要与答案以 JSON 返回。
        assert_eq!(by_request[0].request.as_ref().unwrap()["task"], "hi");
        assert_eq!(by_request[0].response.as_ref().unwrap()["value"], "a");
        assert_eq!(by_request[0].effective_source, "heuristic");

        let by_session = crate::store::judge::list_calls(
            &db,
            &crate::store::judge::JudgeCallFilter {
                session_key: Some("s-1".to_string()),
                limit: 10,
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(by_session.len(), 2);
    }

    /// 游标翻页不得重复行：分钟级时间窗内同一秒的多条记录靠账本 id 兜底，
    /// 且游标里的 created_at 必须是 DB 秒（曾经写毫秒，导致第二页重复第一页）。
    #[test]
    fn cursor_paging_does_not_repeat_rows() {
        let db = seed();
        let base_ms: i64 = 1_763_000_000 * 1000;
        let query = LedgerQuery {
            from: Some(base_ms - 1000),
            to: Some(base_ms + 10_000),
            limit: 1,
            ..Default::default()
        };
        let (page1, cursor1) = list_requests_page(&db, &query).unwrap();
        assert_eq!(page1.len(), 1);
        let cursor = cursor1.expect("满页应给出游标");

        let (page2, _) = list_requests_page(
            &db,
            &LedgerQuery {
                cursor: decode_cursor(&cursor),
                ..query.clone()
            },
        )
        .unwrap();
        assert_eq!(page2.len(), 1);
        assert_ne!(
            page2[0].request_id, page1[0].request_id,
            "第二页不能重复第一页的行"
        );
    }

    /// `judge.failed` 不能只数 outcome='failed'：unavailable / budget_exceeded 同样是
    /// 没拿到结论；`answered` / `selected` / `no_match` 是合法答案。
    #[test]
    fn judge_stats_counts_unavailable_as_failure() {
        let db = seed();
        let request_id = Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap();
        for (index, outcome) in [
            "answered",
            "unavailable",
            "selected",
            "no_match",
            "budget_exceeded",
        ]
        .into_iter()
        .enumerate()
        {
            crate::store::judge::insert_call(
                &db,
                &crate::store::judge::JudgeCallRecord {
                    request_id,
                    question_id: format!("q{index}"),
                    service_id: "judge".to_string(),
                    revision: 1,
                    protocol_digest: None,
                    effective_source: "external_llm".to_string(),
                    outcome: outcome.to_string(),
                    failure_code: None,
                    latency_ms: 1,
                    prompt_tokens: None,
                    completion_tokens: None,
                    cost_micros: Some(1),
                    created_at: 1_763_000_000,
                    detail: Default::default(),
                },
            )
            .unwrap();
        }

        let base_ms: i64 = 1_763_000_000 * 1000;
        let summary = stats_summary(&db, base_ms - 1000, base_ms + 10_000).unwrap();
        assert_eq!(summary.judge.calls, 5);
        assert_eq!(summary.judge.failed, 2, "unavailable + budget_exceeded");
        assert_eq!(summary.judge.cost_micros, 5);
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
