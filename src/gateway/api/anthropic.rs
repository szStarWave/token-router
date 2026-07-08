use axum::{
    Json,
    extract::State,
    http::HeaderMap,
    response::IntoResponse,
};
use bytes::Bytes;
use serde_json::{json, Value};

use crate::gateway::api::chat::{chat_completions_core, ChatOutputFormat};
use crate::gateway::api::compat::{
    apply_anthropic_upstream_options, extract_chat_delta_reasoning, extract_reasoning_field_text,
    finalize_chat_request_value, inject_stream_include_usage, InlineThinkState,
    strip_leading_anthropic_billing_header, ANTHROPIC_REDACTED_THINKING_PLACEHOLDER,
    StreamContentPiece, TOOL_CALL_REASONING_PLACEHOLDER,
};
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
    let base_url = state.config_mgr.get().cloud_base_url.clone();
    let req = anthropic_request_to_openai_with_base(&body, base_url.as_deref())?;
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
    anthropic_request_to_openai_with_base(body, None)
}

pub fn anthropic_request_to_openai_with_base(
    body: &Value,
    upstream_base: Option<&str>,
) -> AppResult<ChatCompletionRequest> {
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
        let converted = convert_tools(tools);
        if !converted.is_empty() {
            oai.insert("tools".into(), json!(converted));
            if let Some(tc) = obj.get("tool_choice") {
                oai.insert("tool_choice".into(), convert_tool_choice(tc));
            }
        }
    }

    oai.insert("messages".into(), json!(build_messages(obj)));

    let model = obj
        .get("model")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    apply_anthropic_upstream_options(&mut oai, body, model, upstream_base);
    inject_stream_include_usage(&mut oai);

    let finalized = finalize_chat_request_value(oai, model);
    serde_json::from_value(finalized)
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
    if let Some(reasoning) = extract_reasoning_field_text(&message) {
        content_blocks.push(json!({"type": "thinking", "thinking": reasoning}));
    }
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
    message_id: String,
    sent_message_start: bool,
    next_content_index: u32,
    non_tool_block_index: Option<u32>,
    non_tool_block_kind: Option<&'static str>,
    tool_blocks: std::collections::HashMap<i32, ToolBlockState>,
    has_emitted_message_delta: bool,
    pending_message_delta: Option<(String, Value)>,
    sent_message_stop: bool,
    done: bool,
    latest_usage: Value,
    inline_think: InlineThinkState,
}

#[derive(Default)]
struct ToolBlockState {
    anthropic_index: u32,
    id: String,
    name: String,
    started: bool,
    pending_args: String,
}

impl AnthropicSseTransform {
    pub fn new(model: String) -> Self {
        Self {
            model,
            message_id: String::new(),
            sent_message_start: false,
            next_content_index: 0,
            non_tool_block_index: None,
            non_tool_block_kind: None,
            tool_blocks: std::collections::HashMap::new(),
            has_emitted_message_delta: false,
            pending_message_delta: None,
            sent_message_stop: false,
            done: false,
            latest_usage: json!({"input_tokens": 0, "output_tokens": 0}),
            inline_think: InlineThinkState::default(),
        }
    }

    fn emit_event(&self, event_type: &str, payload: Value) -> Bytes {
        anthropic_sse_event(
            event_type,
            serde_json::to_vec(&payload).unwrap_or_default().as_slice(),
        )
    }

    fn ensure_message_start(&mut self, out: &mut Vec<Bytes>) {
        if self.sent_message_start {
            return;
        }
        self.sent_message_start = true;
        if self.message_id.is_empty() {
            self.message_id = format!("msg_{:x}", hash_bytes(self.model.as_bytes()));
        }
        let start = json!({
            "type": "message_start",
            "message": {
                "id": self.message_id,
                "type": "message",
                "role": "assistant",
                "content": [],
                "model": self.model,
                "stop_reason": null,
                "stop_sequence": null,
                "usage": self.latest_usage,
            }
        });
        out.push(self.emit_event("message_start", start));
    }

    fn close_non_tool_block(&mut self, out: &mut Vec<Bytes>) {
        let Some(index) = self.non_tool_block_index.take() else {
            self.non_tool_block_kind = None;
            return;
        };
        self.non_tool_block_kind = None;
        out.push(self.emit_event(
            "content_block_stop",
            json!({"type": "content_block_stop", "index": index}),
        ));
    }

    fn start_non_tool_block(&mut self, out: &mut Vec<Bytes>, kind: &'static str, empty: Value) {
        self.close_non_tool_block(out);
        let index = self.next_content_index;
        self.next_content_index += 1;
        self.non_tool_block_index = Some(index);
        self.non_tool_block_kind = Some(kind);
        out.push(self.emit_event(
            "content_block_start",
            json!({
                "type": "content_block_start",
                "index": index,
                "content_block": empty,
            }),
        ));
    }

    fn update_usage_from_chunk(&mut self, chunk: &Value) {
        let Some(usage) = chunk.get("usage").and_then(|u| u.as_object()) else {
            return;
        };
        let prompt = usage
            .get("prompt_tokens")
            .and_then(|v| v.as_i64())
            .unwrap_or(0);
        let completion = usage
            .get("completion_tokens")
            .and_then(|v| v.as_i64())
            .unwrap_or(0);
        self.latest_usage = json!({
            "input_tokens": prompt,
            "output_tokens": completion,
        });
    }

    fn push_thinking_delta(&mut self, reasoning: &str, out: &mut Vec<Bytes>) {
        self.ensure_message_start(out);
        if self.non_tool_block_kind != Some("thinking") {
            self.start_non_tool_block(
                out,
                "thinking",
                json!({"type": "thinking", "thinking": ""}),
            );
        }
        if let Some(index) = self.non_tool_block_index {
            out.push(self.emit_event(
                "content_block_delta",
                json!({
                    "type": "content_block_delta",
                    "index": index,
                    "delta": {"type": "thinking_delta", "thinking": reasoning},
                }),
            ));
        }
    }

    fn push_text_delta(&mut self, content: &str, out: &mut Vec<Bytes>) {
        if content.is_empty() {
            return;
        }
        self.ensure_message_start(out);
        if self.non_tool_block_kind != Some("text") {
            self.start_non_tool_block(
                out,
                "text",
                json!({"type": "text", "text": ""}),
            );
        }
        if let Some(index) = self.non_tool_block_index {
            out.push(self.emit_event(
                "content_block_delta",
                json!({
                    "type": "content_block_delta",
                    "index": index,
                    "delta": {"type": "text_delta", "text": content},
                }),
            ));
        }
    }

    fn push_content_delta(&mut self, delta: &str, out: &mut Vec<Bytes>) {
        let pieces = self.inline_think.push_content(delta);
        for piece in pieces {
            match piece {
                StreamContentPiece::Reasoning(reasoning) => {
                    self.push_thinking_delta(&reasoning, out);
                }
                StreamContentPiece::Text(text) => {
                    self.push_text_delta(&text, out);
                }
            }
        }
    }

    fn flush_inline_think_at_boundary(&mut self, out: &mut Vec<Bytes>) {
        let pieces = self.inline_think.flush();
        for piece in pieces {
            match piece {
                StreamContentPiece::Reasoning(reasoning) => {
                    self.push_thinking_delta(&reasoning, out);
                }
                StreamContentPiece::Text(text) => {
                    self.push_text_delta(&text, out);
                }
            }
        }
    }

    fn queue_message_delta(&mut self, stop_reason: &str) {
        if self.has_emitted_message_delta {
            if let Some((_, usage)) = self.pending_message_delta.as_mut() {
                *usage = self.latest_usage.clone();
            }
            return;
        }
        self.has_emitted_message_delta = true;
        self.pending_message_delta = Some((stop_reason.to_string(), self.latest_usage.clone()));
    }

    fn emit_pending_close(&mut self, out: &mut Vec<Bytes>) {
        self.close_non_tool_block(out);
        for state in self.tool_blocks.values() {
            if state.started {
                out.push(self.emit_event(
                    "content_block_stop",
                    json!({"type": "content_block_stop", "index": state.anthropic_index}),
                ));
            }
        }
    }
}

impl SseTransform for AnthropicSseTransform {
    fn transform_line(&mut self, line: &[u8]) -> Vec<Bytes> {
        if self.done {
            return Vec::new();
        }

        let trimmed = trim_line(line);
        if trimmed.is_empty() {
            return Vec::new();
        }
        if trimmed == DATA_DONE {
            return self.finish();
        }
        if !trimmed.starts_with(DATA_PREFIX) {
            return Vec::new();
        }

        let json_data = &trimmed[DATA_PREFIX.len()..];
        let Ok(chunk) = serde_json::from_slice::<Value>(json_data) else {
            return Vec::new();
        };

        if let Some(id) = chunk.get("id").and_then(|v| v.as_str()) {
            if !id.is_empty() {
                self.message_id = id.to_string();
            }
        }
        self.update_usage_from_chunk(&chunk);

        let choices = chunk.get("choices").and_then(|c| c.as_array());
        let Some(choices) = choices.filter(|c| !c.is_empty()) else {
            return Vec::new();
        };
        let choice = &choices[0];
        let delta = choice.get("delta").cloned().unwrap_or(json!({}));

        let mut out = Vec::new();

        let role = delta.get("role").and_then(|v| v.as_str()).unwrap_or("");
        if role == "assistant" {
            self.ensure_message_start(&mut out);
        } else if !self.sent_message_start
            && (extract_chat_delta_reasoning(&delta).is_some()
                || delta.get("content").and_then(|v| v.as_str()).is_some_and(|s| !s.is_empty())
                || delta.get("tool_calls").and_then(|v| v.as_array()).is_some_and(|t| !t.is_empty()))
        {
            self.ensure_message_start(&mut out);
        }

        if let Some(reasoning) = extract_chat_delta_reasoning(&delta) {
            self.push_thinking_delta(&reasoning, &mut out);
        }

        if let Some(content) = delta.get("content").and_then(|v| v.as_str()) {
            if !content.is_empty() {
                self.push_content_delta(content, &mut out);
            }
        }

        if let Some(tool_calls) = delta.get("tool_calls").and_then(|t| t.as_array()) {
            self.ensure_message_start(&mut out);
            self.flush_inline_think_at_boundary(&mut out);
            self.close_non_tool_block(&mut out);
            for tc in tool_calls {
                let index = tc.get("index").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
                let mut emit_start = None;
                let mut emit_delta = None;

                {
                    let state = self.tool_blocks.entry(index).or_insert_with(|| {
                        let anthropic_index = self.next_content_index;
                        self.next_content_index += 1;
                        ToolBlockState {
                            anthropic_index,
                            ..Default::default()
                        }
                    });
                    if let Some(id) = tc.get("id").and_then(|v| v.as_str()) {
                        if !id.is_empty() {
                            state.id = id.to_string();
                        }
                    }
                    if let Some(name) = tc
                        .pointer("/function/name")
                        .and_then(|v| v.as_str())
                    {
                        if !name.is_empty() {
                            state.name = name.to_string();
                        }
                    }
                    let args = tc
                        .pointer("/function/arguments")
                        .and_then(|v| v.as_str())
                        .unwrap_or("");
                    if !args.is_empty() {
                        if state.started {
                            emit_delta = Some((state.anthropic_index, args.to_string()));
                        } else {
                            state.pending_args.push_str(args);
                        }
                    }
                    let should_start =
                        !state.started && !state.id.is_empty() && !state.name.is_empty();
                    if should_start {
                        state.started = true;
                        let pending = std::mem::take(&mut state.pending_args);
                        emit_start = Some((
                            state.anthropic_index,
                            state.id.clone(),
                            state.name.clone(),
                            pending,
                        ));
                    }
                }

                if let Some((anthropic_index, id, name, pending)) = emit_start {
                    out.push(self.emit_event(
                        "content_block_start",
                        json!({
                            "type": "content_block_start",
                            "index": anthropic_index,
                            "content_block": {
                                "type": "tool_use",
                                "id": id,
                                "name": name,
                                "input": {},
                            },
                        }),
                    ));
                    if !pending.is_empty() {
                        out.push(self.emit_event(
                            "content_block_delta",
                            json!({
                                "type": "content_block_delta",
                                "index": anthropic_index,
                                "delta": {"type": "input_json_delta", "partial_json": pending},
                            }),
                        ));
                    }
                }
                if let Some((anthropic_index, args)) = emit_delta {
                    out.push(self.emit_event(
                        "content_block_delta",
                        json!({
                            "type": "content_block_delta",
                            "index": anthropic_index,
                            "delta": {"type": "input_json_delta", "partial_json": args},
                        }),
                    ));
                }
            }
        }

        if let Some(finish) = choice.get("finish_reason").and_then(|v| v.as_str()) {
            self.emit_pending_close(&mut out);
            self.queue_message_delta(map_finish_reason(finish));
        }

        out
    }

    fn finish(&mut self) -> Vec<Bytes> {
        if self.done {
            return Vec::new();
        }
        self.done = true;
        let mut out = Vec::new();

        if !self.sent_message_start {
            self.ensure_message_start(&mut out);
        }

        self.emit_pending_close(&mut out);

        if let Some((stop_reason, usage)) = self.pending_message_delta.take() {
            out.push(self.emit_event(
                "message_delta",
                json!({
                    "type": "message_delta",
                    "delta": {"stop_reason": stop_reason, "stop_sequence": null},
                    "usage": usage,
                }),
            ));
        } else if self.has_emitted_message_delta {
            out.push(self.emit_event(
                "message_delta",
                json!({
                    "type": "message_delta",
                    "delta": {"stop_reason": "end_turn", "stop_sequence": null},
                    "usage": self.latest_usage,
                }),
            ));
        }

        if !self.sent_message_stop {
            out.push(self.emit_event("message_stop", json!({"type": "message_stop"})));
            self.sent_message_stop = true;
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
            if tool
                .get("name")
                .and_then(|v| v.as_str())
                .is_some_and(|name| name == "BatchTool")
            {
                return None;
            }
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
    let text = match sys {
        Value::String(s) => s.clone(),
        Value::Array(parts) => {
            let text: Vec<_> = parts
                .iter()
                .filter_map(|p| p.get("text").and_then(|t| t.as_str()))
                .collect();
            text.join("")
        }
        _ => return None,
    };
    let stripped = strip_leading_anthropic_billing_header(&text);
    if stripped.is_empty() {
        None
    } else {
        Some(stripped.to_string())
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
    let mut reasoning_parts = Vec::new();

    for block in blocks {
        let Some(b) = block.as_object() else { continue };
        match b.get("type").and_then(|v| v.as_str()).unwrap_or("") {
            "text" => {
                if let Some(t) = b.get("text").and_then(|v| v.as_str()) {
                    text_parts.push(t);
                }
            }
            "thinking" => {
                if let Some(t) = b.get("thinking").and_then(|v| v.as_str()) {
                    if !t.is_empty() {
                        reasoning_parts.push(t.to_string());
                    }
                }
            }
            "redacted_thinking" => {
                reasoning_parts.push(ANTHROPIC_REDACTED_THINKING_PLACEHOLDER.to_string());
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
        if text_parts.is_empty() {
            msg.insert("content".into(), Value::Null);
        }
        let reasoning_content = if reasoning_parts.is_empty() {
            TOOL_CALL_REASONING_PLACEHOLDER.to_string()
        } else {
            reasoning_parts.join("\n")
        };
        msg.insert("reasoning_content".into(), json!(reasoning_content));
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
    fn converts_tool_use_with_thinking_to_reasoning_content() {
        let body = json!({
            "model": "deepseek-v4-flash",
            "max_tokens": 1024,
            "messages": [{
                "role": "assistant",
                "content": [
                    {"type": "thinking", "thinking": "I should call the tool."},
                    {"type": "tool_use", "id": "tu_1", "name": "get_weather", "input": {"city": "Paris"}}
                ]
            }]
        });
        let req = anthropic_request_to_openai(&body).unwrap();
        let msg = &req.messages[0];
        assert_eq!(
            msg.reasoning_content.as_deref(),
            Some("I should call the tool.")
        );
    }

    #[test]
    fn converts_redacted_thinking_placeholder() {
        let body = json!({
            "model": "deepseek-v4-flash",
            "max_tokens": 1024,
            "messages": [{
                "role": "assistant",
                "content": [
                    {"type": "redacted_thinking", "data": "opaque"},
                    {"type": "tool_use", "id": "tu_1", "name": "read", "input": {}}
                ]
            }]
        });
        let req = anthropic_request_to_openai(&body).unwrap();
        assert_eq!(
            req.messages[0].reasoning_content.as_deref(),
            Some(ANTHROPIC_REDACTED_THINKING_PLACEHOLDER)
        );
    }

    #[test]
    fn strips_billing_header_from_system() {
        let body = json!({
            "model": "deepseek-v4-flash",
            "max_tokens": 1024,
            "system": "x-anthropic-billing-header: cch=abc\n\nBe helpful.",
            "messages": [{"role": "user", "content": "hi"}]
        });
        let req = anthropic_request_to_openai(&body).unwrap();
        assert_eq!(req.messages[0].content.as_deref(), Some("Be helpful."));
    }

    #[test]
    fn sse_reasoning_content_emits_thinking_delta() {
        use crate::gateway::api::sse_transform::SseTransform;

        let mut transform = AnthropicSseTransform::new("deepseek".into());
        let chunk = json!({
            "id": "abc",
            "choices": [{"delta": {"reasoning_content": "planning..."}}]
        });
        let line = format!("data: {}\n", chunk);
        let out = transform.transform_line(line.as_bytes());
        let text = out
            .iter()
            .map(|b| String::from_utf8_lossy(b))
            .collect::<String>();
        assert!(text.contains("event: content_block_delta"));
        assert!(text.contains("thinking_delta"));
        assert!(text.contains("planning..."));
    }

    #[test]
    fn sse_reasoning_details_emits_thinking_delta() {
        use crate::gateway::api::sse_transform::SseTransform;

        let mut transform = AnthropicSseTransform::new("minimax".into());
        let chunk = json!({
            "id": "abc",
            "choices": [{"delta": {"reasoning_details": [{"text": "step one"}]}}]
        });
        let line = format!("data: {}\n", chunk);
        let out = transform.transform_line(line.as_bytes());
        let text = out
            .iter()
            .map(|b| String::from_utf8_lossy(b))
            .collect::<String>();
        assert!(text.contains("thinking_delta"));
        assert!(text.contains("step one"));
    }

    #[test]
    fn sse_inline_think_content_emits_thinking_without_tag_leak() {
        use crate::gateway::api::sse_transform::SseTransform;

        let mut transform = AnthropicSseTransform::new("minimax".into());
        let chunk = json!({
            "id": "abc",
            "choices": [{"delta": {"content": "<think>plan</think>Answer"}}]
        });
        let line = format!("data: {}\n", chunk);
        let out = transform.transform_line(line.as_bytes());
        let text = out
            .iter()
            .map(|b| String::from_utf8_lossy(b))
            .collect::<String>();
        assert!(text.contains("thinking_delta"));
        assert!(text.contains("plan"));
        assert!(!text.contains("<think>"));
        assert!(text.contains("text_delta"));
        assert!(text.contains("Answer"));
    }

    #[test]
    fn converts_reasoning_content_in_response() {
        let oai = br#"{"id":"chat-1","model":"m","choices":[{"message":{"role":"assistant","reasoning_content":"plan","content":"hi"},"finish_reason":"stop"}],"usage":{"prompt_tokens":3,"completion_tokens":2}}"#;
        let anth = chat_json_to_anthropic(oai, "m");
        assert_eq!(anth["content"][0]["type"], "thinking");
        assert_eq!(anth["content"][0]["thinking"], "plan");
        assert_eq!(anth["content"][1]["text"], "hi");
    }
}