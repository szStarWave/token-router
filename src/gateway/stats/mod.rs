pub mod data;
pub mod db;
pub mod metrics;

use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde::Serialize;

pub use data::StatsData;
pub use db::{StatsDb, TimelinePoint, TimelineRange, TimelineResponse};
pub use metrics::{FinalResponseMetrics, UpstreamCallMetrics};
pub use crate::gateway::routing::EffectiveRouting;

use crate::gateway::error::AppError;
use crate::gateway::classifier::ClassifierSnapshot;
use crate::gateway::experience::ExperienceSnapshot;
use crate::gateway::routing::RouteDecision;
use crate::config::auth_keys::GatewayAuthKeyView;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StatsScope {
    Session,
    Global,
}

impl StatsScope {
    pub fn as_str(self) -> &'static str {
        match self {
            StatsScope::Session => "session",
            StatsScope::Global => "global",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s.to_ascii_lowercase().as_str() {
            "session" => Some(StatsScope::Session),
            "global" => Some(StatsScope::Global),
            _ => None,
        }
    }
}

use crate::gateway::agent_usage::AgentBudgetUsage;

#[derive(Debug, Clone)]
pub struct AuthKeyContext {
    pub id: String,
    pub name: String,
    pub key_preview: String,
}

pub struct GatewayStats {
    db: StatsDb,
    stats_json_path: PathBuf,
    pending_latency_ms: Mutex<Option<u64>>,
    session_started: Instant,
}

impl GatewayStats {
    pub fn open(data_dir: &Path) -> anyhow::Result<std::sync::Arc<Self>> {
        let stats_json_path = data_dir.join("stats.json");
        let db = StatsDb::open(data_dir, &stats_json_path)?;
        Ok(std::sync::Arc::new(Self {
            db,
            stats_json_path,
            pending_latency_ms: Mutex::new(None),
            session_started: Instant::now(),
        }))
    }

    #[cfg(test)]
    pub fn new_in_memory() -> std::sync::Arc<Self> {
        let dir = std::env::temp_dir().join(format!(
            "flowy-stats-mem-{}-{}",
            std::process::id(),
            uuid::Uuid::new_v4()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        Self::open(&dir).expect("in-memory stats db")
    }

    pub fn stats_file(&self) -> &Path {
        self.db.path()
    }

    pub fn stats_json_path(&self) -> &Path {
        &self.stats_json_path
    }

    pub fn record_request(&self, stream: bool) {
        self.with_mut(|d| d.record_request(stream));
    }

    pub fn record_decision(&self, decision: &RouteDecision) {
        self.with_mut(|d| d.record_decision(decision));
    }

    pub fn record_upstream_metrics(
        &self,
        metrics: &UpstreamCallMetrics,
        auth_key: Option<&AuthKeyContext>,
    ) {
        if metrics.latency_ms > 0 {
            *self.pending_latency_ms.lock().expect("pending latency") =
                Some(metrics.latency_ms);
        }
        self.with_mut(|d| d.record_upstream_metrics(metrics));
        if let Some(ctx) = auth_key {
            self.auth_key_with_mut(ctx, |d| d.record_upstream_metrics(metrics));
        }
    }

    pub fn record_final_response(
        &self,
        metrics: &FinalResponseMetrics,
        auth_key: Option<&AuthKeyContext>,
    ) {
        self.with_mut(|d| d.record_final_response(metrics));
        if let Some(ctx) = auth_key {
            self.auth_key_with_mut(ctx, |d| d.record_final_response(metrics));
        }
    }

    pub fn record_completion_tokens(&self, tokens_out: u32, auth_key: Option<&AuthKeyContext>) {
        self.with_mut(|d| d.record_completion_tokens(tokens_out));
        if let Some(ctx) = auth_key {
            self.auth_key_with_mut(ctx, |d| d.record_completion_tokens(tokens_out));
        }
    }

    pub fn record_upstream_edge(&self) {
        self.with_mut(|d| d.record_upstream_edge());
    }

    pub fn record_upstream_cloud(&self) {
        self.with_mut(|d| d.record_upstream_cloud());
    }

    pub fn record_cascade_edge_ok(&self) {
        self.with_mut(|d| d.record_cascade_edge_ok());
    }

    pub fn record_cascade_fallback(&self) {
        self.with_mut(|d| d.record_cascade_fallback());
    }

    pub fn record_error(&self, err: &AppError) {
        self.with_mut(|d| d.record_error(err));
    }

    pub fn record_request_for_auth_key(&self, ctx: &AuthKeyContext, stream: bool) {
        self.auth_key_with_mut(ctx, |d| d.record_request(stream));
    }

    pub fn record_decision_for_auth_key(&self, ctx: &AuthKeyContext, decision: &RouteDecision) {
        self.auth_key_with_mut(ctx, |d| d.record_decision(decision));
    }

    pub fn record_error_for_auth_key(&self, ctx: &AuthKeyContext, err: &AppError) {
        self.auth_key_with_mut(ctx, |d| d.record_error(err));
    }

    pub fn touch_auth_key_last_used(&self, ctx: &AuthKeyContext) {
        let now = data::now_unix();
        if let Err(e) = self.db.touch_auth_key_last_used(&ctx.id, now) {
            tracing::warn!(error = %e, "auth key touch last used failed");
        }
    }

    pub fn upsert_auth_key_meta(&self, id: &str, name: &str, key_preview: &str) {
        if let Err(e) = self.db.upsert_auth_key_meta(id, name, key_preview) {
            tracing::warn!(error = %e, "auth key meta upsert failed");
        }
    }

    pub fn update_auth_key_meta_name(&self, id: &str, name: &str) {
        if let Err(e) = self.db.update_auth_key_meta_name(id, name) {
            tracing::warn!(error = %e, "auth key meta rename failed");
        }
    }

    fn auth_key_with_mut(&self, ctx: &AuthKeyContext, update: impl Fn(&mut StatsData)) {
        if let Err(e) = self
            .db
            .auth_key_with_mut(&ctx.id, &ctx.name, &ctx.key_preview, update)
        {
            tracing::warn!(error = %e, "auth key stats db write failed");
        }
    }

    pub fn build_auth_key_stats(
        &self,
        scope: StatsScope,
        config_keys: &[GatewayAuthKeyView],
    ) -> Option<Vec<AuthKeyStatsSnapshot>> {
        build_auth_key_stats_snapshots(&self.db, scope, config_keys)
    }

    fn with_mut(&self, update: impl Fn(&mut StatsData)) {
        let bucket_ts = db::hour_bucket_ts(data::now_unix());
        let latency_ms = self.pending_latency_ms.lock().expect("pending latency").take();
        if let Err(e) = self.db.with_mut(&update, bucket_ts, latency_ms) {
            tracing::warn!(error = %e, "stats db write failed");
        }
    }

    pub fn flush_if_dirty(&self) -> anyhow::Result<()> {
        self.db.flush_if_dirty()
    }

    pub fn flush(&self) -> anyhow::Result<()> {
        self.db.flush()
    }

    /// Latest session edge generation TPS from the most recent edge upstream sample.
    pub fn session_edge_tps(&self) -> Option<f64> {
        self.db
            .load_totals(StatsScope::Session)
            .ok()
            .and_then(|d| d.edge_tps())
    }

    pub fn snapshot(
        &self,
        scope: StatsScope,
        session_uptime_secs: u64,
        experience: Option<ExperienceSnapshot>,
        classifier: Option<ClassifierSnapshot>,
        effective_routing: Option<EffectiveRouting>,
        agent_budgets: Option<Vec<AgentBudgetSnapshot>>,
        config_auth_keys: &[GatewayAuthKeyView],
    ) -> StatsSnapshot {
        let data = self
            .db
            .load_totals(scope)
            .unwrap_or_else(|e| {
                tracing::warn!(error = %e, "stats db load totals failed");
                StatsData::default()
            });
        let mut snap = build_snapshot(
            &data,
            scope,
            self.db.path().display().to_string(),
            session_uptime_secs,
            experience,
            classifier,
            effective_routing,
            agent_budgets,
        );
        let samples = self
            .db
            .load_latency_samples(scope)
            .unwrap_or_else(|e| {
                tracing::warn!(error = %e, "stats db load latency failed");
                Vec::new()
            });
        let (p95_ms, p99_ms) = latency_percentiles_from_slice(&samples);
        snap.latency.p95_ms = p95_ms;
        snap.latency.p99_ms = p99_ms;
        snap.auth_key_stats = self.build_auth_key_stats(scope, config_auth_keys);
        snap
    }

    pub fn query_timeline(
        &self,
        scope: StatsScope,
        range: TimelineRange,
        tz_offset_minutes: i32,
    ) -> anyhow::Result<TimelineResponse> {
        self.db.query_timeline(scope, range, tz_offset_minutes)
    }

    pub fn session_uptime_secs(&self) -> u64 {
        self.session_started.elapsed().as_secs()
    }

    pub fn global_data(&self) -> StatsData {
        self.db
            .load_totals(StatsScope::Global)
            .unwrap_or_else(|e| {
                tracing::warn!(error = %e, "stats db load global totals failed");
                StatsData::default()
            })
    }

    pub fn spawn_flush_task(self: &std::sync::Arc<Self>) {
        let stats = self.clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(5));
            interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            loop {
                interval.tick().await;
                if let Err(e) = stats.flush_if_dirty() {
                    tracing::warn!(error = %e, "stats flush failed");
                }
            }
        });
    }
}

pub fn build_snapshot(
    data: &StatsData,
    scope: StatsScope,
    stats_file: String,
    session_uptime_secs: u64,
    experience: Option<ExperienceSnapshot>,
    classifier: Option<ClassifierSnapshot>,
    effective_routing: Option<EffectiveRouting>,
    agent_budgets: Option<Vec<AgentBudgetSnapshot>>,
) -> StatsSnapshot {
    let requests = data.requests_total;
    let difficulty_count = data.difficulty_count;
    let avg_difficulty = if difficulty_count > 0 {
        (data.difficulty_sum as f64) / 1000.0 / difficulty_count as f64
    } else {
        0.0
    };

    let route_edge = data.route_edge;
    let route_cloud = data.route_cloud;
    let route_cascade = data.route_cascade;
    let routed = route_edge + route_cloud + route_cascade;

    let total_in = data.edge_tokens_in + data.cloud_tokens_in;
    let total_out = data.edge_tokens_out + data.cloud_tokens_out;
    let total_cached = data.edge_cached_tokens + data.cloud_cached_tokens;
    let mut edge_tier = tier_token_stats(
        data.edge_tokens_in,
        data.edge_tokens_out,
        data.edge_cached_tokens,
        data.edge_max_input,
        data.edge_max_output,
        data.edge_at_max_input_output,
        data.edge_at_max_input_total,
        data.edge_at_max_output_input,
        data.edge_at_max_output_total,
    );
    let mut cloud_tier = tier_token_stats(
        data.cloud_tokens_in,
        data.cloud_tokens_out,
        data.cloud_cached_tokens,
        data.cloud_max_input,
        data.cloud_max_output,
        data.cloud_at_max_input_output,
        data.cloud_at_max_input_total,
        data.cloud_at_max_output_input,
        data.cloud_at_max_output_total,
    );
    let mut total_tier = tier_token_stats(
        total_in,
        total_out,
        total_cached,
        data.edge_max_input.max(data.cloud_max_input),
        data.edge_max_output.max(data.cloud_max_output),
        0,
        0,
        0,
        0,
    );
    fill_tier_shares(&mut edge_tier, &mut cloud_tier, &mut total_tier);
    let total_tokens = total_in + total_out;
    let edge_token_share = if total_tokens > 0 {
        (data.edge_tokens_in + data.edge_tokens_out) as f64 * 100.0 / total_tokens as f64
    } else {
        0.0
    };
    let cloud_token_share = if total_tokens > 0 {
        100.0 - edge_token_share
    } else {
        0.0
    };

    let cloud_saved_total =
        data.cloud_tokens_saved_input + data.cloud_tokens_saved_output;
    let would_be_cloud = data.cloud_tokens_in
        + data.cloud_tokens_out
        + data.cloud_tokens_saved_input
        + data.cloud_tokens_saved_output;
    let saved_pct = if would_be_cloud > 0 {
        cloud_saved_total as f64 * 100.0 / would_be_cloud as f64
    } else {
        0.0
    };

    let requests_per_minute = match scope {
        StatsScope::Session => {
            if session_uptime_secs > 0 {
                requests as f64 * 60.0 / session_uptime_secs as f64
            } else {
                0.0
            }
        }
        StatsScope::Global => global_requests_per_minute(data, requests),
    };

    StatsSnapshot {
        scope: scope.as_str().to_string(),
        stats_file,
        persisted: scope == StatsScope::Global,
        first_record_at_unix: data.first_record_at_unix,
        last_updated_at_unix: data.last_updated_at_unix,
        session_uptime_secs: match scope {
            StatsScope::Session => session_uptime_secs,
            StatsScope::Global => 0,
        },
        requests_total: requests,
        requests_stream: data.requests_stream,
        requests_non_stream: data.requests_non_stream,
        requests_cancelled: data.requests_cancelled,
        requests_per_minute,
        routing: RouteCounts {
            edge: route_edge,
            cloud: route_cloud,
            cascade: route_cascade,
            edge_pct: pct(route_edge, routed),
            cloud_pct: pct(route_cloud, routed),
            cascade_pct: pct(route_cascade, routed),
        },
        upstream: UpstreamCounts {
            edge_calls: data.upstream_edge_calls,
            cloud_calls: data.upstream_cloud_calls,
        },
        cascade: CascadeCounts {
            edge_ok: data.cascade_edge_ok,
            fallback_to_cloud: data.cascade_fallback,
        },
        tokens: TokenCounts {
            in_estimate: data.tokens_in_estimate,
            out_estimate: data.tokens_out_estimate,
            cloud_input_saved_estimate: data.cloud_input_saved_estimate,
        },
        token_breakdown: TokenBreakdown {
            edge: edge_tier,
            cloud: cloud_tier,
            total: total_tier,
            edge_share_pct: edge_token_share,
            cloud_share_pct: cloud_token_share,
            cloud_saved: CloudTokensSaved {
                input: data.cloud_tokens_saved_input,
                output: data.cloud_tokens_saved_output,
                total: cloud_saved_total,
                pct_of_would_be_cloud: saved_pct,
            },
        },
        cache: CacheStats {
            hit_requests: data.cache_hit_requests,
            cached_tokens: data.cached_tokens_total,
            hit_rate_pct: pct(data.cache_hit_requests, requests),
        },
        latency: LatencyStats {
            avg_request_ms: avg(data.latency_sum_ms, data.latency_count),
            avg_ttft_ms: avg(data.ttft_sum_ms, data.ttft_count),
            avg_tps: latest_tps(data.tps_sum_x1000, data.tps_count),
            edge_tps: latest_tps(data.edge_tps_sum_x1000, data.edge_tps_count),
            cloud_tps: latest_tps(data.cloud_tps_sum_x1000, data.cloud_tps_count),
            p95_ms: 0.0,
            p99_ms: 0.0,
            upstream_samples: data.latency_count,
            ttft_samples: data.ttft_count,
            tps_samples: data.tps_count,
            stream_avg_ms: avg(data.stream_latency_sum_ms, data.stream_latency_count),
            non_stream_avg_ms: avg(data.non_stream_latency_sum_ms, data.non_stream_latency_count),
        },
        served: ServedCounts {
            edge: data.edge_served_responses,
            cloud: data.cloud_served_responses,
            edge_pct: pct(data.edge_served_responses, data.edge_served_responses + data.cloud_served_responses),
            cloud_pct: pct(data.cloud_served_responses, data.edge_served_responses + data.cloud_served_responses),
        },
        difficulty: DifficultyStats {
            avg: avg_difficulty,
            samples: difficulty_count,
        },
        errors: ErrorCounts {
            total: data.errors_total,
            unauthorized: data.errors_unauthorized,
            unavailable: data.errors_unavailable,
            upstream: data.errors_upstream,
            bad_request: data.errors_bad_request,
        },
        step_kinds: data.step_kinds.clone(),
        experience,
        classifier,
        effective_routing,
        agent_budgets,
        auth_key_stats: None,
    }
}

pub fn build_auth_key_stats_snapshots(
    db: &StatsDb,
    scope: StatsScope,
    config_keys: &[GatewayAuthKeyView],
) -> Option<Vec<AuthKeyStatsSnapshot>> {
    use std::collections::{HashMap, HashSet};

    let active_ids: HashSet<String> = config_keys.iter().map(|k| k.id.clone()).collect();
    let config_by_id: HashMap<String, &GatewayAuthKeyView> =
        config_keys.iter().map(|k| (k.id.clone(), k)).collect();

    let mut by_id: HashMap<String, AuthKeyStatsSnapshot> = HashMap::new();

    if let Ok(meta_rows) = db.list_auth_key_meta() {
        for (id, name, key_preview, last_used) in meta_rows {
            let data = db.load_auth_key_totals(scope, &id).unwrap_or_default();
            let deleted = !active_ids.contains(&id);
            by_id.insert(
                id.clone(),
                auth_key_snapshot_from_data(
                    id,
                    name,
                    key_preview,
                    last_used,
                    deleted,
                    &data,
                ),
            );
        }
    }

    for key in config_keys {
        by_id
            .entry(key.id.clone())
            .and_modify(|snap| {
                snap.name = key.name.clone();
                snap.key_preview = key.key_preview.clone();
                snap.deleted = false;
            })
            .or_insert_with(|| {
                auth_key_snapshot_from_data(
                    key.id.clone(),
                    key.name.clone(),
                    key.key_preview.clone(),
                    None,
                    false,
                    &StatsData::default(),
                )
            });
    }

    if by_id.is_empty() {
        return None;
    }

    let mut rows: Vec<AuthKeyStatsSnapshot> = by_id.into_values().collect();
    for snap in &mut rows {
        if let Some(cfg) = config_by_id.get(&snap.id) {
            if active_ids.contains(&snap.id) {
                snap.name = cfg.name.clone();
                snap.key_preview = cfg.key_preview.clone();
                snap.deleted = false;
            }
        }
    }
    rows.sort_by(|a, b| a.id.cmp(&b.id));
    Some(rows)
}

fn auth_key_snapshot_from_data(
    id: String,
    name: String,
    key_preview: String,
    last_used_at_unix: Option<u64>,
    deleted: bool,
    data: &StatsData,
) -> AuthKeyStatsSnapshot {
    let route_edge = data.route_edge;
    let route_cloud = data.route_cloud;
    let route_cascade = data.route_cascade;
    let routed = route_edge + route_cloud + route_cascade;
    let input = data.edge_tokens_in + data.cloud_tokens_in;
    let output = data.edge_tokens_out + data.cloud_tokens_out;
    AuthKeyStatsSnapshot {
        id,
        name,
        key_preview,
        deleted,
        last_used_at_unix,
        requests_total: data.requests_total,
        tokens: AuthKeyTokenStats {
            input,
            output,
            total: input + output,
        },
        latency: AuthKeyLatencyStats {
            avg_request_ms: avg(data.latency_sum_ms, data.latency_count),
            avg_tps: latest_tps(data.tps_sum_x1000, data.tps_count),
            edge_tps: latest_tps(data.edge_tps_sum_x1000, data.edge_tps_count),
            cloud_tps: latest_tps(data.cloud_tps_sum_x1000, data.cloud_tps_count),
        },
        routing: RouteCounts {
            edge: route_edge,
            cloud: route_cloud,
            cascade: route_cascade,
            edge_pct: pct(route_edge, routed),
            cloud_pct: pct(route_cloud, routed),
            cascade_pct: pct(route_cascade, routed),
        },
    }
}

fn global_requests_per_minute(data: &StatsData, requests: u64) -> f64 {
    let Some(first) = data.first_record_at_unix else {
        return 0.0;
    };
    let end = data
        .last_updated_at_unix
        .unwrap_or_else(data::now_unix);
    let span_secs = end.saturating_sub(first).max(1);
    requests as f64 * 60.0 / span_secs as f64
}

fn pct(part: u64, total: u64) -> f64 {
    if total == 0 {
        0.0
    } else {
        part as f64 * 100.0 / total as f64
    }
}

fn avg(sum: u64, count: u64) -> f64 {
    if count == 0 {
        0.0
    } else {
        sum as f64 / count as f64
    }
}

fn latest_tps(tps_x1000: u64, count: u64) -> f64 {
    if count == 0 {
        0.0
    } else {
        (tps_x1000 as f64) / 1000.0
    }
}

fn latency_percentiles_from_slice(samples: &[u64]) -> (f64, f64) {
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

fn tier_token_stats(
    input: u64,
    output: u64,
    cached: u64,
    max_input: u64,
    max_output: u64,
    max_input_request_output: u64,
    max_input_request_total: u64,
    max_output_request_input: u64,
    max_output_request_total: u64,
) -> TierTokenStats {
    TierTokenStats {
        input,
        output,
        cached,
        max_input,
        max_output,
        max_input_request_output,
        max_input_request_total,
        max_output_request_input,
        max_output_request_total,
        input_pct: 0.0,
        output_pct: 0.0,
    }
}

impl TierTokenStats {
    fn with_shares(mut self, total_in: u64, total_out: u64) -> Self {
        self.input_pct = pct(self.input, total_in);
        self.output_pct = pct(self.output, total_out);
        self
    }
}

fn fill_tier_shares(edge: &mut TierTokenStats, cloud: &mut TierTokenStats, total: &mut TierTokenStats) {
    let total_in = edge.input + cloud.input;
    let total_out = edge.output + cloud.output;
    *edge = edge.clone().with_shares(total_in, total_out);
    *cloud = cloud.clone().with_shares(total_in, total_out);
    *total = total.clone().with_shares(total_in, total_out);
}

#[derive(Debug, Clone, Serialize)]
pub struct StatsSnapshot {
    pub scope: String,
    pub stats_file: String,
    pub persisted: bool,
    pub first_record_at_unix: Option<u64>,
    pub last_updated_at_unix: Option<u64>,
    pub session_uptime_secs: u64,
    pub requests_total: u64,
    pub requests_stream: u64,
    pub requests_non_stream: u64,
    pub requests_cancelled: u64,
    pub requests_per_minute: f64,
    pub routing: RouteCounts,
    pub upstream: UpstreamCounts,
    pub cascade: CascadeCounts,
    pub tokens: TokenCounts,
    pub token_breakdown: TokenBreakdown,
    pub cache: CacheStats,
    pub latency: LatencyStats,
    pub served: ServedCounts,
    pub difficulty: DifficultyStats,
    pub errors: ErrorCounts,
    pub step_kinds: std::collections::HashMap<String, u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub experience: Option<ExperienceSnapshot>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub classifier: Option<ClassifierSnapshot>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub effective_routing: Option<EffectiveRouting>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent_budgets: Option<Vec<AgentBudgetSnapshot>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auth_key_stats: Option<Vec<AuthKeyStatsSnapshot>>,
}

#[derive(Debug, Clone, Serialize)]
pub struct AuthKeyStatsSnapshot {
    pub id: String,
    pub name: String,
    pub key_preview: String,
    pub deleted: bool,
    pub last_used_at_unix: Option<u64>,
    pub requests_total: u64,
    pub tokens: AuthKeyTokenStats,
    pub latency: AuthKeyLatencyStats,
    pub routing: RouteCounts,
}

#[derive(Debug, Clone, Serialize)]
pub struct AuthKeyTokenStats {
    pub input: u64,
    pub output: u64,
    pub total: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct AuthKeyLatencyStats {
    pub avg_request_ms: f64,
    pub avg_tps: f64,
    pub edge_tps: f64,
    pub cloud_tps: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct AgentBudgetSnapshot {
    pub agent_id: String,
    pub budget_limit: Option<u64>,
    pub tokens_used: u64,
}

impl AgentBudgetSnapshot {
    pub fn from_config_and_usage(
        configs: &std::collections::HashMap<String, crate::gateway::config::AgentUpstreamConfig>,
        usage: &[AgentBudgetUsage],
    ) -> Vec<AgentBudgetSnapshot> {
        let mut budgets: Vec<AgentBudgetSnapshot> = Vec::new();
        let mut seen = std::collections::HashSet::new();
        for cfg in configs.keys() {
            seen.insert(cfg.clone());
            let limit = configs.get(cfg).and_then(|a| a.cloud_token_budget);
            let used = usage.iter().find(|u| u.agent_id == *cfg).map(|u| u.tokens_used).unwrap_or(0);
            budgets.push(AgentBudgetSnapshot {
                agent_id: cfg.clone(),
                budget_limit: limit,
                tokens_used: used,
            });
        }
        for u in usage {
            if !seen.contains(&u.agent_id) {
                budgets.push(AgentBudgetSnapshot {
                    agent_id: u.agent_id.clone(),
                    budget_limit: None,
                    tokens_used: u.tokens_used,
                });
            }
        }
        budgets
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct RouteCounts {
    pub edge: u64,
    pub cloud: u64,
    pub cascade: u64,
    pub edge_pct: f64,
    pub cloud_pct: f64,
    pub cascade_pct: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct UpstreamCounts {
    pub edge_calls: u64,
    pub cloud_calls: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct CascadeCounts {
    pub edge_ok: u64,
    pub fallback_to_cloud: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct TokenCounts {
    pub in_estimate: u64,
    pub out_estimate: u64,
    pub cloud_input_saved_estimate: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct TokenBreakdown {
    pub edge: TierTokenStats,
    pub cloud: TierTokenStats,
    pub total: TierTokenStats,
    pub edge_share_pct: f64,
    pub cloud_share_pct: f64,
    pub cloud_saved: CloudTokensSaved,
}

#[derive(Debug, Clone, Serialize)]
pub struct TierTokenStats {
    pub input: u64,
    pub output: u64,
    pub cached: u64,
    pub max_input: u64,
    pub max_output: u64,
    pub max_input_request_output: u64,
    pub max_input_request_total: u64,
    pub max_output_request_input: u64,
    pub max_output_request_total: u64,
    pub input_pct: f64,
    pub output_pct: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct CloudTokensSaved {
    pub input: u64,
    pub output: u64,
    pub total: u64,
    pub pct_of_would_be_cloud: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct CacheStats {
    pub hit_requests: u64,
    pub cached_tokens: u64,
    pub hit_rate_pct: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct LatencyStats {
    pub avg_request_ms: f64,
    pub avg_ttft_ms: f64,
    pub avg_tps: f64,
    pub edge_tps: f64,
    pub cloud_tps: f64,
    pub p95_ms: f64,
    pub p99_ms: f64,
    pub upstream_samples: u64,
    pub ttft_samples: u64,
    pub tps_samples: u64,
    pub stream_avg_ms: f64,
    pub non_stream_avg_ms: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct ServedCounts {
    pub edge: u64,
    pub cloud: u64,
    pub edge_pct: f64,
    pub cloud_pct: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct DifficultyStats {
    pub avg: f64,
    pub samples: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct ErrorCounts {
    pub total: u64,
    pub unauthorized: u64,
    pub unavailable: u64,
    pub upstream: u64,
    pub bad_request: u64,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gateway::routing::Profile;
    use crate::gateway::multimodal::MultimodalStrategy;
    use crate::gateway::routing::{RouteDecision, RouteTier, RoutingMode, StepKind, WorkStrategy};

    fn sample_decision(route: RouteTier) -> RouteDecision {
        RouteDecision {
            route,
            profile: Profile::Balanced,
            mode: RoutingMode::Cascade,
            step_kind: StepKind::DirectChat,
            difficulty: 0.2,
            reason_codes: vec![],
            tokens_in_estimate: 100,
            tokens_out_estimate: 50,
            cloud_input_saved_estimate: 100,
            conversation_key: String::new(),
            assistant_failed_recent: false,
            multimodal_strategy: MultimodalStrategy::None,
            work_strategy: WorkStrategy::None,
            force_cloud_sticky: false,
            edge_ok_probability: None,
            classifier_features: None,
            casual_quality_fallback: false,
            lexical_learn: Default::default(),
            routing_log_id: None,
        }
    }

    #[test]
    fn upstream_metrics_aggregates_token_breakdown() {
        let stats = GatewayStats::new_in_memory();
        stats.record_upstream_metrics(
            &UpstreamCallMetrics {
                tier: "edge",
                prompt_tokens: 100,
                completion_tokens: 50,
                cached_tokens: 80,
                latency_ms: 200,
                ttft_ms: Some(50),
                stream: true,
            },
            None,
        );
        stats.record_upstream_metrics(
            &UpstreamCallMetrics {
                tier: "cloud",
                prompt_tokens: 200,
                completion_tokens: 100,
                cached_tokens: 0,
                latency_ms: 500,
                ttft_ms: None,
                stream: false,
            },
            None,
        );
        stats.record_final_response(
            &FinalResponseMetrics {
                served_tier: "edge",
                cloud_input_saved: 100,
                completion_tokens: 50,
            },
            None,
        );
        let snap = stats.snapshot(StatsScope::Session, 60, None, None, None, None, &[]);
        assert_eq!(snap.token_breakdown.edge.input, 100);
        assert_eq!(snap.token_breakdown.cloud.input, 200);
        assert_eq!(snap.token_breakdown.cloud_saved.total, 150);
        assert_eq!(snap.cache.hit_requests, 1);
        assert_eq!(snap.cache.cached_tokens, 80);
        assert!(snap.latency.avg_ttft_ms > 0.0);
        assert!(snap.latency.edge_tps > 0.0);
        assert!(snap.latency.cloud_tps > 0.0);
        assert_eq!(snap.token_breakdown.edge.max_input, 100);
        assert_eq!(snap.token_breakdown.edge.max_output, 50);
        assert_eq!(snap.token_breakdown.edge.max_input_request_output, 50);
        assert_eq!(snap.token_breakdown.edge.max_input_request_total, 150);
        assert_eq!(snap.token_breakdown.edge.max_output_request_input, 100);
        assert_eq!(snap.token_breakdown.edge.max_output_request_total, 150);
        assert_eq!(snap.token_breakdown.cloud.max_input, 200);
        assert_eq!(snap.token_breakdown.cloud.max_output, 100);
        assert_eq!(snap.token_breakdown.cloud.max_input_request_output, 100);
        assert_eq!(snap.token_breakdown.cloud.max_input_request_total, 300);
        assert_eq!(snap.token_breakdown.cloud.max_output_request_input, 200);
        assert_eq!(snap.token_breakdown.cloud.max_output_request_total, 300);
        assert!(snap.latency.p95_ms >= 200.0);
        assert!(snap.latency.p99_ms >= snap.latency.p95_ms);
        assert_eq!(snap.served.edge, 1);
    }

    fn expected_tps(completion_tokens: u32, latency_ms: u64, ttft_ms: Option<u64>) -> f64 {
        let gen_ms = latency_ms.saturating_sub(ttft_ms.unwrap_or(0)).max(1);
        (completion_tokens as f64 * 1000.0) / gen_ms as f64
    }

    #[test]
    fn upstream_metrics_tps_uses_latest_request() {
        let stats = GatewayStats::new_in_memory();
        stats.record_upstream_metrics(
            &UpstreamCallMetrics {
                tier: "edge",
                prompt_tokens: 10,
                completion_tokens: 50,
                cached_tokens: 0,
                latency_ms: 200,
                ttft_ms: Some(50),
                stream: true,
            },
            None,
        );
        let first_edge_tps = expected_tps(50, 200, Some(50));
        stats.record_upstream_metrics(
            &UpstreamCallMetrics {
                tier: "edge",
                prompt_tokens: 10,
                completion_tokens: 100,
                cached_tokens: 0,
                latency_ms: 200,
                ttft_ms: Some(100),
                stream: true,
            },
            None,
        );
        let second_edge_tps = expected_tps(100, 200, Some(100));
        let snap = stats.snapshot(StatsScope::Session, 60, None, None, None, None, &[]);
        assert!((snap.latency.edge_tps - second_edge_tps).abs() < 0.01);
        assert!((snap.latency.avg_tps - second_edge_tps).abs() < 0.01);
        assert_ne!(first_edge_tps, second_edge_tps);

        stats.record_upstream_metrics(
            &UpstreamCallMetrics {
                tier: "cloud",
                prompt_tokens: 10,
                completion_tokens: 80,
                cached_tokens: 0,
                latency_ms: 400,
                ttft_ms: None,
                stream: false,
            },
            None,
        );
        let cloud_tps = expected_tps(80, 400, None);
        let snap = stats.snapshot(StatsScope::Session, 60, None, None, None, None, &[]);
        assert!((snap.latency.edge_tps - second_edge_tps).abs() < 0.01);
        assert!((snap.latency.cloud_tps - cloud_tps).abs() < 0.01);
        assert!((snap.latency.avg_tps - cloud_tps).abs() < 0.01);
    }

    #[test]
    fn global_max_tokens_survive_restart() {
        let dir = std::env::temp_dir().join(format!(
            "flowy-stats-max-{}-{}",
            std::process::id(),
            uuid::Uuid::new_v4()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        {
            let stats = GatewayStats::open(&dir).unwrap();
            stats.record_upstream_metrics(
                &UpstreamCallMetrics {
                    tier: "edge",
                    prompt_tokens: 100,
                    completion_tokens: 50,
                    cached_tokens: 0,
                    latency_ms: 200,
                    ttft_ms: Some(50),
                    stream: true,
                },
                None,
            );
            stats.record_upstream_metrics(
                &UpstreamCallMetrics {
                    tier: "cloud",
                    prompt_tokens: 200,
                    completion_tokens: 100,
                    cached_tokens: 0,
                    latency_ms: 500,
                    ttft_ms: None,
                    stream: false,
                },
                None,
            );
            stats.flush().unwrap();
        }
        {
            let stats = GatewayStats::open(&dir).unwrap();
            let global = stats.snapshot(StatsScope::Global, 10, None, None, None, None, &[]);
            assert_eq!(global.token_breakdown.edge.max_input, 100);
            assert_eq!(global.token_breakdown.edge.max_output, 50);
            assert_eq!(global.token_breakdown.cloud.max_input, 200);
            assert_eq!(global.token_breakdown.cloud.max_output, 100);

            let session = stats.snapshot(StatsScope::Session, 10, None, None, None, None, &[]);
            assert_eq!(session.token_breakdown.edge.max_input, 0);
            assert_eq!(session.token_breakdown.cloud.max_input, 0);
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn snapshot_aggregates_decisions() {
        let stats = GatewayStats::new_in_memory();
        stats.record_request(false);
        stats.record_decision(&sample_decision(RouteTier::Edge));
        stats.record_decision(&sample_decision(RouteTier::Cloud));
        let snap = stats.snapshot(StatsScope::Session, 60, None, None, None, None, &[]);
        assert_eq!(snap.scope, "session");
        assert_eq!(snap.requests_total, 1);
        assert_eq!(snap.routing.edge, 1);
        assert_eq!(snap.routing.cloud, 1);
        assert_eq!(snap.tokens.in_estimate, 200);
        assert!(snap.step_kinds.contains_key("directchat"));

        let global = stats.snapshot(StatsScope::Global, 60, None, None, None, None, &[]);
        assert_eq!(global.scope, "global");
        assert_eq!(global.requests_total, 1);
    }

    #[test]
    fn session_resets_on_reopen_global_persists() {
        let dir = std::env::temp_dir().join(format!(
            "flowy-stats-session-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        {
            let stats = GatewayStats::open(&dir).unwrap();
            stats.record_request(true);
            stats.record_upstream_metrics(
                &UpstreamCallMetrics {
                    tier: "edge",
                    prompt_tokens: 42,
                    completion_tokens: 7,
                    cached_tokens: 0,
                    latency_ms: 100,
                    ttft_ms: None,
                    stream: false,
                },
                None,
            );
            stats.flush().unwrap();
            let session = stats.snapshot(StatsScope::Session, 10, None, None, None, None, &[]);
            assert_eq!(session.requests_total, 1);
            assert_eq!(session.token_breakdown.edge.max_input, 42);
        }
        {
            let stats = GatewayStats::open(&dir).unwrap();
            let session = stats.snapshot(StatsScope::Session, 10, None, None, None, None, &[]);
            let global = stats.snapshot(StatsScope::Global, 10, None, None, None, None, &[]);
            assert_eq!(session.requests_total, 0, "new process session starts at 0");
            assert_eq!(global.requests_total, 1, "global survives restart");
            assert_eq!(global.token_breakdown.edge.max_input, 42);
            assert_eq!(global.token_breakdown.edge.max_output, 7);
            assert_eq!(session.token_breakdown.edge.max_input, 0);
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn auth_key_stats_per_key_in_snapshot() {
        use crate::config::auth_keys::GatewayAuthKeyView;

        let stats = GatewayStats::new_in_memory();
        let ctx1 = AuthKeyContext {
            id: "id-key-a".into(),
            name: "Key A".into(),
            key_preview: "ka-****".into(),
        };
        let ctx2 = AuthKeyContext {
            id: "id-key-b".into(),
            name: "Key B".into(),
            key_preview: "kb-****".into(),
        };
        stats.record_request_for_auth_key(&ctx1, true);
        stats.record_request_for_auth_key(&ctx2, false);
        stats.record_request_for_auth_key(&ctx2, false);

        let config = vec![
            GatewayAuthKeyView {
                id: "id-key-a".into(),
                name: "Key A".into(),
                key_preview: "ka-****".into(),
                created_at: 0,
                is_default: false,
            },
            GatewayAuthKeyView {
                id: "id-key-b".into(),
                name: "Key B".into(),
                key_preview: "kb-****".into(),
                created_at: 0,
                is_default: false,
            },
        ];
        let snap = stats.snapshot(
            StatsScope::Session,
            60,
            None,
            None,
            None,
            None,
            &config,
        );
        let keys = snap.auth_key_stats.expect("auth key stats");
        assert_eq!(
            keys.iter().find(|k| k.id == "id-key-a").unwrap().requests_total,
            1
        );
        assert_eq!(
            keys.iter().find(|k| k.id == "id-key-b").unwrap().requests_total,
            2
        );
    }

    #[test]
    fn persist_roundtrip() {
        let dir = std::env::temp_dir().join(format!(
            "flowy-stats-test-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        {
            let stats = GatewayStats::open(&dir).unwrap();
            stats.record_request(true);
            stats.flush().unwrap();
        }
        let db_path = dir.join("stats.db");
        assert!(db_path.exists());
        let stats = GatewayStats::open(&dir).unwrap();
        let global = stats.snapshot(StatsScope::Global, 0, None, None, None, None, &[]);
        assert_eq!(global.requests_total, 1);
        assert_eq!(global.requests_stream, 1);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
