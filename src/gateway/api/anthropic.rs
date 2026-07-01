use axum::{
    Json,
    extract::State,
    http::HeaderMap,
    response::IntoResponse,
};
use bytes::Bytes;
use serde_json::{json, Value};

use crate::gateway::api::chat::{chat_completions_core, ChatOutputFormat};
use crate::gateway::api::openai::ChatCompletionRequest;
use crate::gateway::api::routes::AppState;
use crate::gateway::api::sse_transform::{anthropic_sse_event, SseTransform};
use crate::gateway::error::{AppError, AppResult};

const DATA_PREFIX: &[u8] = b"data: ";
const DATA_DONE: &[u8] = b"data: [DONE]";

pub async fn anthropic_messages_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> AppResult<impl IntoResponse> {
    let agent_id = super::routes::extract_agent_id(&headers);
    let req = anthropic_request_to_openai(&body)?;
    let model = req.model.clone();
    chat_completions_core(
        state,
        headers,
        agent_id,
        req,
        ChatOutputFormat::Anthropic { model },
    )
    .await
}

pub fn anthropic_request_to_openai(body: &Value) -> AppResult<ChatCompletionRequest> {
    let obj = body
        .as_object()
        .ok_or_else(|| AppError::BadRequest("invalid request body".into()))?;

    let mut oai = serde_json::Map::new();

    if let Some(m) = obj.get("model") {
        oai.insert("model".into(), m.clone());
    }
    if let Some(mt) = obj.get("max_tokens") {
        oai.insert("max_tokens".into(), mt.clone());
    }
    if let Some(s) = obj.get("stream") {
        oai.insert("stream".into(), s.clone());
    } else {
        oai.insert("stream".into(), json!(false));
    }
    if let Some(t) = obj.get("temperature") {
        oai.insert("temperature".into(), t.clone());
    }
    if let Some(tp) = obj.get("top_p") {
        oai.insert("top_p".into(), tp.clone());
    }

    if let Some(tools) = obj.get("tools").and_then(|t| t.as_array()) {
        oai.insert("tools".into(), json!(convert_tools(tools)));
    }
    if let Some(tc) = obj.get("tool_choice") {
        oai.insert("tool_choice".into(), convert_tool_choice(tc));
    }

    oai.insert("messages".into(), json!(build_messages(obj)));

    serde_json::from_value(Value::Object(oai))
        .map_err(|e| AppError::BadRequest(format!("invalid anthropic request: {e}")))
}

pub fn chat_json_to_anthropic(body: &[u8], fallback_model: &str) -> Value {
    let Ok(oai) = serde_json::from_slice::<Value>(body) else {
        return json!({});
    };

    let id = oai.get("id").and_then(|v| v.as_str()).unwrap_or("");
    let model = oai
        .get("model")
        .and_then(|v| v.as_str())
        .unwrap_or(fallback_model);
    let choice = oai
        .get("choices")
        .and_then(|c| c.as_array())
        .and_then(|a| a.first());

    let Some(choice) = choice else {
        return json!({});
    };

    let finish_reason = choice
        .get("finish_reason")
        .and_then(|v| v.as_str())
        .unwrap_or("stop");
    let stop_reason = map_finish_reason(finish_reason);

    let message = choice.get("message").cloned().unwrap_or(json!({}));
    let content_str = message.get("content").and_then(|c| c.as_str());
    let tool_calls = message
        .get("tool_calls")
        .and_then(|t| t.as_array())
        .cloned()
        .unwrap_or_default();

    let mut content_blocks = Vec::new();
    if let Some(text) = content_str.filter(|s| !s.is_empty()) {
        content_blocks.push(json!({"type": "text", "text": text}));
    }
    for tc in tool_calls {
        let id = tc.get("id").and_then(|v| v.as_str()).unwrap_or("");
        let name = tc
            .get("function")
            .and_then(|f| f.get("name"))
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let args_str = tc
            .get("function")
            .and_then(|f| f.get("arguments"))
            .and_then(|v| v.as_str())
            .unwrap_or("{}");
        let args: Value = serde_json::from_str(args_str).unwrap_or(json!({}));
        content_blocks.push(json!({
            "type": "tool_use",
            "id": id,
            "name": name,
            "input": args,
        }));
    }
    if content_blocks.is_empty() {
        content_blocks.push(json!({"type": "text", "text": ""}));
    }

    let input_tokens = oai
        .get("usage")
        .and_then(|u| u.get("prompt_tokens"))
        .cloned()
        .unwrap_or(json!(0));
    let output_tokens = oai
        .get("usage")
        .and_then(|u| u.get("completion_tokens"))
        .cloned()
        .unwrap_or(json!(0));

    let msg_id = if id.is_empty() {
        format!("msg_{:x}", hash_bytes(model.as_bytes()))
    } else {
        id.to_string()
    };

    json!({
        "id": msg_id,
        "type": "message",
        "role": "assistant",
        "content": content_blocks,
        "model": model,
        "stop_reason": stop_reason,
        "stop_sequence": null,
        "usage": {
            "input_tokens": input_tokens,
            "output_tokens": output_tokens,
        }
    })
}

pub struct AnthropicSseTransform {
    model: String,
    role_sent: bool,
    content_start: bool,
    text_block: bool,
    done: bool,
}

impl AnthropicSseTransform {
    pub fn new(model: String) -> Self {
        Self {
            model,
            role_sent: false,
            content_start: false,
            text_block: false,
            done: false,
        }
    }
}

impl SseTransform for AnthropicSseTransform {
    fn transform_line(&mut self, line: &[u8]) -> Vec<Bytes> {
        if self.done {
            return Vec::new();
        }

        let trimmed = trim_line(line);
        if trimmed.is_empty() || trimmed == DATA_DONE {
            return Vec::new();
        }
        if !trimmed.starts_with(DATA_PREFIX) {
            return Vec::new();
        }

        let json_data = &trimmed[DATA_PREFIX.len()..];
        let Ok(chunk) = serde_json::from_slice::<Value>(json_data) else {
            return Vec::new();
        };

        let choices = chunk.get("choices").and_then(|c| c.as_array());
        let Some(choices) = choices else {
            return Vec::new();
        };
        if choices.is_empty() {
            return Vec::new();
        }
        let choice = &choices[0];
        let delta = choice.get("delta").cloned().unwrap_or(json!({}));

        let mut out = Vec::new();

        let role = delta.get("role").and_then(|v| v.as_str()).unwrap_or("");
        if !self.role_sent && role == "assistant" {
            self.role_sent = true;
            let msg_id = format!("msg_{:x}", hash_bytes(json_data));
            let start = json!({
                "type": "message_start",
                "message": {
                    "id": msg_id,
                    "type": "message",
                    "role": "assistant",
                    "content": [],
                    "model": self.model,
                    "stop_reason": null,
                    "stop_sequence": null,
                    "usage": {"input_tokens": 0, "output_tokens": 0}
                }
            });
            out.push(anthropic_sse_event(
                "message_start",
                serde_json::to_vec(&start).unwrap_or_default().as_slice(),
            ));
        }

        let content = delta.get("content").and_then(|v| v.as_str());
        let has_text = content.is_some_and(|c| !c.is_empty());
        let tool_calls = delta
            .get("tool_calls")
            .and_then(|t| t.as_array())
            .cloned()
            .unwrap_or_default();
        let has_tool = !tool_calls.is_empty();

        if (has_text || has_tool) && !self.content_start {
            self.content_start = true;
        }

        if has_text {
            if !self.text_block {
                self.text_block = true;
                let cb = json!({
                    "type": "content_block_start",
                    "index": 0,
                    "content_block": {"type": "text", "text": ""}
                });
                out.push(anthropic_sse_event(
                    "content_block_start",
                    serde_json::to_vec(&cb).unwrap_or_default().as_slice(),
                ));
            }
            let delta_evt = json!({
                "type": "content_block_delta",
                "index": 0,
                "delta": {"type": "text_delta", "text": content.unwrap_or("")}
            });
            out.push(anthropic_sse_event(
                "content_block_delta",
                serde_json::to_vec(&delta_evt).unwrap_or_default().as_slice(),
            ));
        }

        for tc in tool_calls {
            let id = tc.get("id").and_then(|v| v.as_str()).unwrap_or("");
            let func = tc.get("function");
            let name = func
                .and_then(|f| f.get("name"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let args = func
                .and_then(|f| f.get("arguments"))
                .and_then(|v| v.as_str())
                .unwrap_or("");

            if !id.is_empty() || !name.is_empty() {
                let cb = json!({
                    "type": "content_block_start",
                    "index": 0,
                    "content_block": {
                        "type": "tool_use",
                        "id": id,
                        "name": name,
                        "input": {}
                    }
                });
                out.push(anthropic_sse_event(
                    "content_block_start",
                    serde_json::to_vec(&cb).unwrap_or_default().as_slice(),
                ));
                if !args.is_empty() {
                    let delta_evt = json!({
                        "type": "content_block_delta",
                        "index": 0,
                        "delta": {"type": "input_json_delta", "partial_json": args}
                    });
                    out.push(anthropic_sse_event(
                        "content_block_delta",
                        serde_json::to_vec(&delta_evt).unwrap_or_default().as_slice(),
                    ));
                }
            } else if !args.is_empty() {
                let delta_evt = json!({
                    "type": "content_block_delta",
                    "index": 0,
                    "delta": {"type": "input_json_delta", "partial_json": args}
                });
                out.push(anthropic_sse_event(
                    "content_block_delta",
                    serde_json::to_vec(&delta_evt).unwrap_or_default().as_slice(),
                ));
            }
        }

        if let Some(finish) = choice.get("finish_reason").and_then(|v| v.as_str()) {
            let stop_reason = map_finish_reason(finish);
            if self.content_start {
                let sd = json!({"type": "content_block_stop", "index": 0});
                out.push(anthropic_sse_event(
                    "content_block_stop",
                    serde_json::to_vec(&sd).unwrap_or_default().as_slice(),
                ));
            }
            let md = json!({
                "type": "message_delta",
                "delta": {"stop_reason": stop_reason, "stop_sequence": null},
                "usage": {"output_tokens": 0}
            });
            out.push(anthropic_sse_event(
                "message_delta",
                serde_json::to_vec(&md).unwrap_or_default().as_slice(),
            ));
            out.push(anthropic_sse_event(
                "message_stop",
                br#"{"type":"message_stop"}"#,
            ));
            self.done = true;
        }

        out
    }
}

fn map_finish_reason(reason: &str) -> &str {
    match reason {
        "stop" => "end_turn",
        "length" => "max_tokens",
        "tool_calls" => "tool_use",
        "content_filter" => "content_filter",
        other => other,
    }
}

fn convert_tools(tools: &[Value]) -> Vec<Value> {
    tools
        .iter()
        .filter_map(|t| {
            let tool = t.as_object()?;
            let mut func = serde_json::Map::new();
            if let Some(name) = tool.get("name") {
                func.insert("name".into(), name.clone());
            }
            if let Some(desc) = tool.get("description") {
                func.insert("description".into(), desc.clone());
            }
            if let Some(schema) = tool.get("input_schema") {
                func.insert("parameters".into(), schema.clone());
            }
            Some(json!({"type": "function", "function": func}))
        })
        .collect()
}

fn convert_tool_choice(tc: &Value) -> Value {
    match tc {
        Value::String(s) => Value::String(s.clone()),
        Value::Object(map) => {
            let t = map.get("type").and_then(|v| v.as_str()).unwrap_or("");
            match t {
                "auto" => json!("auto"),
                "any" => json!("required"),
                "tool" => {
                    if let Some(name) = map.get("name").and_then(|v| v.as_str()) {
                        json!({"type": "function", "function": {"name": name}})
                    } else {
                        json!("auto")
                    }
                }
                _ => json!("auto"),
            }
        }
        _ => json!("auto"),
    }
}

fn build_messages(obj: &serde_json::Map<String, Value>) -> Vec<Value> {
    let mut messages = Vec::new();

    if let Some(sys) = obj.get("system") {
        if let Some(s) = extract_system_text(sys) {
            if !s.is_empty() {
                messages.push(json!({"role": "system", "content": s}));
            }
        }
    }

    let Some(msgs) = obj.get("messages").and_then(|m| m.as_array()) else {
        return messages;
    };

    for msg in msgs {
        let Some(m) = msg.as_object() else { continue };
        let role = m.get("role").and_then(|v| v.as_str()).unwrap_or("");
        if role == "system" {
            continue;
        }
        let content = m.get("content");

        match role {
            "assistant" => messages.push(convert_assistant_message(role, content)),
            "user" => {
                let (user_msgs, tool_msgs) = convert_user_message(role, content);
                messages.extend(user_msgs);
                messages.extend(tool_msgs);
            }
            _ => {
                messages.push(json!({
                    "role": role,
                    "content": flatten_content(content),
                }));
            }
        }
    }
    messages
}

fn extract_system_text(sys: &Value) -> Option<String> {
    match sys {
        Value::String(s) => Some(s.clone()),
        Value::Array(parts) => {
            let text: Vec<_> = parts
                .iter()
                .filter_map(|p| p.get("text").and_then(|t| t.as_str()))
                .collect();
            Some(text.join(""))
        }
        _ => None,
    }
}

fn flatten_content(content: Option<&Value>) -> String {
    let Some(content) = content else {
        return String::new();
    };
    match content {
        Value::String(s) => s.clone(),
        Value::Array(blocks) => blocks
            .iter()
            .filter_map(|b| {
                let t = b.get("type").and_then(|v| v.as_str()).unwrap_or("");
                match t {
                    "text" => b.get("text").and_then(|v| v.as_str()),
                    "image" => Some("[image]"),
                    _ => b.get("text").and_then(|v| v.as_str()),
                }
            })
            .collect::<Vec<_>>()
            .join(" "),
        _ => String::new(),
    }
}

fn convert_assistant_message(role: &str, content: Option<&Value>) -> Value {
    let Some(Value::Array(blocks)) = content else {
        return json!({"role": role, "content": flatten_content(content)});
    };

    let mut text_parts = Vec::new();
    let mut tool_calls = Vec::new();

    for block in blocks {
        let Some(b) = block.as_object() else { continue };
        match b.get("type").and_then(|v| v.as_str()).unwrap_or("") {
            "text" => {
                if let Some(t) = b.get("text").and_then(|v| v.as_str()) {
                    text_parts.push(t);
                }
            }
            "tool_use" => {
                let id = b.get("id").and_then(|v| v.as_str()).unwrap_or("");
                let name = b.get("name").and_then(|v| v.as_str()).unwrap_or("");
                let args = b.get("input").cloned().unwrap_or(json!({}));
                let args_str = serde_json::to_string(&args).unwrap_or_else(|_| "{}".into());
                tool_calls.push(json!({
                    "id": id,
                    "type": "function",
                    "function": {"name": name, "arguments": args_str}
                }));
            }
            _ => {}
        }
    }

    let mut msg = serde_json::Map::new();
    msg.insert("role".into(), json!(role));
    if !text_parts.is_empty() {
        msg.insert("content".into(), json!(text_parts.join(" ")));
    }
    if !tool_calls.is_empty() {
        msg.insert("tool_calls".into(), json!(tool_calls));
    }
    Value::Object(msg)
}

fn convert_user_message(role: &str, content: Option<&Value>) -> (Vec<Value>, Vec<Value>) {
    let Some(Value::Array(blocks)) = content else {
        return (
            vec![json!({"role": role, "content": flatten_content(content)})],
            Vec::new(),
        );
    };

    let mut text_parts = Vec::new();
    let mut tool_msgs = Vec::new();

    for block in blocks {
        let Some(b) = block.as_object() else { continue };
        match b.get("type").and_then(|v| v.as_str()).unwrap_or("") {
            "text" => {
                if let Some(t) = b.get("text").and_then(|v| v.as_str()) {
                    text_parts.push(t);
                }
            }
            "tool_result" => {
                let tool_use_id = b.get("tool_use_id").and_then(|v| v.as_str()).unwrap_or("");
                let result = extract_tool_result_content(b.get("content"));
                tool_msgs.push(json!({
                    "role": "tool",
                    "tool_call_id": tool_use_id,
                    "content": result,
                }));
            }
            _ => {}
        }
    }

    let mut user_msgs = Vec::new();
    if !text_parts.is_empty() {
        user_msgs.push(json!({"role": role, "content": text_parts.join(" ")}));
    }
    (user_msgs, tool_msgs)
}

fn extract_tool_result_content(content: Option<&Value>) -> String {
    match content {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Array(blocks)) => blocks
            .iter()
            .filter_map(|b| b.get("text").and_then(|t| t.as_str()))
            .collect::<Vec<_>>()
            .join(" "),
        _ => String::new(),
    }
}

fn trim_line(line: &[u8]) -> &[u8] {
    let mut start = 0;
    let mut end = line.len();
    while start < end && line[start].is_ascii_whitespace() {
        start += 1;
    }
    while end > start && line[end - 1].is_ascii_whitespace() {
        end -= 1;
    }
    &line[start..end]
}

fn hash_bytes(b: &[u8]) -> u64 {
    b.iter().fold(0u64, |h, &c| h.wrapping_mul(31).wrapping_add(c as u64))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_system_and_user_message() {
        let body = json!({
            "model": "claude-3",
            "max_tokens": 1024,
            "system": "be helpful",
            "messages": [{"role": "user", "content": "hello"}]
        });
        let req = anthropic_request_to_openai(&body).unwrap();
        assert_eq!(req.model, "claude-3");
        assert_eq!(req.messages.len(), 2);
        assert_eq!(req.messages[0].content.as_deref(), Some("be helpful"));
        assert!(!req.stream);
    }

    #[test]
    fn converts_tool_use_in_assistant_message() {
        let body = json!({
            "model": "claude-3",
            "max_tokens": 1024,
            "messages": [{
                "role": "assistant",
                "content": [{
                    "type": "tool_use",
                    "id": "tu_1",
                    "name": "get_weather",
                    "input": {"city": "Paris"}
                }]
            }]
        });
        let req = anthropic_request_to_openai(&body).unwrap();
        let msg = &req.messages[0];
        assert!(msg.tool_calls.is_some());
    }

    #[test]
    fn converts_non_stream_response() {
        let oai = br#"{"id":"chat-1","model":"m","choices":[{"message":{"role":"assistant","content":"hi"},"finish_reason":"stop"}],"usage":{"prompt_tokens":3,"completion_tokens":2}}"#;
        let anth = chat_json_to_anthropic(oai, "m");
        assert_eq!(anth["type"], "message");
        assert_eq!(anth["stop_reason"], "end_turn");
        assert_eq!(anth["content"][0]["text"], "hi");
    }
}