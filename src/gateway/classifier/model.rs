use super::data::{ClassifierData, Label, LabelCounts};
use super::features::FeatureVector;
use crate::gateway::routing::{RequestSignals, StepKind};

const DEFAULT_PRIOR_EDGE: u64 = 80;
const DEFAULT_PRIOR_CLOUD: u64 = 20;

/// Predict P(edge_ok | features) using log-space naive Bayes.
pub fn predict(data: &ClassifierData, features: &FeatureVector, alpha: f32) -> f32 {
    let alpha = alpha.max(f32::EPSILON) as f64;
    let log_edge = log_class_prior(data, Label::EdgeOk, alpha);
    let log_cloud = log_class_prior(data, Label::CloudNeeded, alpha);

    let mut log_edge_sum = log_edge;
    let mut log_cloud_sum = log_cloud;
    for key in &features.keys {
        log_edge_sum += log_feature(data, key, Label::EdgeOk, alpha);
        log_cloud_sum += log_feature(data, key, Label::CloudNeeded, alpha);
    }

    softmax_edge_prob(log_edge_sum, log_cloud_sum)
}

pub fn fuse_difficulty(
    d_heuristic: f32,
    p_edge_ok: f32,
    total_samples: u64,
    min_samples: u64,
) -> (f32, f32) {
    let w = if min_samples == 0 {
        1.0
    } else {
        (total_samples as f32 / min_samples as f32).min(1.0)
    };
    let d_bayes = 1.0 - p_edge_ok;
    let d_final = (1.0 - w) * d_heuristic + w * d_bayes;
    (d_final.clamp(0.0, 1.0), w)
}

/// Seed pseudo-counts from heuristic difficulty weights when the store is empty.
pub fn seed_heuristic_priors(data: &mut ClassifierData) {
    if data.prior.total() > 0 || !data.features.is_empty() {
        return;
    }

    data.prior.edge_ok = DEFAULT_PRIOR_EDGE;
    data.prior.cloud_needed = DEFAULT_PRIOR_CLOUD;

    for step_kind in all_step_kinds() {
        let bias = step_kind.bias();
        let (edge_w, cloud_w) = bias_to_weights(bias);
        let key = format!("step_kind:{}", step_kind_feature_key(step_kind));
        data.features.insert(
            key,
            LabelCounts {
                edge_ok: (edge_w * 10.0) as u64,
                cloud_needed: (cloud_w * 10.0) as u64,
            },
        );
    }

    seed_feature_prior(data, "intent:hard", 0.25, 0.75);
    seed_feature_prior(data, "intent:easy", 0.85, 0.15);
    seed_feature_prior(data, "intent:plan", 0.20, 0.80);
    seed_feature_prior(data, "ctx_bucket:high", 0.25, 0.75);
    seed_feature_prior(data, "ctx_bucket:mid", 0.55, 0.45);
    seed_feature_prior(data, "ctx_bucket:low", 0.80, 0.20);
    seed_feature_prior(data, "flag:multimodal", 0.45, 0.55);
    seed_feature_prior(data, "flag:risky_tool_hard", 0.10, 0.90);
    seed_feature_prior(data, "flag:risky_tool_soft", 0.35, 0.65);
    seed_feature_prior(data, "flag:assistant_failed_recent", 0.10, 0.90);

    data.meta.total_updates = 0;
}

fn seed_feature_prior(data: &mut ClassifierData, key: &str, edge_frac: f32, cloud_frac: f32) {
    let scale = 10u64;
    data.features.insert(
        key.to_string(),
        LabelCounts {
            edge_ok: (edge_frac * scale as f32) as u64,
            cloud_needed: (cloud_frac * scale as f32) as u64,
        },
    );
}

fn bias_to_weights(bias: f32) -> (f32, f32) {
    let cloud = sigmoid(bias);
    let edge = 1.0 - cloud;
    (edge, cloud)
}

fn sigmoid(x: f32) -> f32 {
    1.0 / (1.0 + (-x).exp())
}

fn log_class_prior(data: &ClassifierData, label: Label, alpha: f64) -> f64 {
    let (count, total) = match label {
        Label::EdgeOk => (data.prior.edge_ok as f64, data.prior.total() as f64),
        Label::CloudNeeded => (data.prior.cloud_needed as f64, data.prior.total() as f64),
    };
    let classes = 2.0;
    ((count + alpha) / (total + alpha * classes)).ln()
}

fn log_feature(data: &ClassifierData, key: &str, label: Label, alpha: f64) -> f64 {
    let entry = data.features.get(key);
    let (feat_count, class_total) = match label {
        Label::EdgeOk => (
            entry.map(|e| e.edge_ok as f64).unwrap_or(0.0),
            data.prior.edge_ok as f64,
        ),
        Label::CloudNeeded => (
            entry.map(|e| e.cloud_needed as f64).unwrap_or(0.0),
            data.prior.cloud_needed as f64,
        ),
    };
    let vocab = 2.0;
    ((feat_count + alpha) / (class_total + alpha * vocab)).ln()
}

fn softmax_edge_prob(log_edge: f64, log_cloud: f64) -> f32 {
    let max = log_edge.max(log_cloud);
    let e = (log_edge - max).exp();
    let c = (log_cloud - max).exp();
    (e / (e + c)) as f32
}

fn all_step_kinds() -> [StepKind; 11] {
    [
        StepKind::HeartbeatAck,
        StepKind::DirectChat,
        StepKind::RecoveryAfterFailure,
        StepKind::ToolSelect,
        StepKind::ToolArgFill,
        StepKind::ToolResultDigest,
        StepKind::InitialPlan,
        StepKind::FinalReply,
        StepKind::SubagentSpawn,
        StepKind::MemoryCompact,
        StepKind::CronBackground,
    ]
}

fn step_kind_feature_key(k: StepKind) -> &'static str {
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

pub fn label_from_outcome(
    outcome: crate::gateway::experience::RequestOutcome,
    tool_error_streak: u32,
) -> Option<Label> {
    if tool_error_streak > 0 {
        return Some(Label::CloudNeeded);
    }
    if outcome.edge_ok && !outcome.cascade_fallback && !outcome.upstream_error {
        Some(Label::EdgeOk)
    } else if outcome.cascade_fallback || outcome.upstream_error {
        Some(Label::CloudNeeded)
    } else {
        None
    }
}

pub fn should_record_outcome(
    outcome: crate::gateway::experience::RequestOutcome,
    route: crate::gateway::routing::RouteTier,
    work_verify: bool,
) -> bool {
    if outcome.edge_ok || outcome.cascade_fallback {
        return true;
    }
    if outcome.upstream_error {
        return matches!(
            route,
            crate::gateway::routing::RouteTier::Edge | crate::gateway::routing::RouteTier::Cascade
        ) || work_verify;
    }
    false
}

#[allow(dead_code)]
pub fn heuristic_difficulty(
    signals: &RequestSignals,
    step_kind: StepKind,
    ctx_edge_max: u32,
    experience_bias: f32,
) -> f32 {
    crate::gateway::routing::DifficultyScore::compute(
        signals,
        step_kind,
        ctx_edge_max,
        experience_bias,
    )
    .0
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gateway::experience::RequestOutcome;
    use crate::gateway::routing::{RouteTier, StepKind};

    #[test]
    fn predict_favors_edge_for_direct_chat_prior() {
        let mut data = ClassifierData::default();
        seed_heuristic_priors(&mut data);
        let features = FeatureVector {
            keys: vec!["step_kind:direct_chat".into()],
        };
        let p = predict(&data, &features, 1.0);
        assert!(p > 0.5, "direct chat should favor edge: {p}");
    }

    #[test]
    fn predict_favors_cloud_for_hard_intent() {
        let mut data = ClassifierData::default();
        seed_heuristic_priors(&mut data);
        let features = FeatureVector {
            keys: vec!["intent:hard".into()],
        };
        let p = predict(&data, &features, 1.0);
        assert!(p < 0.5, "hard intent should favor cloud: {p}");
    }

    #[test]
    fn fuse_transitions_with_sample_count() {
        let (d, w) = fuse_difficulty(0.2, 0.9, 50, 100);
        assert!((w - 0.5).abs() < f32::EPSILON);
        assert!((d - 0.15).abs() < 0.01);
        let (d_full, w_full) = fuse_difficulty(0.2, 0.9, 200, 100);
        assert!((w_full - 1.0).abs() < f32::EPSILON);
        assert!((d_full - 0.1).abs() < f32::EPSILON);
    }

    #[test]
    fn label_from_outcome_mapping() {
        assert_eq!(
            label_from_outcome(RequestOutcome {
                edge_ok: true,
                cascade_fallback: false,
                upstream_error: false,
            }, 0),
            Some(Label::EdgeOk)
        );
        assert_eq!(
            label_from_outcome(RequestOutcome {
                edge_ok: false,
                cascade_fallback: true,
                upstream_error: false,
            }, 0),
            Some(Label::CloudNeeded)
        );
        assert_eq!(
            label_from_outcome(RequestOutcome::default(), 0),
            None
        );
        assert_eq!(
            label_from_outcome(RequestOutcome {
                edge_ok: true,
                cascade_fallback: false,
                upstream_error: false,
            }, 1),
            Some(Label::CloudNeeded)
        );
    }

    #[test]
    fn should_record_only_when_edge_attempted() {
        assert!(should_record_outcome(
            RequestOutcome::success(
                &crate::gateway::routing::RouteDecision {
                    route: RouteTier::Cascade,
                    profile: crate::gateway::routing::Profile::Balanced,
                    mode: crate::gateway::routing::RoutingMode::Cascade,
                    step_kind: StepKind::DirectChat,
                    difficulty: 0.0,
                    reason_codes: vec![],
                    tokens_in_estimate: 0,
                    tokens_out_estimate: 0,
                    cloud_input_saved_estimate: 0,
                    conversation_key: String::new(),
                    assistant_failed_recent: false,
                    consecutive_tool_error_streak: 0,
                    multimodal_strategy: crate::gateway::multimodal::MultimodalStrategy::None,
                    work_strategy: crate::gateway::routing::WorkStrategy::None,
                    force_cloud_sticky: false,
                    edge_ok_probability: None,
                    classifier_features: None,
                    casual_quality_fallback: false,
            lexical_learn: Default::default(),
                    routing_log_id: None,
                },
                true
            ),
            RouteTier::Cascade,
            false
        ));
        assert!(!should_record_outcome(
            RequestOutcome::default(),
            RouteTier::Cloud,
            false
        ));
    }
}
