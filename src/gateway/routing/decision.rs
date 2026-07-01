use std::str::FromStr;

use crate::gateway::api::openai::ChatCompletionRequest;
use crate::gateway::classifier::{ClassifierStore, FeatureVector};
use crate::gateway::config::AppConfig;
use crate::gateway::experience::ExperienceStore;
use crate::gateway::multimodal::{MultimodalStore, MultimodalStrategy};

use super::conversation::conversation_key;
use super::signals::is_simple_multimodal;
use super::difficulty::DifficultyScore;
use super::gates::check_hard_gates;
use super::policy::{self, Profile};
use super::signals::SignalExtractor;
use super::step_kind::{StepKind, resolve_step_kind};
use super::edge_busy::apply_edge_busy_fallback;
use super::upstream_availability::{cloud_configured, edge_configured, finalize_route};
use super::work::{WorkStrategy, apply_work_route, sticky_cascade_applies};
use crate::gateway::edge_load::EdgeInferenceTracker;
use crate::gateway::routing::adaptive::EffectiveRouting;
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
    #[serde(skip)]
    pub multimodal_strategy: crate::gateway::multimodal::MultimodalStrategy,
    #[serde(skip)]
    pub work_strategy: WorkStrategy,
    /// Set when tool-error streak or similar gates recommend cloud stickiness.
    #[serde(skip)]
    pub force_cloud_sticky: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub edge_ok_probability: Option<f32>,
    /// Features captured at decision time for classifier learning.
    #[serde(skip)]
    pub classifier_features: Option<FeatureVector>,
    /// DirectChat/HeartbeatAck: try edge first, cloud on quality gate failure.
    #[serde(skip)]
    pub casual_quality_fallback: bool,
}

pub fn decide(
    config: &AppConfig,
    req: &ChatCompletionRequest,
    sessions: &SessionStore,
    experience: Option<&ExperienceStore>,
    multimodal: Option<&MultimodalStore>,
    routing: &EffectiveRouting,
    edge_load: Option<&EdgeInferenceTracker>,
    classifier: Option<&ClassifierStore>,
) -> RouteDecision {
    let profile = config.default_profile;
    let mode = config.routing_mode;
    let edge_ok = edge_configured(config);

    let conv_key = conversation_key(req);
    let prev_tok = sessions.get_last_tok_in(&conv_key);
    let sticky_until = sessions.cloud_sticky_until(&conv_key);
    let extractor = SignalExtractor {
        ctx_edge_max: config.ctx_edge_max_tokens,
    };
    let signals = extractor.extract(req, prev_tok);
    let step_kind = resolve_step_kind(req, &signals);
    let features = FeatureVector::from_signals(&signals, step_kind, config.ctx_edge_max_tokens);

    let mut reason_codes = Vec::new();
    reason_codes.push(format!("STEP_{}", step_kind_code(step_kind)));

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
        let route = apply_casual_no_cascade(route, step_kind, &mut reason_codes);
        let route = finalize_route(route, config, &mut reason_codes);
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
            signals.assistant_failed_recent,
            mm,
            work,
            None,
            Some(features),
            config,
        );
    }

    if let Some(gate) = check_hard_gates(
        &signals,
        step_kind,
        config.ctx_edge_max_tokens,
        edge_ok,
    ) {
        reason_codes.push(gate.code.to_string());
        let mut route = RouteTier::Cloud;
        route = finalize_route(route, config, &mut reason_codes);
        sessions.record_tokens(&conv_key, signals.tok_total_in);
        let exp_bias = experience.map(|e| e.bias_for(step_kind)).unwrap_or(0.0);
        return finish(
            route,
            profile,
            mode,
            step_kind,
            &signals,
            reason_codes,
            0,
            d_score_for_gate(&signals, step_kind, config.ctx_edge_max_tokens, exp_bias),
            conv_key,
            signals.assistant_failed_recent,
            MultimodalStrategy::None,
            WorkStrategy::None,
            None,
            Some(features),
            config,
        );
    }

    let exp_bias = experience.map(|e| e.bias_for(step_kind)).unwrap_or(0.0);
    if exp_bias.abs() > f32::EPSILON {
        reason_codes.push(format!("EXP_BIAS_{exp_bias:+.2}"));
    }
    let d_heuristic = DifficultyScore::compute(
        &signals,
        step_kind,
        config.ctx_edge_max_tokens,
        exp_bias,
    );
    let (difficulty, edge_ok_probability) = apply_classifier(
        classifier,
        &features,
        d_heuristic.0,
        &mut reason_codes,
    );
    let d = DifficultyScore(difficulty);
    let route = policy::map_policy_with_thresholds(
        d,
        step_kind,
        profile,
        mode,
        routing.theta_edge,
        routing.theta_cloud,
    );

    if routing.enabled {
        reason_codes.push(format!(
            "ADAPTIVE_VERIFY(p={:.2})",
            routing.work_verify_sample_rate
        ));
        reason_codes.push(format!(
            "ADAPTIVE_THETA({:.2},{:.2})",
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

    let (route, work_strategy) = apply_work_route(
        route,
        step_kind,
        &signals,
        config,
        experience,
        &conv_key,
        signals.tok_total_in,
        routing.work_verify_sample_rate,
        &mut reason_codes,
    );

    let route = apply_sticky_cascade(route, step_kind, sticky_until, config, &mut reason_codes);

    let (route, multimodal_strategy) = apply_multimodal_route(
        route,
        &signals,
        step_kind,
        config,
        req,
        multimodal,
        &mut reason_codes,
    );
    let route = apply_casual_no_cascade(route, step_kind, &mut reason_codes);
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
        signals.assistant_failed_recent,
        multimodal_strategy,
        work_strategy,
        edge_ok_probability,
        Some(features),
        config,
    )
}

fn apply_casual_no_cascade(
    route: RouteTier,
    step_kind: StepKind,
    reason_codes: &mut Vec<String>,
) -> RouteTier {
    if !matches!(step_kind, StepKind::DirectChat | StepKind::HeartbeatAck) {
        return route;
    }
    if route != RouteTier::Cascade {
        return route;
    }
    reason_codes.push("CASUAL_PREFER_EDGE".to_string());
    RouteTier::Edge
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
        }
    }
    (pred.difficulty, pred.edge_ok_probability)
}

fn apply_sticky_cascade(
    route: RouteTier,
    step_kind: StepKind,
    sticky_until: Option<u64>,
    config: &AppConfig,
    reason_codes: &mut Vec<String>,
) -> RouteTier {
    if sticky_until.is_none() || !sticky_cascade_applies(step_kind) {
        return route;
    }
    if !edge_configured(config) || !cloud_configured(config) {
        return route;
    }
    reason_codes.push("STICKY_CASCADE_RETRY".to_string());
    RouteTier::Cascade
}

/// Simple multimodal (DirectChat with images) probes edge capability; complex multimodal
/// (tools, long context, agent loops) always uses cloud when available.
fn apply_multimodal_route(
    route: RouteTier,
    signals: &super::signals::RequestSignals,
    step_kind: StepKind,
    config: &AppConfig,
    req: &ChatCompletionRequest,
    multimodal: Option<&MultimodalStore>,
    reason_codes: &mut Vec<String>,
) -> (RouteTier, MultimodalStrategy) {
    if !signals.multimodal {
        return (route, MultimodalStrategy::None);
    }
    if !edge_configured(config) || !cloud_configured(config) {
        return (route, MultimodalStrategy::None);
    }

    if step_kind != StepKind::DirectChat {
        reason_codes.push("MULTIMODAL_COMPLEX_CLOUD".to_string());
        return (RouteTier::Cloud, MultimodalStrategy::None);
    }

    let strategy = match multimodal {
        Some(store) => MultimodalStrategy::from(store.route_hint(config, &req.model)),
        None => MultimodalStrategy::Probe,
    };

    let route = match strategy {
        MultimodalStrategy::None => route,
        MultimodalStrategy::CachedEdge | MultimodalStrategy::CachedEdgeFallback => {
            reason_codes.push("MULTIMODAL_CACHE_EDGE".to_string());
            RouteTier::Edge
        }
        MultimodalStrategy::CachedCloud => {
            reason_codes.push("MULTIMODAL_CACHE_CLOUD".to_string());
            RouteTier::Cloud
        }
        MultimodalStrategy::Probe if is_simple_multimodal(signals) => {
            reason_codes.push("MULTIMODAL_SIMPLE_EDGE".to_string());
            RouteTier::Edge
        }
        MultimodalStrategy::Probe => {
            reason_codes.push("MULTIMODAL_PROBE_EDGE".to_string());
            RouteTier::Edge
        }
    };

    (route, strategy)
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

fn d_score_for_gate(
    signals: &super::signals::RequestSignals,
    step_kind: StepKind,
    ctx_edge_max: u32,
    experience_bias: f32,
) -> f32 {
    DifficultyScore::compute(signals, step_kind, ctx_edge_max, experience_bias).0
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
    assistant_failed_recent: bool,
    multimodal_strategy: MultimodalStrategy,
    work_strategy: WorkStrategy,
    edge_ok_probability: Option<f32>,
    classifier_features: Option<FeatureVector>,
    config: &AppConfig,
) -> RouteDecision {
    let force_cloud_sticky = reason_codes
        .iter()
        .any(|c| c == "GATE_TOOL_ERROR_STREAK");
    let casual_quality_fallback = matches!(step_kind, StepKind::DirectChat | StepKind::HeartbeatAck)
        && route == RouteTier::Edge
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
        assistant_failed_recent,
        multimodal_strategy,
        work_strategy,
        force_cloud_sticky,
        edge_ok_probability,
        classifier_features,
        casual_quality_fallback,
    }
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
