use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use crate::gateway::config::AppConfig;
use crate::gateway::experience::ExperienceStore;

use super::decision::RouteTier;
use super::signals::RequestSignals;
use super::step_kind::StepKind;
use super::upstream_availability::{cloud_configured, edge_configured};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum WorkStrategy {
    #[default]
    None,
    /// Experience shows edge handles this step_kind reliably.
    CachedEdge,
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

pub fn apply_work_route(
    route: RouteTier,
    step_kind: StepKind,
    signals: &RequestSignals,
    config: &AppConfig,
    experience: Option<&ExperienceStore>,
    conv_key: &str,
    tokens_in: u32,
    work_verify_sample_rate: f32,
    reason_codes: &mut Vec<String>,
) -> (RouteTier, WorkStrategy) {
    if !cloud_configured(config) {
        return (route, WorkStrategy::None);
    }

    if is_plan_step(step_kind, signals) {
        reason_codes.push(if signals.intent_plan {
            "PLAN_INTENT_CLOUD".to_string()
        } else {
            "INITIAL_PLAN_CLOUD".to_string()
        });
        return (RouteTier::Cloud, WorkStrategy::None);
    }

    if !is_work_step(step_kind) || !edge_configured(config) {
        return (route, WorkStrategy::None);
    }

    reason_codes.push("WORK_EXEC_EDGE".to_string());

    if experience.is_some_and(|exp| exp.edge_trusted(step_kind)) {
        reason_codes.push("WORK_CACHE_EDGE".to_string());
        return (RouteTier::Edge, WorkStrategy::CachedEdge);
    }

    if should_work_verify_sample(
        conv_key,
        step_kind,
        tokens_in,
        work_verify_sample_rate,
    ) {
        reason_codes.push(format!(
            "WORK_VERIFY_SAMPLE(p={work_verify_sample_rate:.2})"
        ));
        return (RouteTier::Cascade, WorkStrategy::Verify);
    }

    reason_codes.push(format!(
        "WORK_SAMPLE_SKIP(p={work_verify_sample_rate:.2})"
    ));
    (RouteTier::Edge, WorkStrategy::None)
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
        AppConfig::from_file(file, std::path::PathBuf::from("/tmp/flowy-test-config.toml"))
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
            tok_total_in: 512,
            tok_loop_delta: 0,
            tok_out_estimate: 0,
            n_tool_defs: 1,
            n_turns: 1,
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
            risky_tool_tier1: false,
            intent_hard: false,
            intent_easy: false,
            intent_plan: false,
            multimodal: false,
            consecutive_tool_error_streak: 0,
        }
    }

    #[test]
    fn apply_work_route_skips_verify_at_zero_rate() {
        let cfg = app_config(0.0);
        let mut codes = Vec::new();
        let signals = empty_signals();
        let (route, strategy) = apply_work_route(
            RouteTier::Cloud,
            StepKind::ToolSelect,
            &signals,
            &cfg,
            None,
            "conv:sample",
            512,
            0.0,
            &mut codes,
        );
        assert_eq!(route, RouteTier::Edge);
        assert_eq!(strategy, WorkStrategy::None);
        assert!(codes.iter().any(|c| c.starts_with("WORK_SAMPLE_SKIP")));
    }

    #[test]
    fn plan_intent_forces_cloud() {
        let cfg = app_config(0.0);
        let mut signals = empty_signals();
        signals.intent_plan = true;
        signals.loop_steps = 0;
        signals.had_tool_roundtrip = false;
        let mut codes = Vec::new();
        let (route, _) = apply_work_route(
            RouteTier::Edge,
            StepKind::ToolSelect,
            &signals,
            &cfg,
            None,
            "conv:plan",
            512,
            0.0,
            &mut codes,
        );
        assert_eq!(route, RouteTier::Cloud);
        assert!(codes.iter().any(|c| c == "PLAN_INTENT_CLOUD"));
    }

    #[test]
    fn apply_work_route_verifies_at_full_rate() {
        let cfg = app_config(1.0);
        let mut codes = Vec::new();
        let signals = empty_signals();
        let (route, strategy) = apply_work_route(
            RouteTier::Cloud,
            StepKind::ToolSelect,
            &signals,
            &cfg,
            None,
            "conv:sample",
            512,
            1.0,
            &mut codes,
        );
        assert_eq!(strategy, WorkStrategy::Verify);
        assert!(codes.iter().any(|c| c.starts_with("WORK_VERIFY_SAMPLE")));
    }
}
