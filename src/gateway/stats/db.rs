use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use anyhow::Context;
use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;

use super::data::{self, StatsData};
use super::StatsScope;

const SCHEMA_VERSION: i32 = 2;
const MIGRATION_FLAG_STATS_JSON: &str = "migrated_stats_json_v1";
const COUNTER_COLS: &str = "requests_total, requests_stream, requests_non_stream, requests_cancelled, \
     route_edge, route_cloud, route_cascade, \
     upstream_edge_calls, upstream_cloud_calls, cascade_edge_ok, cascade_fallback, \
     errors_total, errors_unauthorized, errors_unavailable, errors_upstream, errors_bad_request, \
     tokens_in_estimate, tokens_out_estimate, cloud_input_saved_estimate, \
     difficulty_sum, difficulty_count, \
     edge_tokens_in, edge_tokens_out, edge_cached_tokens, \
     cloud_tokens_in, cloud_tokens_out, cloud_cached_tokens, \
     cloud_tokens_saved_input, cloud_tokens_saved_output, \
     cache_hit_requests, cached_tokens_total, \
     latency_sum_ms, latency_count, \
     stream_latency_sum_ms, stream_latency_count, \
     non_stream_latency_sum_ms, non_stream_latency_count, \
     ttft_sum_ms, ttft_count, \
     tps_sum_x1000, tps_count, \
     edge_tps_sum_x1000, edge_tps_count, \
     cloud_tps_sum_x1000, cloud_tps_count, \
     edge_served_responses, cloud_served_responses";

fn counter_values(d: &StatsData) -> [i64; 47] {
    [
        d.requests_total as i64,
        d.requests_stream as i64,
        d.requests_non_stream as i64,
        d.requests_cancelled as i64,
        d.route_edge as i64,
        d.route_cloud as i64,
        d.route_cascade as i64,
        d.upstream_edge_calls as i64,
        d.upstream_cloud_calls as i64,
        d.cascade_edge_ok as i64,
        d.cascade_fallback as i64,
        d.errors_total as i64,
        d.errors_unauthorized as i64,
        d.errors_unavailable as i64,
        d.errors_upstream as i64,
        d.errors_bad_request as i64,
        d.tokens_in_estimate as i64,
        d.tokens_out_estimate as i64,
        d.cloud_input_saved_estimate as i64,
        d.difficulty_sum as i64,
        d.difficulty_count as i64,
        d.edge_tokens_in as i64,
        d.edge_tokens_out as i64,
        d.edge_cached_tokens as i64,
        d.cloud_tokens_in as i64,
        d.cloud_tokens_out as i64,
        d.cloud_cached_tokens as i64,
        d.cloud_tokens_saved_input as i64,
        d.cloud_tokens_saved_output as i64,
        d.cache_hit_requests as i64,
        d.cached_tokens_total as i64,
        d.latency_sum_ms as i64,
        d.latency_count as i64,
        d.stream_latency_sum_ms as i64,
        d.stream_latency_count as i64,
        d.non_stream_latency_sum_ms as i64,
        d.non_stream_latency_count as i64,
        d.ttft_sum_ms as i64,
        d.ttft_count as i64,
        d.tps_sum_x1000 as i64,
        d.tps_count as i64,
        d.edge_tps_sum_x1000 as i64,
        d.edge_tps_count as i64,
        d.cloud_tps_sum_x1000 as i64,
        d.cloud_tps_count as i64,
        d.edge_served_responses as i64,
        d.cloud_served_responses as i64,
    ]
}

fn delta_values(before: &StatsData, after: &StatsData) -> [i64; 47] {
    let b = counter_values(before);
    let a = counter_values(after);
    std::array::from_fn(|i| (a[i] - b[i]).max(0))
}

fn row_to_counter_values(row: &rusqlite::Row<'_>, start_idx: usize) -> rusqlite::Result<[i64; 47]> {
    let mut vals = [0i64; 47];
    for (i, v) in vals.iter_mut().enumerate() {
        *v = row.get(start_idx + i)?;
    }
    Ok(vals)
}

fn counters_to_stats_data(
    version: u32,
    first: Option<u64>,
    last: Option<u64>,
    vals: [i64; 47],
    step_kinds: HashMap<String, u64>,
) -> StatsData {
    let u = |i: usize| vals[i].max(0) as u64;
    StatsData {
        version,
        first_record_at_unix: first,
        last_updated_at_unix: last,
        requests_total: u(0),
        requests_stream: u(1),
        requests_non_stream: u(2),
        requests_cancelled: u(3),
        route_edge: u(4),
        route_cloud: u(5),
        route_cascade: u(6),
        upstream_edge_calls: u(7),
        upstream_cloud_calls: u(8),
        cascade_edge_ok: u(9),
        cascade_fallback: u(10),
        errors_total: u(11),
        errors_unauthorized: u(12),
        errors_unavailable: u(13),
        errors_upstream: u(14),
        errors_bad_request: u(15),
        tokens_in_estimate: u(16),
        tokens_out_estimate: u(17),
        cloud_input_saved_estimate: u(18),
        difficulty_sum: u(19),
        difficulty_count: u(20),
        edge_tokens_in: u(21),
        edge_tokens_out: u(22),
        edge_cached_tokens: u(23),
        cloud_tokens_in: u(24),
        cloud_tokens_out: u(25),
        cloud_cached_tokens: u(26),
        cloud_tokens_saved_input: u(27),
        cloud_tokens_saved_output: u(28),
        cache_hit_requests: u(29),
        cached_tokens_total: u(30),
        latency_sum_ms: u(31),
        latency_count: u(32),
        stream_latency_sum_ms: u(33),
        stream_latency_count: u(34),
        non_stream_latency_sum_ms: u(35),
        non_stream_latency_count: u(36),
        ttft_sum_ms: u(37),
        ttft_count: u(38),
        tps_sum_x1000: u(39),
        tps_count: u(40),
        edge_tps_sum_x1000: u(41),
        edge_tps_count: u(42),
        cloud_tps_sum_x1000: u(43),
        cloud_tps_count: u(44),
        edge_served_responses: u(45),
        cloud_served_responses: u(46),
        step_kinds,
    }
}

pub fn hour_bucket_ts(unix_secs: u64) -> u64 {
    unix_secs / 3600 * 3600
}

pub struct StatsDb {
    path: PathBuf,
    conn: Mutex<Connection>,
}

impl StatsDb {
    pub fn open(data_dir: &Path, stats_json_path: &Path) -> anyhow::Result<Self> {
        std::fs::create_dir_all(data_dir)
            .with_context(|| format!("create stats dir {}", data_dir.display()))?;
        let path = data_dir.join("stats.db");
        let conn = Connection::open(&path)
            .with_context(|| format!("open stats db {}", path.display()))?;
        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=NORMAL; PRAGMA busy_timeout=5000;")
            .context("stats db pragmas")?;
        let db = Self {
            path,
            conn: Mutex::new(conn),
        };
        db.migrate()?;
        db.ensure_scope(StatsScope::Global)?;
        db.ensure_scope(StatsScope::Session)?;
        db.clear_session()?;
        db.migrate_stats_json_if_needed(stats_json_path)?;
        Ok(db)
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    fn migrate(&self) -> anyhow::Result<()> {
        let conn = self.conn.lock().expect("stats db mutex");
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS schema_version (version INTEGER NOT NULL);
             CREATE TABLE IF NOT EXISTS migration_flags (
               key TEXT PRIMARY KEY,
               value TEXT NOT NULL
             );
             CREATE TABLE IF NOT EXISTS stats_totals (
               scope TEXT PRIMARY KEY CHECK(scope IN ('session','global')),
               first_record_at_unix INTEGER,
               last_updated_at_unix INTEGER,
               version INTEGER NOT NULL DEFAULT 2,
               requests_total INTEGER NOT NULL DEFAULT 0,
               requests_stream INTEGER NOT NULL DEFAULT 0,
               requests_non_stream INTEGER NOT NULL DEFAULT 0,
               requests_cancelled INTEGER NOT NULL DEFAULT 0,
               route_edge INTEGER NOT NULL DEFAULT 0,
               route_cloud INTEGER NOT NULL DEFAULT 0,
               route_cascade INTEGER NOT NULL DEFAULT 0,
               upstream_edge_calls INTEGER NOT NULL DEFAULT 0,
               upstream_cloud_calls INTEGER NOT NULL DEFAULT 0,
               cascade_edge_ok INTEGER NOT NULL DEFAULT 0,
               cascade_fallback INTEGER NOT NULL DEFAULT 0,
               errors_total INTEGER NOT NULL DEFAULT 0,
               errors_unauthorized INTEGER NOT NULL DEFAULT 0,
               errors_unavailable INTEGER NOT NULL DEFAULT 0,
               errors_upstream INTEGER NOT NULL DEFAULT 0,
               errors_bad_request INTEGER NOT NULL DEFAULT 0,
               tokens_in_estimate INTEGER NOT NULL DEFAULT 0,
               tokens_out_estimate INTEGER NOT NULL DEFAULT 0,
               cloud_input_saved_estimate INTEGER NOT NULL DEFAULT 0,
               difficulty_sum INTEGER NOT NULL DEFAULT 0,
               difficulty_count INTEGER NOT NULL DEFAULT 0,
               edge_tokens_in INTEGER NOT NULL DEFAULT 0,
               edge_tokens_out INTEGER NOT NULL DEFAULT 0,
               edge_cached_tokens INTEGER NOT NULL DEFAULT 0,
               cloud_tokens_in INTEGER NOT NULL DEFAULT 0,
               cloud_tokens_out INTEGER NOT NULL DEFAULT 0,
               cloud_cached_tokens INTEGER NOT NULL DEFAULT 0,
               cloud_tokens_saved_input INTEGER NOT NULL DEFAULT 0,
               cloud_tokens_saved_output INTEGER NOT NULL DEFAULT 0,
               cache_hit_requests INTEGER NOT NULL DEFAULT 0,
               cached_tokens_total INTEGER NOT NULL DEFAULT 0,
               latency_sum_ms INTEGER NOT NULL DEFAULT 0,
               latency_count INTEGER NOT NULL DEFAULT 0,
               stream_latency_sum_ms INTEGER NOT NULL DEFAULT 0,
               stream_latency_count INTEGER NOT NULL DEFAULT 0,
               non_stream_latency_sum_ms INTEGER NOT NULL DEFAULT 0,
               non_stream_latency_count INTEGER NOT NULL DEFAULT 0,
               ttft_sum_ms INTEGER NOT NULL DEFAULT 0,
               ttft_count INTEGER NOT NULL DEFAULT 0,
               tps_sum_x1000 INTEGER NOT NULL DEFAULT 0,
               tps_count INTEGER NOT NULL DEFAULT 0,
               edge_tps_sum_x1000 INTEGER NOT NULL DEFAULT 0,
               edge_tps_count INTEGER NOT NULL DEFAULT 0,
               cloud_tps_sum_x1000 INTEGER NOT NULL DEFAULT 0,
               cloud_tps_count INTEGER NOT NULL DEFAULT 0,
               edge_served_responses INTEGER NOT NULL DEFAULT 0,
               cloud_served_responses INTEGER NOT NULL DEFAULT 0
             );
             CREATE TABLE IF NOT EXISTS stats_step_kinds (
               scope TEXT NOT NULL,
               kind TEXT NOT NULL,
               count INTEGER NOT NULL DEFAULT 0,
               PRIMARY KEY (scope, kind)
             );
             CREATE TABLE IF NOT EXISTS stats_hourly (
               scope TEXT NOT NULL,
               bucket_ts INTEGER NOT NULL,
               requests_total INTEGER NOT NULL DEFAULT 0,
               requests_stream INTEGER NOT NULL DEFAULT 0,
               requests_non_stream INTEGER NOT NULL DEFAULT 0,
               requests_cancelled INTEGER NOT NULL DEFAULT 0,
               route_edge INTEGER NOT NULL DEFAULT 0,
               route_cloud INTEGER NOT NULL DEFAULT 0,
               route_cascade INTEGER NOT NULL DEFAULT 0,
               upstream_edge_calls INTEGER NOT NULL DEFAULT 0,
               upstream_cloud_calls INTEGER NOT NULL DEFAULT 0,
               cascade_edge_ok INTEGER NOT NULL DEFAULT 0,
               cascade_fallback INTEGER NOT NULL DEFAULT 0,
               errors_total INTEGER NOT NULL DEFAULT 0,
               errors_unauthorized INTEGER NOT NULL DEFAULT 0,
               errors_unavailable INTEGER NOT NULL DEFAULT 0,
               errors_upstream INTEGER NOT NULL DEFAULT 0,
               errors_bad_request INTEGER NOT NULL DEFAULT 0,
               tokens_in_estimate INTEGER NOT NULL DEFAULT 0,
               tokens_out_estimate INTEGER NOT NULL DEFAULT 0,
               cloud_input_saved_estimate INTEGER NOT NULL DEFAULT 0,
               difficulty_sum INTEGER NOT NULL DEFAULT 0,
               difficulty_count INTEGER NOT NULL DEFAULT 0,
               edge_tokens_in INTEGER NOT NULL DEFAULT 0,
               edge_tokens_out INTEGER NOT NULL DEFAULT 0,
               edge_cached_tokens INTEGER NOT NULL DEFAULT 0,
               cloud_tokens_in INTEGER NOT NULL DEFAULT 0,
               cloud_tokens_out INTEGER NOT NULL DEFAULT 0,
               cloud_cached_tokens INTEGER NOT NULL DEFAULT 0,
               cloud_tokens_saved_input INTEGER NOT NULL DEFAULT 0,
               cloud_tokens_saved_output INTEGER NOT NULL DEFAULT 0,
               cache_hit_requests INTEGER NOT NULL DEFAULT 0,
               cached_tokens_total INTEGER NOT NULL DEFAULT 0,
               latency_sum_ms INTEGER NOT NULL DEFAULT 0,
               latency_count INTEGER NOT NULL DEFAULT 0,
               stream_latency_sum_ms INTEGER NOT NULL DEFAULT 0,
               stream_latency_count INTEGER NOT NULL DEFAULT 0,
               non_stream_latency_sum_ms INTEGER NOT NULL DEFAULT 0,
               non_stream_latency_count INTEGER NOT NULL DEFAULT 0,
               ttft_sum_ms INTEGER NOT NULL DEFAULT 0,
               ttft_count INTEGER NOT NULL DEFAULT 0,
               tps_sum_x1000 INTEGER NOT NULL DEFAULT 0,
               tps_count INTEGER NOT NULL DEFAULT 0,
               edge_tps_sum_x1000 INTEGER NOT NULL DEFAULT 0,
               edge_tps_count INTEGER NOT NULL DEFAULT 0,
               cloud_tps_sum_x1000 INTEGER NOT NULL DEFAULT 0,
               cloud_tps_count INTEGER NOT NULL DEFAULT 0,
               edge_served_responses INTEGER NOT NULL DEFAULT 0,
               cloud_served_responses INTEGER NOT NULL DEFAULT 0,
               PRIMARY KEY (scope, bucket_ts)
             );
             CREATE INDEX IF NOT EXISTS idx_stats_hourly_scope_ts ON stats_hourly(scope, bucket_ts);
             CREATE TABLE IF NOT EXISTS latency_samples (
               id INTEGER PRIMARY KEY AUTOINCREMENT,
               scope TEXT NOT NULL,
               recorded_at_unix INTEGER NOT NULL,
               latency_ms INTEGER NOT NULL
             );
             CREATE INDEX IF NOT EXISTS idx_latency_samples_scope ON latency_samples(scope, recorded_at_unix);
             CREATE TABLE IF NOT EXISTS auth_key_meta (
               id TEXT PRIMARY KEY,
               name TEXT NOT NULL,
               key_preview TEXT NOT NULL,
               last_used_at_unix INTEGER
             );
             CREATE TABLE IF NOT EXISTS auth_key_totals (
               scope TEXT NOT NULL CHECK(scope IN ('session','global')),
               auth_key_id TEXT NOT NULL,
               first_record_at_unix INTEGER,
               last_updated_at_unix INTEGER,
               version INTEGER NOT NULL DEFAULT 2,
               requests_total INTEGER NOT NULL DEFAULT 0,
               requests_stream INTEGER NOT NULL DEFAULT 0,
               requests_non_stream INTEGER NOT NULL DEFAULT 0,
               requests_cancelled INTEGER NOT NULL DEFAULT 0,
               route_edge INTEGER NOT NULL DEFAULT 0,
               route_cloud INTEGER NOT NULL DEFAULT 0,
               route_cascade INTEGER NOT NULL DEFAULT 0,
               upstream_edge_calls INTEGER NOT NULL DEFAULT 0,
               upstream_cloud_calls INTEGER NOT NULL DEFAULT 0,
               cascade_edge_ok INTEGER NOT NULL DEFAULT 0,
               cascade_fallback INTEGER NOT NULL DEFAULT 0,
               errors_total INTEGER NOT NULL DEFAULT 0,
               errors_unauthorized INTEGER NOT NULL DEFAULT 0,
               errors_unavailable INTEGER NOT NULL DEFAULT 0,
               errors_upstream INTEGER NOT NULL DEFAULT 0,
               errors_bad_request INTEGER NOT NULL DEFAULT 0,
               tokens_in_estimate INTEGER NOT NULL DEFAULT 0,
               tokens_out_estimate INTEGER NOT NULL DEFAULT 0,
               cloud_input_saved_estimate INTEGER NOT NULL DEFAULT 0,
               difficulty_sum INTEGER NOT NULL DEFAULT 0,
               difficulty_count INTEGER NOT NULL DEFAULT 0,
               edge_tokens_in INTEGER NOT NULL DEFAULT 0,
               edge_tokens_out INTEGER NOT NULL DEFAULT 0,
               edge_cached_tokens INTEGER NOT NULL DEFAULT 0,
               cloud_tokens_in INTEGER NOT NULL DEFAULT 0,
               cloud_tokens_out INTEGER NOT NULL DEFAULT 0,
               cloud_cached_tokens INTEGER NOT NULL DEFAULT 0,
               cloud_tokens_saved_input INTEGER NOT NULL DEFAULT 0,
               cloud_tokens_saved_output INTEGER NOT NULL DEFAULT 0,
               cache_hit_requests INTEGER NOT NULL DEFAULT 0,
               cached_tokens_total INTEGER NOT NULL DEFAULT 0,
               latency_sum_ms INTEGER NOT NULL DEFAULT 0,
               latency_count INTEGER NOT NULL DEFAULT 0,
               stream_latency_sum_ms INTEGER NOT NULL DEFAULT 0,
               stream_latency_count INTEGER NOT NULL DEFAULT 0,
               non_stream_latency_sum_ms INTEGER NOT NULL DEFAULT 0,
               non_stream_latency_count INTEGER NOT NULL DEFAULT 0,
               ttft_sum_ms INTEGER NOT NULL DEFAULT 0,
               ttft_count INTEGER NOT NULL DEFAULT 0,
               tps_sum_x1000 INTEGER NOT NULL DEFAULT 0,
               tps_count INTEGER NOT NULL DEFAULT 0,
               edge_tps_sum_x1000 INTEGER NOT NULL DEFAULT 0,
               edge_tps_count INTEGER NOT NULL DEFAULT 0,
               cloud_tps_sum_x1000 INTEGER NOT NULL DEFAULT 0,
               cloud_tps_count INTEGER NOT NULL DEFAULT 0,
               edge_served_responses INTEGER NOT NULL DEFAULT 0,
               cloud_served_responses INTEGER NOT NULL DEFAULT 0,
               PRIMARY KEY (scope, auth_key_id)
             );
             CREATE INDEX IF NOT EXISTS idx_auth_key_totals_scope ON auth_key_totals(scope, auth_key_id);",
        )?;
        let version: Option<i32> = conn
            .query_row("SELECT version FROM schema_version LIMIT 1", [], |r| r.get(0))
            .optional()?;
        if version.is_none() {
            conn.execute(
                "INSERT INTO schema_version (version) VALUES (?1)",
                params![SCHEMA_VERSION],
            )?;
        } else if version.unwrap_or(0) < SCHEMA_VERSION {
            conn.execute_batch(
                "CREATE TABLE IF NOT EXISTS auth_key_meta (
                   id TEXT PRIMARY KEY,
                   name TEXT NOT NULL,
                   key_preview TEXT NOT NULL,
                   last_used_at_unix INTEGER
                 );
                 CREATE TABLE IF NOT EXISTS auth_key_totals (
                   scope TEXT NOT NULL CHECK(scope IN ('session','global')),
                   auth_key_id TEXT NOT NULL,
                   first_record_at_unix INTEGER,
                   last_updated_at_unix INTEGER,
                   version INTEGER NOT NULL DEFAULT 2,
                   requests_total INTEGER NOT NULL DEFAULT 0,
                   requests_stream INTEGER NOT NULL DEFAULT 0,
                   requests_non_stream INTEGER NOT NULL DEFAULT 0,
                   requests_cancelled INTEGER NOT NULL DEFAULT 0,
                   route_edge INTEGER NOT NULL DEFAULT 0,
                   route_cloud INTEGER NOT NULL DEFAULT 0,
                   route_cascade INTEGER NOT NULL DEFAULT 0,
                   upstream_edge_calls INTEGER NOT NULL DEFAULT 0,
                   upstream_cloud_calls INTEGER NOT NULL DEFAULT 0,
                   cascade_edge_ok INTEGER NOT NULL DEFAULT 0,
                   cascade_fallback INTEGER NOT NULL DEFAULT 0,
                   errors_total INTEGER NOT NULL DEFAULT 0,
                   errors_unauthorized INTEGER NOT NULL DEFAULT 0,
                   errors_unavailable INTEGER NOT NULL DEFAULT 0,
                   errors_upstream INTEGER NOT NULL DEFAULT 0,
                   errors_bad_request INTEGER NOT NULL DEFAULT 0,
                   tokens_in_estimate INTEGER NOT NULL DEFAULT 0,
                   tokens_out_estimate INTEGER NOT NULL DEFAULT 0,
                   cloud_input_saved_estimate INTEGER NOT NULL DEFAULT 0,
                   difficulty_sum INTEGER NOT NULL DEFAULT 0,
                   difficulty_count INTEGER NOT NULL DEFAULT 0,
                   edge_tokens_in INTEGER NOT NULL DEFAULT 0,
                   edge_tokens_out INTEGER NOT NULL DEFAULT 0,
                   edge_cached_tokens INTEGER NOT NULL DEFAULT 0,
                   cloud_tokens_in INTEGER NOT NULL DEFAULT 0,
                   cloud_tokens_out INTEGER NOT NULL DEFAULT 0,
                   cloud_cached_tokens INTEGER NOT NULL DEFAULT 0,
                   cloud_tokens_saved_input INTEGER NOT NULL DEFAULT 0,
                   cloud_tokens_saved_output INTEGER NOT NULL DEFAULT 0,
                   cache_hit_requests INTEGER NOT NULL DEFAULT 0,
                   cached_tokens_total INTEGER NOT NULL DEFAULT 0,
                   latency_sum_ms INTEGER NOT NULL DEFAULT 0,
                   latency_count INTEGER NOT NULL DEFAULT 0,
                   stream_latency_sum_ms INTEGER NOT NULL DEFAULT 0,
                   stream_latency_count INTEGER NOT NULL DEFAULT 0,
                   non_stream_latency_sum_ms INTEGER NOT NULL DEFAULT 0,
                   non_stream_latency_count INTEGER NOT NULL DEFAULT 0,
                   ttft_sum_ms INTEGER NOT NULL DEFAULT 0,
                   ttft_count INTEGER NOT NULL DEFAULT 0,
                   tps_sum_x1000 INTEGER NOT NULL DEFAULT 0,
                   tps_count INTEGER NOT NULL DEFAULT 0,
                   edge_tps_sum_x1000 INTEGER NOT NULL DEFAULT 0,
                   edge_tps_count INTEGER NOT NULL DEFAULT 0,
                   cloud_tps_sum_x1000 INTEGER NOT NULL DEFAULT 0,
                   cloud_tps_count INTEGER NOT NULL DEFAULT 0,
                   edge_served_responses INTEGER NOT NULL DEFAULT 0,
                   cloud_served_responses INTEGER NOT NULL DEFAULT 0,
                   PRIMARY KEY (scope, auth_key_id)
                 );
                 CREATE INDEX IF NOT EXISTS idx_auth_key_totals_scope ON auth_key_totals(scope, auth_key_id);",
            )?;
            conn.execute(
                "UPDATE schema_version SET version = ?1",
                params![SCHEMA_VERSION],
            )?;
        }
        Ok(())
    }

    fn ensure_scope(&self, scope: StatsScope) -> anyhow::Result<()> {
        let conn = self.conn.lock().expect("stats db mutex");
        conn.execute(
            "INSERT OR IGNORE INTO stats_totals (scope, version) VALUES (?1, 2)",
            params![scope.as_str()],
        )?;
        Ok(())
    }

    pub fn clear_session(&self) -> anyhow::Result<()> {
        let scope = StatsScope::Session.as_str();
        let conn = self.conn.lock().expect("stats db mutex");
        let tx = conn.unchecked_transaction()?;
        tx.execute("DELETE FROM stats_step_kinds WHERE scope = ?1", params![scope])?;
        tx.execute("DELETE FROM stats_hourly WHERE scope = ?1", params![scope])?;
        tx.execute("DELETE FROM latency_samples WHERE scope = ?1", params![scope])?;
        tx.execute(
            "UPDATE stats_totals SET
               first_record_at_unix = NULL, last_updated_at_unix = NULL, version = 2,
               requests_total = 0, requests_stream = 0, requests_non_stream = 0, requests_cancelled = 0,
               route_edge = 0, route_cloud = 0, route_cascade = 0,
               upstream_edge_calls = 0, upstream_cloud_calls = 0, cascade_edge_ok = 0, cascade_fallback = 0,
               errors_total = 0, errors_unauthorized = 0, errors_unavailable = 0, errors_upstream = 0, errors_bad_request = 0,
               tokens_in_estimate = 0, tokens_out_estimate = 0, cloud_input_saved_estimate = 0,
               difficulty_sum = 0, difficulty_count = 0,
               edge_tokens_in = 0, edge_tokens_out = 0, edge_cached_tokens = 0,
               cloud_tokens_in = 0, cloud_tokens_out = 0, cloud_cached_tokens = 0,
               cloud_tokens_saved_input = 0, cloud_tokens_saved_output = 0,
               cache_hit_requests = 0, cached_tokens_total = 0,
               latency_sum_ms = 0, latency_count = 0,
               stream_latency_sum_ms = 0, stream_latency_count = 0,
               non_stream_latency_sum_ms = 0, non_stream_latency_count = 0,
               ttft_sum_ms = 0, ttft_count = 0,
               tps_sum_x1000 = 0, tps_count = 0,
               edge_tps_sum_x1000 = 0, edge_tps_count = 0,
               cloud_tps_sum_x1000 = 0, cloud_tps_count = 0,
               edge_served_responses = 0, cloud_served_responses = 0
             WHERE scope = ?1",
            params![scope],
        )?;
        tx.execute("DELETE FROM auth_key_totals WHERE scope = ?1", params![scope])?;
        tx.commit()?;
        Ok(())
    }

    pub fn load_totals(&self, scope: StatsScope) -> anyhow::Result<StatsData> {
        let conn = self.conn.lock().expect("stats db mutex");
        let sql = format!(
            "SELECT first_record_at_unix, last_updated_at_unix, version, {COUNTER_COLS}
             FROM stats_totals WHERE scope = ?1"
        );
        let row = conn.query_row(&sql, params![scope.as_str()], |row| {
            let first: Option<i64> = row.get(0)?;
            let last: Option<i64> = row.get(1)?;
            let version: i32 = row.get(2)?;
            let vals = row_to_counter_values(row, 3)?;
            Ok((
                first.map(|v| v.max(0) as u64),
                last.map(|v| v.max(0) as u64),
                version.max(0) as u32,
                vals,
            ))
        })?;
        let step_kinds = self.load_step_kinds_locked(&conn, scope)?;
        Ok(counters_to_stats_data(
            row.2,
            row.0,
            row.1,
            row.3,
            step_kinds,
        ))
    }

    fn load_step_kinds_locked(
        &self,
        conn: &Connection,
        scope: StatsScope,
    ) -> anyhow::Result<HashMap<String, u64>> {
        let mut stmt = conn.prepare(
            "SELECT kind, count FROM stats_step_kinds WHERE scope = ?1 ORDER BY kind",
        )?;
        let rows = stmt.query_map(params![scope.as_str()], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?.max(0) as u64))
        })?;
        let mut map = HashMap::new();
        for row in rows {
            let (k, v) = row?;
            map.insert(k, v);
        }
        Ok(map)
    }

    pub fn load_latency_samples(&self, scope: StatsScope) -> anyhow::Result<Vec<u64>> {
        let conn = self.conn.lock().expect("stats db mutex");
        let mut stmt = conn.prepare(
            "SELECT latency_ms FROM latency_samples WHERE scope = ?1 ORDER BY recorded_at_unix ASC, id ASC",
        )?;
        let rows = stmt.query_map(params![scope.as_str()], |row| {
            Ok(row.get::<_, i64>(0)?.max(0) as u64)
        })?;
        rows.collect::<Result<Vec<_>, _>>().context("load latency samples")
    }

    pub fn latency_percentiles(&self, scope: StatsScope) -> anyhow::Result<(f64, f64)> {
        let samples = self.load_latency_samples(scope)?;
        Ok(latency_percentiles_from_samples(&samples))
    }

    /// Apply an update to both scopes inside one transaction.
    pub fn with_mut<F>(&self, update: F, bucket_ts: u64, latency_ms: Option<u64>) -> anyhow::Result<()>
    where
        F: Fn(&mut StatsData),
    {
        let conn = self.conn.lock().expect("stats db mutex");
        let tx = conn.unchecked_transaction()?;
        for scope in [StatsScope::Global, StatsScope::Session] {
            self.apply_scope_update(&tx, scope, &update, bucket_ts)?;
            if let Some(ms) = latency_ms {
                if ms > 0 {
                    tx.execute(
                        "INSERT INTO latency_samples (scope, recorded_at_unix, latency_ms) VALUES (?1, ?2, ?3)",
                        params![scope.as_str(), data::now_unix() as i64, ms as i64],
                    )?;
                }
            }
        }
        tx.commit()?;
        Ok(())
    }

    fn apply_scope_update<F>(
        &self,
        tx: &rusqlite::Transaction<'_>,
        scope: StatsScope,
        update: &F,
        bucket_ts: u64,
    ) -> anyhow::Result<()>
    where
        F: Fn(&mut StatsData),
    {
        let before = self.load_totals_in_tx(tx, scope)?;
        let mut after = before.clone();
        update(&mut after);

        let step_delta = step_kind_delta(&before.step_kinds, &after.step_kinds);

        self.save_totals_in_tx(tx, scope, &after)?;
        let deltas = delta_values(&before, &after);
        if deltas.iter().any(|&v| v > 0) {
            self.upsert_hourly_delta(tx, scope, bucket_ts, &deltas)?;
        }
        for (kind, count) in step_delta {
            tx.execute(
                "INSERT INTO stats_step_kinds (scope, kind, count) VALUES (?1, ?2, ?3)
                 ON CONFLICT(scope, kind) DO UPDATE SET count = count + excluded.count",
                params![scope.as_str(), kind, count as i64],
            )?;
        }
        Ok(())
    }

    fn load_totals_in_tx(
        &self,
        tx: &rusqlite::Transaction<'_>,
        scope: StatsScope,
    ) -> anyhow::Result<StatsData> {
        let sql = format!(
            "SELECT first_record_at_unix, last_updated_at_unix, version, {COUNTER_COLS}
             FROM stats_totals WHERE scope = ?1"
        );
        let row = tx.query_row(&sql, params![scope.as_str()], |row| {
            let first: Option<i64> = row.get(0)?;
            let last: Option<i64> = row.get(1)?;
            let version: i32 = row.get(2)?;
            let vals = row_to_counter_values(row, 3)?;
            Ok((
                first.map(|v| v.max(0) as u64),
                last.map(|v| v.max(0) as u64),
                version.max(0) as u32,
                vals,
            ))
        })?;
        let mut stmt = tx.prepare("SELECT kind, count FROM stats_step_kinds WHERE scope = ?1")?;
        let rows = stmt.query_map(params![scope.as_str()], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?.max(0) as u64))
        })?;
        let mut step_kinds = HashMap::new();
        for row in rows {
            let (k, v) = row?;
            step_kinds.insert(k, v);
        }
        Ok(counters_to_stats_data(row.2, row.0, row.1, row.3, step_kinds))
    }

    fn save_totals_in_tx(
        &self,
        tx: &rusqlite::Transaction<'_>,
        scope: StatsScope,
        data: &StatsData,
    ) -> anyhow::Result<()> {
        let vals = counter_values(data);
        tx.execute(
            &format!(
                "UPDATE stats_totals SET
                   first_record_at_unix = ?1, last_updated_at_unix = ?2, version = ?3,
                   requests_total = ?4, requests_stream = ?5, requests_non_stream = ?6, requests_cancelled = ?7,
                   route_edge = ?8, route_cloud = ?9, route_cascade = ?10,
                   upstream_edge_calls = ?11, upstream_cloud_calls = ?12, cascade_edge_ok = ?13, cascade_fallback = ?14,
                   errors_total = ?15, errors_unauthorized = ?16, errors_unavailable = ?17, errors_upstream = ?18, errors_bad_request = ?19,
                   tokens_in_estimate = ?20, tokens_out_estimate = ?21, cloud_input_saved_estimate = ?22,
                   difficulty_sum = ?23, difficulty_count = ?24,
                   edge_tokens_in = ?25, edge_tokens_out = ?26, edge_cached_tokens = ?27,
                   cloud_tokens_in = ?28, cloud_tokens_out = ?29, cloud_cached_tokens = ?30,
                   cloud_tokens_saved_input = ?31, cloud_tokens_saved_output = ?32,
                   cache_hit_requests = ?33, cached_tokens_total = ?34,
                   latency_sum_ms = ?35, latency_count = ?36,
                   stream_latency_sum_ms = ?37, stream_latency_count = ?38,
                   non_stream_latency_sum_ms = ?39, non_stream_latency_count = ?40,
                   ttft_sum_ms = ?41, ttft_count = ?42,
                   tps_sum_x1000 = ?43, tps_count = ?44,
                   edge_tps_sum_x1000 = ?45, edge_tps_count = ?46,
                   cloud_tps_sum_x1000 = ?47, cloud_tps_count = ?48,
                   edge_served_responses = ?49, cloud_served_responses = ?50
                 WHERE scope = ?51"
            ),
            params![
                data.first_record_at_unix.map(|v| v as i64),
                data.last_updated_at_unix.map(|v| v as i64),
                data.version as i32,
                vals[0], vals[1], vals[2], vals[3], vals[4], vals[5], vals[6], vals[7], vals[8],
                vals[9], vals[10], vals[11], vals[12], vals[13], vals[14], vals[15], vals[16],
                vals[17], vals[18], vals[19], vals[20], vals[21], vals[22], vals[23], vals[24],
                vals[25], vals[26], vals[27], vals[28], vals[29], vals[30], vals[31], vals[32],
                vals[33], vals[34], vals[35], vals[36], vals[37], vals[38], vals[39], vals[40],
                vals[41], vals[42], vals[43], vals[44], vals[45], vals[46],
                scope.as_str(),
            ],
        )?;
        Ok(())
    }

    fn upsert_hourly_delta(
        &self,
        tx: &rusqlite::Transaction<'_>,
        scope: StatsScope,
        bucket_ts: u64,
        deltas: &[i64; 47],
    ) -> anyhow::Result<()> {
        tx.execute(
            &format!(
                "INSERT INTO stats_hourly (scope, bucket_ts, {COUNTER_COLS}) VALUES (?1, ?2,
                   ?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18,?19,?20,?21,?22,?23,?24,?25,?26,?27,?28,?29,?30,?31,?32,?33,?34,?35,?36,?37,?38,?39,?40,?41,?42,?43,?44,?45,?46,?47,?48,?49)
                 ON CONFLICT(scope, bucket_ts) DO UPDATE SET
                   requests_total = stats_hourly.requests_total + excluded.requests_total,
                   requests_stream = stats_hourly.requests_stream + excluded.requests_stream,
                   requests_non_stream = stats_hourly.requests_non_stream + excluded.requests_non_stream,
                   requests_cancelled = stats_hourly.requests_cancelled + excluded.requests_cancelled,
                   route_edge = stats_hourly.route_edge + excluded.route_edge,
                   route_cloud = stats_hourly.route_cloud + excluded.route_cloud,
                   route_cascade = stats_hourly.route_cascade + excluded.route_cascade,
                   upstream_edge_calls = stats_hourly.upstream_edge_calls + excluded.upstream_edge_calls,
                   upstream_cloud_calls = stats_hourly.upstream_cloud_calls + excluded.upstream_cloud_calls,
                   cascade_edge_ok = stats_hourly.cascade_edge_ok + excluded.cascade_edge_ok,
                   cascade_fallback = stats_hourly.cascade_fallback + excluded.cascade_fallback,
                   errors_total = stats_hourly.errors_total + excluded.errors_total,
                   errors_unauthorized = stats_hourly.errors_unauthorized + excluded.errors_unauthorized,
                   errors_unavailable = stats_hourly.errors_unavailable + excluded.errors_unavailable,
                   errors_upstream = stats_hourly.errors_upstream + excluded.errors_upstream,
                   errors_bad_request = stats_hourly.errors_bad_request + excluded.errors_bad_request,
                   tokens_in_estimate = stats_hourly.tokens_in_estimate + excluded.tokens_in_estimate,
                   tokens_out_estimate = stats_hourly.tokens_out_estimate + excluded.tokens_out_estimate,
                   cloud_input_saved_estimate = stats_hourly.cloud_input_saved_estimate + excluded.cloud_input_saved_estimate,
                   difficulty_sum = stats_hourly.difficulty_sum + excluded.difficulty_sum,
                   difficulty_count = stats_hourly.difficulty_count + excluded.difficulty_count,
                   edge_tokens_in = stats_hourly.edge_tokens_in + excluded.edge_tokens_in,
                   edge_tokens_out = stats_hourly.edge_tokens_out + excluded.edge_tokens_out,
                   edge_cached_tokens = stats_hourly.edge_cached_tokens + excluded.edge_cached_tokens,
                   cloud_tokens_in = stats_hourly.cloud_tokens_in + excluded.cloud_tokens_in,
                   cloud_tokens_out = stats_hourly.cloud_tokens_out + excluded.cloud_tokens_out,
                   cloud_cached_tokens = stats_hourly.cloud_cached_tokens + excluded.cloud_cached_tokens,
                   cloud_tokens_saved_input = stats_hourly.cloud_tokens_saved_input + excluded.cloud_tokens_saved_input,
                   cloud_tokens_saved_output = stats_hourly.cloud_tokens_saved_output + excluded.cloud_tokens_saved_output,
                   cache_hit_requests = stats_hourly.cache_hit_requests + excluded.cache_hit_requests,
                   cached_tokens_total = stats_hourly.cached_tokens_total + excluded.cached_tokens_total,
                   latency_sum_ms = stats_hourly.latency_sum_ms + excluded.latency_sum_ms,
                   latency_count = stats_hourly.latency_count + excluded.latency_count,
                   stream_latency_sum_ms = stats_hourly.stream_latency_sum_ms + excluded.stream_latency_sum_ms,
                   stream_latency_count = stats_hourly.stream_latency_count + excluded.stream_latency_count,
                   non_stream_latency_sum_ms = stats_hourly.non_stream_latency_sum_ms + excluded.non_stream_latency_sum_ms,
                   non_stream_latency_count = stats_hourly.non_stream_latency_count + excluded.non_stream_latency_count,
                   ttft_sum_ms = stats_hourly.ttft_sum_ms + excluded.ttft_sum_ms,
                   ttft_count = stats_hourly.ttft_count + excluded.ttft_count,
                   tps_sum_x1000 = stats_hourly.tps_sum_x1000 + excluded.tps_sum_x1000,
                   tps_count = stats_hourly.tps_count + excluded.tps_count,
                   edge_tps_sum_x1000 = stats_hourly.edge_tps_sum_x1000 + excluded.edge_tps_sum_x1000,
                   edge_tps_count = stats_hourly.edge_tps_count + excluded.edge_tps_count,
                   cloud_tps_sum_x1000 = stats_hourly.cloud_tps_sum_x1000 + excluded.cloud_tps_sum_x1000,
                   cloud_tps_count = stats_hourly.cloud_tps_count + excluded.cloud_tps_count,
                   edge_served_responses = stats_hourly.edge_served_responses + excluded.edge_served_responses,
                   cloud_served_responses = stats_hourly.cloud_served_responses + excluded.cloud_served_responses"
            ),
            params![
                scope.as_str(),
                bucket_ts as i64,
                deltas[0], deltas[1], deltas[2], deltas[3], deltas[4], deltas[5], deltas[6], deltas[7],
                deltas[8], deltas[9], deltas[10], deltas[11], deltas[12], deltas[13], deltas[14], deltas[15],
                deltas[16], deltas[17], deltas[18], deltas[19], deltas[20], deltas[21], deltas[22], deltas[23],
                deltas[24], deltas[25], deltas[26], deltas[27], deltas[28], deltas[29], deltas[30], deltas[31],
                deltas[32], deltas[33], deltas[34], deltas[35], deltas[36], deltas[37], deltas[38], deltas[39],
                deltas[40], deltas[41], deltas[42], deltas[43], deltas[44], deltas[45], deltas[46],
            ],
        )?;
        Ok(())
    }

    fn migrate_stats_json_if_needed(&self, stats_json_path: &Path) -> anyhow::Result<()> {
        let conn = self.conn.lock().expect("stats db mutex");
        let migrated: Option<String> = conn
            .query_row(
                "SELECT value FROM migration_flags WHERE key = ?1",
                params![MIGRATION_FLAG_STATS_JSON],
                |r| r.get(0),
            )
            .optional()?;
        if migrated.is_some() {
            return Ok(());
        }
        if !stats_json_path.exists() {
            conn.execute(
                "INSERT OR REPLACE INTO migration_flags (key, value) VALUES (?1, 'no_file')",
                params![MIGRATION_FLAG_STATS_JSON],
            )?;
            return Ok(());
        }
        let imported = data::load(stats_json_path)?;
        drop(conn);

        let tx_conn = self.conn.lock().expect("stats db mutex");
        let tx = tx_conn.unchecked_transaction()?;
        self.import_global_totals(&tx, &imported)?;
        for (kind, count) in &imported.step_kinds {
            tx.execute(
                "INSERT INTO stats_step_kinds (scope, kind, count) VALUES ('global', ?1, ?2)
                 ON CONFLICT(scope, kind) DO UPDATE SET count = excluded.count",
                params![kind, *count as i64],
            )?;
        }
        let backfill_ts = imported
            .last_updated_at_unix
            .or(imported.first_record_at_unix)
            .map(hour_bucket_ts);
        if let Some(bucket_ts) = backfill_ts {
            let vals = counter_values(&imported);
            if vals.iter().any(|&v| v > 0) {
                self.insert_hourly_absolute(&tx, StatsScope::Global, bucket_ts, &vals)?;
            }
        }
        tx.execute(
            "INSERT OR REPLACE INTO migration_flags (key, value) VALUES (?1, 'done')",
            params![MIGRATION_FLAG_STATS_JSON],
        )?;
        tx.commit()?;
        drop(tx_conn);

        let backup = stats_json_path.with_extension("json.bak");
        if backup.exists() {
            let _ = std::fs::remove_file(&backup);
        }
        std::fs::rename(stats_json_path, &backup).with_context(|| {
            format!(
                "rename {} to {}",
                stats_json_path.display(),
                backup.display()
            )
        })?;
        tracing::info!(
            from = %stats_json_path.display(),
            to = %backup.display(),
            "migrated stats.json to stats.db"
        );
        Ok(())
    }

    fn import_global_totals(
        &self,
        tx: &rusqlite::Transaction<'_>,
        imported: &StatsData,
    ) -> anyhow::Result<()> {
        self.save_totals_in_tx(tx, StatsScope::Global, imported)
    }

    fn insert_hourly_absolute(
        &self,
        tx: &rusqlite::Transaction<'_>,
        scope: StatsScope,
        bucket_ts: u64,
        vals: &[i64; 47],
    ) -> anyhow::Result<()> {
        tx.execute(
            &format!(
                "INSERT OR REPLACE INTO stats_hourly (scope, bucket_ts, {COUNTER_COLS}) VALUES (?1, ?2,
                   ?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18,?19,?20,?21,?22,?23,?24,?25,?26,?27,?28,?29,?30,?31,?32,?33,?34,?35,?36,?37,?38,?39,?40,?41,?42,?43,?44,?45,?46,?47,?48,?49)"
            ),
            params![
                scope.as_str(),
                bucket_ts as i64,
                vals[0], vals[1], vals[2], vals[3], vals[4], vals[5], vals[6], vals[7], vals[8],
                vals[9], vals[10], vals[11], vals[12], vals[13], vals[14], vals[15], vals[16],
                vals[17], vals[18], vals[19], vals[20], vals[21], vals[22], vals[23], vals[24],
                vals[25], vals[26], vals[27], vals[28], vals[29], vals[30], vals[31], vals[32],
                vals[33], vals[34], vals[35], vals[36], vals[37], vals[38], vals[39], vals[40],
                vals[41], vals[42], vals[43], vals[44], vals[45], vals[46],
            ],
        )?;
        Ok(())
    }

    pub fn upsert_auth_key_meta(
        &self,
        id: &str,
        name: &str,
        key_preview: &str,
    ) -> anyhow::Result<()> {
        let conn = self.conn.lock().expect("stats db mutex");
        conn.execute(
            "INSERT INTO auth_key_meta (id, name, key_preview) VALUES (?1, ?2, ?3)
             ON CONFLICT(id) DO UPDATE SET
               name = excluded.name,
               key_preview = CASE
                 WHEN excluded.key_preview != '' THEN excluded.key_preview
                 ELSE auth_key_meta.key_preview
               END",
            params![id, name, key_preview],
        )?;
        Ok(())
    }

    pub fn update_auth_key_meta_name(&self, id: &str, name: &str) -> anyhow::Result<()> {
        let conn = self.conn.lock().expect("stats db mutex");
        conn.execute(
            "UPDATE auth_key_meta SET name = ?1 WHERE id = ?2",
            params![name, id],
        )?;
        Ok(())
    }

    pub fn touch_auth_key_last_used(&self, id: &str, unix: u64) -> anyhow::Result<()> {
        let conn = self.conn.lock().expect("stats db mutex");
        conn.execute(
            "UPDATE auth_key_meta SET last_used_at_unix = ?1 WHERE id = ?2",
            params![unix as i64, id],
        )?;
        Ok(())
    }

    pub fn auth_key_with_mut<F>(
        &self,
        auth_key_id: &str,
        name: &str,
        key_preview: &str,
        update: F,
    ) -> anyhow::Result<()>
    where
        F: Fn(&mut StatsData),
    {
        let conn = self.conn.lock().expect("stats db mutex");
        let tx = conn.unchecked_transaction()?;
        tx.execute(
            "INSERT INTO auth_key_meta (id, name, key_preview) VALUES (?1, ?2, ?3)
             ON CONFLICT(id) DO UPDATE SET
               name = excluded.name,
               key_preview = CASE
                 WHEN excluded.key_preview != '' THEN excluded.key_preview
                 ELSE auth_key_meta.key_preview
               END",
            params![auth_key_id, name, key_preview],
        )?;
        for scope in [StatsScope::Global, StatsScope::Session] {
            self.ensure_auth_key_totals_in_tx(&tx, scope, auth_key_id)?;
            let before = self.load_auth_key_totals_in_tx(&tx, scope, auth_key_id)?;
            let mut after = before.clone();
            update(&mut after);
            self.save_auth_key_totals_in_tx(&tx, scope, auth_key_id, &after)?;
        }
        tx.commit()?;
        Ok(())
    }

    fn ensure_auth_key_totals_in_tx(
        &self,
        tx: &rusqlite::Transaction<'_>,
        scope: StatsScope,
        auth_key_id: &str,
    ) -> anyhow::Result<()> {
        tx.execute(
            "INSERT OR IGNORE INTO auth_key_totals (scope, auth_key_id, version) VALUES (?1, ?2, 2)",
            params![scope.as_str(), auth_key_id],
        )?;
        Ok(())
    }

    fn load_auth_key_totals_in_tx(
        &self,
        tx: &rusqlite::Transaction<'_>,
        scope: StatsScope,
        auth_key_id: &str,
    ) -> anyhow::Result<StatsData> {
        let sql = format!(
            "SELECT first_record_at_unix, last_updated_at_unix, version, {COUNTER_COLS}
             FROM auth_key_totals WHERE scope = ?1 AND auth_key_id = ?2"
        );
        match tx.query_row(&sql, params![scope.as_str(), auth_key_id], |row| {
            let first: Option<i64> = row.get(0)?;
            let last: Option<i64> = row.get(1)?;
            let version: i32 = row.get(2)?;
            let vals = row_to_counter_values(row, 3)?;
            Ok(counters_to_stats_data(
                version.max(0) as u32,
                first.map(|v| v.max(0) as u64),
                last.map(|v| v.max(0) as u64),
                vals,
                HashMap::new(),
            ))
        }) {
            Ok(data) => Ok(data),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(StatsData::default()),
            Err(e) => Err(e.into()),
        }
    }

    fn save_auth_key_totals_in_tx(
        &self,
        tx: &rusqlite::Transaction<'_>,
        scope: StatsScope,
        auth_key_id: &str,
        data: &StatsData,
    ) -> anyhow::Result<()> {
        let vals = counter_values(data);
        tx.execute(
            &format!(
                "UPDATE auth_key_totals SET
                   first_record_at_unix = ?1, last_updated_at_unix = ?2, version = ?3,
                   requests_total = ?4, requests_stream = ?5, requests_non_stream = ?6, requests_cancelled = ?7,
                   route_edge = ?8, route_cloud = ?9, route_cascade = ?10,
                   upstream_edge_calls = ?11, upstream_cloud_calls = ?12, cascade_edge_ok = ?13, cascade_fallback = ?14,
                   errors_total = ?15, errors_unauthorized = ?16, errors_unavailable = ?17, errors_upstream = ?18, errors_bad_request = ?19,
                   tokens_in_estimate = ?20, tokens_out_estimate = ?21, cloud_input_saved_estimate = ?22,
                   difficulty_sum = ?23, difficulty_count = ?24,
                   edge_tokens_in = ?25, edge_tokens_out = ?26, edge_cached_tokens = ?27,
                   cloud_tokens_in = ?28, cloud_tokens_out = ?29, cloud_cached_tokens = ?30,
                   cloud_tokens_saved_input = ?31, cloud_tokens_saved_output = ?32,
                   cache_hit_requests = ?33, cached_tokens_total = ?34,
                   latency_sum_ms = ?35, latency_count = ?36,
                   stream_latency_sum_ms = ?37, stream_latency_count = ?38,
                   non_stream_latency_sum_ms = ?39, non_stream_latency_count = ?40,
                   ttft_sum_ms = ?41, ttft_count = ?42,
                   tps_sum_x1000 = ?43, tps_count = ?44,
                   edge_tps_sum_x1000 = ?45, edge_tps_count = ?46,
                   cloud_tps_sum_x1000 = ?47, cloud_tps_count = ?48,
                   edge_served_responses = ?49, cloud_served_responses = ?50
                 WHERE scope = ?51 AND auth_key_id = ?52"
            ),
            params![
                data.first_record_at_unix.map(|v| v as i64),
                data.last_updated_at_unix.map(|v| v as i64),
                data.version as i32,
                vals[0], vals[1], vals[2], vals[3], vals[4], vals[5], vals[6], vals[7], vals[8],
                vals[9], vals[10], vals[11], vals[12], vals[13], vals[14], vals[15], vals[16],
                vals[17], vals[18], vals[19], vals[20], vals[21], vals[22], vals[23], vals[24],
                vals[25], vals[26], vals[27], vals[28], vals[29], vals[30], vals[31], vals[32],
                vals[33], vals[34], vals[35], vals[36], vals[37], vals[38], vals[39], vals[40],
                vals[41], vals[42], vals[43], vals[44], vals[45], vals[46],
                scope.as_str(),
                auth_key_id,
            ],
        )?;
        Ok(())
    }

    pub fn list_auth_key_meta(&self) -> anyhow::Result<Vec<(String, String, String, Option<u64>)>> {
        let conn = self.conn.lock().expect("stats db mutex");
        let mut stmt = conn.prepare(
            "SELECT id, name, key_preview, last_used_at_unix FROM auth_key_meta ORDER BY id",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, Option<i64>>(3)?.map(|v| v.max(0) as u64),
            ))
        })?;
        rows.collect::<Result<Vec<_>, _>>().context("list auth key meta")
    }

    pub fn load_auth_key_totals(
        &self,
        scope: StatsScope,
        auth_key_id: &str,
    ) -> anyhow::Result<StatsData> {
        let conn = self.conn.lock().expect("stats db mutex");
        let tx = conn.unchecked_transaction()?;
        let data = self.load_auth_key_totals_in_tx(&tx, scope, auth_key_id)?;
        tx.commit()?;
        Ok(data)
    }

    pub fn flush_if_dirty(&self) -> anyhow::Result<()> {
        Ok(())
    }

    pub fn flush(&self) -> anyhow::Result<()> {
        Ok(())
    }
}

fn step_kind_delta(
    before: &HashMap<String, u64>,
    after: &HashMap<String, u64>,
) -> HashMap<String, u64> {
    let mut delta = HashMap::new();
    for (k, v) in after {
        let prev = before.get(k).copied().unwrap_or(0);
        if *v > prev {
            delta.insert(k.clone(), v - prev);
        }
    }
    delta
}

fn latency_percentiles_from_samples(samples: &[u64]) -> (f64, f64) {
    if samples.is_empty() {
        return (0.0, 0.0);
    }
    let mut sorted = samples.to_vec();
    sorted.sort_unstable();
    (percentile(&sorted, 95.0), percentile(&sorted, 99.0))
}

fn percentile(sorted: &[u64], p: f64) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    let idx = ((sorted.len() as f64 - 1.0) * p / 100.0).round() as usize;
    sorted[idx.min(sorted.len() - 1)] as f64
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum TimelineRange {
    H24,
    D7,
    D30,
}

impl TimelineRange {
    pub fn parse(s: &str) -> Option<Self> {
        match s.to_ascii_lowercase().as_str() {
            "h24" => Some(Self::H24),
            "d7" => Some(Self::D7),
            "d30" => Some(Self::D30),
            _ => None,
        }
    }

    fn bucket_limit(self) -> usize {
        match self {
            Self::H24 => 24,
            Self::D7 => 7,
            Self::D30 => 30,
        }
    }

    fn granularity(self) -> &'static str {
        match self {
            Self::H24 => "hour",
            _ => "day",
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct TimelinePoint {
    pub bucket_ts: u64,
    pub edge_in: u64,
    pub edge_out: u64,
    pub cloud_in: u64,
    pub cloud_out: u64,
    pub requests_total: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct TimelineResponse {
    pub scope: String,
    pub range: String,
    pub granularity: String,
    pub points: Vec<TimelinePoint>,
}

impl StatsDb {
    pub fn query_timeline(
        &self,
        scope: StatsScope,
        range: TimelineRange,
        tz_offset_minutes: i32,
    ) -> anyhow::Result<TimelineResponse> {
        match range {
            TimelineRange::H24 => self.query_timeline_hourly(scope, range),
            TimelineRange::D7 | TimelineRange::D30 => {
                self.query_timeline_daily(scope, range, tz_offset_minutes)
            }
        }
    }

    fn query_timeline_hourly(
        &self,
        scope: StatsScope,
        range: TimelineRange,
    ) -> anyhow::Result<TimelineResponse> {
        let now = data::now_unix();
        let end = hour_bucket_ts(now);
        let start = end.saturating_sub((range.bucket_limit() as u64 - 1) * 3600);
        let conn = self.conn.lock().expect("stats db mutex");
        let mut stmt = conn.prepare(
            "SELECT bucket_ts, edge_tokens_in, edge_tokens_out, cloud_tokens_in, cloud_tokens_out, requests_total
             FROM stats_hourly WHERE scope = ?1 AND bucket_ts >= ?2 AND bucket_ts <= ?3
             ORDER BY bucket_ts ASC",
        )?;
        let rows = stmt.query_map(params![scope.as_str(), start as i64, end as i64], |row| {
            Ok(TimelinePoint {
                bucket_ts: row.get::<_, i64>(0)?.max(0) as u64,
                edge_in: row.get::<_, i64>(1)?.max(0) as u64,
                edge_out: row.get::<_, i64>(2)?.max(0) as u64,
                cloud_in: row.get::<_, i64>(3)?.max(0) as u64,
                cloud_out: row.get::<_, i64>(4)?.max(0) as u64,
                requests_total: row.get::<_, i64>(5)?.max(0) as u64,
            })
        })?;
        let mut map: HashMap<u64, TimelinePoint> = HashMap::new();
        for row in rows {
            let p = row?;
            map.insert(p.bucket_ts, p);
        }
        let mut points = Vec::new();
        for ts in (start..=end).step_by(3600) {
            points.push(map.get(&ts).cloned().unwrap_or(TimelinePoint {
                bucket_ts: ts,
                edge_in: 0,
                edge_out: 0,
                cloud_in: 0,
                cloud_out: 0,
                requests_total: 0,
            }));
        }
        Ok(TimelineResponse {
            scope: scope.as_str().to_string(),
            range: "h24".to_string(),
            granularity: range.granularity().to_string(),
            points,
        })
    }

    fn query_timeline_daily(
        &self,
        scope: StatsScope,
        range: TimelineRange,
        tz_offset_minutes: i32,
    ) -> anyhow::Result<TimelineResponse> {
        let now = data::now_unix();
        let tz_secs = (tz_offset_minutes as i64) * 60;
        let local_now = (now as i64 - tz_secs).max(0) as u64;
        let day_secs = 86400u64;
        let local_day = local_now / day_secs * day_secs;
        let days = range.bucket_limit() as u64;
        let start_local = local_day.saturating_sub((days - 1) * day_secs);
        let start_utc = (start_local as i64 + tz_secs).max(0) as u64;
        let end_utc = now;

        let conn = self.conn.lock().expect("stats db mutex");
        let mut stmt = conn.prepare(
            "SELECT bucket_ts, edge_tokens_in, edge_tokens_out, cloud_tokens_in, cloud_tokens_out, requests_total
             FROM stats_hourly WHERE scope = ?1 AND bucket_ts >= ?2 AND bucket_ts <= ?3",
        )?;
        let rows = stmt.query_map(
            params![scope.as_str(), start_utc as i64, end_utc as i64],
            |row| {
                Ok((
                    row.get::<_, i64>(0)?.max(0) as u64,
                    row.get::<_, i64>(1)?.max(0) as u64,
                    row.get::<_, i64>(2)?.max(0) as u64,
                    row.get::<_, i64>(3)?.max(0) as u64,
                    row.get::<_, i64>(4)?.max(0) as u64,
                    row.get::<_, i64>(5)?.max(0) as u64,
                ))
            },
        )?;

        let mut buckets: HashMap<u64, TimelinePoint> = HashMap::new();
        for row in rows {
            let (ts, ei, eo, ci, co, req) = row?;
            let local_ts = (ts as i64 - tz_secs).max(0) as u64;
            let day_start_local = local_ts / day_secs * day_secs;
            let bucket_ts = (day_start_local as i64 + tz_secs).max(0) as u64;
            buckets
                .entry(bucket_ts)
                .and_modify(|p| {
                    p.edge_in += ei;
                    p.edge_out += eo;
                    p.cloud_in += ci;
                    p.cloud_out += co;
                    p.requests_total += req;
                })
                .or_insert(TimelinePoint {
                    bucket_ts,
                    edge_in: ei,
                    edge_out: eo,
                    cloud_in: ci,
                    cloud_out: co,
                    requests_total: req,
                });
        }

        let mut points = Vec::new();
        for i in 0..days {
            let local_ts = start_local + i * day_secs;
            let bucket_ts = (local_ts as i64 + tz_secs).max(0) as u64;
            points.push(buckets.get(&bucket_ts).cloned().unwrap_or(TimelinePoint {
                bucket_ts,
                edge_in: 0,
                edge_out: 0,
                cloud_in: 0,
                cloud_out: 0,
                requests_total: 0,
            }));
        }

        Ok(TimelineResponse {
            scope: scope.as_str().to_string(),
            range: match range {
                TimelineRange::D7 => "d7".to_string(),
                TimelineRange::D30 => "d30".to_string(),
                TimelineRange::H24 => "h24".to_string(),
            },
            granularity: range.granularity().to_string(),
            points,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gateway::stats::metrics::UpstreamCallMetrics;

    fn temp_db() -> (StatsDb, std::path::PathBuf) {
        let dir = std::env::temp_dir().join(format!("flowy-stats-db-{}", uuid::Uuid::new_v4()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let json = dir.join("stats.json");
        let db = StatsDb::open(&dir, &json).unwrap();
        (db, dir)
    }

    #[test]
    fn totals_roundtrip_and_hourly_delta() {
        let (db, dir) = temp_db();
        let bucket = hour_bucket_ts(data::now_unix());
        db.with_mut(
            |d| d.record_request(true),
            bucket,
            None,
        )
        .unwrap();
        db.with_mut(
            |d| {
                d.record_upstream_metrics(&UpstreamCallMetrics {
                    tier: "edge",
                    prompt_tokens: 100,
                    completion_tokens: 50,
                    cached_tokens: 0,
                    latency_ms: 120,
                    ttft_ms: Some(30),
                    stream: true,
                });
            },
            bucket,
            Some(120),
        )
        .unwrap();

        let totals = db.load_totals(StatsScope::Global).unwrap();
        assert_eq!(totals.requests_total, 1);
        assert_eq!(totals.edge_tokens_in, 100);
        assert_eq!(totals.edge_tokens_out, 50);

        let timeline = db
            .query_timeline(StatsScope::Global, TimelineRange::H24, 0)
            .unwrap();
        assert!(timeline.points.iter().any(|p| p.edge_in > 0));

        let samples = db.load_latency_samples(StatsScope::Global).unwrap();
        assert_eq!(samples.len(), 1);
        assert_eq!(samples[0], 120);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn stats_json_migration_idempotent() {
        let dir = std::env::temp_dir().join(format!("flowy-stats-mig-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let json = dir.join("stats.json");
        let mut imported = StatsData::default();
        imported.touch_updated();
        imported.edge_tokens_in = 500;
        imported.last_updated_at_unix = Some(1_700_000_000);
        data::save(&json, &imported).unwrap();

        let db = StatsDb::open(&dir, &json).unwrap();
        assert!(!json.exists());
        assert!(dir.join("stats.json.bak").exists());
        let totals = db.load_totals(StatsScope::Global).unwrap();
        assert_eq!(totals.edge_tokens_in, 500);

        let db2 = StatsDb::open(&dir, &json).unwrap();
        let totals2 = db2.load_totals(StatsScope::Global).unwrap();
        assert_eq!(totals2.edge_tokens_in, 500);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn session_cleared_on_open() {
        let (db, dir) = temp_db();
        db.with_mut(|d| d.record_request(false), hour_bucket_ts(data::now_unix()), None)
            .unwrap();
        assert_eq!(
            db.load_totals(StatsScope::Session).unwrap().requests_total,
            1
        );
        db.clear_session().unwrap();
        assert_eq!(
            db.load_totals(StatsScope::Session).unwrap().requests_total,
            0
        );
        assert_eq!(
            db.load_totals(StatsScope::Global).unwrap().requests_total,
            1
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn auth_key_meta_upsert_and_totals_delta() {
        let (db, dir) = temp_db();
        let id = "auth-key-test-1";
        db.upsert_auth_key_meta(id, "Test Key", "token-****")
            .unwrap();
        db.auth_key_with_mut(
            id,
            "Test Key",
            "token-****",
            |d| {
                d.record_request(true);
                d.record_upstream_metrics(&UpstreamCallMetrics {
                    tier: "edge",
                    prompt_tokens: 10,
                    completion_tokens: 5,
                    cached_tokens: 0,
                    latency_ms: 80,
                    ttft_ms: None,
                    stream: true,
                });
            },
        )
        .unwrap();
        db.touch_auth_key_last_used(id, 1_700_000_000).unwrap();

        let totals = db.load_auth_key_totals(StatsScope::Global, id).unwrap();
        assert_eq!(totals.requests_total, 1);
        assert_eq!(totals.edge_tokens_in, 10);
        assert_eq!(totals.edge_tokens_out, 5);

        let meta = db.list_auth_key_meta().unwrap();
        assert_eq!(meta.len(), 1);
        assert_eq!(meta[0].0, id);
        assert_eq!(meta[0].1, "Test Key");
        assert_eq!(meta[0].3, Some(1_700_000_000));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn auth_key_session_clear_preserves_meta() {
        let (db, dir) = temp_db();
        let id = "auth-key-test-2";
        db.upsert_auth_key_meta(id, "Session Key", "sk-****")
            .unwrap();
        db.auth_key_with_mut(
            id,
            "Session Key",
            "sk-****",
            |d| d.record_request(false),
        )
        .unwrap();
        assert_eq!(
            db.load_auth_key_totals(StatsScope::Session, id)
                .unwrap()
                .requests_total,
            1
        );
        db.clear_session().unwrap();
        assert_eq!(
            db.load_auth_key_totals(StatsScope::Session, id)
                .unwrap()
                .requests_total,
            0
        );
        assert_eq!(
            db.load_auth_key_totals(StatsScope::Global, id)
                .unwrap()
                .requests_total,
            1
        );
        assert_eq!(db.list_auth_key_meta().unwrap().len(), 1);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn auth_key_snapshot_merge_marks_deleted() {
        use crate::config::auth_keys::GatewayAuthKeyView;
        use crate::gateway::stats::build_auth_key_stats_snapshots;

        let (db, dir) = temp_db();
        let deleted_id = "removed-key-id";
        db.upsert_auth_key_meta(deleted_id, "Old Name", "old-****")
            .unwrap();
        db.auth_key_with_mut(
            deleted_id,
            "Old Name",
            "old-****",
            |d| d.record_request(true),
        )
        .unwrap();

        let active = GatewayAuthKeyView {
            id: "active-key-id".into(),
            name: "Active".into(),
            key_preview: "act-****".into(),
            created_at: 0,
            is_default: false,
        };
        let snaps =
            build_auth_key_stats_snapshots(&db, StatsScope::Global, &[active]).unwrap();
        assert_eq!(snaps.len(), 2);

        let deleted = snaps.iter().find(|s| s.id == deleted_id).unwrap();
        assert!(deleted.deleted);
        assert_eq!(deleted.requests_total, 1);
        assert_eq!(deleted.name, "Old Name");

        let active_snap = snaps.iter().find(|s| s.id == "active-key-id").unwrap();
        assert!(!active_snap.deleted);
        assert_eq!(active_snap.requests_total, 0);

        let _ = std::fs::remove_dir_all(&dir);
    }
}
