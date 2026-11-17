//! 用量聚合（v3 P5-C）：按 agent / model / candidate / account / session 分组。
//!
//! 数据源是账本；`created_at` 在库里是 Unix **秒**，入参是毫秒。
//! 维度口径：
//! - `agent` = `authority_id`（每个 agent 一个 token 的约定）；
//! - `model` = 虚拟模型（`selection_model`）；
//! - `candidate` = 实际物理模型（`dispatch_model`；失败请求为空 → `(none)`）；
//! - `account` = 候选 → 账号（经 `quota_windows` 编译期映射；无映射归入 `(unknown)`）；
//! - `session` = `session_key`。

use std::collections::BTreeMap;

use rusqlite::params;
use tr_contracts::{HeatmapCell, HeatmapResponse, UsageDimension, UsageResponse, UsageRow};

use crate::store::{Db, StoreError};

struct RawRow {
    key: String,
    requests: u64,
    successes: u64,
    input_tokens: u64,
    output_tokens: u64,
    cache_read: u64,
    cache_write: u64,
    answer_cost: i64,
    routing_cost: i64,
    ttft_sum: i64,
    ttft_count: u64,
}

impl RawRow {
    fn new(key: String) -> Self {
        Self {
            key,
            requests: 0,
            successes: 0,
            input_tokens: 0,
            output_tokens: 0,
            cache_read: 0,
            cache_write: 0,
            answer_cost: 0,
            routing_cost: 0,
            ttft_sum: 0,
            ttft_count: 0,
        }
    }

    fn into_row(self) -> UsageRow {
        let avg_ttft_ms = if self.ttft_count == 0 {
            None
        } else {
            Some(self.ttft_sum as f64 / self.ttft_count as f64)
        };
        let cache_hit_ratio = if self.input_tokens + self.cache_read == 0 {
            None
        } else {
            Some(self.cache_read as f64 / (self.input_tokens + self.cache_read) as f64)
        };
        UsageRow {
            key: self.key,
            requests: self.requests,
            successes: self.successes,
            input_tokens: self.input_tokens,
            output_tokens: self.output_tokens,
            cache_read_tokens: self.cache_read,
            cache_write_tokens: self.cache_write,
            answer_cost_micros: self.answer_cost,
            routing_cost_micros: self.routing_cost,
            avg_ttft_ms,
            cache_hit_ratio,
        }
    }
}

/// 分组键的 SQL 表达式。
fn key_expression(dimension: UsageDimension) -> &'static str {
    match dimension {
        UsageDimension::Agent => {
            "COALESCE(json_extract(authority_json, '$.authority_id'), '(anonymous)')"
        }
        UsageDimension::Model => "selection_model",
        UsageDimension::Candidate => "COALESCE(dispatch_model, '(none)')",
        // account 维度先按候选聚合，再经映射合并（见 usage_by）。
        UsageDimension::Account => "COALESCE(dispatch_model, '(none)')",
        UsageDimension::Session => "session_key",
    }
}

pub fn usage_by(
    db: &Db,
    dimension: UsageDimension,
    from_ms: i64,
    to_ms: i64,
) -> Result<UsageResponse, StoreError> {
    // 窗口右边界：账本秒级，to_ms 的秒向上取整（对齐既有 stats 口径）。
    let from_secs = from_ms / 1000;
    let to_secs = (to_ms / 1000) + 1;
    let key_expr = key_expression(dimension);

    let sql = format!(
        "SELECT {key_expr} AS k,
                COUNT(*),
                SUM(CASE WHEN outcome = 'success' THEN 1 ELSE 0 END),
                COALESCE(SUM(COALESCE(input_tokens,0)), 0),
                COALESCE(SUM(COALESCE(output_tokens,0)), 0),
                COALESCE(SUM(COALESCE(cache_read_tokens,0)), 0),
                COALESCE(SUM(COALESCE(cache_write_tokens,0)), 0),
                COALESCE(SUM(COALESCE(answer_cost_micros,0)), 0),
                COALESCE(SUM(COALESCE(routing_cost_micros,0)), 0),
                COALESCE(SUM(COALESCE(ttft_ms,0)), 0),
                SUM(CASE WHEN ttft_ms IS NOT NULL THEN 1 ELSE 0 END)
           FROM ledger
          WHERE created_at >= ?1 AND created_at < ?2
          GROUP BY k"
    );

    let connection = db.read();
    let mut statement = connection.prepare(&sql)?;
    let mut rows: BTreeMap<String, RawRow> = BTreeMap::new();
    let mut totals = RawRow::new("__total__".to_string());
    let raw = statement
        .query_map(params![from_secs, to_secs], |row| {
            Ok((
                row.get::<_, String>(0)?,
                RawRow {
                    key: String::new(),
                    requests: row.get::<_, i64>(1)? as u64,
                    successes: row.get::<_, i64>(2)? as u64,
                    input_tokens: row.get::<_, i64>(3)? as u64,
                    output_tokens: row.get::<_, i64>(4)? as u64,
                    cache_read: row.get::<_, i64>(5)? as u64,
                    cache_write: row.get::<_, i64>(6)? as u64,
                    answer_cost: row.get::<_, i64>(7)?,
                    routing_cost: row.get::<_, i64>(8)?,
                    ttft_sum: row.get::<_, i64>(9)?,
                    ttft_count: row.get::<_, i64>(10)? as u64,
                },
            ))
        })?
        .collect::<Result<Vec<_>, _>>()?;

    for (key, mut row) in raw {
        row.key = key.clone();
        accumulate(&mut totals, &row);
        rows.insert(key, row);
    }

    // account 维度：候选 → 账号映射（quota_windows），无映射归 (unknown)。
    let rows = if dimension == UsageDimension::Account {
        let mapping = account_mapping(&connection)?;
        let mut merged: BTreeMap<String, RawRow> = BTreeMap::new();
        for row in rows.into_values() {
            let account = mapping
                .get(&row.key)
                .cloned()
                .unwrap_or_else(|| "(unknown)".to_string());
            let entry = merged
                .entry(account.clone())
                .or_insert_with(|| RawRow::new(account));
            accumulate(entry, &row);
        }
        merged
    } else {
        rows
    };

    let mut rows: Vec<UsageRow> = rows.into_values().map(RawRow::into_row).collect();
    // 按请求数降序，稳定排序靠 key。
    rows.sort_by(|left, right| {
        right
            .requests
            .cmp(&left.requests)
            .then_with(|| left.key.cmp(&right.key))
    });
    let totals = totals.into_row();

    Ok(UsageResponse {
        by: dimension.as_str().to_string(),
        from: from_ms,
        to: to_ms,
        rows,
        totals,
    })
}

fn accumulate(target: &mut RawRow, source: &RawRow) {
    target.requests += source.requests;
    target.successes += source.successes;
    target.input_tokens += source.input_tokens;
    target.output_tokens += source.output_tokens;
    target.cache_read += source.cache_read;
    target.cache_write += source.cache_write;
    target.answer_cost += source.answer_cost;
    target.routing_cost += source.routing_cost;
    target.ttft_sum += source.ttft_sum;
    target.ttft_count += source.ttft_count;
}

fn account_mapping(
    connection: &rusqlite::Connection,
) -> Result<BTreeMap<String, String>, StoreError> {
    let mut statement = connection.prepare(
        "SELECT candidate_id, account_id FROM quota_windows WHERE account_id IS NOT NULL",
    )?;
    let rows = statement
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows.into_iter().collect())
}

/// 热力图：按 UTC 天 × 小时聚合。
pub fn usage_heatmap(db: &Db, from_ms: i64, to_ms: i64) -> Result<HeatmapResponse, StoreError> {
    let from_secs = from_ms / 1000;
    let to_secs = (to_ms / 1000) + 1;
    let connection = db.read();
    let mut statement = connection.prepare(
        "SELECT strftime('%Y-%m-%d', created_at, 'unixepoch') AS day,
                CAST(strftime('%H', created_at, 'unixepoch') AS INTEGER) AS hour,
                COUNT(*),
                COALESCE(SUM(COALESCE(answer_cost_micros,0)), 0),
                COALESCE(SUM(COALESCE(input_tokens,0)), 0),
                COALESCE(SUM(COALESCE(output_tokens,0)), 0)
           FROM ledger
          WHERE created_at >= ?1 AND created_at < ?2
          GROUP BY day, hour
          ORDER BY day, hour",
    )?;
    let cells = statement
        .query_map(params![from_secs, to_secs], |row| {
            Ok(HeatmapCell {
                day: row.get(0)?,
                hour: row.get::<_, i64>(1)? as u32,
                requests: row.get::<_, i64>(2)? as u64,
                answer_cost_micros: row.get(3)?,
                input_tokens: row.get::<_, i64>(4)? as u64,
                output_tokens: row.get::<_, i64>(5)? as u64,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    let max_requests = cells.iter().map(|cell| cell.requests).max().unwrap_or(0);
    Ok(HeatmapResponse {
        from: from_ms,
        to: to_ms,
        cells,
        max_requests,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn db() -> Db {
        Db::open_in_memory().unwrap()
    }

    #[allow(clippy::too_many_arguments)]
    fn insert(
        db: &Db,
        request_id: &str,
        created_at_secs: i64,
        authority: &str,
        model: &str,
        dispatch: Option<&str>,
        session: &str,
        input: i64,
        output: i64,
        cache_read: i64,
        cost: i64,
        success: bool,
    ) {
        let connection = db.write();
        connection
            .execute(
                "INSERT INTO ledger (
                    request_id, plan_id, session_key, branch_id, session_key_source,
                    ingress_protocol, selection_model, selection_level, plan_mode,
                    preference_json, failover_json, attempts_json, switches,
                    reason_codes_json, outcome, http_status,
                    input_tokens, output_tokens, cache_read_tokens,
                    answer_cost_micros, routing_cost_micros,
                    created_at, authority_json, facts_completeness, dispatch_model, ttft_ms
                 ) VALUES (?1, 'p', ?2, 'b', 'client', 'chat_completions',
                    ?3, 'off', 'single', '[]', '[]', '[]', 0, '[]', ?4, 200,
                    ?5, ?6, ?7, ?8, 0, ?9, ?10, 'complete', ?11, 100)",
                params![
                    request_id,
                    session,
                    model,
                    if success { "success" } else { "upstream_error" },
                    input,
                    output,
                    cache_read,
                    cost,
                    created_at_secs,
                    format!("{{\"authority_id\":\"{authority}\"}}"),
                    dispatch,
                ],
            )
            .unwrap();
    }

    #[test]
    fn groups_by_agent_and_totals() {
        let db = db();
        insert(
            &db,
            "r1",
            1_700_000_000,
            "agent-a",
            "auto",
            Some("phy-a"),
            "s1",
            100,
            50,
            20,
            10,
            true,
        );
        insert(
            &db,
            "r2",
            1_700_000_100,
            "agent-a",
            "auto",
            Some("phy-b"),
            "s1",
            200,
            60,
            0,
            20,
            true,
        );
        insert(
            &db,
            "r3",
            1_700_000_200,
            "agent-b",
            "auto",
            None,
            "s2",
            10,
            5,
            0,
            1,
            false,
        );

        let usage = usage_by(
            &db,
            UsageDimension::Agent,
            1_699_999_000_000,
            1_700_001_000_000,
        )
        .unwrap();
        assert_eq!(usage.rows.len(), 2);
        assert_eq!(usage.rows[0].key, "agent-a");
        assert_eq!(usage.rows[0].requests, 2);
        assert_eq!(usage.rows[0].input_tokens, 300);
        assert_eq!(usage.rows[0].successes, 2);
        assert_eq!(usage.totals.requests, 3);
        assert_eq!(usage.totals.answer_cost_micros, 31);
        // 缓存命中率 = 20 / (310 + 20)
        assert!((usage.totals.cache_hit_ratio.unwrap() - 20.0 / 330.0).abs() < 1e-9);
        assert_eq!(usage.totals.avg_ttft_ms, Some(100.0));
    }

    #[test]
    fn candidate_dimension_counts_failures_as_none() {
        let db = db();
        insert(
            &db,
            "r1",
            1_700_000_000,
            "a",
            "auto",
            Some("phy-a"),
            "s1",
            1,
            1,
            0,
            1,
            true,
        );
        insert(
            &db,
            "r2",
            1_700_000_000,
            "a",
            "auto",
            None,
            "s1",
            1,
            0,
            0,
            0,
            false,
        );
        let usage = usage_by(
            &db,
            UsageDimension::Candidate,
            1_699_999_000_000,
            1_700_001_000_000,
        )
        .unwrap();
        assert!(usage.rows.iter().any(|row| row.key == "(none)"));
    }

    #[test]
    fn account_dimension_uses_mapping() {
        let db = db();
        crate::store::quota::ensure_mapping(&db, "phy-a", Some("acc-1"), Some("prov-1")).unwrap();
        insert(
            &db,
            "r1",
            1_700_000_000,
            "a",
            "auto",
            Some("phy-a"),
            "s1",
            1,
            1,
            0,
            1,
            true,
        );
        insert(
            &db,
            "r2",
            1_700_000_000,
            "a",
            "auto",
            Some("phy-z"),
            "s1",
            1,
            1,
            0,
            2,
            true,
        );
        let usage = usage_by(
            &db,
            UsageDimension::Account,
            1_699_999_000_000,
            1_700_001_000_000,
        )
        .unwrap();
        let keys: Vec<_> = usage.rows.iter().map(|row| row.key.as_str()).collect();
        assert!(keys.contains(&"acc-1"));
        assert!(keys.contains(&"(unknown)"));
    }

    #[test]
    fn heatmap_buckets_by_hour() {
        let db = db();
        insert(
            &db,
            "r1",
            1_700_000_000,
            "a",
            "auto",
            Some("p"),
            "s",
            1,
            1,
            0,
            5,
            true,
        );
        insert(
            &db,
            "r2",
            1_700_000_030,
            "a",
            "auto",
            Some("p"),
            "s",
            1,
            1,
            0,
            5,
            true,
        );
        let heatmap = usage_heatmap(&db, 1_699_999_000_000, 1_700_001_000_000).unwrap();
        assert_eq!(heatmap.max_requests, 2);
        assert!(heatmap.cells.iter().any(|cell| cell.requests == 2));
    }
}
