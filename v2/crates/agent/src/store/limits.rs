//! 网关 key 限额用量统计（v3 P5-B）。
//!
//! 用量来源是账本（`ledger.authority_json` 的 `authority_id`），
//! 窗口起点由 `tr_contracts::window_start_millis` 给出。
//! **注意**：`ledger.created_at` 在库里是 Unix **秒**（契约 API 是毫秒）。

use rusqlite::params;
use tr_contracts::KeyLimitHit;

use crate::store::{Db, StoreError};

/// 某授权身份在一个窗口内的累计用量。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct AuthorityUsage {
    pub tokens: u64,
    pub cost_micros: u64,
    pub requests: u64,
}

/// 统计 `authority_id` 自 `since_millis` 起的用量。
pub fn usage_for_authority(
    db: &Db,
    authority_id: &str,
    since_millis: i64,
) -> Result<AuthorityUsage, StoreError> {
    let since_secs = since_millis / 1000;
    let connection = db.read();
    let mut statement = connection.prepare(
        "SELECT COALESCE(SUM(COALESCE(input_tokens,0) + COALESCE(output_tokens,0)), 0),
                COALESCE(SUM(COALESCE(answer_cost_micros,0) + COALESCE(routing_cost_micros,0)), 0),
                COUNT(*)
           FROM ledger
          WHERE created_at >= ?1
            AND json_extract(authority_json, '$.authority_id') = ?2",
    )?;
    let (tokens, cost_micros, requests): (i64, i64, i64) = statement
        .query_row(params![since_secs, authority_id], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?))
        })?;
    Ok(AuthorityUsage {
        tokens: tokens.max(0) as u64,
        cost_micros: cost_micros.max(0) as u64,
        requests: requests.max(0) as u64,
    })
}

/// 检查限额；超限返回命中详情。
///
/// 语义：
/// - `tokens = 0` 且 `cost_micros = 0` → 不限；
/// - `models` 非空时，只有命中模型才计入（调用方传当前请求模型）；
/// - 限额按「已达上限即拒绝」，用 `>=` 而不是 `>`。
pub fn check_limits(
    db: &Db,
    token_id: &str,
    limits: &tr_contracts::KeyLimits,
    request_model: &str,
    now_millis: i64,
) -> Result<Option<KeyLimitHit>, StoreError> {
    let unlimited = limits.tokens == 0 && limits.cost_micros == 0;
    if unlimited {
        return Ok(None);
    }
    if !limits.models.is_empty() && !limits.models.iter().any(|model| model == request_model) {
        return Ok(None);
    }

    let window_start = tr_contracts::window_start_millis(&limits.window, now_millis);
    let usage = usage_for_authority(db, token_id, window_start)?;

    let tokens_hit = limits.tokens > 0 && usage.tokens >= limits.tokens;
    let cost_hit = limits.cost_micros > 0 && usage.cost_micros >= limits.cost_micros;
    if !tokens_hit && !cost_hit {
        return Ok(None);
    }

    let reset_at = tr_contracts::window_end_millis(&limits.window, now_millis);
    Ok(Some(KeyLimitHit {
        token_id: token_id.to_string(),
        window: limits.window.clone(),
        used_tokens: usage.tokens,
        limit_tokens: limits.tokens,
        used_cost_micros: usage.cost_micros,
        limit_cost_micros: limits.cost_micros,
        reset_at,
        retry_after_secs: ((reset_at - now_millis).max(0) / 1000) as u64,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tr_contracts::KeyLimits;

    fn db() -> Db {
        Db::open_in_memory().unwrap()
    }

    /// 直接插账本行，避免构造完整 LedgerRecord。
    fn insert_ledger(db: &Db, request_id: &str, authority: &str, created_at_secs: i64) {
        let connection = db.write();
        connection
            .execute(
                "INSERT INTO ledger (
                    request_id, plan_id, session_key, branch_id, session_key_source,
                    ingress_protocol, selection_model, selection_level, plan_mode,
                    preference_json, failover_json, attempts_json, switches,
                    reason_codes_json, outcome, http_status,
                    input_tokens, output_tokens, answer_cost_micros, routing_cost_micros,
                    created_at, authority_json, facts_completeness
                 ) VALUES (?1, 'p', 's', 'b', 'client', 'chat_completions',
                    'vm', 'off', 'single', '[]', '[]', '[]', 0, '[]', 'success', 200,
                    10, 5, 100, 0, ?2, ?3, 'complete')",
                rusqlite::params![
                    request_id,
                    created_at_secs,
                    format!("{{\"authority_id\":\"{authority}\"}}")
                ],
            )
            .unwrap();
    }

    #[test]
    fn usage_sums_window() {
        let db = db();
        insert_ledger(&db, "r1", "tok-1", 1_700_000_000);
        insert_ledger(&db, "r2", "tok-1", 1_700_000_100);
        insert_ledger(&db, "r3", "other", 1_700_000_100);
        let usage = usage_for_authority(&db, "tok-1", 1_700_000_000_000).unwrap();
        assert_eq!(usage.tokens, 30);
        assert_eq!(usage.cost_micros, 200);
        assert_eq!(usage.requests, 2);
    }

    #[test]
    fn limit_hit_returns_reset_time() {
        let db = db();
        insert_ledger(&db, "r1", "tok-1", 1_700_000_000);
        let limits = KeyLimits {
            window: "day".to_string(),
            tokens: 10,
            cost_micros: 0,
            models: Vec::new(),
        };
        let now = 1_700_000_500_000i64;
        let hit = check_limits(&db, "tok-1", &limits, "gpt-4o", now).unwrap();
        let hit = hit.expect("must hit");
        assert_eq!(hit.used_tokens, 15);
        assert!(hit.retry_after_secs > 0);
    }

    #[test]
    fn model_subset_skips_other_models() {
        let db = db();
        insert_ledger(&db, "r1", "tok-1", 1_700_000_000);
        let limits = KeyLimits {
            window: "day".to_string(),
            tokens: 1,
            cost_micros: 0,
            models: vec!["only-this".to_string()],
        };
        assert!(
            check_limits(&db, "tok-1", &limits, "other", 1_700_000_500_000)
                .unwrap()
                .is_none()
        );
        assert!(
            check_limits(&db, "tok-1", &limits, "only-this", 1_700_000_500_000)
                .unwrap()
                .is_some()
        );
    }

    #[test]
    fn unlimited_when_zero() {
        let db = db();
        let limits = KeyLimits {
            window: "day".to_string(),
            tokens: 0,
            cost_micros: 0,
            models: Vec::new(),
        };
        assert!(
            check_limits(&db, "tok-1", &limits, "any", 1)
                .unwrap()
                .is_none()
        );
    }
}

#[cfg(test)]
mod five_hour_window_tests {
    use super::*;
    use tr_contracts::KeyLimits;

    /// 计划 key 的 5h 窗口：窗口内累计超限即拒绝，且 Retry-After 不超过 5h。
    #[test]
    fn five_hour_window_enforced() {
        let db = Db::open_in_memory().unwrap();
        let connection = db.write();
        connection
            .execute(
                "INSERT INTO ledger (
                    request_id, plan_id, session_key, branch_id, session_key_source,
                    ingress_protocol, selection_model, selection_level, plan_mode,
                    preference_json, failover_json, attempts_json, switches,
                    reason_codes_json, outcome, http_status,
                    input_tokens, output_tokens, answer_cost_micros, routing_cost_micros,
                    created_at, authority_json, facts_completeness
                 ) VALUES ('r5', 'p', 's', 'b', 'client', 'chat_completions',
                    'vm', 'off', 'single', '[]', '[]', '[]', 0, '[]', 'success', 200,
                    100, 100, 0, 0, ?1, '{\"authority_id\":\"tok-5h\"}', 'complete')",
                rusqlite::params![1_700_000_000i64],
            )
            .unwrap();
        drop(connection);

        let limits = KeyLimits {
            window: "hours:5".to_string(),
            tokens: 150,
            cost_micros: 0,
            models: Vec::new(),
        };
        let now = 1_700_000_000_000i64 + 60_000;
        let hit = check_limits(&db, "tok-5h", &limits, "any", now)
            .unwrap()
            .expect("must hit within 5h window");
        assert_eq!(hit.used_tokens, 200);
        assert!(hit.retry_after_secs > 0 && hit.retry_after_secs <= 5 * 3600);
    }
}
