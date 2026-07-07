use super::signals::RequestSignals;
use super::step_kind::StepKind;

#[derive(Debug, Clone, Copy, Default)]
pub struct DecisionContext {
    pub edge_tps: Option<f64>,
    pub edge_busy: bool,
}

#[derive(Debug, Clone)]
pub struct DifficultyPart {
    pub key: String,
    pub linear: f32,
}

#[derive(Debug, Clone, Default)]
pub struct DifficultyBreakdown {
    pub parts: Vec<DifficultyPart>,
    pub linear_sum: f32,
    pub heuristic_score: f32,
}

impl DifficultyBreakdown {
    fn push(&mut self, key: impl Into<String>, linear: f32) {
        if linear.abs() <= f32::EPSILON {
            return;
        }
        self.parts.push(DifficultyPart {
            key: key.into(),
            linear,
        });
        self.linear_sum += linear;
    }

    fn finish(&mut self) -> f32 {
        self.heuristic_score = sigmoid(self.linear_sum).clamp(0.0, 1.0);
        self.heuristic_score
    }

    pub fn context_reason_codes(&self) -> Vec<&str> {
        self.parts
            .iter()
            .filter_map(|p| match p.key.as_str() {
                "CLOUD_CACHE_BOOST"
                | "REQ_ROUTE_CACHE_CLOUD"
                | "GATE_EDGE_BUSY"
                | "MULTIMODAL_COMPLEX_CLOUD"
                | "LONG_GEN_EDGE"
                | "LONG_GEN_EDGE_TPS_LOW" => Some(p.key.as_str()),
                _ => None,
            })
            .collect()
    }
}

pub fn emit_difficulty_breakdown(breakdown: &DifficultyBreakdown, codes: &mut Vec<String>) {
    for part in &breakdown.parts {
        codes.push(format!("DIFF_L:{}:{:+.3}", part.key, part.linear));
    }
    codes.push(format!("DIFF_LINEAR_SUM:{:.3}", breakdown.linear_sum));
    let full = breakdown.heuristic_score;
    for part in &breakdown.parts {
        let without = sigmoid(breakdown.linear_sum - part.linear);
        let delta = full - without;
        codes.push(format!("DIFF_D:{}:{:+.4}", part.key, delta));
    }
    codes.push(format!("DIFF_HEUR:{:.4}", full));
}

#[derive(Debug, Clone, Copy)]
pub struct DifficultyScore(pub f32);

impl DifficultyScore {
    pub fn compute(
        signals: &RequestSignals,
        step_kind: StepKind,
        ctx_edge_max: u32,
        experience_bias: f32,
    ) -> Self {
        Self::compute_with_context(
            signals,
            step_kind,
            ctx_edge_max,
            experience_bias,
            &[],
            &DecisionContext::default(),
        )
        .0
    }

    pub fn compute_with_context(
        signals: &RequestSignals,
        step_kind: StepKind,
        ctx_edge_max: u32,
        experience_bias: f32,
        external_parts: &[(String, f32)],
        ctx: &DecisionContext,
    ) -> (Self, DifficultyBreakdown) {
        let ctx_tokens = match step_kind {
            StepKind::DirectChat | StepKind::HeartbeatAck => signals.tok_rest,
            _ => signals.tok_loop_delta,
        };
        let ctx_ratio = (ctx_tokens as f32) / ctx_edge_max as f32;
        let user_ctx_ratio = (signals.last_user_tok as f32) / ctx_edge_max as f32;
        let tool_ratio = (signals.n_tool_defs as f32) / 20.0;

        let mut breakdown = DifficultyBreakdown::default();

        breakdown.push("CTX_RATIO", 0.20 * ctx_ratio.min(1.0));
        breakdown.push("USER_CTX_RATIO", 0.25 * user_ctx_ratio.min(1.0));
        breakdown.push("TOOL_RATIO", 0.10 * tool_ratio.min(1.0));
        if signals.intent_hard {
            breakdown.push("INTENT_HARD", 0.30);
        }
        if signals.intent_cloud {
            breakdown.push("INTENT_CLOUD", 0.25);
        }
        if signals.intent_long_gen {
            breakdown.push("INTENT_LONG_GEN", 0.25);
        }
        if signals.intent_edge {
            breakdown.push("INTENT_EDGE", -0.30);
        }
        if signals.intent_easy {
            breakdown.push("INTENT_EASY", -0.40);
        }
        if signals.user_multimodal {
            breakdown.push("USER_MULTIMODAL", 0.12);
        }

        let step_bias = step_kind.bias();
        breakdown.push(format!("STEP_{}", step_kind_code(step_kind)), step_bias);

        if experience_bias.abs() > f32::EPSILON {
            breakdown.push(format!("EXP_BIAS_{experience_bias:+.2}"), experience_bias);
        }

        if signals.assistant_failed_recent {
            breakdown.push("ASSISTANT_FAILED", 0.15);
        }

        let tool_err = tool_error_streak_bias(signals.consecutive_tool_error_streak);
        if tool_err > 0.0 {
            breakdown.push(
                format!("TOOL_ERROR_STREAK_{}", signals.consecutive_tool_error_streak),
                tool_err,
            );
        }

        let tool_loop = tool_loop_bias(signals.tool_invocations_since_last_user);
        if tool_loop > 0.0 {
            breakdown.push(
                format!("TOOL_LOOP_{}", signals.tool_invocations_since_last_user),
                tool_loop,
            );
        }

        let lex = lexical_rarity_bias(signals.rare_lexical, signals.special_lexical);
        if lex > 0.0 {
            let key = match (signals.rare_lexical, signals.special_lexical) {
                (true, true) => "LEXICAL_BOTH",
                (false, true) => "LEXICAL_SPECIAL",
                (true, false) => "LEXICAL_RARE",
                _ => "LEXICAL",
            };
            breakdown.push(key, lex);
        }

        let risky_soft = risky_tool_soft_bias(signals, step_kind);
        if risky_soft > 0.0 {
            breakdown.push("RISKY_TOOL_SOFT", risky_soft);
        }

        push_cognitive_parts(signals, &mut breakdown);
        push_context_parts(signals, step_kind, ctx, &mut breakdown);

        for (key, linear) in external_parts {
            breakdown.push(key.clone(), *linear);
        }

        let d = breakdown.finish();
        (Self(d), breakdown)
    }
}

fn step_kind_code(step_kind: StepKind) -> &'static str {
    match step_kind {
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

fn push_cognitive_parts(signals: &RequestSignals, breakdown: &mut DifficultyBreakdown) {
    if !super::signals::cognitive_task_applies(signals) {
        return;
    }
    if signals.intent_plan {
        breakdown.push("PLAN_INTENT", 0.15);
    }
    if signals.intent_analysis {
        breakdown.push("ANALYSIS_INTENT", 0.18);
    }
    if signals.intent_decision {
        breakdown.push("DECISION_INTENT", 0.15);
    }
    if signals.intent_research {
        breakdown.push("RESEARCH_INTENT", 0.15);
    }
}

fn push_context_parts(
    signals: &RequestSignals,
    step_kind: StepKind,
    ctx: &DecisionContext,
    breakdown: &mut DifficultyBreakdown,
) {
    if ctx.edge_busy && !matches!(step_kind, StepKind::DirectChat | StepKind::HeartbeatAck) {
        breakdown.push("GATE_EDGE_BUSY", 0.30);
    }

    if signals.multimodal && step_kind != StepKind::DirectChat {
        breakdown.push("MULTIMODAL_COMPLEX_CLOUD", 0.35);
    }

    if signals.intent_long_gen && !signals.intent_cloud {
        if super::keywords::edge_tps_is_low(ctx.edge_tps) {
            breakdown.push("LONG_GEN_EDGE_TPS_LOW", 0.50);
        } else {
            breakdown.push("LONG_GEN_EDGE", -0.15);
        }
    }
}

fn tool_error_streak_bias(streak: u32) -> f32 {
    match streak {
        0 => 0.0,
        1 => 0.10,
        2 => 0.28,
        3 => 0.38,
        _ => 0.48,
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

pub fn risky_tool_soft_bias(signals: &RequestSignals, _step_kind: StepKind) -> f32 {
    if signals.risky_tool_soft {
        0.22
    } else {
        0.0
    }
}

pub fn apply_privacy_cap(d: f32) -> f32 {
    d.min(0.20)
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
            consecutive_tool_error_streak: streak,
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

    fn base_signals_with_tool_loop(invocations: u32) -> RequestSignals {
        let mut s = base_signals(0);
        s.tool_invocations_since_last_user = invocations;
        s
    }

    #[test]
    fn tool_error_streak_increases_difficulty_from_first() {
        let ctx_max = 65536;
        let step = StepKind::ToolResultDigest;
        let d0 = DifficultyScore::compute(&base_signals(0), step, ctx_max, 0.0);
        let d1 = DifficultyScore::compute(&base_signals(1), step, ctx_max, 0.0);
        let d2 = DifficultyScore::compute(&base_signals(2), step, ctx_max, 0.0);
        assert!(d1.0 > d0.0);
        assert!(d2.0 > d1.0);
        let d3 = DifficultyScore::compute(
            &base_signals(3),
            StepKind::RecoveryAfterFailure,
            ctx_max,
            0.0,
        );
        assert!(d3.0 > d2.0);
    }

    #[test]
    fn tool_error_streak_bias_values() {
        assert_eq!(tool_error_streak_bias(0), 0.0);
        assert_eq!(tool_error_streak_bias(1), 0.10);
        assert_eq!(tool_error_streak_bias(2), 0.28);
        assert_eq!(tool_error_streak_bias(3), 0.38);
        assert_eq!(tool_error_streak_bias(5), 0.48);
    }

    #[test]
    fn cognitive_task_bias_only_on_first_hop() {
        let ctx_max = 65536;
        let step = StepKind::InitialPlan;
        let mut first = base_signals(0);
        first.intent_analysis = true;
        let mut mid_loop = base_signals(0);
        mid_loop.intent_analysis = true;
        mid_loop.tool_invocations_since_last_user = 1;
        let d_first = DifficultyScore::compute(&first, step, ctx_max, 0.0);
        let d_mid = DifficultyScore::compute(&mid_loop, step, ctx_max, 0.0);
        assert!(d_first.0 > d_mid.0);
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
    fn risky_tool_soft_bias_values() {
        let mut soft = base_signals(0);
        soft.risky_tool_soft = true;
        assert_eq!(risky_tool_soft_bias(&soft, StepKind::ToolArgFill), 0.22);
        assert_eq!(risky_tool_soft_bias(&soft, StepKind::ToolResultDigest), 0.22);
    }

    #[test]
    fn risky_tool_soft_increases_difficulty() {
        let ctx_max = 65536;
        let step = StepKind::ToolArgFill;
        let mut soft = base_signals(0);
        soft.risky_tool_soft = true;
        let base = DifficultyScore::compute(&base_signals(0), step, ctx_max, 0.0);
        let d_soft = DifficultyScore::compute(&soft, step, ctx_max, 0.0);
        assert!(d_soft.0 > base.0);
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

    #[test]
    fn gate_and_context_bias_increase_difficulty() {
        let ctx_max = 65536;
        let step = StepKind::ToolSelect;
        let base = DifficultyScore::compute(&base_signals(0), step, ctx_max, 0.0);
        let gate_parts = vec![("GATE_CTX_OVERFLOW".to_string(), 0.50)];
        let (with_gate, _) = DifficultyScore::compute_with_context(
            &base_signals(0),
            step,
            ctx_max,
            0.0,
            &gate_parts,
            &DecisionContext::default(),
        );
        assert!(with_gate.0 > base.0);

        let (_, breakdown) = DifficultyScore::compute_with_context(
            &base_signals(0),
            step,
            ctx_max,
            0.0,
            &[("CLOUD_CACHE_BOOST".to_string(), 0.12)],
            &DecisionContext::default(),
        );
        assert!(
            breakdown
                .parts
                .iter()
                .any(|p| p.key == "CLOUD_CACHE_BOOST")
        );
    }

    #[test]
    fn privacy_cap_limits_difficulty() {
        assert_eq!(apply_privacy_cap(0.64), 0.20);
        assert_eq!(apply_privacy_cap(0.15), 0.15);
    }

    #[test]
    fn emit_breakdown_includes_linear_and_score_parts() {
        let ctx_max = 65536;
        let step = StepKind::DirectChat;
        let mut signals = base_signals(1);
        signals.rare_lexical = true;
        let (_, breakdown) = DifficultyScore::compute_with_context(
            &signals,
            step,
            ctx_max,
            0.0,
            &[],
            &DecisionContext::default(),
        );
        let mut codes = Vec::new();
        emit_difficulty_breakdown(&breakdown, &mut codes);
        assert!(codes.iter().any(|c| c.starts_with("DIFF_L:TOOL_ERROR_STREAK_1:")));
        assert!(codes.iter().any(|c| c.starts_with("DIFF_D:LEXICAL_RARE:")));
        assert!(codes.iter().any(|c| c.starts_with("DIFF_HEUR:")));
    }
}
