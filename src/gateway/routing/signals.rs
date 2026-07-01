use crate::gateway::api::openai::{ChatCompletionRequest, Message, Role};

#[derive(Debug, Clone)]
pub struct RequestSignals {
    pub tok_system: u32,
    pub tok_tools_schema: u32,
    /// Messages excluding system (user/assistant/tool transcript).
    pub tok_rest: u32,
    pub tok_total_in: u32,
    pub tok_loop_delta: u32,
    pub tok_out_estimate: u32,
    pub n_tool_defs: u32,
    pub n_turns: u32,
    /// Token estimate for the latest user message only.
    pub last_user_tok: u32,
    pub loop_steps: u32,
    pub pending_tool_calls: bool,
    pub tool_arg_ready: bool,
    pub last_role_tool: bool,
    pub synthetic_tool_result: bool,
    pub assistant_failed_recent: bool,
    pub is_heartbeat_poll: bool,
    pub voice_repair_loop: bool,
    pub subagent_spawn_hint: bool,
    pub memory_compact_hint: bool,
    pub cron_background: bool,
    pub tools_enabled: bool,
    pub had_tool_roundtrip: bool,
    pub risky_tool_tier1: bool,
    pub intent_hard: bool,
    pub intent_easy: bool,
    pub intent_plan: bool,
    pub multimodal: bool,
    /// Trailing consecutive `role=tool` messages whose content matches error keywords.
    pub consecutive_tool_error_streak: u32,
    /// Latest user message rejects the immediately preceding assistant reply.
    pub user_rejects_answer: bool,
}

/// Short, tool-loop-free turn (daily chat). Tool definitions in the request are ignored.
pub fn is_casual_chat(signals: &RequestSignals) -> bool {
    if signals.had_tool_roundtrip
        || signals.pending_tool_calls
        || signals.intent_hard
        || signals.assistant_failed_recent
    {
        return false;
    }
    if signals.tok_rest >= 8192 || signals.n_turns > 8 {
        return false;
    }
    // Vision + tool schema together usually means an agent task, not daily chat.
    if signals.multimodal && signals.tools_enabled {
        return false;
    }
    // Mid-loop agent with tools: require easy intent (OpenClaw tool loop).
    if signals.loop_steps > 0 && signals.tools_enabled {
        if signals.intent_plan {
            return false;
        }
        return signals.intent_easy;
    }
    // Mid-loop without tools: short follow-up counts as casual.
    if signals.loop_steps > 0 {
        if signals.intent_plan {
            return false;
        }
        if signals.intent_easy {
            return true;
        }
        const MID_LOOP_USER_MAX: u32 = 512;
        if signals.last_user_tok > MID_LOOP_USER_MAX {
            return false;
        }
    }
    true
}

/// Simple multimodal daily chat — eligible for edge-first routing.
pub fn is_simple_multimodal(signals: &RequestSignals) -> bool {
    signals.multimodal && is_casual_chat(signals)
}

pub struct SignalExtractor {
    pub ctx_edge_max: u32,
}

impl SignalExtractor {
    pub fn extract(
        &self,
        req: &ChatCompletionRequest,
        prev_tok_total_in: Option<u32>,
    ) -> RequestSignals {
        let tok_tools_schema = estimate_tokens(&serde_json::to_string(&req.tools).unwrap_or_default());
        let mut tok_system = 0u32;
        let mut tok_rest = 0u32;
        let mut n_turns = 0u32;
        let mut had_tool = false;
        let mut multimodal = false;

        for (i, msg) in req.messages.iter().enumerate() {
            let t = estimate_message_tokens(msg);
            if i == 0 && msg.role == Role::System {
                tok_system = t;
            } else {
                tok_rest += t;
            }
            if msg.role == Role::User {
                n_turns += 1;
            }
            if msg.role == Role::Tool {
                had_tool = true;
            }
            if message_has_image(msg) {
                multimodal = true;
            }
        }

        let tok_total_in = tok_system.saturating_add(tok_rest).saturating_add(tok_tools_schema);
        let tok_loop_delta = prev_tok_total_in.map_or(tok_total_in, |prev| {
            tok_total_in.saturating_sub(prev)
        });

        let tail = req.messages.iter().rev().take(4).collect::<Vec<_>>();
        let last = req.messages.last();
        let prev_assistant = req
            .messages
            .iter()
            .rev()
            .filter(|m| m.role == Role::Assistant)
            .nth(0);

        let pending_tool_calls = prev_assistant
            .and_then(|m| m.tool_calls.as_ref())
            .is_some_and(|tc| !tc.is_empty());

        let tool_arg_ready = prev_assistant
            .and_then(|m| m.tool_calls.as_ref())
            .is_some_and(|calls| {
                calls.iter().all(|c| {
                    c.function
                        .arguments
                        .starts_with('{')
                        && c.function.arguments.contains('}')
                })
            });

        let last_role_tool = last.is_some_and(|m| m.role == Role::Tool);
        let synthetic_tool_result = last.is_some_and(|m| {
            message_text(m).contains("[openclaw] missing tool result")
                || message_text(m).contains("prompt lock was released")
        });

        let assistant_failed_recent = tail.iter().any(|m| {
            m.role == Role::Assistant
                && message_text(m).contains("[assistant turn failed")
        });

        let is_heartbeat_poll = last.is_some_and(|m| {
            m.role == Role::User && message_text(m).trim() == "[OpenClaw heartbeat poll]"
        });

        let voice_repair_loop = last.is_some_and(|m| {
            m.role == Role::User
                && message_text(m).contains("[Audio transcript")
                && had_tool
        });

        // Do not scan system/tool-schema boilerplate (OpenClaw documents sessions_spawn in system).
        let subagent_spawn_hint = subagent_spawn_in_transcript(req)
            || prev_assistant
                .and_then(|m| m.tool_calls.as_ref())
                .is_some_and(|calls| {
                    calls
                        .iter()
                        .any(|c| c.function.name == "sessions_spawn")
                });

        let memory_compact_hint = tok_system > 0
            && req.messages.iter().any(|m| {
                let t = message_text(m);
                t.contains("Dynamic Project Context") || t.contains("compaction")
            });

        let cron_background = req.messages.iter().any(|m| {
            let t = message_text(m).to_ascii_lowercase();
            t.contains("[cron]") || t.contains("cron background") || t.contains("cron job")
        });

        let loop_steps = req
            .messages
            .iter()
            .filter(|m| m.role == Role::Assistant)
            .count() as u32;

        let intent_hard = last
            .map(|m| message_text(m))
            .is_some_and(|t| contains_hard_intent(&t));
        let intent_easy = last
            .map(|m| message_text(m))
            .is_some_and(|t| contains_easy_intent(&t));
        let intent_plan = last
            .filter(|m| m.role == Role::User)
            .map(|m| message_text(m))
            .is_some_and(|t| contains_plan_intent(&t));

        let last_user_tok = req
            .messages
            .iter()
            .rev()
            .find(|m| m.role == Role::User)
            .map(estimate_message_tokens)
            .unwrap_or(0);

        let consecutive_tool_error_streak = consecutive_tool_error_tail(&req.messages);

        let user_rejects_answer = req.messages.len() >= 2
            && req.messages[req.messages.len() - 1].role == Role::User
            && req.messages[req.messages.len() - 2].role == Role::Assistant
            && super::reject_intent::contains_reject_intent(&message_text(
                &req.messages[req.messages.len() - 1],
            ));

        let risky_tool_tier1 = prev_assistant
            .and_then(|m| m.tool_calls.as_ref())
            .is_some_and(|calls| {
                calls.iter().any(|c| {
                    matches!(
                        c.function.name.as_str(),
                        "exec" | "write" | "edit" | "browser" | "sessions_spawn" | "message"
                    )
                })
            });

        RequestSignals {
            tok_system,
            tok_tools_schema,
            tok_rest,
            tok_total_in,
            tok_loop_delta,
            tok_out_estimate: 0,
            n_tool_defs: req.tools.len() as u32,
            n_turns,
            last_user_tok,
            loop_steps,
            pending_tool_calls,
            tool_arg_ready,
            last_role_tool,
            synthetic_tool_result,
            assistant_failed_recent,
            is_heartbeat_poll,
            voice_repair_loop,
            subagent_spawn_hint,
            memory_compact_hint,
            cron_background,
            tools_enabled: !req.tools.is_empty(),
            had_tool_roundtrip: had_tool,
            risky_tool_tier1,
            intent_hard,
            intent_easy,
            intent_plan,
            multimodal,
            consecutive_tool_error_streak,
            user_rejects_answer,
        }
    }
}

/// Count trailing tool messages (from the end) that contain error keywords.
pub fn consecutive_tool_error_tail(messages: &[Message]) -> u32 {
    let mut count = 0u32;
    for msg in messages.iter().rev() {
        if msg.role != Role::Tool {
            break;
        }
        if tool_result_has_error(&message_text(msg)) {
            count += 1;
        } else {
            break;
        }
    }
    count
}

pub fn tool_result_has_error(text: &str) -> bool {
    const KWS: &[&str] = &[
        "error",
        "failed",
        "failure",
        "exception",
        "traceback",
        "errno",
        "non-zero",
        "nonzero",
        "exit code 1",
        "exit code: 1",
        "exit status 1",
        "command failed",
        "command not found",
        "permission denied",
        "错误",
        "失败",
        "异常",
    ];
    let lower = text.to_ascii_lowercase();
    KWS.iter().any(|k| lower.contains(k))
}

fn contains_hard_intent(text: &str) -> bool {
    const KWS: &[&str] = &[
        "架构",
        "证明",
        "refactor",
        "distributed",
        "legal",
        "medical",
        "跨仓库",
        "修复",
        "bug",
        "fix",
        "debug",
    ];
    let lower = text.to_ascii_lowercase();
    KWS.iter().any(|k| lower.contains(k))
}

fn contains_plan_intent(text: &str) -> bool {
    const KWS: &[&str] = &[
        "规划",
        "计划",
        "方案",
        "roadmap",
        "planning",
        " step plan",
        "make a plan",
        "制定计划",
        "执行计划",
        "任务拆解",
        "拆解任务",
        "分解任务",
    ];
    let lower = text.to_ascii_lowercase();
    KWS.iter().any(|k| {
        if k.is_ascii() {
            lower.contains(k)
        } else {
            text.contains(k)
        }
    }) || {
        let trimmed = lower.trim();
        trimmed.starts_with("plan ") || trimmed == "plan"
    }
}

fn contains_easy_intent(text: &str) -> bool {
    const KWS: &[&str] = &[
        "分类",
        "提取",
        "格式化",
        "translate",
        "yes/no",
        "是否",
        "你好",
        "hello",
        "hi",
        "嗨",
        "几点",
        "what time",
        "current time",
        "现在几点",
        "什么时间",
        "天气",
        "weather",
        "笑话",
        "joke",
        "谢谢",
        "thanks",
        "再见",
        "bye",
        "聊聊",
        "chat",
        "介绍一下",
        "是谁",
        "什么意思",
        "怎么样",
        "可以吗",
        "继续",
        "还有吗",
        "再说",
        "再讲",
    ];
    let lower = text.to_ascii_lowercase();
    KWS.iter().any(|k| lower.contains(k))
}

pub fn estimate_tokens(text: &str) -> u32 {
    // Fast heuristic (~4 chars/token); replace with tiktoken later.
    ((text.len() as f64) / 4.0).ceil() as u32
}

fn estimate_message_tokens(msg: &Message) -> u32 {
    let mut n = 0u32;
    if let Some(c) = &msg.content {
        n += estimate_tokens(c);
    }
    if let Some(parts) = &msg.content_parts {
        for p in parts {
            if let Some(t) = &p.text {
                n += estimate_tokens(t);
            }
            if p.image_url.is_some() {
                n += 512;
            }
        }
    }
    if let Some(calls) = &msg.tool_calls {
        for c in calls {
            n += estimate_tokens(&c.function.name);
            n += estimate_tokens(&c.function.arguments);
        }
    }
    n
}

fn message_text(msg: &Message) -> String {
    if let Some(c) = &msg.content {
        return c.clone();
    }
    msg.content_parts
        .as_ref()
        .map(|parts| {
            parts
                .iter()
                .filter_map(|p| p.text.as_ref())
                .cloned()
                .collect::<Vec<_>>()
                .join("\n")
        })
        .unwrap_or_default()
}

/// True when the live transcript (not system prompt docs) indicates sub-agent work.
fn subagent_spawn_in_transcript(req: &ChatCompletionRequest) -> bool {
    req.messages.iter().any(|m| {
        if m.role == Role::System {
            return false;
        }
        let t = message_text(m);
        t.contains("[Subagent Task]")
            || (m.role == Role::User
                && (t.contains("sessions_spawn(")
                    || t.contains("spawn a sub-agent")
                    || t.contains("spawn subagent")
                    || t.contains("子代理")
                    || t.contains("子 agent")))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gateway::api::openai::{Message, Role};

    #[test]
    fn consecutive_tool_errors_at_tail() {
        let messages = vec![
            Message {
                role: Role::User,
                content: Some("run".into()),
                content_parts: None,
                tool_calls: None,
                tool_call_id: None,
            },
            Message {
                role: Role::Tool,
                content: Some("Error: command failed".into()),
                content_parts: None,
                tool_calls: None,
                tool_call_id: Some("t1".into()),
            },
            Message {
                role: Role::Tool,
                content: Some("失败: exit code 1".into()),
                content_parts: None,
                tool_calls: None,
                tool_call_id: Some("t2".into()),
            },
        ];
        assert_eq!(consecutive_tool_error_tail(&messages), 2);
    }

    #[test]
    fn tool_error_streak_breaks_on_success() {
        let messages = vec![
            Message {
                role: Role::Tool,
                content: Some("ok".into()),
                content_parts: None,
                tool_calls: None,
                tool_call_id: Some("t1".into()),
            },
            Message {
                role: Role::Tool,
                content: Some("error".into()),
                content_parts: None,
                tool_calls: None,
                tool_call_id: Some("t2".into()),
            },
        ];
        assert_eq!(consecutive_tool_error_tail(&messages), 1);
    }

    #[test]
    fn casual_ignores_system_and_tools_in_token_budget() {
        let signals = RequestSignals {
            tok_system: 50_000,
            tok_tools_schema: 20_000,
            tok_rest: 50,
            tok_total_in: 70_050,
            tok_loop_delta: 0,
            tok_out_estimate: 0,
            n_tool_defs: 3,
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
            tools_enabled: true,
            had_tool_roundtrip: false,
            risky_tool_tier1: false,
            intent_hard: false,
            intent_easy: true,
            intent_plan: false,
            multimodal: false,
            consecutive_tool_error_streak: 0,
            user_rejects_answer: false,
        };
        assert!(is_casual_chat(&signals));
    }

    #[test]
    fn casual_mid_loop_with_tools_requires_easy_keyword() {
        let signals = RequestSignals {
            tok_system: 1000,
            tok_tools_schema: 500,
            tok_rest: 200,
            tok_total_in: 1700,
            tok_loop_delta: 0,
            tok_out_estimate: 0,
            n_tool_defs: 1,
            n_turns: 2,
            last_user_tok: 30,
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
            had_tool_roundtrip: false,
            risky_tool_tier1: false,
            intent_hard: false,
            intent_easy: false,
            intent_plan: false,
            multimodal: false,
            consecutive_tool_error_streak: 0,
            user_rejects_answer: false,
        };
        assert!(!is_casual_chat(&signals));
    }

    #[test]
    fn casual_mid_loop_with_tools_and_easy_keyword() {
        let mut signals = RequestSignals {
            tok_system: 1000,
            tok_tools_schema: 500,
            tok_rest: 200,
            tok_total_in: 1700,
            tok_loop_delta: 0,
            tok_out_estimate: 0,
            n_tool_defs: 1,
            n_turns: 2,
            last_user_tok: 30,
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
            had_tool_roundtrip: false,
            risky_tool_tier1: false,
            intent_hard: false,
            intent_easy: true,
            intent_plan: false,
            multimodal: false,
            consecutive_tool_error_streak: 0,
            user_rejects_answer: false,
        };
        assert!(is_casual_chat(&signals));
        signals.intent_easy = false;
        assert!(!is_casual_chat(&signals));
    }
}

fn message_has_image(msg: &Message) -> bool {
    msg.content_parts
        .as_ref()
        .is_some_and(|p| p.iter().any(|part| part.image_url.is_some()))
}
