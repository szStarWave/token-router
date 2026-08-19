use std::str::FromStr;

use crate::gateway::api::openai::ChatCompletionRequest;
use crate::gateway::classifier::{ClassifierStore, FeatureVector};
use crate::gateway::config::AppConfig;
use crate::gateway::experience::ExperienceStore;
use crate::gateway::multimodal::{MultimodalRouteHint, MultimodalStore, MultimodalStrategy};

use super::conversation::conversation_key;
use super::difficulty::{apply_privacy_cap, emit_difficulty_breakdown, DecisionContext, DifficultyScore};
use super::edge_busy::edge_busy_applies;
use super::edge_busy::apply_edge_busy_fallback;
use super::gates::collect_gate_biases;
use super::policy::{self, Profile};
use super::signals::{is_simple_multimodal, last_user_message_text, RequestSignals, SignalExtractor};
use super::step_kind::{StepKind, resolve_step_kind};
use super::cloud_cache::cloud_cache_extra_parts;
use super::request_hash::request_context_hash;
use super::upstream_availability::{cloud_configured, edge_configured, finalize_route};
use super::work::{attach_work_verify, WorkStrategy};
use crate::gateway::edge_load::EdgeInferenceTracker;
use crate::gateway::routing::adaptive::EffectiveRouting;
use crate::gateway::routing_log::RoutingLogStore;
use crate::gateway::session::SessionStore;

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RouteTier {
    Edge,
    Cloud,
    Cascade,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub enum RoutingMode {
    Single,
    Cascade,
    Split,
}

impl FromStr for RoutingMode {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_ascii_lowercase().as_str() {
            "single" => Ok(RoutingMode::Single),
            "cascade" => Ok(RoutingMode::Cascade),
            "split" => Ok(RoutingMode::Split),
            _ => Err(()),
        }
    }
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct RouteDecision {
    pub route: RouteTier,
    pub profile: Profile,
    pub mode: RoutingMode,
    pub step_kind: StepKind,
    pub difficulty: f32,
    pub reason_codes: Vec<String>,
    pub tokens_in_estimate: u32,
    pub tokens_out_estimate: u32,
    pub cloud_input_saved_estimate: u32,
    /// Set by `decide` for outcome recording.
    #[serde(skip)]
    pub conversation_key: String,
    #[serde(skip)]
    pub assistant_failed_recent: bool,
    /// Trailing consecutive tool errors at decision time (for learning + stats).
    #[serde(skip)]
    pub consecutive_tool_error_streak: u32,
    #[serde(skip)]
    pub multimodal_strategy: crate::gateway::multimodal::MultimodalStrategy,
    #[serde(skip)]
    pub work_strategy: WorkStrategy,
    /// Set when tool-error streak recommends cloud stickiness.
    #[serde(skip)]
    pub force_cloud_sticky: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub edge_ok_probability: Option<f32>,
    /// Features captured at decision time for classifier learning.
    #[serde(skip)]
    pub classifier_features: Option<FeatureVector>,
    /// DirectChat/HeartbeatAck in auto mode: try edge first, cloud on quality gate failure.
    /// Never set when `gateway.route` is fixed to `edge` / `cloud`.
    #[serde(skip)]
    pub casual_quality_fallback: bool,
    /// Context for wordfreq runtime learning after the request completes.
    #[serde(skip)]
    pub lexical_learn: super::LexicalLearnContext,
    /// Row id in `routing_logs.db`; set after `log_route_decision`.
    #[serde(skip)]
    pub routing_log_id: Option<i64>,
}

pub fn decide(
    config: &AppConfig,
    req: &ChatCompletionRequest,
    sessions: &SessionStore,
    experience: Option<&ExperienceStore>,
    multimodal: Option<&MultimodalStore>,
    routing: &EffectiveRouting,
    edge_load: Option<&EdgeInferenceTracker>,
    edge_tps: Option<f64>,
    classifier: Option<&ClassifierStore>,
    routing_logs: Option<&RoutingLogStore>,
    wordfreq: &super::WordFreqStore,
) -> RouteDecision {
    let profile = config.default_profile;
    let mode = config.routing_mode;
    let edge_ok = edge_configured(config);

    let conv_key = conversation_key(req);
    let prev_tok = sessions.get_last_tok_in(&conv_key);
    let cloud_cache = sessions.cloud_cache_state(&conv_key);
    let ctx_hash = request_context_hash(req);
    let extractor = SignalExtractor {
        ctx_edge_max: config.ctx_edge_max_tokens,
        wordfreq,
    };
    let signals = extractor.extract(req, prev_tok);
    let last_user_text = last_user_message_text(req);
    let step_kind = resolve_step_kind(req, &signals);
    let features = FeatureVector::from_signals(&signals, step_kind, config.ctx_edge_max_tokens);

    let mut reason_codes = Vec::new();
    reason_codes.push(format!("STEP_{}", step_kind_code(step_kind)));
    if config.request_route_cache_enabled {
        if let Some(store) = routing_logs {
            if let Ok(Some(hint)) = store.lookup_route_cache(&ctx_hash) {
                if hint.route == "cloud" {
                    reason_codes.push(format!("REQ_ROUTE_CACHE_CLOUD:{}", hint.model));
                } else if hint.route == "edge" {
                    reason_codes.push("REQ_ROUTE_CACHE_EDGE".to_string());
                }
            }
        }
    }

    if let Some(fixed) = config.fixed_route {
        reason_codes.push(format!("CONFIG_ROUTE_{}", tier_name(fixed)));
        let (route, work, mm) = apply_edge_busy_fallback(
            fixed,
            WorkStrategy::None,
            MultimodalStrategy::None,
            step_kind,
            config,
            edge_load,
            &mut reason_codes,
        );
        // Fixed edge/cloud must fail closed: do not remap to the other upstream
        // when the requested tier is unset (finalize_route would rewrite the tier).
        sessions.record_tokens(&conv_key, signals.tok_total_in);
        return finish(
            route,
            profile,
            mode,
            step_kind,
            &signals,
            reason_codes,
            cloud_input_saved(route, &signals),
            0.0,
            conv_key,
            false,
            mm,
            work,
            None,
            Some(features),
            &last_user_text,
            config,
        );
    }

    let gate_biases = collect_gate_biases(
        &signals,
        step_kind,
        config.ctx_edge_max_tokens,
        edge_ok,
    );
    reason_codes.extend(gate_biases.reason_codes.clone());
    if gate_biases.reason_codes.iter().any(|c| c == "GATE_RISKY_TOOL") {
        push_risky_tool_hard_reason(&signals, &mut reason_codes);
    }

    let exp_bias = experience.map(|e| e.bias_for(step_kind)).unwrap_or(0.0);
    if exp_bias.abs() > f32::EPSILON {
        reason_codes.push(format!("EXP_BIAS_{exp_bias:+.2}"));
    }

    let mut extra_parts: Vec<(String, f32)> = Vec::new();

    if signals.multimodal && step_kind == StepKind::DirectChat {
        if let Some(store) = multimodal {
            match store.route_hint(config, &req.model) {
                MultimodalRouteHint::CachedCloud => {
                    extra_parts.push(("MULTIMODAL_CACHE_CLOUD".to_string(), 0.55));
                    reason_codes.push("MULTIMODAL_CACHE_CLOUD".to_string());
                }
                MultimodalRouteHint::CachedEdge | MultimodalRouteHint::CachedEdgeFallback => {
                    extra_parts.push(("MULTIMODAL_CACHE_EDGE".to_string(), -0.30));
                    reason_codes.push("MULTIMODAL_CACHE_EDGE".to_string());
                }
                MultimodalRouteHint::Probe => {}
            }
        }
    }

    if step_kind == StepKind::MemoryCompact
        && !super::gates::ctx_overflow_triggers(
            &signals,
            step_kind,
            config.ctx_edge_max_tokens,
        )
    {
        extra_parts.push(("MEMORY_COMPACT_IN_BUDGET".to_string(), -0.75));
    }

    if signals.intent_cloud && cloud_configured(config) {
        reason_codes.push("CLOUD_INTENT".to_string());
        extra_parts.push(("CLOUD_INTENT".to_string(), 0.40));
    } else if signals.intent_edge
        && !signals.intent_long_gen
        && edge_configured(config)
    {
        reason_codes.push("EDGE_INTENT".to_string());
        extra_parts.push(("EDGE_INTENT".to_string(), -0.25));
    }

    let now_unix = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let route_hint_route = if config.request_route_cache_enabled {
        routing_logs.and_then(|store| {
            store
                .lookup_route_cache(&ctx_hash)
                .ok()
                .flatten()
                .map(|h| h.route)
        })
    } else {
        None
    };
    let cache_parts = cloud_cache_extra_parts(
        cloud_cache.anchor_unix,
        cloud_cache.peak_linear,
        route_hint_route.as_deref(),
        now_unix,
        config.cloud_cache_decay_half_life_secs,
        config.cloud_cache_boost_max,
    );
    for (key, linear) in &cache_parts {
        if linear.abs() > f32::EPSILON && !reason_codes.iter().any(|c| c == key) {
            reason_codes.push(key.clone());
        }
    }

    let mut external_parts = gate_biases.parts.clone();
    external_parts.extend(extra_parts);
    external_parts.extend(cache_parts);

    let decision_ctx_base = DecisionContext {
        edge_tps,
        edge_busy: false,
    };
    let (d_preliminary, preliminary_breakdown) = DifficultyScore::compute_with_context(
        &signals,
        step_kind,
        config.ctx_edge_max_tokens,
        exp_bias,
        &external_parts,
        &decision_ctx_base,
    );

    let pre_difficulty = if profile == Profile::Privacy {
        if step_kind == StepKind::RecoveryAfterFailure {
            d_preliminary.0.max(routing.theta_cloud)
        } else {
            apply_privacy_cap(d_preliminary.0)
        }
    } else {
        d_preliminary.0
    };
    let pre_route = policy::map_policy_with_thresholds(
        DifficultyScore(pre_difficulty),
        step_kind,
        profile,
        mode,
        routing.theta_edge,
        routing.theta_cloud,
    );
    let edge_busy = edge_busy_applies(
        pre_route,
        WorkStrategy::None,
        MultimodalStrategy::None,
        step_kind,
        config,
        edge_load,
    );
    let decision_ctx = DecisionContext {
        edge_tps,
        edge_busy,
    };
    let (d_heuristic, heuristic_breakdown) = if edge_busy {
        DifficultyScore::compute_with_context(
            &signals,
            step_kind,
            config.ctx_edge_max_tokens,
            exp_bias,
            &external_parts,
            &decision_ctx,
        )
    } else {
        (d_preliminary, preliminary_breakdown)
    };
    for code in heuristic_breakdown.context_reason_codes() {
        if !reason_codes.iter().any(|c| c == code) {
            reason_codes.push(code.to_string());
        }
    }

    if signals.consecutive_tool_error_streak >= super::signals::TOOL_ERROR_STREAK_DIFFICULTY {
        reason_codes.push(format!(
            "TOOL_ERROR_STREAK_{}",
            signals.consecutive_tool_error_streak
        ));
    }
    if signals.tool_invocations_since_last_user >= 15
        && super::difficulty::tool_loop_bias_value(&signals, config.ctx_edge_max_tokens) > 0.0
    {
        reason_codes.push(format!(
            "TOOL_LOOP_{}",
            signals.tool_invocations_since_last_user
        ));
    }
    if signals.rare_lexical && signals.special_lexical {
        reason_codes.push("LEXICAL_BOTH".to_string());
    } else if signals.special_lexical {
        reason_codes.push("LEXICAL_SPECIAL".to_string());
    } else if signals.rare_lexical {
        reason_codes.push("LEXICAL_RARE".to_string());
    }
    push_risky_tool_soft_reason(&signals, &mut reason_codes);
    push_cognitive_task_reasons(&signals, &mut reason_codes);

    emit_difficulty_breakdown(&heuristic_breakdown, &mut reason_codes);

    let (mut difficulty, edge_ok_probability) = apply_classifier(
        classifier,
        &features,
        d_heuristic.0,
        &mut reason_codes,
    );
    if matches!(step_kind, StepKind::DirectChat | StepKind::HeartbeatAck)
        && d_heuristic.0 < routing.theta_cloud
    {
        let cap = routing.theta_cloud - f32::EPSILON;
        if difficulty >= routing.theta_cloud {
            let delta = cap - difficulty;
            reason_codes.push(format!("DIFF_D:CASUAL_CLASSIFIER_GUARD:{delta:+.4}"));
            reason_codes.push("CASUAL_CLASSIFIER_GUARD".to_string());
            difficulty = difficulty.min(cap);
        }
    }

    if profile == Profile::Privacy {
        if step_kind == StepKind::RecoveryAfterFailure {
            let floor = routing.theta_cloud;
            if difficulty < floor {
                let delta = floor - difficulty;
                reason_codes.push(format!("DIFF_D:PRIVACY_RECOVERY_FLOOR:{delta:+.4}"));
                difficulty = difficulty.max(floor);
            }
        } else {
            let capped = apply_privacy_cap(difficulty);
            if capped < difficulty {
                let delta = capped - difficulty;
                reason_codes.push(format!("DIFF_D:PRIVACY_CAP:{delta:+.4}"));
                difficulty = capped;
            }
        }
    }

    let multimodal_cloud_hint = signals.multimodal
        && step_kind == StepKind::DirectChat
        && multimodal.is_some_and(|store| {
            matches!(
                store.route_hint(config, &req.model),
                MultimodalRouteHint::CachedCloud
            )
        });
    difficulty = calibrate_difficulty(
        difficulty,
        &signals,
        step_kind,
        config,
        routing,
        edge_tps,
        multimodal_cloud_hint,
        &mut reason_codes,
    );

    if routing.enabled {
        reason_codes.push(format!(
            "ADAPTIVE_VERIFY(p={:.2})",
            routing.work_verify_sample_rate
        ));
        reason_codes.push(format!(
            "ADAPTIVE_THETA({:.2}|{:.2})",
            routing.theta_edge, routing.theta_cloud
        ));
        for r in &routing.reasons {
            if r.starts_with("ADAPTIVE_") && !reason_codes.iter().any(|x| x == r) {
                reason_codes.push(r.clone());
            }
        }
    }

    reason_codes.push(format!("DIFFICULTY_{:.2}", difficulty));
    reason_codes.push(format!("TOK_IN_{}", signals.tok_total_in));
    reason_codes.push(format!("TOK_DELTA_{}", signals.tok_loop_delta));

    let d = DifficultyScore(difficulty);
    let route = policy::map_policy_with_thresholds(
        d,
        step_kind,
        profile,
        mode,
        routing.theta_edge,
        routing.theta_cloud,
    );

    let work_strategy = attach_work_verify(
        route,
        step_kind,
        &signals,
        config,
        &conv_key,
        signals.tok_total_in,
        routing.work_verify_sample_rate,
        &mut reason_codes,
    );

    let multimodal_strategy = derive_multimodal_strategy(
        route,
        &signals,
        step_kind,
        config,
        req,
        multimodal,
        &mut reason_codes,
    );

    let (route, work_strategy, multimodal_strategy) = apply_edge_busy_fallback(
        route,
        work_strategy,
        multimodal_strategy,
        step_kind,
        config,
        edge_load,
        &mut reason_codes,
    );

    let route = finalize_route(route, config, &mut reason_codes);
    sessions.record_tokens(&conv_key, signals.tok_total_in);

    finish(
        route,
        profile,
        mode,
        step_kind,
        &signals,
        reason_codes,
        cloud_input_saved(route, &signals),
        difficulty,
        conv_key,
        false,
        multimodal_strategy,
        work_strategy,
        edge_ok_probability,
        Some(features),
        &last_user_text,
        config,
    )
}

fn derive_multimodal_strategy(
    route: RouteTier,
    signals: &super::signals::RequestSignals,
    step_kind: StepKind,
    config: &AppConfig,
    req: &ChatCompletionRequest,
    multimodal: Option<&MultimodalStore>,
    reason_codes: &mut Vec<String>,
) -> MultimodalStrategy {
    if !signals.multimodal || step_kind != StepKind::DirectChat {
        return MultimodalStrategy::None;
    }
    if !edge_configured(config) || !cloud_configured(config) {
        return MultimodalStrategy::None;
    }

    let hint = multimodal.map(|store| store.route_hint(config, &req.model));

    match route {
        RouteTier::Cloud => {
            if !reason_codes.iter().any(|c| c == "MULTIMODAL_CACHE_CLOUD") {
                reason_codes.push("MULTIMODAL_CACHE_CLOUD".to_string());
            }
            MultimodalStrategy::CachedCloud
        }
        RouteTier::Edge => match hint {
            Some(MultimodalRouteHint::CachedEdge) => {
                if !reason_codes.iter().any(|c| c == "MULTIMODAL_CACHE_EDGE") {
                    reason_codes.push("MULTIMODAL_CACHE_EDGE".to_string());
                }
                MultimodalStrategy::CachedEdge
            }
            Some(MultimodalRouteHint::CachedEdgeFallback) => {
                if !reason_codes.iter().any(|c| c == "MULTIMODAL_CACHE_EDGE") {
                    reason_codes.push("MULTIMODAL_CACHE_EDGE".to_string());
                }
                MultimodalStrategy::CachedEdgeFallback
            }
            _ if is_simple_multimodal(signals) => {
                reason_codes.push("MULTIMODAL_SIMPLE_EDGE".to_string());
                MultimodalStrategy::Probe
            }
            _ => {
                reason_codes.push("MULTIMODAL_PROBE_EDGE".to_string());
                MultimodalStrategy::Probe
            }
        },
        RouteTier::Cascade => MultimodalStrategy::None,
    }
}

fn calibrate_difficulty(
    difficulty: f32,
    signals: &RequestSignals,
    step_kind: StepKind,
    config: &AppConfig,
    routing: &EffectiveRouting,
    edge_tps: Option<f64>,
    multimodal_cloud_hint: bool,
    reason_codes: &mut Vec<String>,
) -> f32 {
    let mut d = difficulty;
    let cloud_floor = routing.theta_cloud + f32::EPSILON;
    let edge_ceiling = routing.theta_cloud - f32::EPSILON;

    if signals.user_rejects_answer {
        let next = d.max(cloud_floor);
        if next > d {
            reason_codes.push(format!("DIFF_D:CALIB_USER_REJECT:{:+.4}", next - d));
        }
        d = next;
    }
    if signals.intent_cloud
        && cloud_configured(config)
        && matches!(step_kind, StepKind::DirectChat | StepKind::HeartbeatAck)
    {
        let next = d.max(cloud_floor);
        if next > d {
            reason_codes.push(format!("DIFF_D:CALIB_CLOUD_INTENT:{:+.4}", next - d));
        }
        d = next;
    }
    if multimodal_cloud_hint {
        let next = d.max(cloud_floor);
        if next > d {
            reason_codes.push(format!("DIFF_D:CALIB_MULTIMODAL_CACHE:{:+.4}", next - d));
        }
        d = next;
    }
    if signals.intent_long_gen
        && !signals.intent_cloud
        && super::keywords::edge_tps_is_low(edge_tps)
    {
        let next = d.max(cloud_floor);
        if next > d {
            reason_codes.push(format!("DIFF_D:CALIB_LONG_GEN_TPS:{:+.4}", next - d));
        }
        d = next;
    }
    if step_kind == StepKind::MemoryCompact
        && !super::gates::ctx_overflow_triggers(
            signals,
            step_kind,
            config.ctx_edge_max_tokens,
        )
    {
        let next = d.min(edge_ceiling);
        if next < d {
            reason_codes.push(format!("DIFF_D:CALIB_MEMORY_COMPACT:{:+.4}", next - d));
        }
        d = next;
    }
    if signals.intent_edge
        && !signals.intent_cloud
        && !signals.intent_long_gen
        && edge_configured(config)
        && matches!(step_kind, StepKind::DirectChat | StepKind::HeartbeatAck)
    {
        let ceiling = routing.theta_edge - f32::EPSILON;
        let next = d.min(ceiling);
        if next < d {
            reason_codes.push(format!("DIFF_D:CALIB_EDGE_INTENT:{:+.4}", next - d));
        }
        d = next;
    }
    d.clamp(0.0, 1.0)
}

fn apply_classifier(
    classifier: Option<&ClassifierStore>,
    features: &FeatureVector,
    d_heuristic: f32,
    reason_codes: &mut Vec<String>,
) -> (f32, Option<f32>) {
    let Some(clf) = classifier else {
        return (d_heuristic, None);
    };
    let pred = clf.predict_and_fuse(features, d_heuristic);
    if pred.bayes_weight > f32::EPSILON {
        if let Some(p) = pred.edge_ok_probability {
            reason_codes.push(format!("BAYES_P({p:.2})"));
            if !pred.warmed_up {
                reason_codes.push("BAYES_COLD_START".to_string());
            }
            let d_bayes = 1.0 - p;
            let delta = pred.difficulty - d_heuristic;
            reason_codes.push(format!(
                "DIFF_FUSE:heur={:.4}|bayes={:.4}|w={:.2}|final={:.4}",
                d_heuristic, d_bayes, pred.bayes_weight, pred.difficulty
            ));
            if delta.abs() > f32::EPSILON {
                reason_codes.push(format!("DIFF_D:BAYES_FUSE:{delta:+.4}"));
            }
        }
    }
    (pred.difficulty, pred.edge_ok_probability)
}

fn cloud_input_saved(route: RouteTier, signals: &super::signals::RequestSignals) -> u32 {
    match route {
        RouteTier::Edge => signals.tok_total_in,
        RouteTier::Cascade => signals.tok_total_in / 2,
        RouteTier::Cloud => 0,
    }
}

fn tier_name(t: RouteTier) -> &'static str {
    match t {
        RouteTier::Edge => "EDGE",
        RouteTier::Cloud => "CLOUD",
        RouteTier::Cascade => "CASCADE",
    }
}

fn finish(
    route: RouteTier,
    profile: Profile,
    mode: RoutingMode,
    step_kind: StepKind,
    signals: &super::signals::RequestSignals,
    mut reason_codes: Vec<String>,
    cloud_input_saved: u32,
    difficulty: f32,
    conversation_key: String,
    force_cloud_sticky: bool,
    multimodal_strategy: MultimodalStrategy,
    work_strategy: WorkStrategy,
    edge_ok_probability: Option<f32>,
    classifier_features: Option<FeatureVector>,
    last_user_text: &str,
    config: &AppConfig,
) -> RouteDecision {
    // Fixed `route=edge` is strict: never escalate to cloud (even on quality fail).
    // Quality fallback only applies in auto mode when the decision landed on edge.
    let casual_quality_fallback = matches!(step_kind, StepKind::DirectChat | StepKind::HeartbeatAck)
        && route == RouteTier::Edge
        && config.fixed_route.is_none()
        && edge_configured(config)
        && cloud_configured(config);
    if casual_quality_fallback {
        reason_codes.push("CASUAL_EDGE_FALLBACK".to_string());
    }
    RouteDecision {
        route,
        profile,
        mode,
        step_kind,
        difficulty,
        reason_codes,
        tokens_in_estimate: signals.tok_total_in,
        tokens_out_estimate: signals.tok_out_estimate,
        cloud_input_saved_estimate: cloud_input_saved,
        conversation_key,
        assistant_failed_recent: signals.assistant_failed_recent,
        consecutive_tool_error_streak: signals.consecutive_tool_error_streak,
        multimodal_strategy,
        work_strategy,
        force_cloud_sticky,
        edge_ok_probability,
        classifier_features,
        casual_quality_fallback,
        lexical_learn: super::LexicalLearnContext {
            last_user_text: last_user_text.to_string(),
            intent_easy: signals.intent_easy,
            rare_lexical: signals.rare_lexical,
            special_lexical: signals.special_lexical,
        },
        routing_log_id: None,
    }
}

fn push_risky_tool_hard_reason(signals: &RequestSignals, reason_codes: &mut Vec<String>) {
    if signals.risky_tool_hard_names.is_empty() {
        return;
    }
    reason_codes.push(format!(
        "GATE_RISKY_TOOL:{}",
        signals.risky_tool_hard_names.join(",")
    ));
}

fn push_cognitive_task_reasons(signals: &RequestSignals, reason_codes: &mut Vec<String>) {
    if !super::signals::cognitive_task_applies(signals) {
        return;
    }
    if signals.intent_plan {
        reason_codes.push("PLAN_INTENT".to_string());
    }
    if signals.intent_analysis {
        reason_codes.push("ANALYSIS_INTENT".to_string());
    }
    if signals.intent_decision {
        reason_codes.push("DECISION_INTENT".to_string());
    }
    if signals.intent_research {
        reason_codes.push("RESEARCH_INTENT".to_string());
    }
}

fn push_risky_tool_soft_reason(signals: &RequestSignals, reason_codes: &mut Vec<String>) {
    if !signals.risky_tool_soft || signals.risky_tool_soft_names.is_empty() {
        return;
    }
    reason_codes.push(format!(
        "RISKY_TOOL_SOFT:{}",
        signals.risky_tool_soft_names.join(",")
    ));
}

fn step_kind_code(k: StepKind) -> &'static str {
    match k {
        StepKind::HeartbeatAck => "HEARTBEAT_ACK",
        StepKind::DirectChat => "DIRECT_CHAT",
        StepKind::RecoveryAfterFailure => "RECOVERY_AFTER_FAILURE",
        StepKind::ToolSelect => "TOOL_SELECT",
        StepKind::ToolArgFill => "TOOL_ARG_FILL",
        StepKind::ToolResultDigest => "TOOL_RESULT_DIGEST",
        StepKind::InitialPlan => "INITIAL_PLAN",
        StepKind::FinalReply => "FINAL_REPLY",
        StepKind::SubagentSpawn => "SUBAGENT_SPAWN",
        StepKind::MemoryCompact => "MEMORY_COMPACT",
        StepKind::CronBackground => "CRON_BACKGROUND",
    }
}
