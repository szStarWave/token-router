pub mod data;
pub mod metrics;

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

const MAX_LATENCY_SAMPLES: usize = 2048;

use serde::Serialize;

pub use data::StatsData;
pub use metrics::{FinalResponseMetrics, UpstreamCallMetrics};
pub use crate::gateway::routing::EffectiveRouting;

use crate::gateway::error::AppError;
use crate::gateway::classifier::ClassifierSnapshot;
use crate::gateway::experience::ExperienceSnapshot;
use crate::gateway::routing::RouteDecision;
use crate::gateway::agent_usage::AgentBudgetUsage;

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

pub struct GatewayStats {
    global: Mutex<StatsData>,
    session: Mutex<StatsData>,
    latency_samples: Mutex<VecDeque<u64>>,
    path: PathBuf,
    dirty: AtomicBool,
    session_started: Instant,
}

impl GatewayStats {
    pub fn open(data_dir: &Path) -> anyhow::Result<std::sync::Arc<Self>> {
        let path = data_dir.join("stats.json");
        let data = data::load(&path)?;
        Ok(std::sync::Arc::new(Self {
            global: Mutex::new(data),
            session: Mutex::new(StatsData::default()),
            latency_samples: Mutex::new(VecDeque::new()),
            path,
            dirty: AtomicBool::new(false),
            session_started: Instant::now(),
        }))
    }

    #[cfg(test)]
    pub fn new_in_memory() -> std::sync::Arc<Self> {
        std::sync::Arc::new(Self {
            global: Mutex::new(StatsData::default()),
            session: Mutex::new(StatsData::default()),
            latency_samples: Mutex::new(VecDeque::new()),
            path: PathBuf::from("/tmp/flowy-test-stats.json"),
            dirty: AtomicBool::new(false),
            session_started: Instant::now(),
        })
    }

    pub fn stats_file(&self) -> &Path {
        &self.path
    }

    pub fn record_request(&self, stream: bool) {
        self.with_mut(|d| d.record_request(stream));
    }

    pub fn record_decision(&self, decision: &RouteDecision) {
        self.with_mut(|d| d.record_decision(decision));
    }

    pub fn record_completion_tokens(&self, tokens_out: u32) {
        self.with_mut(|d| d.record_completion_tokens(tokens_out));
    }

    pub fn record_upstream_metrics(&self, metrics: &UpstreamCallMetrics) {
        self.with_mut(|d| d.record_upstream_metrics(metrics));
        if metrics.latency_ms > 0 {
            let mut samples = self
                .latency_samples
                .lock()
                .expect("stats latency_samples mutex");
            if samples.len() >= MAX_LATENCY_SAMPLES {
                samples.pop_front();
            }
            samples.push_back(metrics.latency_ms);
        }
    }

    pub fn record_final_response(&self, metrics: &FinalResponseMetrics) {
        self.with_mut(|d| d.record_final_response(metrics));
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

    fn with_mut(&self, update: impl Fn(&mut StatsData)) {
        update(&mut self.global.lock().expect("stats global mutex"));
        update(&mut self.session.lock().expect("stats session mutex"));
        self.dirty.store(true, Ordering::Release);
    }

    pub fn flush_if_dirty(&self) -> anyhow::Result<()> {
        if !self.dirty.swap(false, Ordering::AcqRel) {
            return Ok(());
        }
        let data = self.global.lock().expect("stats global mutex").clone();
        data::save(&self.path, &data)
    }

    pub fn flush(&self) -> anyhow::Result<()> {
        self.dirty.store(true, Ordering::Release);
        self.flush_if_dirty()
    }

    pub fn snapshot(
        &self,
        scope: StatsScope,
        session_uptime_secs: u64,
        experience: Option<ExperienceSnapshot>,
        classifier: Option<ClassifierSnapshot>,
        effective_routing: Option<EffectiveRouting>,
        agent_budgets: Option<Vec<AgentBudgetSnapshot>>,
    ) -> StatsSnapshot {
        let data = match scope {
            StatsScope::Session => self.session.lock().expect("stats session mutex").clone(),
            StatsScope::Global => self.global.lock().expect("stats global mutex").clone(),
        };
        let mut snap = build_snapshot(
            &data,
            scope,
            self.path.display().to_string(),
            session_uptime_secs,
            experience,
            classifier,
            effective_routing,
            agent_budgets,
        );
        let samples = self
            .latency_samples
            .lock()
            .expect("stats latency_samples mutex");
        let (p95_ms, p99_ms) = latency_percentiles(&samples);
        snap.latency.p95_ms = p95_ms;
        snap.latency.p99_ms = p99_ms;
        snap
    }

    pub fn session_uptime_secs(&self) -> u64 {
        self.session_started.elapsed().as_secs()
    }

    pub fn global_data(&self) -> StatsData {
        self.global.lock().expect("stats global mutex").clone()
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
    );
    let mut cloud_tier = tier_token_stats(
        data.cloud_tokens_in,
        data.cloud_tokens_out,
        data.cloud_cached_tokens,
    );
    let mut total_tier = tier_token_stats(total_in, total_out, total_cached);
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
            avg_tps: avg_tps(data.tps_sum_x1000, data.tps_count),
            edge_tps: avg_tps(data.edge_tps_sum_x1000, data.edge_tps_count),
            cloud_tps: avg_tps(data.cloud_tps_sum_x1000, data.cloud_tps_count),
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

fn avg_tps(sum_x1000: u64, count: u64) -> f64 {
    if count == 0 {
        0.0
    } else {
        (sum_x1000 as f64) / 1000.0 / count as f64
    }
}

fn latency_percentiles(samples: &VecDeque<u64>) -> (f64, f64) {
    if samples.is_empty() {
        return (0.0, 0.0);
    }
    let mut sorted: Vec<u64> = samples.iter().copied().collect();
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

fn tier_token_stats(input: u64, output: u64, cached: u64) -> TierTokenStats {
    TierTokenStats {
        input,
        output,
        cached,
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
        }
    }

    #[test]
    fn upstream_metrics_aggregates_token_breakdown() {
        let stats = GatewayStats::new_in_memory();
        stats.record_upstream_metrics(&UpstreamCallMetrics {
            tier: "edge",
            prompt_tokens: 100,
            completion_tokens: 50,
            cached_tokens: 80,
            latency_ms: 200,
            ttft_ms: Some(50),
            stream: true,
        });
        stats.record_upstream_metrics(&UpstreamCallMetrics {
            tier: "cloud",
            prompt_tokens: 200,
            completion_tokens: 100,
            cached_tokens: 0,
            latency_ms: 500,
            ttft_ms: None,
            stream: false,
        });
        stats.record_final_response(&FinalResponseMetrics {
            served_tier: "edge",
            cloud_input_saved: 100,
            completion_tokens: 50,
        });
        let snap = stats.snapshot(StatsScope::Session, 60, None, None, None, None);
        assert_eq!(snap.token_breakdown.edge.input, 100);
        assert_eq!(snap.token_breakdown.cloud.input, 200);
        assert_eq!(snap.token_breakdown.cloud_saved.total, 150);
        assert_eq!(snap.cache.hit_requests, 1);
        assert_eq!(snap.cache.cached_tokens, 80);
        assert!(snap.latency.avg_ttft_ms > 0.0);
        assert!(snap.latency.edge_tps > 0.0);
        assert!(snap.latency.cloud_tps > 0.0);
        assert!(snap.latency.p95_ms >= 200.0);
        assert!(snap.latency.p99_ms >= snap.latency.p95_ms);
        assert_eq!(snap.served.edge, 1);
    }

    #[test]
    fn snapshot_aggregates_decisions() {
        let stats = GatewayStats::new_in_memory();
        stats.record_request(false);
        stats.record_decision(&sample_decision(RouteTier::Edge));
        stats.record_decision(&sample_decision(RouteTier::Cloud));
        let snap = stats.snapshot(StatsScope::Session, 60, None, None, None, None);
        assert_eq!(snap.scope, "session");
        assert_eq!(snap.requests_total, 1);
        assert_eq!(snap.routing.edge, 1);
        assert_eq!(snap.routing.cloud, 1);
        assert_eq!(snap.tokens.in_estimate, 200);
        assert!(snap.step_kinds.contains_key("directchat"));

        let global = stats.snapshot(StatsScope::Global, 60, None, None, None, None);
        assert_eq!(global.scope, "global");
        assert_eq!(global.requests_total, 1);
    }

    #[test]
    fn session_resets_on_reopen_global_persists() {
        let dir = std::env::temp_dir().join(format!(
            "flowy-stats-session-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        {
            let stats = GatewayStats::open(&dir).unwrap();
            stats.record_request(true);
            stats.flush().unwrap();
            let session = stats.snapshot(StatsScope::Session, 10, None, None, None, None);
            assert_eq!(session.requests_total, 1);
        }
        {
            let stats = GatewayStats::open(&dir).unwrap();
            let session = stats.snapshot(StatsScope::Session, 10, None, None, None, None);
            let global = stats.snapshot(StatsScope::Global, 10, None, None, None, None);
            assert_eq!(session.requests_total, 0, "new process session starts at 0");
            assert_eq!(global.requests_total, 1, "global survives restart");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn persist_roundtrip() {
        let dir = std::env::temp_dir().join(format!(
            "flowy-stats-test-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("stats.json");
        {
            let stats = GatewayStats::open(&dir).unwrap();
            stats.record_request(true);
            stats.flush().unwrap();
        }
        let loaded = data::load(&path).unwrap();
        assert_eq!(loaded.requests_total, 1);
        assert_eq!(loaded.requests_stream, 1);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
