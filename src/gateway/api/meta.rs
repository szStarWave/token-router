use axum::http::{HeaderMap, HeaderValue};

use crate::gateway::api::openai::{ChatCompletionResponse, TokenRouterMeta};
use crate::gateway::error::AppError;
use crate::gateway::stats::metrics::tokens_from_response;
use crate::gateway::multimodal::MultimodalStrategy;
use crate::gateway::routing::{Profile, RouteDecision, RouteTier, RoutingMode, StepKind, WorkStrategy};

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

/// Upstream that actually served the response (after cascade / quality fallback).
pub fn served_tier_name(decision: &RouteDecision, fallback: bool) -> &'static str {
    if fallback {
        "cloud"
    } else {
        match decision.route {
            RouteTier::Cloud => "cloud",
            RouteTier::Edge | RouteTier::Cascade => "edge",
        }
    }
}

/// Human-readable summary of why this route was chosen (gates, work, policy, etc.).
pub fn summarize_route_reasons(decision: &RouteDecision) -> String {
    let codes = &decision.reason_codes;
    let mut parts: Vec<String> = codes
        .iter()
        .filter(|c| is_decisive_reason_code(c))
        .cloned()
        .collect();

    if parts.is_empty() {
        if decision.profile == Profile::Privacy {
            parts.push("privacy_profile".into());
        } else if let Some(d) = codes.iter().find(|c| c.starts_with("DIFFICULTY_")) {
            parts.push(format!(
                "policy {d} 鈫?{}",
                tier_name(decision.route)
            ));
        }
    }

    if let Some(theta) = codes.iter().find(|c| c.starts_with("ADAPTIVE_THETA")) {
        parts.push(theta.clone());
    }

    if parts.is_empty() {
        codes.join(", ")
    } else {
        parts.join("; ")
    }
}

fn is_decisive_reason_code(code: &str) -> bool {
    const PREFIXES: &[&str] = &[
        "GATE_",
        "CONFIG_ROUTE_",
        "UPSTREAM_",
        "PLAN_",
        "INITIAL_PLAN_",
        "MULTIMODAL_",
        "WORK_",
        "STICKY_",
        "CASUAL_",
        "LEXICAL_",
        "BAYES_",
        "EXP_BIAS_",
        "TOOL_ERROR_STREAK_",
        "TOOL_LOOP_",
    ];
    PREFIXES.iter().any(|p| code.starts_with(p))
}

const LOG_USER_PREVIEW_MAX: usize = 120;

/// Collapse whitespace and cap length for structured log fields.
pub fn truncate_user_preview_for_log(text: &str) -> String {
    let collapsed: String = text
        .chars()
        .map(|c| if c == '\n' || c == '\r' { ' ' } else { c })
        .collect();
    let trimmed = collapsed.trim();
    let char_count = trimmed.chars().count();
    if char_count <= LOG_USER_PREVIEW_MAX {
        return trimmed.to_string();
    }
    let mut out: String = trimmed.chars().take(LOG_USER_PREVIEW_MAX).collect();
    out.push_str("\u{2026}");
    out
}

pub fn log_route_decision(
    store: Option<&crate::gateway::routing_log::RoutingLogStore>,
    decision: &RouteDecision,
    model: &str,
    user_preview: &str,
    stream: bool,
    agent_id: Option<&str>,
) -> Option<i64> {
    let summary = summarize_route_reasons(decision);
    let user_preview = truncate_user_preview_for_log(user_preview);
    let message = format!(
        "routing: {} 鈫?{} | {}",
        step_kind_name(decision.step_kind),
        tier_name(decision.route),
        summary,
    );
    tracing::info!(
        agent_id = agent_id.unwrap_or("default"),
        model,
        route = tier_name(decision.route),
        step_kind = step_kind_name(decision.step_kind),
        profile = profile_name(decision.profile),
        mode = routing_mode_name(decision.mode),
        difficulty = decision.difficulty,
        stream,
        tok_in = decision.tokens_in_estimate,
        work = work_strategy_name(decision.work_strategy),
        multimodal = multimodal_strategy_name(decision.multimodal_strategy),
        casual_quality_fallback = decision.casual_quality_fallback,
        edge_prob = ?decision.edge_ok_probability,
        reason_codes = %decision.reason_codes.join(","),
        user_preview = %user_preview,
        "{message}"
    );
    store.and_then(|store| {
        store
            .record_decision(decision, model, user_preview.as_str(), stream, agent_id)
            .map(Some)
            .unwrap_or_else(|e| {
                tracing::warn!(error = %e, "routing log db write failed");
                None
            })
    })
}

pub fn log_upstream_served(
    store: Option<&crate::gateway::routing_log::RoutingLogStore>,
    routing_log_id: Option<i64>,
    decision: &RouteDecision,
    served_tier: &str,
    fallback: bool,
    stream: bool,
    served_model: Option<&str>,
    agent_id: Option<&str>,
) {
    let summary = summarize_route_reasons(decision);
    let message = if fallback {
        format!(
            "served: {} (fallback from {}) | {}",
            served_tier,
            tier_name(decision.route),
            summary,
        )
    } else {
        format!("served: {} | {}", served_tier, summary)
    };
    tracing::info!(
        agent_id = agent_id.unwrap_or("default"),
        route = tier_name(decision.route),
        served = served_tier,
        fallback,
        stream,
        step_kind = step_kind_name(decision.step_kind),
        reason_codes = %decision.reason_codes.join(","),
        "{message}"
    );
    if let (Some(store), Some(id)) = (store, routing_log_id) {
        if let Err(e) = store.mark_served(id, served_tier, served_model) {
            tracing::warn!(error = %e, routing_log_id = id, "routing log served update failed");
        }
    }
}

pub fn error_reason_from_app_error(err: &AppError) -> String {
    match err {
        AppError::BadRequest(msg) => msg.clone(),
        AppError::Unauthorized(msg) => msg.clone(),
        AppError::Upstream(msg) => msg.clone(),
        AppError::Unavailable(msg) => msg.clone(),
        AppError::NotFound(msg) => msg.clone(),
        AppError::Internal(e) => e.to_string(),
    }
}

pub fn log_request_error(
    store: Option<&crate::gateway::routing_log::RoutingLogStore>,
    routing_log_id: Option<i64>,
    err: &AppError,
) {
    let reason = error_reason_from_app_error(err);
    if let (Some(store), Some(id)) = (store, routing_log_id) {
        if let Err(e) = store.mark_error(id, &reason) {
            tracing::warn!(error = %e, routing_log_id = id, "routing log error update failed");
        }
    }
    tracing::info!(
        routing_log_id = ?routing_log_id,
        error_reason = %reason,
        "routing request failed"
    );
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

pub fn routing_mode_name(m: RoutingMode) -> &'static str {
    match m {
        RoutingMode::Single => "single",
        RoutingMode::Cascade => "cascade",
        RoutingMode::Split => "split",
    }
}

pub fn work_strategy_name(w: WorkStrategy) -> &'static str {
    match w {
        WorkStrategy::None => "none",
        WorkStrategy::Verify => "verify",
    }
}

pub fn multimodal_strategy_name(m: MultimodalStrategy) -> &'static str {
    match m {
        MultimodalStrategy::None => "none",
        MultimodalStrategy::Probe => "probe",
        MultimodalStrategy::CachedEdge => "cached_edge",
        MultimodalStrategy::CachedCloud => "cached_cloud",
        MultimodalStrategy::CachedEdgeFallback => "cached_edge_fallback",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gateway::routing::{RouteDecision, RouteTier, StepKind, Profile, RoutingMode, WorkStrategy};
    use crate::gateway::multimodal::MultimodalStrategy;

    fn sample_decision(route: RouteTier) -> RouteDecision {
        RouteDecision {
            route,
            profile: Profile::Balanced,
            mode: RoutingMode::Cascade,
            step_kind: StepKind::DirectChat,
            difficulty: 0.2,
            reason_codes: vec!["STEP_DIRECT_CHAT".into()],
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
    fn served_tier_edge_direct() {
        let d = sample_decision(RouteTier::Edge);
        assert_eq!(served_tier_name(&d, false), "edge");
    }

    #[test]
    fn served_tier_cloud_on_fallback() {
        let d = sample_decision(RouteTier::Edge);
        assert_eq!(served_tier_name(&d, true), "cloud");
    }

    #[test]
    fn served_tier_cloud_direct() {
        let d = sample_decision(RouteTier::Cloud);
        assert_eq!(served_tier_name(&d, false), "cloud");
    }

    #[test]
    fn summarize_gate_reasons() {
        let mut d = sample_decision(RouteTier::Cloud);
        d.reason_codes = vec![
            "STEP_DIRECT_CHAT".into(),
            "GATE_CTX_OVERFLOW".into(),
            "DIFFICULTY_0.90".into(),
        ];
        let summary = summarize_route_reasons(&d);
        assert!(summary.contains("GATE_CTX_OVERFLOW"));
        assert!(!summary.contains("STEP_DIRECT_CHAT"));
    }

    #[test]
    fn summarize_policy_reasons() {
        let mut d = sample_decision(RouteTier::Edge);
        d.reason_codes = vec![
            "STEP_DIRECT_CHAT".into(),
            "DIFFICULTY_0.20".into(),
            "TOK_IN_120".into(),
        ];
        let summary = summarize_route_reasons(&d);
        assert!(summary.contains("policy DIFFICULTY_0.20"));
        assert!(summary.contains("edge"));
    }

    #[test]
    fn truncate_user_preview_collapses_and_caps() {
        assert_eq!(truncate_user_preview_for_log("  hello  "), "hello");
        assert_eq!(
            truncate_user_preview_for_log("line1\nline2"),
            "line1 line2"
        );
        let long = "a".repeat(150);
        let out = truncate_user_preview_for_log(&long);
        assert_eq!(out.chars().count(), LOG_USER_PREVIEW_MAX + 1);
        assert!(out.ends_with("\u{2026}"));
    }
}

