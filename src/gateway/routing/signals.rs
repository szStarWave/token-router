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
    /// Pending tool calls delete/move files — hard gate to cloud.
    pub risky_tool_hard: bool,
    /// Pending tool calls include soft-tier tools (browser/message/sessions_spawn).
    pub risky_tool_soft: bool,
    pub risky_tool_names: Vec<String>,
    pub risky_tool_hard_names: Vec<String>,
    pub risky_tool_soft_names: Vec<String>,
    pub intent_hard: bool,
    pub intent_easy: bool,
    pub intent_plan: bool,
    /// Cloud/edge tier preference within the recent user-turn window (see `resolve_recent_tier_intent`).
    pub intent_cloud: bool,
    /// Latest user message or high max_tokens implies lengthy generation.
    pub intent_long_gen: bool,
    /// Edge tier preference within the recent user-turn window (see `resolve_recent_tier_intent`).
    pub intent_edge: bool,
    /// Any message in the request carries image content.
    pub multimodal: bool,
    /// Latest user message carries image content (highest weight for difficulty).
    pub user_multimodal: bool,
    /// Trailing consecutive `role=tool` messages whose content matches error keywords.
    pub consecutive_tool_error_streak: u32,
    /// `role=tool` messages after the last `role=user` in the transcript.
    pub tool_invocations_since_last_user: u32,
    /// Latest user message rejects the immediately preceding assistant reply.
    pub user_rejects_answer: bool,
    /// Statistical rare-word signal from the latest user message.
    pub rare_lexical: bool,
    /// Domain-specific lexical signal from keyword tables.
    pub special_lexical: bool,
    /// Ratio of rare tokens in the latest user message (0.0–1.0).
    pub rare_token_ratio: f32,
    pub intent_analysis: bool,
    pub intent_decision: bool,
    pub intent_research: bool,
}

/// Consecutive trailing tool errors before difficulty bias / reason codes.
pub const TOOL_ERROR_STREAK_DIFFICULTY: u32 = 1;
/// Consecutive trailing tool errors before Recovery step kind and hard cloud gate.
pub const TOOL_ERROR_STREAK_ESCALATE: u32 = 3;

/// Cognitive difficulty applies only to the first routing hop after the triggering user
/// message — not to mid-loop tool/assistant follow-ups.
pub fn cognitive_task_applies(signals: &RequestSignals) -> bool {
    signals.tool_invocations_since_last_user == 0 && !signals.pending_tool_calls
}

fn cognitive_intent_blocks_casual(signals: &RequestSignals) -> bool {
    if !cognitive_task_applies(signals) {
        return false;
    }
    signals.intent_plan
        || signals.intent_analysis
        || signals.intent_decision
        || signals.intent_research
}

/// Short, tool-loop-free turn (daily chat). Tool definitions in the request are ignored.
pub fn is_casual_chat(signals: &RequestSignals) -> bool {
    if signals.had_tool_roundtrip
        || signals.pending_tool_calls
        || signals.intent_hard
        || signals.assistant_failed_recent
        || cognitive_intent_blocks_casual(signals)
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

pub struct SignalExtractor<'a> {
    pub ctx_edge_max: u32,
    pub wordfreq: &'a super::WordFreqStore,
}

impl SignalExtractor<'_> {
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
        let mut user_multimodal = false;

        let last_user_idx = req
            .messages
            .iter()
            .rposition(|m| m.role == Role::User);

        for (i, msg) in req.messages.iter().enumerate() {
            let t = if msg.role == Role::User {
                user_message_tokens(msg, Some(i) == last_user_idx)
            } else {
                estimate_message_tokens(msg)
            };
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
                if msg.role == Role::User {
                    user_multimodal = true;
                }
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
            m.role == Role::User && user_routing_text(m).trim() == "[OpenClaw heartbeat poll]"
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

        let memory_compact_hint = memory_compact_in_transcript(req);

        let cron_background = req.messages.iter().any(|m| {
            let t = message_text(m).to_ascii_lowercase();
            t.contains("[cron]") || t.contains("cron background") || t.contains("cron job")
        });

        let loop_steps = req
            .messages
            .iter()
            .filter(|m| m.role == Role::Assistant)
            .count() as u32;

        let last_user = req
            .messages
            .iter()
            .rev()
            .find(|m| m.role == Role::User);

        let intent_hard = last_user
            .map(user_routing_text)
            .is_some_and(|t| super::keywords::contains_hard_intent(&t));
        let intent_easy = last_user
            .map(user_routing_text)
            .is_some_and(|t| super::keywords::contains_easy_intent(&t));
        let intent_plan = last_user
            .map(user_routing_text)
            .is_some_and(|t| super::keywords::contains_plan_intent(&t));
        let intent_analysis = last_user
            .map(user_routing_text)
            .is_some_and(|t| super::cognitive_intent::contains_analysis_intent(&t));
        let intent_decision = last_user
            .map(user_routing_text)
            .is_some_and(|t| super::cognitive_intent::contains_decision_intent(&t));
        let intent_research = last_user
            .map(user_routing_text)
            .is_some_and(|t| super::cognitive_intent::contains_research_intent(&t));
        let tier_intent = resolve_recent_tier_intent(&req.messages);
        let intent_cloud = tier_intent == TierIntent::Cloud;
        let intent_long_gen = super::keywords::long_gen_from_max_tokens(
            req.max_tokens,
            req.max_completion_tokens,
        ) || last_user
            .map(user_routing_text)
            .is_some_and(|t| super::keywords::contains_long_gen_intent(&t));
        let intent_edge = tier_intent == TierIntent::Edge;

        let last_user_text = last_user
            .map(user_routing_text)
            .unwrap_or_default();
        let lexical = super::lexical::analyze_lexical(&last_user_text, self.wordfreq);

        let last_user_tok = last_user
            .map(|m| estimate_tokens(&user_routing_text(m)))
            .unwrap_or(0);

        let consecutive_tool_error_streak =
            consecutive_tool_errors_since_last_user(&req.messages);

        let tool_invocations_since_last_user =
            count_tool_invocations_since_last_user(&req.messages);

        let user_rejects_answer = req.messages.len() >= 2
            && req.messages[req.messages.len() - 1].role == Role::User
            && req.messages[req.messages.len() - 2].role == Role::Assistant
            && super::reject_intent::contains_reject_intent(&user_routing_text(
                &req.messages[req.messages.len() - 1],
            ));

        let tool_risk = prev_assistant
            .and_then(|m| m.tool_calls.as_ref())
            .map(|calls| super::tool_risk::assess_tool_calls(calls))
            .unwrap_or_default();

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
            risky_tool_hard: tool_risk.risky_tool_hard,
            risky_tool_soft: tool_risk.risky_tool_soft,
            risky_tool_names: tool_risk.risky_tool_names,
            risky_tool_hard_names: tool_risk.risky_tool_hard_names,
            risky_tool_soft_names: tool_risk.risky_tool_soft_names,
            intent_hard,
            intent_easy,
            intent_plan,
            intent_cloud,
            intent_long_gen,
            intent_edge,
            multimodal,
            user_multimodal,
            consecutive_tool_error_streak,
            tool_invocations_since_last_user,
            user_rejects_answer,
            rare_lexical: lexical.rare_lexical,
            special_lexical: lexical.special_lexical,
            rare_token_ratio: lexical.rare_token_ratio,
            intent_analysis,
            intent_decision,
            intent_research,
        }
    }
}

/// Count `role=tool` messages after the last `role=user` in the transcript.
pub fn count_tool_invocations_since_last_user(messages: &[Message]) -> u32 {
    match messages.iter().rposition(|m| m.role == Role::User) {
        None => messages
            .iter()
            .filter(|m| m.role == Role::Tool)
            .count() as u32,
        Some(i) => messages[i + 1..]
            .iter()
            .filter(|m| m.role == Role::Tool)
            .count() as u32,
    }
}

/// Trailing consecutive tool errors after the last `role=user` message.
pub fn consecutive_tool_errors_since_last_user(messages: &[Message]) -> u32 {
    let start = match messages.iter().rposition(|m| m.role == Role::User) {
        None => 0,
        Some(i) => i + 1,
    };
    consecutive_tool_error_tail(&messages[start..])
}

/// Count trailing tool messages (from the end) that contain error keywords.
pub fn consecutive_tool_error_tail(messages: &[Message]) -> u32 {
    let mut count = 0u32;
    for msg in messages.iter().rev() {
        if msg.role != Role::Tool {
            break;
        }
        if super::keywords::tool_result_has_error(&message_text(msg)) {
            count += 1;
        } else {
            break;
        }
    }
    count
}

pub fn tool_result_has_error(text: &str) -> bool {
    super::keywords::tool_result_has_error(text)
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

/// OpenClaw-injected wrappers stripped for routing signals only (not upstream).
/// Only the latest user turn is stripped (once at the front); older user messages
/// are kept verbatim so pasted metadata/timestamps in history are not removed.
pub fn user_routing_text(msg: &Message) -> String {
    if msg.role != Role::User {
        return message_text(msg);
    }
    strip_openclaw_user_wrappers(&message_text(msg))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TierIntent {
    None,
    Cloud,
    Edge,
}

/// Recent user turns scanned for cloud/edge tier preference (快速/云端/端侧等).
pub const TIER_INTENT_RECENT_USER_TURNS: usize = 4;

/// Within the last [`TIER_INTENT_RECENT_USER_TURNS`] user messages, walk newest-first and
/// apply the latest explicit cloud/edge preference (neutral turns in between still inherit).
/// Older preferences fall outside the window and reset.
pub fn resolve_recent_tier_intent(messages: &[Message]) -> TierIntent {
    resolve_recent_tier_intent_with_limit(messages, TIER_INTENT_RECENT_USER_TURNS)
}

pub fn resolve_recent_tier_intent_with_limit(
    messages: &[Message],
    max_user_turns: usize,
) -> TierIntent {
    for msg in messages
        .iter()
        .filter(|m| m.role == Role::User)
        .rev()
        .take(max_user_turns)
    {
        let text = user_routing_text(msg);
        let cloud = super::keywords::contains_cloud_intent(&text);
        let edge = super::keywords::contains_edge_intent(&text);
        if cloud || edge {
            if cloud {
                return TierIntent::Cloud;
            }
            return TierIntent::Edge;
        }
    }
    TierIntent::None
}

fn user_message_tokens(msg: &Message, strip_wrappers: bool) -> u32 {
    if strip_wrappers {
        estimate_tokens(&user_routing_text(msg))
    } else {
        estimate_message_tokens(msg)
    }
}

const SENDER_UNTRUSTED_METADATA: &str = "Sender (untrusted metadata):";

/// Strip OpenClaw wrappers once at the message front: optional `Sender (untrusted metadata)`
/// block, then optional inbound `[Mon YYYY-MM-DD HH:MM GMT±N]` prefix. Content after that
/// is never touched (e.g. user-pasted metadata/timestamps later in the message).
pub fn strip_openclaw_user_wrappers(text: &str) -> String {
    let mut s = text.trim();
    if let Some(prefix) = s.get(..SENDER_UNTRUSTED_METADATA.len()) {
        if prefix.eq_ignore_ascii_case(SENDER_UNTRUSTED_METADATA) {
            s = s
                .get(SENDER_UNTRUSTED_METADATA.len()..)
                .unwrap_or("")
                .trim_start();
            if s.starts_with("```") {
                let mut i = 3;
                while i < s.len() && s.as_bytes()[i] != b'\n' {
                    i += 1;
                }
                if i < s.len() && s.as_bytes()[i] == b'\n' {
                    i += 1;
                }
                if let Some(close) = s[i..].find("```") {
                    s = s[i + close + 3..].trim_start();
                }
            }
        }
    }
    strip_openclaw_timestamp_prefix(s).trim().to_string()
}

fn strip_openclaw_timestamp_prefix(s: &str) -> &str {
    let s = s.trim_start();
    if !s.starts_with('[') {
        return s;
    }
    let Some(close) = s.find(']') else {
        return s;
    };
    if looks_like_openclaw_timestamp(&s[1..close]) {
        return s[close + 1..].trim_start();
    }
    s
}

fn looks_like_openclaw_timestamp(inner: &str) -> bool {
    let parts: Vec<&str> = inner.split_whitespace().collect();
    if parts.len() < 4 {
        return false;
    }
    parts[0].len() == 3
        && parts[0].chars().all(|c| c.is_ascii_alphabetic())
        && parts[1].len() == 10
        && parts[1].chars().nth(4).is_some_and(|c| c == '-')
        && parts[1].chars().nth(7).is_some_and(|c| c == '-')
        && parts[2].contains(':')
        && parts[3].to_ascii_uppercase().starts_with("GMT")
}

/// Latest user message text for lexical learning context.
pub fn last_user_message_text(req: &ChatCompletionRequest) -> String {
    req.messages
        .iter()
        .rev()
        .find(|m| m.role == Role::User)
        .map(user_routing_text)
        .unwrap_or_default()
}

/// Latest message text in the transcript (any role), for routing log preview.
pub fn last_message_text(req: &ChatCompletionRequest) -> String {
    req.messages
        .iter()
        .rev()
        .find_map(|m| {
            let text = message_text(m);
            if text.trim().is_empty() {
                None
            } else {
                Some(text)
            }
        })
        .unwrap_or_default()
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

fn transcript_text(m: &Message) -> String {
    message_text(m)
}

/// True when the live transcript (not static system prompt) indicates memory compaction.
fn memory_compact_in_transcript(req: &ChatCompletionRequest) -> bool {
    req.messages.iter().any(|m| {
        if m.role == Role::System {
            return false;
        }
        let lower = transcript_text(m).to_ascii_lowercase();
        lower.contains("[openclaw memory compact]")
            || (matches!(m.role, Role::User | Role::Assistant)
                && (lower.contains("memory compaction") || lower.contains("compaction")))
    })
}

/// True when the live transcript (not system prompt docs) indicates sub-agent work.
fn subagent_spawn_in_transcript(req: &ChatCompletionRequest) -> bool {
    req.messages.iter().any(|m| {
        if m.role == Role::System {
            return false;
        }
        let t = transcript_text(m);
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
        assert!(is_casual_chat(&signals));
    }

    #[test]
    fn tool_invocations_since_last_user_counts_after_user() {
        let messages = vec![
            Message {
                role: Role::User,
                content: Some("go".into()),
                content_parts: None,
                tool_calls: None,
                tool_call_id: None,
            },
            Message {
                role: Role::Tool,
                content: Some("a".into()),
                content_parts: None,
                tool_calls: None,
                tool_call_id: Some("t1".into()),
            },
            Message {
                role: Role::Tool,
                content: Some("b".into()),
                content_parts: None,
                tool_calls: None,
                tool_call_id: Some("t2".into()),
            },
        ];
        assert_eq!(count_tool_invocations_since_last_user(&messages), 2);
    }

    #[test]
    fn tool_invocations_without_user_counts_all_tools() {
        let messages = vec![
            Message {
                role: Role::Tool,
                content: Some("a".into()),
                content_parts: None,
                tool_calls: None,
                tool_call_id: Some("t1".into()),
            },
            Message {
                role: Role::Tool,
                content: Some("b".into()),
                content_parts: None,
                tool_calls: None,
                tool_call_id: Some("t2".into()),
            },
        ];
        assert_eq!(count_tool_invocations_since_last_user(&messages), 2);
    }

    #[test]
    fn tool_invocations_ignores_tools_before_last_user() {
        let messages = vec![
            Message {
                role: Role::User,
                content: Some("first".into()),
                content_parts: None,
                tool_calls: None,
                tool_call_id: None,
            },
            Message {
                role: Role::Tool,
                content: Some("old".into()),
                content_parts: None,
                tool_calls: None,
                tool_call_id: Some("t0".into()),
            },
            Message {
                role: Role::User,
                content: Some("second".into()),
                content_parts: None,
                tool_calls: None,
                tool_call_id: None,
            },
            Message {
                role: Role::Tool,
                content: Some("new".into()),
                content_parts: None,
                tool_calls: None,
                tool_call_id: Some("t1".into()),
            },
        ];
        assert_eq!(count_tool_invocations_since_last_user(&messages), 1);
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
        assert!(is_casual_chat(&signals));
        signals.intent_easy = false;
        assert!(!is_casual_chat(&signals));
    }

    #[test]
    fn intent_keywords_only_from_last_user_message() {
        use crate::gateway::routing::WordFreqStore;
        use std::sync::LazyLock;

        static WF: LazyLock<WordFreqStore> =
            LazyLock::new(|| WordFreqStore::open_in_memory().expect("wordfreq"));

        let extractor = SignalExtractor {
            ctx_edge_max: 65536,
            wordfreq: &WF,
        };
        let neutral_user = ChatCompletionRequest {
            model: "test".into(),
            messages: vec![
                Message {
                    role: Role::User,
                    content: Some("what about item two".into()),
                    content_parts: None,
                    tool_calls: None,
                    tool_call_id: None,
                },
                Message {
                    role: Role::Tool,
                    content: Some("debug this code and fix the bug".into()),
                    content_parts: None,
                    tool_calls: None,
                    tool_call_id: Some("t1".into()),
                },
            ],
            tools: vec![],
            stream: false,
            ..Default::default()
        };
        let signals = extractor.extract(&neutral_user, None);
        assert!(
            !signals.intent_hard,
            "hard intent must not come from tool result"
        );
        assert!(!signals.intent_easy);

        let hard_user = ChatCompletionRequest {
            model: "test".into(),
            messages: vec![Message {
                role: Role::User,
                content: Some("please debug this module".into()),
                content_parts: None,
                tool_calls: None,
                tool_call_id: None,
            }],
            tools: vec![],
            stream: false,
            ..Default::default()
        };
        let hard_signals = extractor.extract(&hard_user, None);
        assert!(hard_signals.intent_hard);
    }

    #[test]
    fn dynamic_project_context_in_system_is_not_memory_compact() {
        use crate::gateway::routing::WordFreqStore;
        use std::sync::LazyLock;

        static WF: LazyLock<WordFreqStore> =
            LazyLock::new(|| WordFreqStore::open_in_memory().expect("wordfreq"));

        let extractor = SignalExtractor {
            ctx_edge_max: 262_144,
            wordfreq: &WF,
        };
        let req = ChatCompletionRequest {
            model: "auto".into(),
            messages: vec![
                Message {
                    role: Role::System,
                    content: Some(
                        "OpenClaw agent.\n# Dynamic Project Context\nHEARTBEAT.md content".into(),
                    ),
                    content_parts: None,
                    tool_calls: None,
                    tool_call_id: None,
                },
                Message {
                    role: Role::User,
                    content: Some("你好".into()),
                    content_parts: None,
                    tool_calls: None,
                    tool_call_id: None,
                },
            ],
            tools: vec![],
            stream: false,
            ..Default::default()
        };
        let signals = extractor.extract(&req, None);
        assert!(
            !signals.memory_compact_hint,
            "static OpenClaw cache marker must not imply memory compact"
        );
    }

    #[test]
    fn memory_compact_marker_in_user_message_is_detected() {
        use crate::gateway::routing::WordFreqStore;
        use std::sync::LazyLock;

        static WF: LazyLock<WordFreqStore> =
            LazyLock::new(|| WordFreqStore::open_in_memory().expect("wordfreq"));

        let extractor = SignalExtractor {
            ctx_edge_max: 262_144,
            wordfreq: &WF,
        };
        let req = ChatCompletionRequest {
            model: "auto".into(),
            messages: vec![Message {
                role: Role::User,
                content: Some("[OpenClaw memory compact] summarize session".into()),
                content_parts: None,
                tool_calls: None,
                tool_call_id: None,
            }],
            tools: vec![],
            stream: false,
            ..Default::default()
        };
        let signals = extractor.extract(&req, None);
        assert!(signals.memory_compact_hint);
    }

    #[test]
    fn strip_only_once_at_front_preserves_body_timestamp() {
        let text = "[Mon 2026-07-06 16:18 GMT+8] 你好\n\n参考：[Mon 2026-07-07 10:00 GMT+8] 另一段";
        assert_eq!(
            strip_openclaw_user_wrappers(text),
            "你好\n\n参考：[Mon 2026-07-07 10:00 GMT+8] 另一段"
        );
    }

    #[test]
    fn historical_user_message_not_stripped_for_tokens() {
        use crate::gateway::routing::WordFreqStore;
        use std::sync::LazyLock;

        static WF: LazyLock<WordFreqStore> =
            LazyLock::new(|| WordFreqStore::open_in_memory().expect("wordfreq"));

        let wrapped = "Sender (untrusted metadata):\n```json\n{}\n```\n\n[Mon 2026-07-06 16:18 GMT+8] 旧问题";
        let latest = "Sender (untrusted metadata):\n```json\n{}\n```\n\n[Mon 2026-07-06 16:19 GMT+8] 新问题";
        let extractor = SignalExtractor {
            ctx_edge_max: 65536,
            wordfreq: &WF,
        };
        let req = ChatCompletionRequest {
            model: "auto".into(),
            messages: vec![
                Message {
                    role: Role::User,
                    content: Some(wrapped.into()),
                    content_parts: None,
                    tool_calls: None,
                    tool_call_id: None,
                },
                Message {
                    role: Role::Assistant,
                    content: Some("好的".into()),
                    content_parts: None,
                    tool_calls: None,
                    tool_call_id: None,
                },
                Message {
                    role: Role::User,
                    content: Some(latest.into()),
                    content_parts: None,
                    tool_calls: None,
                    tool_call_id: None,
                },
            ],
            tools: vec![],
            stream: false,
            ..Default::default()
        };
        let signals = extractor.extract(&req, None);
        assert_eq!(last_user_message_text(&req), "新问题");
        let hist_tok = estimate_message_tokens(&Message {
            role: Role::User,
            content: Some(wrapped.into()),
            content_parts: None,
            tool_calls: None,
            tool_call_id: None,
        });
        let latest_stripped_tok = estimate_tokens("新问题");
        let assistant_tok = estimate_tokens("好的");
        assert!(
            hist_tok > latest_stripped_tok,
            "historical wrapped user text should count full size"
        );
        assert_eq!(
            signals.tok_rest,
            hist_tok + assistant_tok + latest_stripped_tok,
            "only the latest user turn is stripped; historical wrapper stays in tok_rest"
        );
    }

    #[test]
    fn openclaw_wrapped_user_strips_before_lexical() {
        use crate::gateway::routing::WordFreqStore;
        use std::sync::LazyLock;

        static WF: LazyLock<WordFreqStore> =
            LazyLock::new(|| WordFreqStore::open_in_memory().expect("wordfreq"));

        let extractor = SignalExtractor {
            ctx_edge_max: 50_000,
            wordfreq: &WF,
        };
        let wrapped = "Sender (untrusted metadata):\n```json\n{\n  \"label\": \"FlowyAIPC (gateway-client)\",\n  \"id\": \"gateway-client\"\n}\n```\n\n[Mon 2026-07-06 16:18 GMT+8] 你能干什么？";
        let req = ChatCompletionRequest {
            model: "auto".into(),
            messages: vec![Message {
                role: Role::User,
                content: Some(wrapped.into()),
                content_parts: None,
                tool_calls: None,
                tool_call_id: None,
            }],
            tools: vec![],
            stream: false,
            ..Default::default()
        };
        let signals = extractor.extract(&req, None);
        assert!(!signals.rare_lexical, "metadata JSON must not inflate lexical rarity");
        assert!(signals.last_user_tok < 20, "stripped user tok: {}", signals.last_user_tok);
    }

    #[test]
    fn openclaw_untrusted_metadata_stripped_text() {
        let wrapped = "Sender (untrusted metadata):\n```json\n{\n  \"label\": \"FlowyAIPC (gateway-client)\",\n  \"id\": \"gateway-client\",\n  \"name\": \"FlowyAIPC\",\n  \"username\": \"FlowyAIPC\"\n}\n```\n\n[Mon 2026-07-06 16:18 GMT+8] 你能干什么？";
        assert_eq!(strip_openclaw_user_wrappers(wrapped), "你能干什么？");
    }

    #[test]
    fn openclaw_timestamp_only_stripped() {
        assert_eq!(
            strip_openclaw_user_wrappers("[Mon 2026-07-06 16:18 GMT+8] 你好"),
            "你好"
        );
    }

    #[test]
    fn plain_user_text_unchanged() {
        assert_eq!(strip_openclaw_user_wrappers("你能干什么？"), "你能干什么？");
    }

    #[test]
    fn recent_tier_intent_latest_edge_overrides_earlier_cloud() {
        let messages = vec![
            Message {
                role: Role::User,
                content: Some("用云端快速回答".into()),
                content_parts: None,
                tool_calls: None,
                tool_call_id: None,
            },
            Message {
                role: Role::Assistant,
                content: Some("好的".into()),
                content_parts: None,
                tool_calls: None,
                tool_call_id: None,
            },
            Message {
                role: Role::User,
                content: Some("用端侧省积分".into()),
                content_parts: None,
                tool_calls: None,
                tool_call_id: None,
            },
        ];
        assert_eq!(resolve_recent_tier_intent(&messages), TierIntent::Edge);
    }

    #[test]
    fn recent_tier_intent_latest_cloud_overrides_earlier_edge() {
        let messages = vec![
            Message {
                role: Role::User,
                content: Some("走端侧".into()),
                content_parts: None,
                tool_calls: None,
                tool_call_id: None,
            },
            Message {
                role: Role::Assistant,
                content: Some("ok".into()),
                content_parts: None,
                tool_calls: None,
                tool_call_id: None,
            },
            Message {
                role: Role::User,
                content: Some("太慢了，用云端".into()),
                content_parts: None,
                tool_calls: None,
                tool_call_id: None,
            },
        ];
        assert_eq!(resolve_recent_tier_intent(&messages), TierIntent::Cloud);
    }

    #[test]
    fn recent_tier_intent_carries_forward_within_window() {
        let messages = vec![
            Message {
                role: Role::User,
                content: Some("用云端".into()),
                content_parts: None,
                tool_calls: None,
                tool_call_id: None,
            },
            Message {
                role: Role::Assistant,
                content: Some("好的".into()),
                content_parts: None,
                tool_calls: None,
                tool_call_id: None,
            },
            Message {
                role: Role::User,
                content: Some("帮我写个函数".into()),
                content_parts: None,
                tool_calls: None,
                tool_call_id: None,
            },
        ];
        assert_eq!(resolve_recent_tier_intent(&messages), TierIntent::Cloud);
    }

    #[test]
    fn recent_tier_intent_resets_outside_window() {
        let mut messages = Vec::new();
        for i in 0..6 {
            messages.push(Message {
                role: Role::User,
                content: Some(format!("闲聊 {i}")),
                content_parts: None,
                tool_calls: None,
                tool_call_id: None,
            });
            messages.push(Message {
                role: Role::Assistant,
                content: Some("嗯".into()),
                content_parts: None,
                tool_calls: None,
                tool_call_id: None,
            });
        }
        messages[0] = Message {
            role: Role::User,
            content: Some("用云端".into()),
            content_parts: None,
            tool_calls: None,
            tool_call_id: None,
        };
        assert_eq!(resolve_recent_tier_intent(&messages), TierIntent::None);
    }

    #[test]
    fn consecutive_tool_errors_since_last_user_ignores_prior_user() {
        let messages = vec![
            Message {
                role: Role::User,
                content: Some("first".into()),
                content_parts: None,
                tool_calls: None,
                tool_call_id: None,
            },
            Message {
                role: Role::Tool,
                content: Some("Error: old failure".into()),
                content_parts: None,
                tool_calls: None,
                tool_call_id: Some("t0".into()),
            },
            Message {
                role: Role::User,
                content: Some("retry".into()),
                content_parts: None,
                tool_calls: None,
                tool_call_id: None,
            },
            Message {
                role: Role::Tool,
                content: Some("Error: new failure".into()),
                content_parts: None,
                tool_calls: None,
                tool_call_id: Some("t1".into()),
            },
        ];
        assert_eq!(consecutive_tool_errors_since_last_user(&messages), 1);
    }

    #[test]
    fn consecutive_tool_errors_since_last_user_success_breaks_streak() {
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
                content: Some("Error: failed once".into()),
                content_parts: None,
                tool_calls: None,
                tool_call_id: Some("t1".into()),
            },
            Message {
                role: Role::Tool,
                content: Some("ok".into()),
                content_parts: None,
                tool_calls: None,
                tool_call_id: Some("t2".into()),
            },
            Message {
                role: Role::Tool,
                content: Some("Error: failed again".into()),
                content_parts: None,
                tool_calls: None,
                tool_call_id: Some("t3".into()),
            },
        ];
        assert_eq!(consecutive_tool_errors_since_last_user(&messages), 1);
    }

    #[test]
    fn cognitive_intent_only_from_last_user_message() {
        use crate::gateway::routing::WordFreqStore;
        use std::sync::LazyLock;

        static WF: LazyLock<WordFreqStore> =
            LazyLock::new(|| WordFreqStore::open_in_memory().expect("wordfreq"));

        let extractor = SignalExtractor {
            ctx_edge_max: 65536,
            wordfreq: &WF,
        };
        let req = ChatCompletionRequest {
            model: "test".into(),
            messages: vec![
                Message {
                    role: Role::User,
                    content: Some("分析一下旧话题".into()),
                    content_parts: None,
                    tool_calls: None,
                    tool_call_id: None,
                },
                Message {
                    role: Role::Assistant,
                    content: Some("好的".into()),
                    content_parts: None,
                    tool_calls: None,
                    tool_call_id: None,
                },
                Message {
                    role: Role::User,
                    content: Some("继续".into()),
                    content_parts: None,
                    tool_calls: None,
                    tool_call_id: None,
                },
            ],
            tools: vec![],
            stream: false,
            ..Default::default()
        };
        let signals = extractor.extract(&req, None);
        assert!(!signals.intent_analysis);
        assert!(!signals.intent_decision);
        assert!(!signals.intent_research);
    }

    #[test]
    fn openclaw_heartbeat_poll_with_metadata_wrapper() {
        use crate::gateway::routing::WordFreqStore;
        use std::sync::LazyLock;

        static WF: LazyLock<WordFreqStore> =
            LazyLock::new(|| WordFreqStore::open_in_memory().expect("wordfreq"));

        let extractor = SignalExtractor {
            ctx_edge_max: 65536,
            wordfreq: &WF,
        };
        let wrapped = format!(
            "Sender (untrusted metadata):\n```json\n{{\"id\":\"gateway-client\"}}\n```\n\n[Mon 2026-07-06 16:18 GMT+8] [OpenClaw heartbeat poll]"
        );
        let req = ChatCompletionRequest {
            model: "auto".into(),
            messages: vec![Message {
                role: Role::User,
                content: Some(wrapped),
                content_parts: None,
                tool_calls: None,
                tool_call_id: None,
            }],
            tools: vec![],
            stream: false,
            ..Default::default()
        };
        let signals = extractor.extract(&req, None);
        assert!(signals.is_heartbeat_poll);
    }

    #[test]
    fn last_message_text_uses_latest_non_empty_any_role() {
        let req = ChatCompletionRequest {
            model: "test".into(),
            messages: vec![
                Message {
                    role: Role::User,
                    content: Some("Sender (untrusted metadata): ...".into()),
                    content_parts: None,
                    tool_calls: None,
                    tool_call_id: None,
                },
                Message {
                    role: Role::Assistant,
                    content: Some("Here is the summary you asked for.".into()),
                    content_parts: None,
                    tool_calls: None,
                    tool_call_id: None,
                },
            ],
            tools: vec![],
            stream: false,
            ..Default::default()
        };
        assert_eq!(
            last_message_text(&req),
            "Here is the summary you asked for."
        );
        assert_eq!(last_user_message_text(&req), "...");
    }
}

fn message_has_image(msg: &Message) -> bool {
    msg.content_parts
        .as_ref()
        .is_some_and(|p| p.iter().any(|part| part.image_url.is_some()))
}
