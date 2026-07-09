use crate::gateway::api::openai::{ChatCompletionRequest, ChatCompletionResponse};
use crate::gateway::experience::RequestOutcome;
use crate::gateway::routing::request_context_hash;
use crate::gateway::routing::RouteDecision;
use crate::gateway::routing::RouteTier;
use crate::gateway::routing_log::RoutingLogStore;
use crate::gateway::session::SessionStore;
use crate::gateway::stats::metrics::{effective_upstream_model, tokens_from_response};

/// Result of a completed upstream call (actual tier + token usage).
#[derive(Debug, Clone)]
pub struct ServedOutcome {
    pub outcome: RequestOutcome,
    pub served_tier: String,
    pub served_model: String,
    pub cached_tokens: u32,
    pub prompt_tokens: u32,
}

impl ServedOutcome {
    pub fn from_non_stream(
        decision: &RouteDecision,
        resp: &ChatCompletionResponse,
        fallback: bool,
    ) -> Self {
        let outcome = RequestOutcome::success(decision, fallback);
        let (prompt, _completion, cached) =
            tokens_from_response(resp, decision.tokens_in_estimate);
        let served_tier = infer_served_tier(decision, fallback);
        let forwarded = resp.upstream_forwarded_model.as_deref().unwrap_or("");
        Self {
            outcome,
            served_tier,
            served_model: effective_upstream_model(forwarded, &resp.model),
            cached_tokens: cached,
            prompt_tokens: prompt,
        }
    }

    pub fn from_stream(
        decision: &RouteDecision,
        tier: &str,
        fallback: bool,
        prompt_tokens: u32,
        cached_tokens: u32,
        model: &str,
    ) -> Self {
        Self {
            outcome: RequestOutcome::success(decision, fallback),
            served_tier: tier.to_string(),
            served_model: model.to_string(),
            cached_tokens,
            prompt_tokens,
        }
    }
}

fn infer_served_tier(decision: &RouteDecision, fallback: bool) -> String {
    if fallback {
        "cloud".to_string()
    } else {
        match decision.route {
            RouteTier::Cloud => "cloud".to_string(),
            _ => "edge".to_string(),
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct CloudCacheSettings {
    pub boost_max: f32,
    pub decay_half_life_secs: u64,
    pub route_cache_enabled: bool,
}

impl CloudCacheSettings {
    pub fn from_config(c: &crate::gateway::config::AppConfig) -> Self {
        Self {
            boost_max: c.cloud_cache_boost_max,
            decay_half_life_secs: c.cloud_cache_decay_half_life_secs,
            route_cache_enabled: c.request_route_cache_enabled,
        }
    }
}

pub fn record_request_completion(
    sessions: &SessionStore,
    routing_logs: &RoutingLogStore,
    settings: &CloudCacheSettings,
    req: &ChatCompletionRequest,
    decision: &RouteDecision,
    served: &ServedOutcome,
    assistant_failed_signal: bool,
) {
    sessions.apply_served_outcome(
        &decision.conversation_key,
        decision,
        served,
        settings,
        assistant_failed_signal,
    );
    if settings.route_cache_enabled {
        let hash = request_context_hash(req);
        let _ = routing_logs.upsert_route_cache(&hash, &served.served_tier, &served.served_model);
    }
}

#[derive(Clone)]
pub struct StreamPostServe {
    pub sessions: std::sync::Arc<SessionStore>,
    pub routing_logs: std::sync::Arc<RoutingLogStore>,
    pub settings: CloudCacheSettings,
    pub req: ChatCompletionRequest,
    pub decision: RouteDecision,
    pub assistant_failed: bool,
    pub fallback: bool,
    pub served_model: String,
}

pub fn run_stream_post_serve(
    bundle: &StreamPostServe,
    tier: &str,
    prompt_tokens: u32,
    cached_tokens: u32,
) {
    let served = ServedOutcome::from_stream(
        &bundle.decision,
        tier,
        bundle.fallback,
        prompt_tokens,
        cached_tokens,
        &bundle.served_model,
    );
    record_request_completion(
        bundle.sessions.as_ref(),
        bundle.routing_logs.as_ref(),
        &bundle.settings,
        &bundle.req,
        &bundle.decision,
        &served,
        bundle.assistant_failed,
    );
}