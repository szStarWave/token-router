//! 候选性能统计（v3 P5-D `fast` 策略的信号源）。
//!
//! 从账本 `attempts_json` 提取每个候选在时间窗内的平均 TTFT。
//! 数据只用于排序（偏好更快候选），不参与健康/熔断判断。

use std::collections::BTreeMap;

use rusqlite::params;

use crate::store::{Db, StoreError};

/// 候选最近窗口的平均 TTFT（毫秒）与样本数。
pub fn recent_ttft(
    db: &Db,
    since_secs: i64,
    min_samples: u32,
) -> Result<BTreeMap<String, (f64, u64)>, StoreError> {
    let connection = db.read();
    let mut statement = connection.prepare(
        "SELECT json_extract(attempt.value, '$.candidate_id') AS candidate,
                AVG(json_extract(attempt.value, '$.ttft_ms')) AS avg_ttft,
                COUNT(*) AS samples
           FROM ledger, json_each(ledger.attempts_json) AS attempt
          WHERE ledger.created_at >= ?1
            AND json_extract(attempt.value, '$.ttft_ms') IS NOT NULL
          GROUP BY candidate
         HAVING samples >= ?2",
    )?;
    let rows = statement
        .query_map(params![since_secs, min_samples], |row| {
            Ok((
                row.get::<_, Option<String>>(0)?,
                row.get::<_, Option<f64>>(1)?,
                row.get::<_, i64>(2)? as u64,
            ))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    let mut out = BTreeMap::new();
    for (candidate, avg_ttft, samples) in rows {
        if let (Some(candidate), Some(avg)) = (candidate, avg_ttft) {
            out.insert(candidate, (avg, samples));
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn insert_attempts(db: &Db, request_id: &str, created_at: i64, attempts_json: &str) {
        let connection = db.write();
        connection
            .execute(
                "INSERT INTO ledger (
                    request_id, plan_id, session_key, branch_id, session_key_source,
                    ingress_protocol, selection_model, selection_level, plan_mode,
                    preference_json, failover_json, attempts_json, switches,
                    reason_codes_json, outcome, http_status,
                    created_at, authority_json, facts_completeness
                 ) VALUES (?1, 'p', 's', 'b', 'client', 'chat_completions',
                    'vm', 'off', 'single', '[]', '[]', ?2, 0, '[]', 'success', 200,
                    ?3, '{}', 'complete')",
                params![request_id, attempts_json, created_at],
            )
            .unwrap();
    }

    #[test]
    fn averages_per_candidate_with_min_samples() {
        let db = Db::open_in_memory().unwrap();
        insert_attempts(
            &db,
            "r1",
            1_700_000_000,
            r#"[{"candidate_id":"a","ttft_ms":100},{"candidate_id":"b","ttft_ms":300}]"#,
        );
        insert_attempts(
            &db,
            "r2",
            1_700_000_100,
            r#"[{"candidate_id":"a","ttft_ms":200},{"candidate_id":"b","ttft_ms":null}]"#,
        );
        let stats = recent_ttft(&db, 1_699_999_000, 2).unwrap();
        assert_eq!(stats.get("a"), Some(&(150.0, 2)));
        assert!(!stats.contains_key("b"), "b 只有 1 个有效样本");

        let stats = recent_ttft(&db, 1_699_999_000, 1).unwrap();
        assert_eq!(stats.get("b"), Some(&(300.0, 1)));
    }

    #[test]
    fn window_excludes_old_rows() {
        let db = Db::open_in_memory().unwrap();
        insert_attempts(&db, "r1", 100, r#"[{"candidate_id":"a","ttft_ms":50}]"#);
        let stats = recent_ttft(&db, 1_000, 1).unwrap();
        assert!(stats.is_empty());
    }
}
