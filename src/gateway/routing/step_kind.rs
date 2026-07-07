use serde::{Deserialize, Serialize};

use crate::gateway::api::openai::ChatCompletionRequest;

use super::signals::{RequestSignals, cognitive_task_applies, is_casual_chat};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StepKind {
    HeartbeatAck,
    /// Short, tool-free user turn (e.g. greeting) — prefer edge.
    DirectChat,
    RecoveryAfterFailure,
    ToolSelect,
    ToolArgFill,
    ToolResultDigest,
    InitialPlan,
    FinalReply,
    SubagentSpawn,
    MemoryCompact,
    CronBackground,
}

impl StepKind {
    pub fn bias(self) -> f32 {
        match self {
            StepKind::HeartbeatAck => -0.60,
            StepKind::DirectChat => -0.55,
            StepKind::ToolResultDigest => -0.45,
            StepKind::ToolArgFill => -0.25,
            StepKind::ToolSelect => -0.10,
            StepKind::FinalReply => 0.05,
            StepKind::InitialPlan => 0.55,
            StepKind::MemoryCompact => 0.20,
            StepKind::RecoveryAfterFailure => 0.55,
            StepKind::SubagentSpawn => 0.50,
            StepKind::CronBackground => -0.15,
        }
    }
}

fn cognitive_initial_plan(signals: &RequestSignals) -> bool {
    cognitive_task_applies(signals)
        && (signals.intent_analysis || signals.intent_decision || signals.intent_research)
}

pub fn resolve_step_kind(_req: &ChatCompletionRequest, signals: &RequestSignals) -> StepKind {
    if signals.is_heartbeat_poll {
        return StepKind::HeartbeatAck;
    }

    if signals.assistant_failed_recent {
        return StepKind::RecoveryAfterFailure;
    }

    if signals.pending_tool_calls {
        return if signals.tool_arg_ready {
            StepKind::ToolArgFill
        } else {
            StepKind::ToolSelect
        };
    }

    if signals.last_role_tool && !signals.synthetic_tool_result {
        return StepKind::ToolResultDigest;
    }

    if signals.voice_repair_loop {
        return StepKind::ToolResultDigest;
    }

    if signals.subagent_spawn_hint {
        return StepKind::SubagentSpawn;
    }

    if !signals.pending_tool_calls
        && !signals.last_role_tool
        && is_casual_chat(signals)
    {
        return StepKind::DirectChat;
    }

    if signals.memory_compact_hint {
        return StepKind::MemoryCompact;
    }

    if signals.cron_background {
        return StepKind::CronBackground;
    }

    // Planning turn (explicit 规划/计划/plan or first non-casual agent/cognitive task).
    if !signals.pending_tool_calls
        && !signals.last_role_tool
        && (signals.intent_plan
            || cognitive_initial_plan(signals)
            || (signals.tools_enabled
                && signals.loop_steps == 0
                && !signals.had_tool_roundtrip
                && !is_casual_chat(signals)))
    {
        return StepKind::InitialPlan;
    }

    if is_casual_chat(signals) {
        return StepKind::DirectChat;
    }

    if !signals.tools_enabled && signals.had_tool_roundtrip {
        return StepKind::FinalReply;
    }

    if signals.tools_enabled {
        StepKind::ToolSelect
    } else {
        StepKind::FinalReply
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn signals_with_tool_error_streak(streak: u32) -> RequestSignals {
        RequestSignals {
            tok_system: 0,
            tok_tools_schema: 0,
            tok_rest: 100,
            tok_total_in: 100,
            tok_loop_delta: 0,
            tok_out_estimate: 0,
            n_tool_defs: 1,
            n_turns: 2,
            last_user_tok: 20,
            loop_steps: 0,
            pending_tool_calls: false,
            tool_arg_ready: false,
            last_role_tool: true,
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

    #[test]
    fn tool_error_streak_stays_tool_result_digest_from_second() {
        let req = ChatCompletionRequest::default();
        let signals = signals_with_tool_error_streak(1);
        assert_eq!(
            resolve_step_kind(&req, &signals),
            StepKind::ToolResultDigest
        );
        let signals = signals_with_tool_error_streak(2);
        assert_eq!(
            resolve_step_kind(&req, &signals),
            StepKind::ToolResultDigest
        );
    }

    #[test]
    fn successful_tool_tail_stays_tool_result_digest() {
        let req = ChatCompletionRequest::default();
        let mut signals = signals_with_tool_error_streak(0);
        signals.last_role_tool = true;
        assert_eq!(
            resolve_step_kind(&req, &signals),
            StepKind::ToolResultDigest
        );
    }

    #[test]
    fn casual_greeting_wins_over_memory_compact_hint() {
        let req = ChatCompletionRequest::default();
        let signals = RequestSignals {
            tok_system: 40_000,
            tok_tools_schema: 20_000,
            tok_rest: 20,
            tok_total_in: 60_020,
            tok_loop_delta: 0,
            tok_out_estimate: 0,
            n_tool_defs: 3,
            n_turns: 1,
            last_user_tok: 5,
            loop_steps: 0,
            pending_tool_calls: false,
            tool_arg_ready: false,
            last_role_tool: false,
            synthetic_tool_result: false,
            assistant_failed_recent: false,
            is_heartbeat_poll: false,
            voice_repair_loop: false,
            subagent_spawn_hint: false,
            memory_compact_hint: true,
            cron_background: false,
            tools_enabled: true,
            had_tool_roundtrip: false,
            risky_tool_hard: false,
            risky_tool_soft: false,
            risky_tool_names: Vec::new(),
            risky_tool_hard_names: Vec::new(),
            risky_tool_soft_names: Vec::new(),
            intent_hard: false,
            intent_easy: true,
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
        };
        assert_eq!(resolve_step_kind(&req, &signals), StepKind::DirectChat);
    }
}

