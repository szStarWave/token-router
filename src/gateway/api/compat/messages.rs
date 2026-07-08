use serde_json::{json, Value};

use crate::gateway::api::compat::reasoning::{
    apply_reasoning_options_to_chat_request, apply_reasoning_to_upstream_messages,
};
use crate::gateway::api::openai::{ChatCompletionRequest, Message, Role};

fn message_has_visible_content(msg: &Message) -> bool {
    msg.content.as_deref().is_some_and(|s| !s.trim().is_empty())
        || msg
            .reasoning_content
            .as_deref()
            .is_some_and(|s| !s.trim().is_empty())
}

/// Strip unfulfilled tool_calls from a trailing assistant message so upstream
/// providers accept the transcript (agent clients often include in-flight turns).
pub fn normalize_trailing_assistant_tool_calls(messages: &mut Vec<Message>) {
    let Some(last) = messages.last_mut() else {
        return;
    };
    if last.role != Role::Assistant {
        return;
    }
    if !last
        .tool_calls
        .as_ref()
        .is_some_and(|calls| !calls.is_empty())
    {
        return;
    }
    last.tool_calls = None;
    if !message_has_visible_content(last) {
        messages.pop();
    }
}

pub fn collapse_system_messages(messages: Vec<Value>) -> Vec<Value> {
    let mut systems = Vec::new();
    let mut others = Vec::new();
    for msg in messages {
        if msg.get("role").and_then(|v| v.as_str()) == Some("system") {
            if let Some(text) = message_text(&msg) {
                if !text.is_empty() {
                    systems.push(text);
                }
            }
        } else {
            others.push(msg);
        }
    }
    if systems.is_empty() {
        return others;
    }
    let mut out = vec![json!({"role": "system", "content": systems.join("\n\n")})];
    out.extend(others);
    out
}

fn message_text(msg: &Value) -> Option<String> {
    match msg.get("content") {
        Some(Value::String(s)) => Some(s.clone()),
        Some(Value::Array(parts)) => {
            let text: Vec<_> = parts
                .iter()
                .filter_map(|p| p.get("text").and_then(|t| t.as_str()))
                .collect();
            if text.is_empty() {
                None
            } else {
                Some(text.join("\n"))
            }
        }
        _ => None,
    }
}

pub fn finalize_chat_request_value(mut result: serde_json::Map<String, Value>, model: &str) -> Value {
    let has_tools = result
        .get("tools")
        .and_then(|v| v.as_array())
        .is_some_and(|tools| !tools.is_empty());
    if !has_tools {
        result.remove("tool_choice");
    }
    result.remove("parallel_tool_calls");
    if let Some(Value::Array(messages)) = result.get_mut("messages") {
        *messages = collapse_system_messages(std::mem::take(messages));
        crate::gateway::api::compat::reasoning::backfill_tool_call_reasoning_placeholders(messages);
    }
    let _ = model;
    Value::Object(result)
}

pub fn finalize_upstream_request(
    mut req: ChatCompletionRequest,
    upstream_base: Option<&str>,
) -> ChatCompletionRequest {
    req.store = None;
    if req.stream {
        req.stream_options = Some(serde_json::json!({"include_usage": true}));
    } else {
        req.stream_options = None;
    }

    let has_tools = !req.tools.is_empty();
    if !has_tools {
        req.tool_choice = None;
    }
    // parallel_tool_calls is a Responses API concept, not standard in Chat Completions
    req.parallel_tool_calls = None;

    let msgs: Vec<_> = req
        .messages
        .iter()
        .map(Message::normalized_for_upstream)
        .collect();

    let (systems, others): (Vec<_>, Vec<_>) =
        msgs.into_iter().partition(|m| m.role == Role::System);

    let mut merged = Vec::with_capacity(1 + others.len());
    if !systems.is_empty() {
        let content = systems
            .iter()
            .filter_map(|m| m.content.as_deref())
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
            .join("\n\n");
        merged.push(Message {
            role: Role::System,
            content: if content.is_empty() { None } else { Some(content) },
            content_parts: None,
            tool_calls: None,
            tool_call_id: None,
            reasoning_content: None,
        });
    }
    merged.extend(others);
    req.messages = merged;

    apply_reasoning_to_upstream_messages(&mut req.messages, &req.model, upstream_base);
    normalize_trailing_assistant_tool_calls(&mut req.messages);
    apply_reasoning_options_to_chat_request(&mut req, upstream_base);
    req
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gateway::api::openai::{FunctionCallPayload, ToolCall};

    #[test]
    fn merges_multiple_system_messages() {
        let msgs = vec![
            json!({"role": "system", "content": "a"}),
            json!({"role": "user", "content": "hi"}),
            json!({"role": "system", "content": "b"}),
        ];
        let out = collapse_system_messages(msgs);
        assert_eq!(out.len(), 2);
        assert_eq!(out[0]["content"], "a\n\nb");
    }

    #[test]
    fn drops_tool_choice_without_tools() {
        let mut map = serde_json::Map::new();
        map.insert("model".into(), json!("m"));
        map.insert("tool_choice".into(), json!("auto"));
        map.insert("messages".into(), json!([]));
        let value = finalize_chat_request_value(map, "m");
        assert!(value.get("tool_choice").is_none());
    }

    #[test]
    fn upstream_finalize_adds_reasoning_for_deepseek_tool_calls() {
        let req = ChatCompletionRequest {
            model: "deepseek-v4-flash".into(),
            messages: vec![Message {
                role: Role::Assistant,
                content: None,
                content_parts: None,
                tool_calls: Some(vec![ToolCall {
                    id: "call_1".into(),
                    call_type: "function".into(),
                    function: FunctionCallPayload {
                        name: "read".into(),
                        arguments: "{}".into(),
                    },
                }]),
                tool_call_id: None,
                reasoning_content: None,
            }],
            ..Default::default()
        };
        let out = finalize_upstream_request(req, Some("https://api.deepseek.com/v1"));
        assert_eq!(
            out.messages[0].reasoning_content.as_deref(),
            Some("tool call")
        );
        assert_eq!(out.thinking, Some(json!({"type": "enabled"})));
        assert_eq!(out.reasoning_effort.as_deref(), Some("high"));
    }

    #[test]
    fn upstream_finalize_enables_thinking_for_reasoning_content() {
        let req = ChatCompletionRequest {
            model: "deepseek-v4-flash".into(),
            messages: vec![
                Message {
                    role: Role::Assistant,
                    content: Some("answer".into()),
                    content_parts: None,
                    tool_calls: None,
                    tool_call_id: None,
                    reasoning_content: Some("prior reasoning".into()),
                },
                Message {
                    role: Role::User,
                    content: Some("follow-up".into()),
                    content_parts: None,
                    tool_calls: None,
                    tool_call_id: None,
                    reasoning_content: None,
                },
            ],
            ..Default::default()
        };
        let out = finalize_upstream_request(req, Some("https://api.deepseek.com/v1"));
        assert_eq!(out.thinking, Some(json!({"type": "enabled"})));
        assert_eq!(out.reasoning_effort.as_deref(), Some("high"));
    }

    #[test]
    fn strips_trailing_assistant_tool_calls_keeps_content() {
        let mut msgs = vec![
            Message {
                role: Role::User,
                content: Some("hi".into()),
                content_parts: None,
                tool_calls: None,
                tool_call_id: None,
                reasoning_content: None,
            },
            Message {
                role: Role::Assistant,
                content: Some("Found it!".into()),
                content_parts: None,
                tool_calls: Some(vec![ToolCall {
                    id: "call_1".into(),
                    call_type: "function".into(),
                    function: FunctionCallPayload {
                        name: "Bash".into(),
                        arguments: "{}".into(),
                    },
                }]),
                tool_call_id: None,
                reasoning_content: Some("thinking".into()),
            },
        ];
        normalize_trailing_assistant_tool_calls(&mut msgs);
        assert_eq!(msgs.len(), 2);
        assert!(msgs[1].tool_calls.is_none());
        assert_eq!(msgs[1].content.as_deref(), Some("Found it!"));
    }

    #[test]
    fn strips_trailing_assistant_tool_calls_removes_empty_message() {
        let mut msgs = vec![Message {
            role: Role::Assistant,
            content: None,
            content_parts: None,
            tool_calls: Some(vec![ToolCall {
                id: "call_1".into(),
                call_type: "function".into(),
                function: FunctionCallPayload {
                    name: "read".into(),
                    arguments: "{}".into(),
                },
            }]),
            tool_call_id: None,
            reasoning_content: None,
        }];
        normalize_trailing_assistant_tool_calls(&mut msgs);
        assert!(msgs.is_empty());
    }

    #[test]
    fn preserves_mid_history_assistant_tool_calls() {
        let mut msgs = vec![
            Message {
                role: Role::Assistant,
                content: None,
                content_parts: None,
                tool_calls: Some(vec![ToolCall {
                    id: "call_1".into(),
                    call_type: "function".into(),
                    function: FunctionCallPayload {
                        name: "read".into(),
                        arguments: "{}".into(),
                    },
                }]),
                tool_call_id: None,
                reasoning_content: None,
            },
            Message {
                role: Role::Tool,
                content: Some("file contents".into()),
                content_parts: None,
                tool_calls: None,
                tool_call_id: Some("call_1".into()),
                reasoning_content: None,
            },
            Message {
                role: Role::User,
                content: Some("thanks".into()),
                content_parts: None,
                tool_calls: None,
                tool_call_id: None,
                reasoning_content: None,
            },
        ];
        normalize_trailing_assistant_tool_calls(&mut msgs);
        assert_eq!(msgs.len(), 3);
        assert!(msgs[0].tool_calls.as_ref().is_some_and(|c| !c.is_empty()));
    }
}
