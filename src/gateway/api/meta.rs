use axum::http::{HeaderMap, HeaderValue};

use crate::gateway::api::openai::{ChatCompletionResponse, TokenRouterMeta};
use crate::gateway::stats::metrics::tokens_from_response;
use crate::gateway::routing::{Profile, RouteDecision, RouteTier, StepKind};

pub fn build_token_router_meta(
    decision: &RouteDecision,
    fallback: bool,
    resp: &ChatCompletionResponse,
) -> TokenRouterMeta {
    let (tokens_in, tokens_out, _) = tokens_from_response(resp, decision.tokens_in_estimate);
    let input_ratio = if tokens_in + tokens_out > 0 {
        tokens_in as f32 / (tokens_in + tokens_out) as f32
    } else {
        0.9933
    };

    TokenRouterMeta {
        route: tier_name(decision.route).to_string(),
        fallback,
        difficulty_score: decision.difficulty,
        step_kind: step_kind_name(decision.step_kind).to_string(),
        reason_codes: decision.reason_codes.clone(),
        tokens_in,
        tokens_out,
        input_ratio,
        cloud_input_saved: decision.cloud_input_saved_estimate,
        profile: profile_name(decision.profile).to_string(),
    }
}

pub fn token_router_meta_headers(decision: &RouteDecision, fallback: bool) -> HeaderMap {
    let mut headers = HeaderMap::new();
    insert(&mut headers, "x-token-router-route", tier_name(decision.route));
    insert(
        &mut headers,
        "x-token-router-fallback",
        if fallback { "true" } else { "false" },
    );
    insert(
        &mut headers,
        "x-token-router-step-kind",
        step_kind_name(decision.step_kind),
    );
    insert(
        &mut headers,
        "x-token-router-profile",
        profile_name(decision.profile),
    );
    if let Ok(v) = HeaderValue::from_str(&format!("{:.4}", decision.difficulty)) {
        headers.insert("x-token-router-difficulty", v);
    }
    if let Some(p) = decision.edge_ok_probability {
        if let Ok(v) = HeaderValue::from_str(&format!("{:.4}", p)) {
            headers.insert("x-token-router-edge-prob", v);
        }
    }
    if !decision.reason_codes.is_empty() {
        let joined = decision.reason_codes.join(",");
        if let Ok(v) = HeaderValue::from_str(&joined) {
            headers.insert("x-token-router-reason-codes", v);
        }
    }
    headers
}

fn insert(headers: &mut HeaderMap, name: &'static str, value: &str) {
    if let Ok(v) = HeaderValue::from_str(value) {
        headers.insert(name, v);
    }
}

pub fn tier_name(t: RouteTier) -> &'static str {
    match t {
        RouteTier::Edge => "edge",
        RouteTier::Cloud => "cloud",
        RouteTier::Cascade => "cascade",
    }
}

pub fn step_kind_name(k: StepKind) -> &'static str {
    match k {
        StepKind::HeartbeatAck => "heartbeat_ack",
        StepKind::DirectChat => "direct_chat",
        StepKind::RecoveryAfterFailure => "recovery_after_failure",
        StepKind::ToolSelect => "tool_select",
        StepKind::ToolArgFill => "tool_arg_fill",
        StepKind::ToolResultDigest => "tool_result_digest",
        StepKind::InitialPlan => "initial_plan",
        StepKind::FinalReply => "final_reply",
        StepKind::SubagentSpawn => "subagent_spawn",
        StepKind::MemoryCompact => "memory_compact",
        StepKind::CronBackground => "cron_background",
    }
}

pub fn profile_name(p: Profile) -> &'static str {
    match p {
        Profile::Economy => "economy",
        Profile::Balanced => "balanced",
        Profile::Premium => "premium",
        Profile::Privacy => "privacy",
    }
}
