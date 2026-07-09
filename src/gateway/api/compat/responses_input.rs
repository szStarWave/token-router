use serde_json::{json, Value};

use crate::gateway::api::compat::reasoning::{
    backfill_tool_call_reasoning_placeholders,
};

pub fn convert_responses_input_to_messages(input: &Value, instructions: &str) -> Vec<Value> {
    let mut messages = Vec::new();
    if !instructions.trim().is_empty() {
        messages.push(json!({"role": "system", "content": instructions}));
    }

    let mut pending_tool_calls: Vec<Value> = Vec::new();
    let mut pending_reasoning: Option<String> = None;
    let mut last_assistant_index: Option<usize> = None;
    let mut deferred_messages: Vec<Value> = Vec::new();

    match input {
        Value::String(s) => messages.push(json!({"role": "user", "content": s})),
        Value::Array(items) => {
            for item in items {
                append_responses_item(
                    item,
                    &mut messages,
                    &mut pending_tool_calls,
                    &mut pending_reasoning,
                    &mut last_assistant_index,
                    &mut deferred_messages,
                );
            }
        }
        Value::Object(_) => {
            append_responses_item(
                input,
                &mut messages,
                &mut pending_tool_calls,
                &mut pending_reasoning,
                &mut last_assistant_index,
                &mut deferred_messages,
            );
        }
        _ => {}
    }

    flush_pending_tool_calls(
        &mut messages,
        &mut pending_tool_calls,
        &mut pending_reasoning,
        &mut last_assistant_index,
    );
    flush_deferred_messages(&mut messages, &mut deferred_messages);
    backfill_tool_call_reasoning_placeholders(&mut messages);

    let has_non_system = messages.iter().any(|m| {
        m.get("role")
            .and_then(|r| r.as_str())
            .is_some_and(|r| r != "system")
    });
    if !has_non_system {
        messages.push(json!({"role": "user", "content": ""}));
    }
    messages
}

fn append_responses_item(
    item: &Value,
    messages: &mut Vec<Value>,
    pending_tool_calls: &mut Vec<Value>,
    pending_reasoning: &mut Option<String>,
    last_assistant_index: &mut Option<usize>,
    deferred_messages: &mut Vec<Value>,
) {
    let item_type = item.get("type").and_then(|v| v.as_str()).unwrap_or("");

    match item_type {
        "function_call" | "custom_tool_call" | "tool_search_call" => {
            append_pending_reasoning(pending_reasoning, item_reasoning_text(item));
            let call_id = item.get("call_id").and_then(|v| v.as_str()).unwrap_or("");
            let name = item.get("name").and_then(|v| v.as_str()).unwrap_or("");
            let arguments = item.get("arguments").and_then(|v| v.as_str()).unwrap_or("");
            if !call_id.is_empty() && !name.is_empty() {
                pending_tool_calls.push(json!({
                    "id": call_id,
                    "type": "function",
                    "function": {"name": name, "arguments": arguments}
                }));
            }
        }
        "function_call_output" | "custom_tool_call_output" | "tool_search_call_output" => {
            flush_pending_tool_calls(
                messages,
                pending_tool_calls,
                pending_reasoning,
                last_assistant_index,
            );
            let call_id = item.get("call_id").and_then(|v| v.as_str()).unwrap_or("");
            let output = stringify_function_output(item.get("output"));
            if !call_id.is_empty() {
                messages.push(json!({
                    "role": "tool",
                    "tool_call_id": call_id,
                    "content": output,
                }));
            }
        }
        "reasoning" => {
            let reasoning = reasoning_item_text(item);
            let attached = pending_tool_calls.is_empty()
                && attach_reasoning_to_last_assistant(messages, *last_assistant_index, &reasoning);
            if !attached {
                append_pending_reasoning(pending_reasoning, reasoning);
            }
        }
        "message" | "" => {
            if item.get("role").is_none() && item.get("content").is_none() {
                return;
            }
            let role = item.get("role").and_then(|v| v.as_str()).unwrap_or("user");
            if role == "tool" {
                flush_pending_tool_calls(
                    messages,
                    pending_tool_calls,
                    pending_reasoning,
                    last_assistant_index,
                );
                let call_id = item.get("tool_call_id").and_then(|v| v.as_str()).unwrap_or("");
                let output = item.get("content").and_then(|v| match v {
                    Value::String(s) => Some(s.clone()),
                    _ => None,
                }).unwrap_or_default();
                if call_id.is_empty() {
                    return;
                }
                let has_preceding_assistant = last_assistant_index
                    .and_then(|idx| messages.get(idx))
                    .and_then(|last| last.get("tool_calls"))
                    .and_then(|tc| tc.as_array())
                    .is_some_and(|arr| !arr.is_empty());
                if !has_preceding_assistant {
                    messages.push(json!({
                        "role": "assistant",
                        "content": null,
                        "tool_calls": [{
                            "id": call_id,
                            "type": "function",
                            "function": {"name": "", "arguments": "{}"}
                        }],
                        "reasoning_content": "tool call",
                    }));
                    *last_assistant_index = Some(messages.len() - 1);
                }
                messages.push(json!({
                    "role": "tool",
                    "tool_call_id": call_id,
                    "content": output,
                }));
            } else if should_defer_during_pending_tool_calls(role, pending_tool_calls) {
                deferred_messages.push(message_item_to_chat(item, pending_reasoning));
            } else {
                flush_pending_tool_calls(
                    messages,
                    pending_tool_calls,
                    pending_reasoning,
                    last_assistant_index,
                );
                let message = message_item_to_chat(item, pending_reasoning);
                update_last_assistant_index(messages, &message, last_assistant_index);
                messages.push(message);
            }
        }
        _ => {
            if item.get("role").is_some() || item.get("content").is_some() {
                let role = item.get("role").and_then(|v| v.as_str()).unwrap_or("user");
                if role == "tool" {
                    flush_pending_tool_calls(
                        messages,
                        pending_tool_calls,
                        pending_reasoning,
                        last_assistant_index,
                    );
                    let call_id = item.get("tool_call_id").and_then(|v| v.as_str()).unwrap_or("");
                    let output = item.get("content").and_then(|v| match v {
                        Value::String(s) => Some(s.clone()),
                        _ => None,
                    }).unwrap_or_default();
                    if call_id.is_empty() {
                        return;
                    }
                    let has_preceding_assistant = last_assistant_index
                        .and_then(|idx| messages.get(idx))
                        .and_then(|last| last.get("tool_calls"))
                        .and_then(|tc| tc.as_array())
                        .is_some_and(|arr| !arr.is_empty());
                    if !has_preceding_assistant {
                        messages.push(json!({
                            "role": "assistant",
                            "content": null,
                            "tool_calls": [{
                                "id": call_id,
                                "type": "function",
                                "function": {"name": "", "arguments": "{}"}
                            }],
                            "reasoning_content": "tool call",
                        }));
                        *last_assistant_index = Some(messages.len() - 1);
                    }
                    messages.push(json!({
                        "role": "tool",
                        "tool_call_id": call_id,
                        "content": output,
                    }));
                } else if should_defer_during_pending_tool_calls(role, pending_tool_calls) {
                    deferred_messages.push(message_item_to_chat(item, pending_reasoning));
                } else {
                    flush_pending_tool_calls(
                        messages,
                        pending_tool_calls,
                        pending_reasoning,
                        last_assistant_index,
                    );
                    let message = message_item_to_chat(item, pending_reasoning);
                    update_last_assistant_index(messages, &message, last_assistant_index);
                    messages.push(message);
                }
            }
        }
    }
}

fn should_defer_during_pending_tool_calls(role: &str, pending_tool_calls: &[Value]) -> bool {
    !pending_tool_calls.is_empty() && role != "assistant" && role != "tool"
}

fn flush_deferred_messages(messages: &mut Vec<Value>, deferred_messages: &mut Vec<Value>) {
    messages.append(deferred_messages);
}

fn flush_pending_tool_calls(
    messages: &mut Vec<Value>,
    pending_tool_calls: &mut Vec<Value>,
    pending_reasoning: &mut Option<String>,
    last_assistant_index: &mut Option<usize>,
) {
    if pending_tool_calls.is_empty() {
        return;
    }
    let mut message = json!({
        "role": "assistant",
        "content": null,
        "tool_calls": std::mem::take(pending_tool_calls),
    });
    attach_pending_reasoning_to_assistant(&mut message, pending_reasoning);
    *last_assistant_index = Some(messages.len());
    messages.push(message);
}

fn message_item_to_chat(item: &Value, pending_reasoning: &mut Option<String>) -> Value {
    let mut role = item
        .get("role")
        .and_then(|v| v.as_str())
        .unwrap_or("user")
        .to_string();
    if role == "developer" {
        role = "user".to_string();
    }

    let mut has_content = false;
    let content = match item.get("content") {
        Some(Value::String(s)) => { has_content = true; json!(s) }
        Some(Value::Array(parts)) => {
            let chat_content = convert_input_content_parts(parts);
            if chat_content.is_empty() {
                Value::Null
            } else {
                has_content = true;
                json!(chat_content)
            }
        }
        _ => Value::Null,
    };

    let mut message = json!({"role": role, "content": content});

    // Drop empty assistant messages (no content, no tool_calls) to avoid upstream rejection
    if role == "assistant" && !has_content && item.get("tool_calls").is_none() {
        message = json!({"role": "assistant", "content": ""});
    }
    if role == "assistant" {
        append_pending_reasoning(pending_reasoning, message_item_reasoning_text(item));
        attach_pending_reasoning_to_assistant(&mut message, pending_reasoning);
    } else if pending_reasoning.is_some() {
        pending_reasoning.take();
    }
    message
}

fn convert_input_content_parts(parts: &[Value]) -> Vec<Value> {
    let mut out = Vec::new();
    for part in parts {
        let Some(p) = part.as_object() else { continue };
        match p.get("type").and_then(|v| v.as_str()).unwrap_or("") {
            "input_text" | "text" => {
                if let Some(text) = p.get("text").and_then(|v| v.as_str()) {
                    out.push(json!({"type": "text", "text": text}));
                }
            }
            "input_image" | "image_url" => {
                if let Some(url) = p.get("image_url").or_else(|| p.get("url")) {
                    out.push(json!({"type": "image_url", "image_url": url}));
                }
            }
            _ => {}
        }
    }
    out
}

fn append_pending_reasoning(pending: &mut Option<String>, reasoning: Option<String>) {
    let Some(reasoning) = reasoning else { return };
    let reasoning = reasoning.trim();
    if reasoning.is_empty() {
        return;
    }
    match pending {
        Some(existing) if !existing.is_empty() => {
            existing.push_str("\n\n");
            existing.push_str(reasoning);
        }
        _ => *pending = Some(reasoning.to_string()),
    }
}

fn attach_pending_reasoning_to_assistant(message: &mut Value, pending: &mut Option<String>) {
    let Some(reasoning) = pending.take() else { return };
    if reasoning.trim().is_empty() {
        return;
    }
    if let Some(obj) = message.as_object_mut() {
        obj.insert("reasoning_content".into(), json!(reasoning));
    }
}

fn attach_reasoning_to_last_assistant(
    messages: &mut [Value],
    last_assistant_index: Option<usize>,
    reasoning: &Option<String>,
) -> bool {
    let Some(reasoning) = reasoning
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    else {
        return true;
    };
    let Some(index) = last_assistant_index else {
        return false;
    };
    let Some(message) = messages.get_mut(index) else {
        return false;
    };
    if message.get("role").and_then(|v| v.as_str()) != Some("assistant") {
        return false;
    }
    let Some(obj) = message.as_object_mut() else {
        return false;
    };
    let has_reasoning = obj
        .get("reasoning_content")
        .and_then(|v| v.as_str())
        .is_some_and(|text| !text.trim().is_empty());
    if !has_reasoning {
        obj.insert("reasoning_content".into(), json!(reasoning));
    }
    true
}

fn update_last_assistant_index(
    messages: &[Value],
    message: &Value,
    last_assistant_index: &mut Option<usize>,
) {
    match message.get("role").and_then(|v| v.as_str()) {
        Some("assistant") => *last_assistant_index = Some(messages.len()),
        Some("tool") => {}
        _ => *last_assistant_index = None,
    }
}

fn item_reasoning_text(item: &Value) -> Option<String> {
    item.get("reasoning_content")
        .or_else(|| item.get("reasoning"))
        .and_then(|v| v.as_str())
        .map(str::to_string)
        .filter(|s| !s.trim().is_empty())
}

fn message_item_reasoning_text(item: &Value) -> Option<String> {
    if let Some(text) = item
        .get("reasoning_content")
        .and_then(|v| v.as_str())
        .filter(|s| !s.trim().is_empty())
    {
        return Some(text.to_string());
    }
    item.get("content")
        .and_then(|c| c.as_array())
        .and_then(|parts| {
            parts.iter().find_map(|p| {
                if p.get("type").and_then(|t| t.as_str()) == Some("reasoning") {
                    p.get("text").and_then(|t| t.as_str()).map(str::to_string)
                } else {
                    None
                }
            })
        })
}

fn reasoning_item_text(item: &Value) -> Option<String> {
    if let Some(text) = item
        .get("summary")
        .or_else(|| item.get("text"))
        .and_then(|v| v.as_str())
        .filter(|s| !s.trim().is_empty())
    {
        return Some(text.to_string());
    }
    if let Some(parts) = item.get("content").and_then(|v| v.as_array()) {
        let texts: Vec<&str> = parts
            .iter()
            .filter_map(|p| p.get("text").and_then(|t| t.as_str()))
            .filter(|t| !t.trim().is_empty())
            .collect();
        if !texts.is_empty() {
            return Some(texts.join("\n\n"));
        }
    }
    None
}

fn stringify_function_output(output: Option<&Value>) -> String {
    match output {
        Some(Value::String(s)) => s.clone(),
        None => String::new(),
        Some(v) => serde_json::to_string(v).unwrap_or_default(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gateway::api::compat::TOOL_CALL_REASONING_PLACEHOLDER;

    #[test]
    fn batches_parallel_function_calls_into_one_assistant_message() {
        let input = json!([
            {"type": "function_call", "call_id": "c1", "name": "a", "arguments": "{}"},
            {"type": "function_call", "call_id": "c2", "name": "b", "arguments": "{}"},
            {"type": "function_call_output", "call_id": "c1", "output": "ok"}
        ]);
        let msgs = convert_responses_input_to_messages(&input, "");
        let assistant = msgs
            .iter()
            .find(|m| m.get("role") == Some(&json!("assistant")))
            .unwrap();
        assert_eq!(assistant["tool_calls"].as_array().unwrap().len(), 2);
        assert_eq!(
            assistant.get("reasoning_content").and_then(|v| v.as_str()),
            Some(TOOL_CALL_REASONING_PLACEHOLDER)
        );
    }

    #[test]
    fn injects_stub_assistant_before_orphaned_tool_message() {
        let input = json!([
            {"role": "user", "content": "hello"},
            {"role": "assistant", "content": "hi there"},
            {"role": "tool", "tool_call_id": "call_1", "content": "output"}
        ]);
        let msgs = convert_responses_input_to_messages(&input, "");
        let assistant_idx = msgs.iter().position(|m| m.get("role") == Some(&json!("assistant")) && m.get("tool_calls").is_some());
        assert!(assistant_idx.is_some(), "should have injected a stub assistant with tool_calls");
        let idx = assistant_idx.unwrap();
        assert_eq!(msgs[idx]["tool_calls"][0]["id"], "call_1");
        assert_eq!(msgs[idx].get("reasoning_content").and_then(|v| v.as_str()), Some("tool call"));
        assert_eq!(msgs[idx + 1]["role"], "tool");
        assert_eq!(msgs[idx + 1]["tool_call_id"], "call_1");
    }

    #[test]
    fn does_not_inject_stub_when_assistant_already_has_tool_calls() {
        let input = json!([
            {"role": "user", "content": "hello"},
            {"type": "function_call", "call_id": "c1", "name": "read", "arguments": "{}"},
            {"type": "function_call_output", "call_id": "c1", "output": "data"}
        ]);
        let msgs = convert_responses_input_to_messages(&input, "");
        let assistants: Vec<_> = msgs.iter().filter(|m| m.get("role") == Some(&json!("assistant"))).collect();
        assert_eq!(assistants.len(), 1);
        assert_eq!(assistants[0]["tool_calls"][0]["id"], "c1");
    }

    #[test]
    fn defers_user_message_until_function_call_outputs_arrive() {
        let input = json!([
            {"type": "function_call", "call_id": "c1", "name": "read", "arguments": "{}"},
            {"role": "user", "content": "[Request interrupted by user]\n继续"},
            {"type": "function_call_output", "call_id": "c1", "output": "ok"}
        ]);
        let msgs = convert_responses_input_to_messages(&input, "");
        assert_eq!(msgs[0]["role"], "assistant");
        assert_eq!(msgs[0]["tool_calls"][0]["id"], "c1");
        assert_eq!(msgs[1]["role"], "tool");
        assert_eq!(msgs[1]["tool_call_id"], "c1");
        assert_eq!(msgs[2]["role"], "user");
    }

    #[test]
    fn keeps_parallel_tool_calls_adjacent_to_outputs_with_interrupted_user() {
        let input = json!([
            {"type": "function_call", "call_id": "c1", "name": "read", "arguments": "{}"},
            {"type": "function_call", "call_id": "c2", "name": "list", "arguments": "{}"},
            {"role": "user", "content": "Continue"},
            {"type": "function_call_output", "call_id": "c1", "output": "one"},
            {"type": "function_call_output", "call_id": "c2", "output": "two"}
        ]);
        let msgs = convert_responses_input_to_messages(&input, "");
        assert_eq!(msgs[0]["role"], "assistant");
        assert_eq!(msgs[1]["role"], "tool");
        assert_eq!(msgs[1]["tool_call_id"], "c1");
        assert_eq!(msgs[2]["role"], "tool");
        assert_eq!(msgs[2]["tool_call_id"], "c2");
        assert_eq!(msgs[3]["role"], "user");
    }
}
