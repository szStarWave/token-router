use serde::{Deserialize, Serialize};

use crate::gateway::api::openai::ChatCompletionRequest;

use super::signals::{RequestSignals, is_casual_chat};

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
            StepKind::InitialPlan => 0.35,
            StepKind::MemoryCompact => 0.20,
            StepKind::RecoveryAfterFailure => 0.55,
            StepKind::SubagentSpawn => 0.50,
            StepKind::CronBackground => -0.15,
        }
    }
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

    if signals.memory_compact_hint {
        return StepKind::MemoryCompact;
    }

    if signals.cron_background {
        return StepKind::CronBackground;
    }

    // Planning turn (explicit 规划/计划/plan or first non-casual agent task).
    if !signals.pending_tool_calls
        && !signals.last_role_tool
        && (signals.intent_plan
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

