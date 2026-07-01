use std::collections::HashMap;
use std::path::Path;

use anyhow::Context;
use serde::{Deserialize, Serialize};

use crate::gateway::error::AppError;
use crate::gateway::routing::{RouteDecision, RouteTier, StepKind};
use crate::gateway::stats::metrics::{FinalResponseMetrics, UpstreamCallMetrics};

pub const STATS_VERSION: u32 = 2;

/// Cumulative counters persisted to `stats.json`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct StatsData {
    pub version: u32,
    #[serde(default)]
    pub first_record_at_unix: Option<u64>,
    #[serde(default)]
    pub last_updated_at_unix: Option<u64>,
    pub requests_total: u64,
    pub requests_stream: u64,
    pub requests_non_stream: u64,
    #[serde(default)]
    pub requests_cancelled: u64,
    pub route_edge: u64,
    pub route_cloud: u64,
    pub route_cascade: u64,
    pub upstream_edge_calls: u64,
    pub upstream_cloud_calls: u64,
    pub cascade_edge_ok: u64,
    pub cascade_fallback: u64,
    pub errors_total: u64,
    pub errors_unauthorized: u64,
    pub errors_unavailable: u64,
    pub errors_upstream: u64,
    pub errors_bad_request: u64,
    pub tokens_in_estimate: u64,
    pub tokens_out_estimate: u64,
    pub cloud_input_saved_estimate: u64,
    pub difficulty_sum: u64,
    pub difficulty_count: u64,
    #[serde(default)]
    pub step_kinds: HashMap<String, u64>,
    // --- v2: upstream usage & latency ---
    #[serde(default)]
    pub edge_tokens_in: u64,
    #[serde(default)]
    pub edge_tokens_out: u64,
    #[serde(default)]
    pub edge_cached_tokens: u64,
    #[serde(default)]
    pub cloud_tokens_in: u64,
    #[serde(default)]
    pub cloud_tokens_out: u64,
    #[serde(default)]
    pub cloud_cached_tokens: u64,
    #[serde(default)]
    pub cloud_tokens_saved_input: u64,
    #[serde(default)]
    pub cloud_tokens_saved_output: u64,
    #[serde(default)]
    pub cache_hit_requests: u64,
    #[serde(default)]
    pub cached_tokens_total: u64,
    #[serde(default)]
    pub latency_sum_ms: u64,
    #[serde(default)]
    pub latency_count: u64,
    #[serde(default)]
    pub stream_latency_sum_ms: u64,
    #[serde(default)]
    pub stream_latency_count: u64,
    #[serde(default)]
    pub non_stream_latency_sum_ms: u64,
    #[serde(default)]
    pub non_stream_latency_count: u64,
    #[serde(default)]
    pub ttft_sum_ms: u64,
    #[serde(default)]
    pub ttft_count: u64,
    #[serde(default)]
    pub tps_sum_x1000: u64,
    #[serde(default)]
    pub tps_count: u64,
    #[serde(default)]
    pub edge_tps_sum_x1000: u64,
    #[serde(default)]
    pub edge_tps_count: u64,
    #[serde(default)]
    pub cloud_tps_sum_x1000: u64,
    #[serde(default)]
    pub cloud_tps_count: u64,
    #[serde(default)]
    pub edge_served_responses: u64,
    #[serde(default)]
    pub cloud_served_responses: u64,
}

impl StatsData {
    pub fn touch_updated(&mut self) {
        let now = now_unix();
        if self.first_record_at_unix.is_none() {
            self.first_record_at_unix = Some(now);
        }
        self.last_updated_at_unix = Some(now);
        self.version = STATS_VERSION;
    }

    pub fn record_request(&mut self, stream: bool) {
        self.touch_updated();
        self.requests_total += 1;
        if stream {
            self.requests_stream += 1;
        } else {
            self.requests_non_stream += 1;
        }
    }

    pub fn record_decision(&mut self, decision: &RouteDecision) {
        self.touch_updated();
        match decision.route {
            RouteTier::Edge => self.route_edge += 1,
            RouteTier::Cloud => self.route_cloud += 1,
            RouteTier::Cascade => self.route_cascade += 1,
        }
        self.tokens_in_estimate += decision.tokens_in_estimate as u64;
        self.cloud_input_saved_estimate += decision.cloud_input_saved_estimate as u64;
        let scaled = (decision.difficulty * 1000.0).round() as u64;
        self.difficulty_sum += scaled;
        self.difficulty_count += 1;
        let key = step_kind_key(decision.step_kind);
        *self.step_kinds.entry(key).or_insert(0) += 1;
    }

    pub fn record_completion_tokens(&mut self, tokens_out: u32) {
        self.touch_updated();
        self.tokens_out_estimate += tokens_out as u64;
    }

    pub fn record_upstream_edge(&mut self) {
        self.touch_updated();
        self.upstream_edge_calls += 1;
    }

    pub fn record_upstream_cloud(&mut self) {
        self.touch_updated();
        self.upstream_cloud_calls += 1;
    }

    pub fn record_cascade_edge_ok(&mut self) {
        self.touch_updated();
        self.cascade_edge_ok += 1;
    }

    pub fn record_cascade_fallback(&mut self) {
        self.touch_updated();
        self.cascade_fallback += 1;
    }

    pub fn record_upstream_metrics(&mut self, m: &UpstreamCallMetrics) {
        self.touch_updated();
        self.add_tier_tokens(m.tier, m.prompt_tokens, m.completion_tokens, m.cached_tokens);
        self.latency_sum_ms += m.latency_ms;
        self.latency_count += 1;
        if m.stream {
            self.stream_latency_sum_ms += m.latency_ms;
            self.stream_latency_count += 1;
        } else {
            self.non_stream_latency_sum_ms += m.latency_ms;
            self.non_stream_latency_count += 1;
        }
        if let Some(ttft) = m.ttft_ms {
            self.ttft_sum_ms += ttft;
            self.ttft_count += 1;
        }
        if m.completion_tokens > 0 {
            let gen_ms = m
                .latency_ms
                .saturating_sub(m.ttft_ms.unwrap_or(0))
                .max(1);
            let tps_x1000 =
                (m.completion_tokens as u64 * 1000 * 1000).saturating_div(gen_ms);
            self.tps_sum_x1000 += tps_x1000;
            self.tps_count += 1;
            match m.tier {
                "edge" => {
                    self.edge_tps_sum_x1000 += tps_x1000;
                    self.edge_tps_count += 1;
                }
                "cloud" => {
                    self.cloud_tps_sum_x1000 += tps_x1000;
                    self.cloud_tps_count += 1;
                }
                _ => {}
            }
        }
    }

    pub fn record_final_response(&mut self, m: &FinalResponseMetrics) {
        self.touch_updated();
        match m.served_tier {
            "edge" => {
                self.edge_served_responses += 1;
                self.cloud_tokens_saved_input += m.cloud_input_saved as u64;
                self.cloud_tokens_saved_output += m.completion_tokens as u64;
            }
            "cloud" => self.cloud_served_responses += 1,
            _ => {}
        }
    }

    fn add_tier_tokens(&mut self, tier: &str, prompt: u32, completion: u32, cached: u32) {
        match tier {
            "edge" => {
                self.edge_tokens_in += prompt as u64;
                self.edge_tokens_out += completion as u64;
                self.edge_cached_tokens += cached as u64;
            }
            "cloud" => {
                self.cloud_tokens_in += prompt as u64;
                self.cloud_tokens_out += completion as u64;
                self.cloud_cached_tokens += cached as u64;
            }
            _ => {}
        }
        if cached > 0 {
            self.cache_hit_requests += 1;
        }
        self.cached_tokens_total += cached as u64;
    }

    pub fn record_error(&mut self, err: &AppError) {
        self.touch_updated();
        self.errors_total += 1;
        match err {
            AppError::Unauthorized(_) => self.errors_unauthorized += 1,
            AppError::Unavailable(_) => self.errors_unavailable += 1,
            AppError::Upstream(_) => self.errors_upstream += 1,
            AppError::BadRequest(_) => self.errors_bad_request += 1,
            AppError::Internal(_) => {}
        }
    }
}

pub fn load(path: &Path) -> anyhow::Result<StatsData> {
    if !path.exists() {
        return Ok(StatsData::default());
    }
    let text = std::fs::read_to_string(path)
        .with_context(|| format!("read stats {}", path.display()))?;
    match serde_json::from_str(&text) {
        Ok(data) => Ok(data),
        Err(e) => {
            tracing::warn!(
                path = %path.display(),
                error = %e,
                "invalid stats file, starting fresh"
            );
            Ok(StatsData::default())
        }
    }
}

pub fn save(path: &Path, data: &StatsData) -> anyhow::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("create stats dir {}", parent.display()))?;
    }
    let tmp = path.with_extension("json.tmp");
    let json = serde_json::to_string_pretty(data)?;
    std::fs::write(&tmp, json).with_context(|| format!("write stats {}", tmp.display()))?;
    std::fs::rename(&tmp, path).with_context(|| format!("rename stats {}", path.display()))?;
    Ok(())
}

fn step_kind_key(k: StepKind) -> String {
    format!("{:?}", k).to_ascii_lowercase()
}

pub fn now_unix() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}
