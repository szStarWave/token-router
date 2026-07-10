use super::signals::RequestSignals;
use super::step_kind::StepKind;

#[derive(Debug, Clone, Default)]
pub struct GateBiasResult {
    pub linear_delta: f32,
    pub parts: Vec<(String, f32)>,
    pub reason_codes: Vec<String>,
}

/// Former hard gates — now contribute linear difficulty bias + observability reason codes.
pub fn collect_gate_biases(
    signals: &RequestSignals,
    step_kind: StepKind,
    ctx_edge_max: u32,
    edge_available: bool,
) -> GateBiasResult {
    let mut linear_delta = 0.0f32;
    let mut reason_codes = Vec::new();

    let mut parts = Vec::new();

    if !edge_available {
        reason_codes.push("GATE_EDGE_DOWN".to_string());
        return GateBiasResult {
            linear_delta,
            parts,
            reason_codes,
        };
    }

    if signals.user_rejects_answer {
        reason_codes.push("GATE_USER_REJECT".to_string());
        linear_delta += 0.55;
        parts.push(("GATE_USER_REJECT".to_string(), 0.55));
    }

    if step_kind == StepKind::MemoryCompact
        && ctx_overflow_triggers(signals, step_kind, ctx_edge_max)
    {
        reason_codes.push("GATE_OPENCLAW_COMPACT".to_string());
        linear_delta += 0.45;
        parts.push(("GATE_OPENCLAW_COMPACT".to_string(), 0.45));
    } else if step_kind != StepKind::MemoryCompact
        && ctx_overflow_triggers(signals, step_kind, ctx_edge_max)
    {
        reason_codes.push("GATE_CTX_OVERFLOW".to_string());
        let delta = match step_kind {
            StepKind::DirectChat | StepKind::HeartbeatAck => 0.35,
            _ => 0.50,
        };
        linear_delta += delta;
        parts.push(("GATE_CTX_OVERFLOW".to_string(), delta));
    }

    if signals.assistant_failed_recent && step_kind != StepKind::HeartbeatAck {
        reason_codes.push("GATE_ASSISTANT_FAILURE".to_string());
        linear_delta += 0.20;
        parts.push(("GATE_ASSISTANT_FAILURE".to_string(), 0.20));
    }

    if signals.risky_tool_hard
        && matches!(step_kind, StepKind::ToolSelect | StepKind::ToolArgFill)
        && !signals.last_role_tool
    {
        reason_codes.push("GATE_RISKY_TOOL".to_string());
        linear_delta += 0.30;
        parts.push(("GATE_RISKY_TOOL".to_string(), 0.30));
    }

    GateBiasResult {
        linear_delta,
        parts,
        reason_codes,
    }
}

/// Token budget for overflow gate. Casual turns ignore static OpenClaw system + tool schema.
pub(crate) fn ctx_overflow_tokens(signals: &RequestSignals, step_kind: StepKind) -> u32 {
    match step_kind {
        StepKind::DirectChat | StepKind::HeartbeatAck => signals.tok_rest,
        _ => signals.tok_total_in,
    }
}

/// Returns true when estimated prompt tokens exceed ~80% of the configured edge context budget.
pub(crate) fn ctx_overflow_triggers(
    signals: &RequestSignals,
    step_kind: StepKind,
    ctx_edge_max: u32,
) -> bool {
    let threshold = (ctx_edge_max as f64 * 0.8) as u32;
    ctx_overflow_tokens(signals, step_kind) > threshold
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gateway::routing::signals::RequestSignals;

    fn empty_signals() -> RequestSignals {
        RequestSignals {
            tok_system: 0,
            tok_tools_schema: 0,
            tok_rest: 100,
            tok_total_in: 100,
            tok_loop_delta: 0,
            tok_out_estimate: 0,
            n_tool_defs: 0,
            n_turns: 1,
            last_user_tok: 20,
            loop_steps: 0,
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
            tools_enabled: false,
            had_tool_roundtrip: false,
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
    fn tool_error_streak_has_no_escalate_gate() {
        let mut signals = empty_signals();
        signals.consecutive_tool_error_streak = 2;
        let result = collect_gate_biases(&signals, StepKind::ToolResultDigest, 65536, true);
        assert!(!result.reason_codes.iter().any(|c| c == "GATE_TOOL_ERROR_STREAK"));
        assert!(result.linear_delta < 0.50);
    }

    #[test]
    fn single_tool_error_does_not_add_escalate_gate() {
        let mut signals = empty_signals();
        signals.consecutive_tool_error_streak = 1;
        let result = collect_gate_biases(&signals, StepKind::RecoveryAfterFailure, 65536, true);
        assert!(!result.reason_codes.iter().any(|c| c == "GATE_TOOL_ERROR_STREAK"));
    }

    #[test]
    fn ctx_overflow_direct_chat_ignores_system_and_tool_schema() {
        let mut signals = empty_signals();
        signals.tok_system = 50_000;
        signals.tok_tools_schema = 10_000;
        signals.tok_rest = 100;
        signals.tok_total_in = 60_100;
        let result = collect_gate_biases(&signals, StepKind::DirectChat, 55_000, true);
        assert!(!result.reason_codes.iter().any(|c| c == "GATE_CTX_OVERFLOW"));
    }

    #[test]
    fn ctx_overflow_direct_chat_when_transcript_large() {
        let mut signals = empty_signals();
        signals.tok_rest = 60_000;
        signals.tok_total_in = 120_000;
        let result = collect_gate_biases(&signals, StepKind::DirectChat, 55_000, true);
        assert!(result.reason_codes.iter().any(|c| c == "GATE_CTX_OVERFLOW"));
        assert!(result.linear_delta >= 0.35);
    }

    #[test]
    fn ctx_overflow_uses_full_prompt_for_work_steps() {
        let mut signals = empty_signals();
        signals.tok_rest = 100;
        signals.tok_total_in = 60_000;
        let result = collect_gate_biases(&signals, StepKind::ToolSelect, 55_000, true);
        assert!(result.reason_codes.iter().any(|c| c == "GATE_CTX_OVERFLOW"));
        assert!(result.linear_delta >= 0.50);
    }

    #[test]
    fn openclaw_compact_gate_follows_ctx_edge_max() {
        let mut signals = empty_signals();
        signals.tok_total_in = 50_000;
        let ok = collect_gate_biases(&signals, StepKind::MemoryCompact, 262_144, true);
        assert!(!ok.reason_codes.iter().any(|c| c == "GATE_OPENCLAW_COMPACT"));
        let overflow = collect_gate_biases(&signals, StepKind::MemoryCompact, 55_000, true);
        assert!(overflow.reason_codes.iter().any(|c| c == "GATE_OPENCLAW_COMPACT"));
        let direct = collect_gate_biases(&signals, StepKind::DirectChat, 55_000, true);
        assert!(!direct.reason_codes.iter().any(|c| c == "GATE_OPENCLAW_COMPACT"));
    }

    #[test]
    fn risky_tool_hard_bias_on_tool_arg_fill() {
        let mut signals = empty_signals();
        signals.risky_tool_hard = true;
        signals.risky_tool_hard_names = vec!["exec".into()];
        let result = collect_gate_biases(&signals, StepKind::ToolArgFill, 65536, true);
        assert!(result.reason_codes.iter().any(|c| c == "GATE_RISKY_TOOL"));
        assert!(result.linear_delta >= 0.30);
    }

    #[test]
    fn risky_tool_hard_skipped_after_tool_result() {
        let mut signals = empty_signals();
        signals.risky_tool_hard = true;
        signals.last_role_tool = true;
        let result = collect_gate_biases(&signals, StepKind::ToolArgFill, 65536, true);
        assert!(!result.reason_codes.iter().any(|c| c == "GATE_RISKY_TOOL"));
    }

    #[test]
    fn edge_down_emits_reason_without_bias() {
        let result = collect_gate_biases(&empty_signals(), StepKind::ToolArgFill, 65536, false);
        assert!(result.reason_codes.iter().any(|c| c == "GATE_EDGE_DOWN"));
        assert_eq!(result.linear_delta, 0.0);
    }
}
