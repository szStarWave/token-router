use crate::gateway::routing::{RequestSignals, StepKind};

/// Discrete routing features derived from request signals (no message text).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FeatureVector {
    pub keys: Vec<String>,
}

impl FeatureVector {
    pub fn from_signals(signals: &RequestSignals, step_kind: StepKind, ctx_edge_max: u32) -> Self {
        let mut keys = Vec::with_capacity(20);

        keys.push(format!("step_kind:{}", step_kind_key(step_kind)));
        keys.push(format!("ctx_bucket:{}", ctx_bucket(signals, ctx_edge_max)));
        keys.push(format!("tool_bucket:{}", tool_bucket(signals.n_tool_defs)));
        keys.push(format!("loop_bucket:{}", loop_bucket(signals.loop_steps)));
        keys.push(format!("turn_bucket:{}", turn_bucket(signals.n_turns)));
        keys.push(format!("intent:{}", intent_bucket(signals)));

        push_flag(&mut keys, "multimodal", signals.multimodal);
        push_flag(&mut keys, "risky_tool_tier1", signals.risky_tool_tier1);
        push_flag(&mut keys, "pending_tool_calls", signals.pending_tool_calls);
        push_flag(
            &mut keys,
            "assistant_failed_recent",
            signals.assistant_failed_recent,
        );
        push_flag(&mut keys, "is_heartbeat_poll", signals.is_heartbeat_poll);
        push_flag(&mut keys, "had_tool_roundtrip", signals.had_tool_roundtrip);
        push_flag(&mut keys, "tools_enabled", signals.tools_enabled);

        Self { keys }
    }
}

fn push_flag(keys: &mut Vec<String>, name: &str, active: bool) {
    if active {
        keys.push(format!("flag:{name}"));
    }
}

fn step_kind_key(k: StepKind) -> &'static str {
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

fn ctx_bucket(signals: &RequestSignals, ctx_edge_max: u32) -> &'static str {
    if ctx_edge_max == 0 {
        return "low";
    }
    let ratio = signals.tok_loop_delta as f32 / ctx_edge_max as f32;
    if ratio < 0.2 {
        "low"
    } else if ratio < 0.6 {
        "mid"
    } else {
        "high"
    }
}

fn tool_bucket(n_tool_defs: u32) -> &'static str {
    if n_tool_defs == 0 {
        "none"
    } else if n_tool_defs <= 8 {
        "few"
    } else {
        "many"
    }
}

fn loop_bucket(loop_steps: u32) -> &'static str {
    if loop_steps == 0 {
        "none"
    } else if loop_steps <= 4 {
        "short"
    } else {
        "long"
    }
}

fn turn_bucket(n_turns: u32) -> &'static str {
    if n_turns <= 8 { "short" } else { "long" }
}

fn intent_bucket(signals: &RequestSignals) -> &'static str {
    if signals.intent_plan {
        "plan"
    } else if signals.intent_hard {
        "hard"
    } else if signals.intent_easy {
        "easy"
    } else {
        "neutral"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gateway::routing::SignalExtractor;

    fn signals_with_delta(delta: u32, ctx_max: u32) -> (RequestSignals, u32) {
        let extractor = SignalExtractor {
            ctx_edge_max: ctx_max,
        };
        let req = crate::gateway::api::openai::ChatCompletionRequest {
            model: "test".into(),
            messages: vec![crate::gateway::api::openai::Message {
                role: crate::gateway::api::openai::Role::User,
                content: Some("hi".into()),
                content_parts: None,
                tool_calls: None,
                tool_call_id: None,
            }],
            tools: vec![],
            stream: false,
            ..Default::default()
        };
        let mut s = extractor.extract(&req, Some(0));
        s.tok_loop_delta = delta;
        (s, ctx_max)
    }

    #[test]
    fn ctx_bucket_boundaries() {
        let (s, max) = signals_with_delta(1000, 10_000);
        assert_eq!(ctx_bucket(&s, max), "low");
        let (s, max) = signals_with_delta(3000, 10_000);
        assert_eq!(ctx_bucket(&s, max), "mid");
        let (s, max) = signals_with_delta(7000, 10_000);
        assert_eq!(ctx_bucket(&s, max), "high");
    }

    #[test]
    fn feature_vector_includes_step_kind() {
        let (signals, max) = signals_with_delta(0, 65536);
        let fv = FeatureVector::from_signals(&signals, StepKind::DirectChat, max);
        assert!(fv.keys.iter().any(|k| k == "step_kind:direct_chat"));
    }
}
