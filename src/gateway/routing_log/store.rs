use std::path::{Path, PathBuf};
use std::sync::Mutex;

use anyhow::Context;
use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;

use crate::gateway::api::meta::{
    multimodal_strategy_name, profile_name, routing_mode_name, step_kind_name, summarize_route_reasons,
    tier_name, truncate_user_preview_for_log, work_strategy_name,
};
use crate::gateway::routing::RouteDecision;

const SCHEMA_VERSION: i32 = 5;
/// Keep the newest N routing decisions on disk.
const MAX_ROWS: i64 = 50_000;
const DEFAULT_LIMIT: u32 = 100;
const MAX_LIMIT: u32 = 500;
const MAX_ERROR_REASON_CHARS: usize = 2000;

#[derive(Debug, Clone, Serialize)]
pub struct RoutingLogEntryJson {
    pub id: i64,
    pub timestamp: String,
    /// Planned route at decision time.
    pub route: String,
    pub served_model: Option<String>,
    /// Upstream that actually served the response, when known.
    pub served_route: Option<String>,
    pub step_kind: String,
    pub model: String,
    pub user_preview: String,
    pub difficulty: f64,
    pub reason_codes: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_reason: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct RoutingLogsResponse {
    pub entries: Vec<RoutingLogEntryJson>,
    pub has_older: bool,
}

#[derive(Debug, Default)]
pub struct RoutingLogsQuery {
    pub after_id: Option<i64>,
    pub before_id: Option<i64>,
    pub limit: Option<u32>,
}

pub struct RoutingLogStore {
    path: PathBuf,
    pub(crate) conn: Mutex<Connection>,
}

impl RoutingLogStore {
    pub fn open(data_dir: &Path) -> anyhow::Result<std::sync::Arc<Self>> {
        std::fs::create_dir_all(data_dir)
            .with_context(|| format!("create routing log dir {}", data_dir.display()))?;
        let path = data_dir.join("routing_logs.db");
        let conn = Connection::open(&path)
            .with_context(|| format!("open routing log db {}", path.display()))?;
        conn.execute_batch(
            "PRAGMA journal_mode=WAL; PRAGMA synchronous=NORMAL; PRAGMA busy_timeout=5000;",
        )
        .context("routing log db pragmas")?;
        let store = Self {
            path,
            conn: Mutex::new(conn),
        };
        store.migrate()?;
        Ok(std::sync::Arc::new(store))
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    fn migrate(&self) -> anyhow::Result<()> {
        let conn = self.conn.lock().expect("routing log db mutex");
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS schema_version (version INTEGER NOT NULL);
             CREATE TABLE IF NOT EXISTS routing_logs (
               id INTEGER PRIMARY KEY AUTOINCREMENT,
               recorded_at_unix INTEGER NOT NULL,
               recorded_at_iso TEXT NOT NULL,
               agent_id TEXT NOT NULL,
               model TEXT NOT NULL,
               route TEXT NOT NULL,
               step_kind TEXT NOT NULL,
               profile TEXT NOT NULL,
               mode TEXT NOT NULL,
               difficulty REAL NOT NULL,
               stream INTEGER NOT NULL,
               tokens_in_estimate INTEGER NOT NULL,
               work_strategy TEXT NOT NULL,
               multimodal_strategy TEXT NOT NULL,
               casual_quality_fallback INTEGER NOT NULL,
               edge_prob REAL,
               reason_codes TEXT NOT NULL,
               user_preview TEXT NOT NULL,
               summary TEXT NOT NULL,
               log_line TEXT NOT NULL
             );
             CREATE INDEX IF NOT EXISTS idx_routing_logs_id ON routing_logs(id);
             CREATE INDEX IF NOT EXISTS idx_routing_logs_recorded ON routing_logs(recorded_at_unix, id);",
        )?;
        let version: Option<i32> = conn
            .query_row("SELECT version FROM schema_version LIMIT 1", [], |r| r.get(0))
            .optional()?;
        let version = version.unwrap_or(0);
        if version < 1 {
            conn.execute(
                "INSERT INTO schema_version (version) VALUES (?1)",
                params![SCHEMA_VERSION],
            )?;
        }
        if version < 2 {
            let has_served: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM pragma_table_info('routing_logs') WHERE name = 'served_route'",
                    [],
                    |r| r.get(0),
                )
                .unwrap_or(0);
            if has_served == 0 {
                conn.execute("ALTER TABLE routing_logs ADD COLUMN served_route TEXT", [])?;
            }
            conn.execute("UPDATE schema_version SET version = ?1", params![SCHEMA_VERSION])?;
        }

        if version < 3 {
            conn.execute_batch(
                "CREATE TABLE IF NOT EXISTS request_route_cache (
                   id INTEGER PRIMARY KEY AUTOINCREMENT,
                   hash_code TEXT NOT NULL UNIQUE,
                   route TEXT NOT NULL,
                   model TEXT NOT NULL,
                   created_at INTEGER NOT NULL,
                   updated_at INTEGER NOT NULL
                 );
                 CREATE INDEX IF NOT EXISTS idx_request_route_cache_hash ON request_route_cache(hash_code);
                 CREATE INDEX IF NOT EXISTS idx_request_route_cache_updated ON request_route_cache(updated_at);",
            )?;
            conn.execute("UPDATE schema_version SET version = ?1", params![SCHEMA_VERSION])?;
        }
        if version < 4 {
            let has_served_model: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM pragma_table_info('routing_logs') WHERE name = 'served_model'",
                    [],
                    |r| r.get(0),
                )
                .unwrap_or(0);
            if has_served_model == 0 {
                conn.execute("ALTER TABLE routing_logs ADD COLUMN served_model TEXT", [])?;
            }
            conn.execute("UPDATE schema_version SET version = ?1", params![SCHEMA_VERSION])?;
        }
        if version < 5 {
            let has_error_reason: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM pragma_table_info('routing_logs') WHERE name = 'error_reason'",
                    [],
                    |r| r.get(0),
                )
                .unwrap_or(0);
            if has_error_reason == 0 {
                conn.execute("ALTER TABLE routing_logs ADD COLUMN error_reason TEXT", [])?;
            }
            conn.execute("UPDATE schema_version SET version = ?1", params![SCHEMA_VERSION])?;
        }
        Ok(())
    }

    pub fn record_decision(
        &self,
        decision: &RouteDecision,
        model: &str,
        user_preview: &str,
        stream: bool,
        agent_id: Option<&str>,
    ) -> anyhow::Result<i64> {
        let summary = summarize_route_reasons(decision);
        let user_preview = truncate_user_preview_for_log(user_preview);
        let message = format!(
            "routing: {} 锟?{} | {}",
            step_kind_name(decision.step_kind),
            tier_name(decision.route),
            summary,
        );
        let recorded_at_unix = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let recorded_at_iso = time::OffsetDateTime::now_utc()
            .format(
                &time::macros::format_description!(
                    "[year]-[month]-[day]T[hour]:[minute]:[second].[subsecond digits:6]Z"
                ),
            )
            .unwrap_or_else(|_| "unknown".to_string());
        let agent_id = agent_id.unwrap_or("default");
        let reason_codes = decision.reason_codes.join(",");
        let edge_prob = decision.edge_ok_probability;

        let conn = self.conn.lock().expect("routing log db mutex");
        conn.execute(
            "INSERT INTO routing_logs (
               recorded_at_unix, recorded_at_iso, agent_id, model, route, step_kind,
               profile, mode, difficulty, stream, tokens_in_estimate, work_strategy,
               multimodal_strategy, casual_quality_fallback, edge_prob, reason_codes,
               user_preview, summary, log_line
             ) VALUES (
               ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19
             )",
            params![
                recorded_at_unix as i64,
                recorded_at_iso,
                agent_id,
                model,
                tier_name(decision.route),
                step_kind_name(decision.step_kind),
                profile_name(decision.profile),
                routing_mode_name(decision.mode),
                decision.difficulty as f64,
                if stream { 1 } else { 0 },
                decision.tokens_in_estimate as i64,
                work_strategy_name(decision.work_strategy),
                multimodal_strategy_name(decision.multimodal_strategy),
                if decision.casual_quality_fallback { 1 } else { 0 },
                edge_prob,
                reason_codes,
                user_preview,
                summary,
                message,
            ],
        )?;
        conn.execute(
            "DELETE FROM routing_logs WHERE id <= (
               SELECT MAX(id) - ?1 FROM routing_logs
             )",
            params![MAX_ROWS],
        )?;
        Ok(conn.last_insert_rowid())
    }

pub fn mark_served(&self, id: i64, served_route: &str, served_model: Option<&str>) -> anyhow::Result<()> {
        let conn = self.conn.lock().expect("routing log db mutex");
        conn.execute(
            "UPDATE routing_logs SET served_route = ?1, served_model = ?2 WHERE id = ?3",
            params![served_route, served_model, id],
        )?;
        Ok(())
    }

    pub fn mark_error(&self, id: i64, error_reason: &str) -> anyhow::Result<()> {
        let reason = truncate_error_reason(error_reason);
        let conn = self.conn.lock().expect("routing log db mutex");
        conn.execute(
            "UPDATE routing_logs SET error_reason = ?1 WHERE id = ?2",
            params![reason, id],
        )?;
        Ok(())
    }

    pub fn query(&self, query: RoutingLogsQuery) -> anyhow::Result<RoutingLogsResponse> {
        let limit = query
            .limit
            .unwrap_or(DEFAULT_LIMIT)
            .clamp(1, MAX_LIMIT) as i64;

        let conn = self.conn.lock().expect("routing log db mutex");

        if let Some(before_id) = query.before_id {
            let mut stmt = conn.prepare(
                "SELECT id, recorded_at_iso, route, served_route, step_kind, model, user_preview, difficulty, reason_codes, served_model, error_reason
                 FROM routing_logs
                 WHERE id < ?1
                 ORDER BY id DESC
                 LIMIT ?2",
            )?;
            let rows = stmt.query_map(params![before_id, limit], row_to_entry)?;
            let mut entries: Vec<RoutingLogEntryJson> = rows.collect::<Result<_, _>>()?;
            entries.reverse();
            let has_older = if let Some(first) = entries.first() {
                conn.query_row(
                    "SELECT EXISTS(SELECT 1 FROM routing_logs WHERE id < ?1)",
                    params![first.id],
                    |r| r.get::<_, i64>(0),
                )? != 0
            } else {
                false
            };
            return Ok(RoutingLogsResponse { entries, has_older });
        }

        if let Some(after_id) = query.after_id {
            let mut stmt = conn.prepare(
                "SELECT id, recorded_at_iso, route, served_route, step_kind, model, user_preview, difficulty, reason_codes, served_model, error_reason
                 FROM routing_logs
                 WHERE id > ?1
                 ORDER BY id ASC
                 LIMIT ?2",
            )?;
            let entries = stmt
                .query_map(params![after_id, limit], row_to_entry)?
                .collect::<Result<Vec<_>, _>>()?;
            return Ok(RoutingLogsResponse {
                entries,
                has_older: true,
            });
        }

        let mut stmt = conn.prepare(
            "SELECT id, recorded_at_iso, route, served_route, step_kind, model, user_preview, difficulty, reason_codes, served_model, error_reason
             FROM routing_logs
             ORDER BY id DESC
             LIMIT ?1",
        )?;
        let mut entries: Vec<RoutingLogEntryJson> = stmt
            .query_map(params![limit], row_to_entry)?
            .collect::<Result<_, _>>()?;
        entries.reverse();
        let has_older = if let Some(first) = entries.first() {
            conn.query_row(
                "SELECT EXISTS(SELECT 1 FROM routing_logs WHERE id < ?1)",
                params![first.id],
                |r| r.get::<_, i64>(0),
            )? != 0
        } else {
            false
        };
        Ok(RoutingLogsResponse { entries, has_older })
    }
}

fn truncate_error_reason(reason: &str) -> String {
    let trimmed = reason.trim();
    let char_count = trimmed.chars().count();
    if char_count <= MAX_ERROR_REASON_CHARS {
        return trimmed.to_string();
    }
    let mut out: String = trimmed.chars().take(MAX_ERROR_REASON_CHARS).collect();
    out.push_str("\u{2026}");
    out
}

fn row_to_entry(row: &rusqlite::Row<'_>) -> rusqlite::Result<RoutingLogEntryJson> {
    let reason_codes_raw: String = row.get(8)?;
    let error_reason: Option<String> = row.get(10)?;
    Ok(RoutingLogEntryJson {
        id: row.get(0)?,
        timestamp: row.get(1)?,
        route: row.get(2)?,
        served_model: row.get(9)?,
        served_route: row.get(3)?,
        step_kind: row.get(4)?,
        model: row.get(5)?,
        user_preview: row.get(6)?,
        difficulty: row.get(7)?,
        reason_codes: reason_codes_raw
            .split(',')
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .collect(),
        error_reason: error_reason.filter(|s| !s.trim().is_empty()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gateway::multimodal::MultimodalStrategy;
    use crate::gateway::routing::{
        Profile, RouteDecision, RouteTier, RoutingMode, StepKind, WorkStrategy,
    };

    fn sample_decision() -> RouteDecision {
        RouteDecision {
            route: RouteTier::Edge,
            profile: Profile::Balanced,
            mode: RoutingMode::Cascade,
            step_kind: StepKind::DirectChat,
            difficulty: 0.2,
            reason_codes: vec!["STEP_DIRECT_CHAT".into(), "DIFFICULTY_0.20".into()],
            tokens_in_estimate: 100,
            tokens_out_estimate: 50,
            cloud_input_saved_estimate: 100,
            conversation_key: "conv:test".into(),
            assistant_failed_recent: false,
            consecutive_tool_error_streak: 0,
            multimodal_strategy: MultimodalStrategy::None,
            work_strategy: WorkStrategy::None,
            force_cloud_sticky: false,
            edge_ok_probability: None,
            classifier_features: None,
            casual_quality_fallback: true,
            lexical_learn: Default::default(),
            routing_log_id: None,
        }
    }

    #[test]
    fn record_and_query_routing_logs() {
        let dir = std::env::temp_dir().join(format!(
            "flowy-routing-log-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let store = RoutingLogStore::open(&dir).unwrap();
        store
            .record_decision(
                &sample_decision(),
                "gpt-4o",
                "hello world",
                false,
                Some("agent-a"),
            )
            .unwrap();

        let initial = store.query(RoutingLogsQuery::default()).unwrap();
        assert_eq!(initial.entries.len(), 1);
        assert!(!initial.has_older);
        assert_eq!(initial.entries[0].model, "gpt-4o");
        assert_eq!(initial.entries[0].user_preview, "hello world");
        assert_eq!(initial.entries[0].route, "edge");
        assert_eq!(initial.entries[0].served_route, None);

        store.mark_served(initial.entries[0].id, "edge", None).unwrap();
        let served = store.query(RoutingLogsQuery::default()).unwrap();
        assert_eq!(served.entries[0].served_route.as_deref(), Some("edge"));

        store
            .mark_error(initial.entries[0].id, "Upstream request failed: timeout")
            .unwrap();
        let failed = store.query(RoutingLogsQuery::default()).unwrap();
        assert_eq!(
            failed.entries[0].error_reason.as_deref(),
            Some("Upstream request failed: timeout")
        );

        let after = store
            .query(RoutingLogsQuery {
                after_id: Some(initial.entries[0].id),
                ..Default::default()
            })
            .unwrap();
        assert!(after.entries.is_empty());

        store
            .record_decision(&sample_decision(), "auto", "second", true, None)
            .unwrap();
        let polled = store
            .query(RoutingLogsQuery {
                after_id: Some(initial.entries[0].id),
                ..Default::default()
            })
            .unwrap();
        assert_eq!(polled.entries.len(), 1);
        assert_eq!(polled.entries[0].model, "auto");
    }
}







