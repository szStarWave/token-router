use super::signals::RequestSignals;
use super::step_kind::StepKind;

#[derive(Debug, Clone)]
pub struct HardGate {
    pub code: &'static str,
}

pub fn check_hard_gates(
    signals: &RequestSignals,
    step_kind: StepKind,
    ctx_edge_max: u32,
    edge_available: bool,
) -> Option<HardGate> {
    if !edge_available {
        return Some(HardGate {
            code: "GATE_EDGE_DOWN",
        });
    }

    if signals.user_rejects_answer {
        return Some(HardGate {
            code: "GATE_USER_REJECT",
        });
    }

    if step_kind == StepKind::MemoryCompact
        && ctx_overflow_triggers(signals, step_kind, ctx_edge_max)
    {
        return Some(HardGate {
            code: "GATE_OPENCLAW_COMPACT",
        });
    }

    if ctx_overflow_triggers(signals, step_kind, ctx_edge_max) {
        return Some(HardGate {
            code: "GATE_CTX_OVERFLOW",
        });
    }

    if signals.assistant_failed_recent && step_kind != StepKind::HeartbeatAck {
        return Some(HardGate {
            code: "GATE_ASSISTANT_FAILURE",
        });
    }

    if signals.consecutive_tool_error_streak >= 2 {
        return Some(HardGate {
            code: "GATE_TOOL_ERROR_STREAK",
        });
    }

    if signals.risky_tool_tier1 {
        return Some(HardGate {
            code: "GATE_RISKY_TOOL",
        });
    }

    None
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
            risky_tool_tier1: false,
            intent_hard: false,
            intent_easy: false,
            intent_plan: false,
            multimodal: false,
            user_multimodal: false,
            consecutive_tool_error_streak: 0,
            tool_invocations_since_last_user: 0,
            user_rejects_answer: false,
            rare_lexical: false,
            special_lexical: false,
            rare_token_ratio: 0.0,
        }
    }

    #[test]
    fn tool_error_streak_gate_fires() {
        let mut signals = empty_signals();
        signals.consecutive_tool_error_streak = 2;
        let gate = check_hard_gates(
            &signals,
            StepKind::ToolResultDigest,
            65536,
            true,
        );
        assert_eq!(gate.unwrap().code, "GATE_TOOL_ERROR_STREAK");
    }

    #[test]
    fn single_tool_error_does_not_trigger_gate() {
        let mut signals = empty_signals();
        signals.consecutive_tool_error_streak = 1;
        assert!(check_hard_gates(
            &signals,
            StepKind::RecoveryAfterFailure,
            65536,
            true,
        )
        .is_none());
    }

    #[test]
    fn ctx_overflow_direct_chat_ignores_system_and_tool_schema() {
        let mut signals = empty_signals();
        signals.tok_system = 50_000;
        signals.tok_tools_schema = 10_000;
        signals.tok_rest = 100;
        signals.tok_total_in = 60_100;
        assert!(check_hard_gates(&signals, StepKind::DirectChat, 55_000, true).is_none());
    }

    #[test]
    fn ctx_overflow_direct_chat_when_transcript_large() {
        let mut signals = empty_signals();
        signals.tok_rest = 60_000;
        signals.tok_total_in = 120_000;
        assert_eq!(
            check_hard_gates(&signals, StepKind::DirectChat, 55_000, true)
                .unwrap()
                .code,
            "GATE_CTX_OVERFLOW"
        );
    }

    #[test]
    fn ctx_overflow_uses_full_prompt_for_work_steps() {
        let mut signals = empty_signals();
        signals.tok_rest = 100;
        signals.tok_total_in = 60_000;
        assert_eq!(
            check_hard_gates(&signals, StepKind::ToolSelect, 55_000, true)
                .unwrap()
                .code,
            "GATE_CTX_OVERFLOW"
        );
    }

    #[test]
    fn openclaw_compact_gate_follows_ctx_edge_max() {
        let mut signals = empty_signals();
        signals.tok_total_in = 50_000;
        assert!(check_hard_gates(&signals, StepKind::MemoryCompact, 262_144, true).is_none());
        assert_eq!(
            check_hard_gates(&signals, StepKind::MemoryCompact, 55_000, true)
                .unwrap()
                .code,
            "GATE_OPENCLAW_COMPACT"
        );
        assert!(check_hard_gates(&signals, StepKind::DirectChat, 55_000, true).is_none());
    }
}
