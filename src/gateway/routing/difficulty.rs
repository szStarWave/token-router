use super::signals::RequestSignals;
use super::step_kind::StepKind;

#[derive(Debug, Clone, Copy)]
pub struct DifficultyScore(pub f32);

impl DifficultyScore {
    pub fn compute(
        signals: &RequestSignals,
        step_kind: StepKind,
        ctx_edge_max: u32,
        experience_bias: f32,
    ) -> Self {
        let ctx_tokens = match step_kind {
            StepKind::DirectChat | StepKind::HeartbeatAck => signals.tok_rest,
            _ => signals.tok_loop_delta,
        };
        let ctx_ratio = (ctx_tokens as f32) / ctx_edge_max as f32;
        let user_ctx_ratio = (signals.last_user_tok as f32) / ctx_edge_max as f32;
        let tool_ratio = (signals.n_tool_defs as f32) / 20.0;
        let code_hint = 0.0f32;

        let mut linear = 0.20 * ctx_ratio.min(1.0)
            + 0.25 * user_ctx_ratio.min(1.0)
            + 0.10 * tool_ratio.min(1.0)
            + 0.30 * if signals.intent_hard { 1.0 } else { 0.0 }
            - 0.40 * if signals.intent_easy { 1.0 } else { 0.0 }
            + 0.12 * if signals.user_multimodal { 1.0 } else { 0.0 }
            + code_hint
            + step_kind.bias()
            + experience_bias;

        if signals.assistant_failed_recent {
            linear += 0.15;
        }

        linear += tool_error_streak_bias(signals.consecutive_tool_error_streak);
        linear += tool_loop_bias(signals.tool_invocations_since_last_user);
        linear += lexical_rarity_bias(signals.rare_lexical, signals.special_lexical);

        let d = sigmoid(linear);
        Self(d.clamp(0.0, 1.0))
    }
}

fn tool_error_streak_bias(streak: u32) -> f32 {
    match streak {
        0 => 0.0,
        1 => 0.15,
        2 => 0.30,
        _ => 0.40,
    }
}

fn tool_loop_bias(invocations: u32) -> f32 {
    match invocations {
        0..=4 => 0.0,
        5..=6 => 0.10,
        7 => 0.18,
        _ => 0.25,
    }
}

pub fn lexical_rarity_bias(rare: bool, special: bool) -> f32 {
    match (rare, special) {
        (true, true) => 0.18,
        (false, true) => 0.12,
        (true, false) => 0.08,
        (false, false) => 0.0,
    }
}

fn sigmoid(x: f32) -> f32 {
    1.0 / (1.0 + (-x).exp())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gateway::routing::signals::RequestSignals;

    fn base_signals(streak: u32) -> RequestSignals {
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
            consecutive_tool_error_streak: streak,
            tool_invocations_since_last_user: 0,
            user_rejects_answer: false,
            rare_lexical: false,
            special_lexical: false,
            rare_token_ratio: 0.0,
        }
    }

    fn base_signals_with_tool_loop(invocations: u32) -> RequestSignals {
        let mut s = base_signals(0);
        s.tool_invocations_since_last_user = invocations;
        s
    }

    #[test]
    fn tool_error_streak_increases_difficulty() {
        let ctx_max = 65536;
        let d0 = DifficultyScore::compute(&base_signals(0), StepKind::ToolResultDigest, ctx_max, 0.0);
        let d1 = DifficultyScore::compute(&base_signals(1), StepKind::RecoveryAfterFailure, ctx_max, 0.0);
        let d2 = DifficultyScore::compute(&base_signals(2), StepKind::RecoveryAfterFailure, ctx_max, 0.0);
        let d3 = DifficultyScore::compute(&base_signals(3), StepKind::RecoveryAfterFailure, ctx_max, 0.0);
        assert!(d1.0 > d0.0);
        assert!(d2.0 > d1.0);
        assert!(d3.0 > d2.0);
        let d5 = DifficultyScore::compute(&base_signals(5), StepKind::RecoveryAfterFailure, ctx_max, 0.0);
        assert_eq!(d3.0, d5.0);
    }

    #[test]
    fn tool_error_streak_bias_values() {
        assert_eq!(tool_error_streak_bias(0), 0.0);
        assert_eq!(tool_error_streak_bias(1), 0.15);
        assert_eq!(tool_error_streak_bias(2), 0.30);
        assert_eq!(tool_error_streak_bias(5), 0.40);
    }

    #[test]
    fn tool_loop_bias_values() {
        assert_eq!(tool_loop_bias(4), 0.0);
        assert_eq!(tool_loop_bias(5), 0.10);
        assert_eq!(tool_loop_bias(6), 0.10);
        assert_eq!(tool_loop_bias(7), 0.18);
        assert_eq!(tool_loop_bias(8), 0.25);
        assert_eq!(tool_loop_bias(12), 0.25);
    }

    #[test]
    fn tool_loop_increases_difficulty_from_fifth_invocation() {
        let ctx_max = 65536;
        let step = StepKind::ToolResultDigest;
        let d4 = DifficultyScore::compute(&base_signals_with_tool_loop(4), step, ctx_max, 0.0);
        let d5 = DifficultyScore::compute(&base_signals_with_tool_loop(5), step, ctx_max, 0.0);
        let d6 = DifficultyScore::compute(&base_signals_with_tool_loop(6), step, ctx_max, 0.0);
        let d7 = DifficultyScore::compute(&base_signals_with_tool_loop(7), step, ctx_max, 0.0);
        let d8 = DifficultyScore::compute(&base_signals_with_tool_loop(8), step, ctx_max, 0.0);
        assert!(d4.0 < d5.0);
        assert_eq!(d5.0, d6.0);
        assert!(d6.0 < d7.0);
        assert!(d7.0 < d8.0);
    }

    #[test]
    fn lexical_rarity_bias_values() {
        assert_eq!(lexical_rarity_bias(false, false), 0.0);
        assert_eq!(lexical_rarity_bias(true, false), 0.08);
        assert_eq!(lexical_rarity_bias(false, true), 0.12);
        assert_eq!(lexical_rarity_bias(true, true), 0.18);
    }

    #[test]
    fn user_context_weights_more_than_loop_delta_alone() {
        let ctx_max = 10_000;
        let step = StepKind::DirectChat;
        let mut loop_only = base_signals(0);
        loop_only.tok_loop_delta = 8000;
        loop_only.last_user_tok = 50;
        let mut user_heavy = base_signals(0);
        user_heavy.tok_loop_delta = 500;
        user_heavy.last_user_tok = 8000;
        let d_loop = DifficultyScore::compute(&loop_only, step, ctx_max, 0.0);
        let d_user = DifficultyScore::compute(&user_heavy, step, ctx_max, 0.0);
        assert!(d_user.0 > d_loop.0);
    }

    #[test]
    fn lexical_rarity_increases_difficulty() {
        let ctx_max = 65536;
        let step = StepKind::DirectChat;
        let mut rare = base_signals(0);
        rare.rare_lexical = true;
        let mut special = base_signals(0);
        special.special_lexical = true;
        let mut both = base_signals(0);
        both.rare_lexical = true;
        both.special_lexical = true;
        let base = DifficultyScore::compute(&base_signals(0), step, ctx_max, 0.0);
        let d_rare = DifficultyScore::compute(&rare, step, ctx_max, 0.0);
        let d_special = DifficultyScore::compute(&special, step, ctx_max, 0.0);
        let d_both = DifficultyScore::compute(&both, step, ctx_max, 0.0);
        assert!(d_rare.0 > base.0);
        assert!(d_special.0 > d_rare.0);
        assert!(d_both.0 > d_special.0);
    }
}
