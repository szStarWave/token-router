use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use crate::gateway::config::AppConfig;

use super::decision::RouteTier;
use super::signals::RequestSignals;
use super::step_kind::StepKind;
use super::upstream_availability::{cloud_configured, edge_configured};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum WorkStrategy {
    #[default]
    None,
    /// Edge first, cloud validates; outcomes feed experience.
    Verify,
}

/// Planning tasks → cloud; execution steps (tool digest/select) may use edge.
pub fn is_plan_step(step_kind: StepKind, signals: &RequestSignals) -> bool {
    if step_kind == StepKind::InitialPlan {
        return true;
    }
    signals.intent_plan
        && !signals.last_role_tool
        && !signals.pending_tool_calls
        && matches!(
            step_kind,
            StepKind::ToolSelect | StepKind::ToolArgFill | StepKind::FinalReply
        )
}

pub fn is_work_step(step_kind: StepKind) -> bool {
    matches!(
        step_kind,
        StepKind::ToolSelect
            | StepKind::ToolArgFill
            | StepKind::ToolResultDigest
            | StepKind::FinalReply
            | StepKind::SubagentSpawn
            | StepKind::MemoryCompact
            | StepKind::CronBackground
    )
}

/// Work execution steps that retry edge via Cascade while cloud sticky is active.
pub fn sticky_cascade_applies(step_kind: StepKind) -> bool {
    is_work_step(step_kind)
}

/// Deterministic per-request sampling from conversation key + step + token estimate.
pub fn should_work_verify_sample(
    conv_key: &str,
    step_kind: StepKind,
    tokens_in: u32,
    rate: f32,
) -> bool {
    let rate = rate.clamp(0.0, 1.0);
    if rate >= 1.0 {
        return true;
    }
    if rate <= 0.0 {
        return false;
    }
    let mut h = DefaultHasher::new();
    conv_key.hash(&mut h);
    format!("{step_kind:?}").hash(&mut h);
    tokens_in.hash(&mut h);
    let bucket = (h.finish() % 10_000) as f32 / 10_000.0;
    bucket < rate
}

/// Attach work verify metadata without overriding the policy route.
pub fn attach_work_verify(
    route: RouteTier,
    step_kind: StepKind,
    signals: &RequestSignals,
    config: &AppConfig,
    conv_key: &str,
    tokens_in: u32,
    work_verify_sample_rate: f32,
    reason_codes: &mut Vec<String>,
) -> WorkStrategy {
    if !cloud_configured(config) {
        return WorkStrategy::None;
    }

    if is_plan_step(step_kind, signals) && super::signals::cognitive_task_applies(signals) {
        reason_codes.push(if signals.intent_plan {
            "PLAN_INTENT".to_string()
        } else {
            "INITIAL_PLAN".to_string()
        });
    }

    if !is_work_step(step_kind) || !edge_configured(config) {
        return WorkStrategy::None;
    }

    reason_codes.push("WORK_EXEC_EDGE".to_string());

    if route == RouteTier::Cascade
        && should_work_verify_sample(conv_key, step_kind, tokens_in, work_verify_sample_rate)
    {
        reason_codes.push(format!(
            "WORK_VERIFY_SAMPLE(p={work_verify_sample_rate:.2})"
        ));
        return WorkStrategy::Verify;
    }

    if route == RouteTier::Cascade && work_verify_sample_rate > 0.0 {
        reason_codes.push(format!(
            "WORK_SAMPLE_SKIP(p={work_verify_sample_rate:.2})"
        ));
    }

    WorkStrategy::None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{ConfigFile, UpstreamEndpoint};
    use crate::gateway::config::AppConfig;
    use crate::gateway::routing::signals::RequestSignals;

    fn app_config(rate: f32) -> AppConfig {
        let mut file = ConfigFile::default();
        file.gateway.work_verify_sample_rate = rate;
        file.upstream.edge = Some(UpstreamEndpoint {
            base_url: "http://127.0.0.1:11434/v1".into(),
            api_key: None,
            model: None,
        });
        file.upstream.cloud = Some(UpstreamEndpoint {
            base_url: "https://api.example.com/v1".into(),
            api_key: None,
            model: None,
        });
        AppConfig::from_file(file, std::path::PathBuf::from("/tmp/flowy-test"))
            .unwrap()
    }

    #[test]
    fn sample_rate_zero_never_verifies() {
        assert!(!should_work_verify_sample("conv:a", StepKind::ToolSelect, 100, 0.0));
    }

    #[test]
    fn sample_rate_one_always_verifies() {
        assert!(should_work_verify_sample("conv:a", StepKind::ToolSelect, 100, 1.0));
    }

    fn empty_signals() -> RequestSignals {
        RequestSignals {
            tok_system: 0,
            tok_tools_schema: 0,
            tok_rest: 512,
            tok_total_in: 512,
            tok_loop_delta: 0,
            tok_out_estimate: 0,
            n_tool_defs: 1,
            n_turns: 1,
            last_user_tok: 100,
            loop_steps: 1,
            pending_tool_calls: false,
            tool_arg_ready: false,
            last_role_tool: false,
            synthetic_tool_result: false,
            assistant_failed_recent: false,
            is_heartbeat_poll: false,
            voice_repair_loop: false,
            subagent_spawn_hint: false,
            memory_compact_hint: false,
            cron_background: false,
            tools_enabled: true,
            had_tool_roundtrip: true,
            risky_tool_hard: false,
            risky_tool_soft: false,
            risky_tool_names: Vec::new(),
            risky_tool_hard_names: Vec::new(),
            risky_tool_soft_names: Vec::new(),
            intent_hard: false,
            intent_easy: false,
            intent_plan: false,
            intent_cloud: false,
            intent_long_gen: false,
            intent_edge: false,
            multimodal: false,
            user_multimodal: false,
            consecutive_tool_error_streak: 0,
            tool_invocations_since_last_user: 0,
            user_rejects_answer: false,
            rare_lexical: false,
            special_lexical: false,
            rare_token_ratio: 0.0,
            intent_analysis: false,
            intent_decision: false,
            intent_research: false,
        }
    }

    #[test]
    fn attach_work_verify_preserves_cloud_route() {
        let cfg = app_config(0.0);
        let mut codes = Vec::new();
        let signals = empty_signals();
        let strategy = attach_work_verify(
            RouteTier::Cloud,
            StepKind::ToolSelect,
            &signals,
            &cfg,
            "conv:sample",
            512,
            0.0,
            &mut codes,
        );
        assert_eq!(strategy, WorkStrategy::None);
        assert!(codes.iter().any(|c| c == "WORK_EXEC_EDGE"));
        assert!(!codes.iter().any(|c| c.starts_with("WORK_SAMPLE_SKIP")));
    }

    #[test]
    fn plan_intent_emits_reason_without_changing_route() {
        let cfg = app_config(0.0);
        let mut signals = empty_signals();
        signals.intent_plan = true;
        signals.loop_steps = 0;
        signals.had_tool_roundtrip = false;
        let mut codes = Vec::new();
        let _ = attach_work_verify(
            RouteTier::Edge,
            StepKind::ToolSelect,
            &signals,
            &cfg,
            "conv:plan",
            512,
            0.0,
            &mut codes,
        );
        assert!(codes.iter().any(|c| c == "PLAN_INTENT"));
    }

    #[test]
    fn attach_work_verify_on_cascade_at_full_rate() {
        let cfg = app_config(1.0);
        let mut codes = Vec::new();
        let signals = empty_signals();
        let strategy = attach_work_verify(
            RouteTier::Cascade,
            StepKind::ToolSelect,
            &signals,
            &cfg,
            "conv:sample",
            512,
            1.0,
            &mut codes,
        );
        assert_eq!(strategy, WorkStrategy::Verify);
        assert!(codes.iter().any(|c| c.starts_with("WORK_VERIFY_SAMPLE")));
    }

    #[test]
    fn single_tool_error_streak_does_not_force_verify() {
        let cfg = app_config(0.0);
        let mut signals = empty_signals();
        signals.consecutive_tool_error_streak = 1;
        let mut codes = Vec::new();
        let strategy = attach_work_verify(
            RouteTier::Edge,
            StepKind::ToolSelect,
            &signals,
            &cfg,
            "conv:tool-err",
            512,
            0.0,
            &mut codes,
        );
        assert_eq!(strategy, WorkStrategy::None);
        assert!(codes.iter().any(|c| c == "WORK_EXEC_EDGE"));
    }
}
