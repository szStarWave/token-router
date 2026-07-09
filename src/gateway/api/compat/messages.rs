use std::collections::HashSet;

use serde_json::{json, Value};

use crate::gateway::api::compat::reasoning::{
    apply_reasoning_options_to_chat_request, apply_reasoning_to_upstream_messages,
};
use crate::gateway::api::openai::{ChatCompletionRequest, Message, Role};

fn assistant_has_tool_calls(msg: &Message) -> bool {
    msg.role == Role::Assistant
        && msg
            .tool_calls
            .as_ref()
            .is_some_and(|calls| !calls.is_empty())
}

fn assistant_tool_call_ids(msg: &Message) -> Vec<String> {
    msg.tool_calls
        .as_ref()
        .map(|calls| calls.iter().map(|c| c.id.clone()).collect())
        .unwrap_or_default()
}

/// Reorder messages so every `assistant(tool_calls)` turn is immediately followed
/// by its `tool` responses. Agent clients (e.g. Claude Code after an interrupt) may
/// insert a user message between the assistant call and the tool result.
pub fn repair_assistant_tool_call_adjacency(messages: &mut Vec<Message>) {
    let mut repaired = Vec::with_capacity(messages.len());
    let mut i = 0;

    while i < messages.len() {
        let msg = messages[i].clone();
        if !assistant_has_tool_calls(&msg) {
            repaired.push(msg);
            i += 1;
            continue;
        }

        let expected_ids = assistant_tool_call_ids(&msg);
        let mut found_ids = HashSet::new();
        let mut found_tools = Vec::new();
        let mut deferred = Vec::new();
        let mut j = i + 1;

        while j < messages.len() {
            let next = &messages[j];
            if assistant_has_tool_calls(next) {
                break;
            }
            if next.role == Role::Tool {
                if let Some(id) = next.tool_call_id.as_deref() {
                    if expected_ids.iter().any(|expected| expected == id)
                        && found_ids.insert(id.to_string())
                    {
                        found_tools.push(next.clone());
                        j += 1;
                        continue;
                    }
                }
            }
            if found_ids.len() == expected_ids.len() {
                break;
            }
            deferred.push(next.clone());
            j += 1;
        }

        let mut assistant_msg = msg;
        if found_ids.len() < expected_ids.len() {
            if let Some(tool_calls) = assistant_msg.tool_calls.as_mut() {
                tool_calls.retain(|call| found_ids.contains(&call.id));
                if tool_calls.is_empty() {
                    assistant_msg.tool_calls = None;
                }
            }
        }

        repaired.push(assistant_msg);
        for id in &expected_ids {
            if let Some(tool) = found_tools
                .iter()
                .find(|tool| tool.tool_call_id.as_deref() == Some(id.as_str()))
            {
                repaired.push(tool.clone());
            }
        }
        repaired.extend(deferred);
        i = j;
    }

    *messages = repaired;
}

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
    repair_assistant_tool_call_adjacency(&mut req.messages);
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

    fn assistant_with_tool_call(id: &str) -> Message {
        Message {
            role: Role::Assistant,
            content: None,
            content_parts: None,
            tool_calls: Some(vec![ToolCall {
                id: id.into(),
                call_type: "function".into(),
                function: FunctionCallPayload {
                    name: "Bash".into(),
                    arguments: "{}".into(),
                },
            }]),
            tool_call_id: None,
            reasoning_content: Some("thinking".into()),
        }
    }

    fn tool_message(id: &str, content: &str) -> Message {
        Message {
            role: Role::Tool,
            content: Some(content.into()),
            content_parts: None,
            tool_calls: None,
            tool_call_id: Some(id.into()),
            reasoning_content: None,
        }
    }

    fn user_message(content: &str) -> Message {
        Message {
            role: Role::User,
            content: Some(content.into()),
            content_parts: None,
            tool_calls: None,
            tool_call_id: None,
            reasoning_content: None,
        }
    }

    #[test]
    fn repairs_interrupted_tool_response_order() {
        let mut msgs = vec![
            assistant_with_tool_call("call_1"),
            user_message("[Request interrupted by user]\n继续"),
            tool_message("call_1", "Thu, Jul  9, 2026 10:27:35 AM"),
        ];
        repair_assistant_tool_call_adjacency(&mut msgs);
        assert_eq!(msgs.len(), 3);
        assert_eq!(msgs[0].role, Role::Assistant);
        assert_eq!(msgs[1].role, Role::Tool);
        assert_eq!(msgs[1].tool_call_id.as_deref(), Some("call_1"));
        assert_eq!(msgs[2].role, Role::User);
    }

    #[test]
    fn repairs_parallel_tool_calls_with_interrupt() {
        let mut msgs = vec![
            Message {
                role: Role::Assistant,
                content: None,
                content_parts: None,
                tool_calls: Some(vec![
                    ToolCall {
                        id: "c1".into(),
                        call_type: "function".into(),
                        function: FunctionCallPayload {
                            name: "a".into(),
                            arguments: "{}".into(),
                        },
                    },
                    ToolCall {
                        id: "c2".into(),
                        call_type: "function".into(),
                        function: FunctionCallPayload {
                            name: "b".into(),
                            arguments: "{}".into(),
                        },
                    },
                ]),
                tool_call_id: None,
                reasoning_content: None,
            },
            user_message("continue"),
            tool_message("c2", "out2"),
            tool_message("c1", "out1"),
        ];
        repair_assistant_tool_call_adjacency(&mut msgs);
        assert_eq!(msgs[1].tool_call_id.as_deref(), Some("c1"));
        assert_eq!(msgs[2].tool_call_id.as_deref(), Some("c2"));
        assert_eq!(msgs[3].role, Role::User);
    }

    #[test]
    fn preserves_correct_tool_order() {
        let mut msgs = vec![
            assistant_with_tool_call("call_1"),
            tool_message("call_1", "ok"),
            user_message("thanks"),
        ];
        repair_assistant_tool_call_adjacency(&mut msgs);
        assert_eq!(msgs[1].role, Role::Tool);
        assert_eq!(msgs[2].role, Role::User);
    }

    #[test]
    fn strips_unfulfilled_tool_calls_when_response_missing() {
        let mut msgs = vec![
            assistant_with_tool_call("call_1"),
            user_message("interrupted"),
        ];
        repair_assistant_tool_call_adjacency(&mut msgs);
        assert!(msgs[0].tool_calls.is_none());
        assert_eq!(msgs[1].role, Role::User);
    }

    #[test]
    fn finalize_upstream_repairs_interrupted_tool_turn_for_deepseek() {
        let req = ChatCompletionRequest {
            model: "deepseek-v4-flash".into(),
            messages: vec![
                assistant_with_tool_call("call_1"),
                user_message("[Request interrupted by user]\n继续"),
                tool_message("call_1", "Thu, Jul  9, 2026 10:27:35 AM"),
            ],
            ..Default::default()
        };
        let out = finalize_upstream_request(req, Some("https://api.deepseek.com/v1"));
        assert_eq!(out.messages[0].role, Role::Assistant);
        assert_eq!(out.messages[1].role, Role::Tool);
        assert_eq!(out.messages[2].role, Role::User);
    }
}
