use std::collections::HashMap;
use std::sync::Arc;

use axum::{
    Json,
    extract::State,
    http::HeaderMap,
    response::IntoResponse,
};
use bytes::Bytes;
use serde_json::{json, Map, Value};

use crate::gateway::api::chat::{chat_completions_core, ChatOutputFormat};
use crate::gateway::api::compat::{
    apply_reasoning_options, convert_responses_input_to_messages, finalize_chat_request_value,
    response_id_from_chat_id, CodexChatHistoryStore,
};
use crate::gateway::api::openai::{ChatCompletionRequest, ChatCompletionResponse, Role};
use crate::gateway::api::routes::AppState;
use crate::gateway::api::sse_transform::{responses_sse_event, SseTransform};
use crate::gateway::error::{AppError, AppResult};

const DATA_PREFIX: &[u8] = b"data: ";
const DATA_DONE: &[u8] = b"data: [DONE]";

macro_rules! event_map {
    ($($k:expr => $v:expr),* $(,)?) => {{
        let mut m = Map::new();
        $(m.insert($k, $v);)*
        m
    }};
}

pub async fn responses_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(mut body): Json<Value>,
) -> AppResult<impl IntoResponse> {
    let original_body = body.clone();
    let input_before = body
        .get("input")
        .and_then(|v| v.as_array())
        .map(|a| a.len())
        .unwrap_or(0);
    state.codex_history.enrich_request(&mut body);
    let input_after = body
        .get("input")
        .and_then(|v| v.as_array())
        .map(|a| a.len())
        .unwrap_or(0);
    let restored_calls = input_after.saturating_sub(input_before);
    state.codex_history.record_calls_from_input(&body);
    let agent_id = super::routes::extract_agent_id(&headers);
    let base_url = state.config_mgr.get().cloud_base_url.clone();
    let req = responses_request_to_openai_with_base(&body, base_url.as_deref())?;
    log_responses_convert(&body, &req, restored_calls);
    tracing::info!(
        original = %serde_json::to_string(&original_body).unwrap_or_default(),
        "responses raw request"
    );
    let converted_json = serde_json::to_value(&req).unwrap_or_default();
    tracing::info!(
        converted = %serde_json::to_string(&converted_json).unwrap_or_default(),
        "responses converted request"
    );
    chat_completions_core(
        state,
        headers,
        agent_id,
        req,
        ChatOutputFormat::Responses,
    )
    .await
}

fn log_responses_convert(body: &Value, req: &ChatCompletionRequest, restored_calls: usize) {
    let previous_response_id = body
        .get("previous_response_id")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let input_items = body
        .get("input")
        .and_then(|v| v.as_array())
        .map(|a| a.len())
        .unwrap_or(0);
    let tool_calls_in_req = req
        .tools
        .len();
    let assistant_tool_calls = req
        .messages
        .iter()
        .filter(|m| {
            m.role == Role::Assistant
                && m.tool_calls.as_ref().is_some_and(|c| !c.is_empty())
        })
        .count();
    let reasoning_placeholders = req
        .messages
        .iter()
        .filter(|m| {
            m.role == Role::Assistant
                && m.tool_calls.as_ref().is_some_and(|c| !c.is_empty())
                && m.reasoning_content.as_deref() == Some(crate::gateway::api::compat::TOOL_CALL_REASONING_PLACEHOLDER)
        })
        .count();
    let missing_reasoning = req
        .messages
        .iter()
        .filter(|m| {
            m.role == Role::Assistant
                && m.tool_calls.as_ref().is_some_and(|c| !c.is_empty())
                && m.reasoning_content.as_deref().map(|s| s.trim().is_empty()).unwrap_or(true)
        })
        .count();

    tracing::info!(
        previous_response_id = %previous_response_id,
        input_items,
        messages = req.messages.len(),
        tools = tool_calls_in_req,
        restored_calls,
        assistant_tool_calls,
        reasoning_placeholders,
        missing_reasoning,
        "responses convert"
    );
}

pub fn responses_request_to_openai(body: &Value) -> AppResult<ChatCompletionRequest> {
    responses_request_to_openai_with_base(body, None)
}

pub fn responses_request_to_openai_with_base(body: &Value, upstream_base: Option<&str>) -> AppResult<ChatCompletionRequest> {
    let obj = body
        .as_object()
        .ok_or_else(|| AppError::BadRequest("invalid request body".into()))?;

    let mut result = Map::new();

    if let Some(m) = obj.get("model") {
        result.insert("model".into(), m.clone());
    }
    if let Some(s) = obj.get("stream") {
        result.insert("stream".into(), s.clone());
    } else {
        result.insert("stream".into(), json!(false));
    }
    if let Some(t) = obj.get("temperature") {
        result.insert("temperature".into(), t.clone());
    }
    if let Some(tp) = obj.get("top_p") {
        result.insert("top_p".into(), tp.clone());
    }
    if let Some(v) = obj.get("max_output_tokens") {
        result.insert("max_tokens".into(), json!(as_i64(v)));
    } else if let Some(v) = obj.get("max_tokens") {
        result.insert("max_tokens".into(), json!(as_i64(v)));
    }

    let mut chat_tools = obj
        .get("tools")
        .and_then(|t| t.as_array())
        .map(|tools| convert_responses_tools_to_chat(tools))
        .unwrap_or_default();

    if let Some(tc) = obj.get("tool_choice") {
        if let Some(choice) = convert_responses_tool_choice_to_chat(tc, &mut chat_tools) {
            result.insert("tool_choice".into(), choice);
        }
    }
    if !chat_tools.is_empty() {
        result.insert("tools".into(), json!(chat_tools));
    }

    if let Some(v) = obj.get("parallel_tool_calls") {
        result.insert("parallel_tool_calls".into(), v.clone());
    }

    let instructions = obj
        .get("instructions")
        .and_then(|v| v.as_str())
        .map(|s| crate::gateway::api::compat::strip_leading_anthropic_billing_header(s).to_string())
        .unwrap_or_default();
    if let Some(input) = obj.get("input") {
        result.insert(
            "messages".into(),
            json!(convert_responses_input_to_messages(input, &instructions)),
        );
    } else if !instructions.trim().is_empty() {
        result.insert(
            "messages".into(),
            json!(convert_responses_input_to_messages(&Value::Null, &instructions)),
        );
    }

    let model = obj
        .get("model")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    apply_reasoning_options(&mut result, body, model, upstream_base);

    let finalized = finalize_chat_request_value(result, model);
    serde_json::from_value(finalized)
        .map_err(|e| AppError::BadRequest(format!("invalid responses request: {e}")))
}

pub fn chat_response_to_responses(resp: &ChatCompletionResponse) -> Value {
    let body = serde_json::to_value(resp).unwrap_or(json!({}));
    chat_json_to_responses(&body)
}

pub fn chat_json_to_responses(chat: &Value) -> Value {
    let id_raw = chat.get("id").and_then(|v| v.as_str());
    let responses_id = json!(response_id_from_chat_id(id_raw));
    let model = chat.get("model").cloned().unwrap_or(json!(""));
    let created = as_i64(chat.get("created").unwrap_or(&Value::Null));

    let mut output = Vec::new();
    if let Some(choices) = chat.get("choices").and_then(|c| c.as_array()) {
        for (i, choice) in choices.iter().enumerate() {
            if let Some(msg) = choice.get("message") {
                output.extend(convert_chat_message_to_responses_output_items(
                    &responses_id,
                    i,
                    msg,
                ));
            }
        }
    }

    let mut responses = Map::new();
    responses.insert("id".into(), responses_id);
    responses.insert("object".into(), json!("response"));
    responses.insert("created_at".into(), json!(created));
    responses.insert("model".into(), model);
    responses.insert("status".into(), json!("completed"));
    responses.insert("output".into(), json!(output));
    responses.insert("parallel_tool_calls".into(), json!(true));

    if let Some(usage) = chat.get("usage").and_then(|u| u.as_object()) {
        let input_tokens = as_i64(usage.get("prompt_tokens").unwrap_or(&Value::Null));
        let output_tokens = as_i64(usage.get("completion_tokens").unwrap_or(&Value::Null));
        let cached = usage
            .get("prompt_tokens_details")
            .and_then(|d| d.get("cached_tokens"))
            .map(as_i64)
            .unwrap_or(0);
        responses.insert(
            "usage".into(),
            json!({
                "input_tokens": input_tokens,
                "output_tokens": output_tokens,
                "input_tokens_details": {"cached_tokens": cached},
                "output_tokens_details": {"reasoning_tokens": 0},
                "total_tokens": input_tokens + output_tokens,
            }),
        );
    }

    Value::Object(responses)
}

pub struct ResponsesSseTransform {
    sent_created: bool,
    text_started: bool,
    response_id: String,
    message_id: String,
    model: String,
    role: String,
    created_at: i64,
    next_output_index: i32,
    text_output_index: i32,
    output_text: String,
    tool_calls: HashMap<i32, ToolCallState>,
    tool_call_order: Vec<i32>,
    sequence: i64,
    done: bool,
    prompt_tokens: i64,
    completion_tokens: i64,
    cached_tokens: i64,
    reasoning_text: String,
    reasoning_started: bool,
    reasoning_output_index: i32,
    codex_history: Option<Arc<CodexChatHistoryStore>>,
}

#[derive(Clone)]
struct ToolCallState {
    output_index: i32,
    item_id: String,
    call_id: String,
    name: String,
    arguments: String,
}

impl ResponsesSseTransform {
    pub fn new() -> Self {
        Self::with_codex_history(None)
    }

    pub fn with_codex_history(codex_history: Option<Arc<CodexChatHistoryStore>>) -> Self {
        Self {
            sent_created: false,
            text_started: false,
            response_id: "resp_completed".into(),
            message_id: "msg_completed".into(),
            model: String::new(),
            role: "assistant".into(),
            created_at: 0,
            next_output_index: 0,
            text_output_index: -1,
            output_text: String::new(),
            tool_calls: HashMap::new(),
            tool_call_order: Vec::new(),
            sequence: 0,
            done: false,
            prompt_tokens: 0,
            completion_tokens: 0,
            cached_tokens: 0,
            reasoning_text: String::new(),
            reasoning_started: false,
            reasoning_output_index: -1,
            codex_history,
        }
    }

    fn emit(&mut self, event_type: &str, mut payload: Map<String, Value>) -> Bytes {
        self.sequence += 1;
        payload.insert("sequence_number".into(), json!(self.sequence));
        payload.insert("type".into(), json!(event_type));
        let bytes = serde_json::to_vec(&Value::Object(payload)).unwrap_or_default();
        responses_sse_event(event_type, &bytes)
    }

    fn ensure_created(&mut self, out: &mut Vec<Bytes>) {
        if self.sent_created {
            return;
        }
        out.push(self.emit(
            "response.created",
            responses_created_event(&self.response_id, &self.model, self.created_at),
        ));
        out.push(self.emit(
            "response.in_progress",
            responses_in_progress_event(&self.response_id, &self.model, self.created_at),
        ));
        self.sent_created = true;
    }
}

impl Default for ResponsesSseTransform {
    fn default() -> Self {
        Self::new()
    }
}

impl SseTransform for ResponsesSseTransform {
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

        let mut out = Vec::new();

        if let Some(usage) = chunk.get("usage").and_then(|u| u.as_object()) {
            self.prompt_tokens = as_i64(usage.get("prompt_tokens").unwrap_or(&Value::Null));
            self.completion_tokens =
                as_i64(usage.get("completion_tokens").unwrap_or(&Value::Null));
            self.cached_tokens = usage
                .get("prompt_cache_hit_tokens")
                .map(as_i64)
                .unwrap_or(0);
        }

        if let Some(id) = chunk.get("id").and_then(|v| v.as_str()) {
            self.response_id = format!("resp_{id}");
            self.message_id = format!("msg_{id}");
        }
        if let Some(m) = chunk.get("model").and_then(|v| v.as_str()) {
            self.model = m.to_string();
        }
        if let Some(c) = chunk.get("created") {
            self.created_at = as_i64(c);
        }

        let choices = chunk.get("choices").and_then(|c| c.as_array());
        if let Some(choices) = choices.filter(|c| !c.is_empty()) {
            let choice = &choices[0];
            if let Some(delta) = choice.get("delta") {
                if let Some(role) = delta.get("role").and_then(|v| v.as_str()) {
                    self.role = role.to_string();
                }
                if let Some(content) = delta.get("content").and_then(|v| v.as_str()) {
                    if !content.is_empty() {
                        self.ensure_created(&mut out);
                        if !self.text_started {
                            self.text_output_index = self.next_output_index;
                            self.next_output_index += 1;
                            let item = responses_message_item_with_status(
                                &self.message_id,
                                &self.role,
                                &[],
                                "in_progress",
                            );
                            out.push(self.emit(
                                "response.output_item.added",
                                event_map! {
                                    "response_id".into() => json!(self.response_id),
                                    "output_index".into() => json!(self.text_output_index),
                                    "item".into() => item,
                                },
                            ));
                            out.push(self.emit(
                                "response.content_part.added",
                                event_map! {
                                    "response_id".into() => json!(self.response_id),
                                    "item_id".into() => json!(self.message_id),
                                    "output_index".into() => json!(self.text_output_index),
                                    "content_index".into() => json!(0),
                                    "part".into() => responses_output_text_part(""),
                                },
                            ));
                            self.text_started = true;
                        }
                        self.output_text.push_str(content);
                        out.push(self.emit(
                            "response.output_text.delta",
                            event_map! {
                                "response_id".into() => json!(self.response_id),
                                "item_id".into() => json!(self.message_id),
                                "output_index".into() => json!(self.text_output_index),
                                "content_index".into() => json!(0),
                                "delta".into() => json!(content),
                            },
                        ));
                    }
                }

                if let Some(reasoning) = delta.get("reasoning_content").and_then(|v| v.as_str()) {
                    if !reasoning.is_empty() {
                        self.ensure_created(&mut out);
                        if !self.reasoning_started {
                            self.reasoning_output_index = self.next_output_index;
                            self.next_output_index += 1;
                            self.reasoning_started = true;
                        }
                        self.reasoning_text.push_str(reasoning);
                    }
                }

                if let Some(tool_calls) = delta.get("tool_calls").and_then(|t| t.as_array()) {
                    for tc in tool_calls {
                        let index = tc.get("index").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
                        let call_id = tc.get("id").and_then(|v| v.as_str()).unwrap_or("");
                        let name = tc
                            .get("function")
                            .and_then(|f| f.get("name"))
                            .and_then(|v| v.as_str())
                            .unwrap_or("");
                        let args = tc
                            .get("function")
                            .and_then(|f| f.get("arguments"))
                            .and_then(|v| v.as_str())
                            .unwrap_or("");

                        self.ensure_created(&mut out);
                        let created = !self.tool_calls.contains_key(&index);
                        if created {
                            let cid = if call_id.is_empty() {
                                format!("call_{}_{}", self.response_id, index)
                            } else {
                                call_id.to_string()
                            };
                            let state = ToolCallState {
                                output_index: self.next_output_index,
                                item_id: format!("fc_{cid}"),
                                call_id: cid,
                                name: name.to_string(),
                                arguments: String::new(),
                            };
                            self.next_output_index += 1;
                            self.tool_call_order.push(index);
                            let item = tool_call_item(&state, false);
                            out.push(self.emit(
                                "response.output_item.added",
                                event_map! {
                                    "response_id".into() => json!(self.response_id),
                                    "output_index".into() => json!(state.output_index),
                                    "item".into() => item,
                                },
                            ));
                            self.tool_calls.insert(index, state);
                        }
                        if !args.is_empty() {
                            if let Some(state) = self.tool_calls.get_mut(&index) {
                                state.arguments.push_str(args);
                                let item_id = state.item_id.clone();
                                let output_index = state.output_index;
                                out.push(self.emit(
                                    "response.function_call_arguments.delta",
                                    event_map! {
                                        "response_id".into() => json!(self.response_id),
                                        "item_id".into() => json!(item_id),
                                        "output_index".into() => json!(output_index),
                                        "delta".into() => json!(args),
                                    },
                                ));
                            }
                        } else if created {
                            if let Some(state) = self.tool_calls.get_mut(&index) {
                                if !call_id.is_empty() {
                                    state.call_id = call_id.to_string();
                                    state.item_id = format!("fc_{call_id}");
                                }
                                if !name.is_empty() {
                                    state.name = name.to_string();
                                }
                            }
                        }
                    }
                }
            }

            if choice.get("finish_reason").is_some() {
                // finish handled in finish()
            }
        }

        out
    }

    fn finish(&mut self) -> Vec<Bytes> {
        if self.done {
            return Vec::new();
        }
        self.done = true;
        let mut out = Vec::new();

        if !self.sent_created {
            out.push(self.emit(
                "response.created",
                responses_created_event(&self.response_id, &self.model, self.created_at),
            ));
            out.push(self.emit(
                "response.in_progress",
                responses_in_progress_event(&self.response_id, &self.model, self.created_at),
            ));
            self.sent_created = true;
        }

        let mut output_items: Vec<Option<Value>> =
            vec![None; self.next_output_index.max(0) as usize];

        if self.reasoning_started {
            let reasoning = std::mem::take(&mut self.reasoning_text);
            let item_id = format!("reason_{}", self.message_id);
            let item = json!({
                "id": item_id,
                "type": "reasoning",
                "status": "completed",
                "content": [{"type": "text", "text": reasoning, "annotations": []}],
            });
            if self.reasoning_output_index >= 0 {
                output_items[self.reasoning_output_index as usize] = Some(item);
            }
        }

        if self.text_started {
            let text = self.output_text.clone();
            out.push(self.emit(
                "response.output_text.done",
                        event_map! {
                    "response_id".into() => json!(self.response_id),
                    "item_id".into() => json!(self.message_id),
                    "output_index".into() => json!(self.text_output_index),
                    "content_index".into() => json!(0),
                    "text".into() => json!(text),
                },
            ));
            let part = responses_output_text_part(&text);
            out.push(self.emit(
                "response.content_part.done",
                        event_map! {
                    "response_id".into() => json!(self.response_id),
                    "item_id".into() => json!(self.message_id),
                    "output_index".into() => json!(self.text_output_index),
                    "content_index".into() => json!(0),
                    "part".into() => part.clone(),
                },
            ));
            let item = responses_message_item_with_status(
                &self.message_id,
                &self.role,
                &[part],
                "completed",
            );
            out.push(self.emit(
                "response.output_item.done",
                        event_map! {
                    "response_id".into() => json!(self.response_id),
                    "output_index".into() => json!(self.text_output_index),
                    "item".into() => item.clone(),
                },
            ));
            if self.text_output_index >= 0 {
                output_items[self.text_output_index as usize] = Some(item);
            }
        }

        let tool_order: Vec<i32> = self.tool_call_order.clone();
        for index in tool_order {
            let Some(state) = self.tool_calls.get(&index).cloned() else {
                continue;
            };
            out.push(self.emit(
                "response.function_call_arguments.done",
                event_map! {
                    "response_id".into() => json!(self.response_id),
                    "item_id".into() => json!(state.item_id),
                    "output_index".into() => json!(state.output_index),
                    "name".into() => json!(state.name),
                    "arguments".into() => json!(state.arguments),
                },
            ));
            let item = tool_call_item(&state, true);
            out.push(self.emit(
                "response.output_item.done",
                event_map! {
                    "response_id".into() => json!(self.response_id),
                    "output_index".into() => json!(state.output_index),
                    "item".into() => item.clone(),
                },
            ));
            if let Some(store) = &self.codex_history {
                store.record_call_item(&self.response_id, &item);
            }
            output_items[state.output_index as usize] = Some(item);
        }

        let function_calls = self.tool_calls.len();
        let text_blocks = if self.text_started { 1 } else { 0 };

        // Log upstream accumulated response
        let mut upstream_choices = Vec::new();
        let upstream_message = json!({
            "role": self.role,
            "content": self.output_text,
            "tool_calls": self.tool_calls.values().map(|tc| json!({
                "id": tc.call_id,
                "type": "function",
                "function": {"name": tc.name, "arguments": tc.arguments}
            })).collect::<Vec<_>>(),
        });
        upstream_choices.push(json!({
            "index": 0,
            "message": upstream_message,
            "finish_reason": "stop",
        }));
        let upstream_response = json!({
            "id": self.response_id.trim_start_matches("resp_"),
            "object": "chat.completion",
            "created": self.created_at,
            "model": self.model,
            "choices": upstream_choices,
            "usage": {
                "prompt_tokens": self.prompt_tokens,
                "completion_tokens": self.completion_tokens,
            },
        });
        tracing::info!(
            upstream = %serde_json::to_string(&upstream_response).unwrap_or_default(),
            "responses sse upstream accumulated"
        );

        let compact: Vec<Value> = output_items.into_iter().flatten().collect();
        let completed_response = responses_completed_response(
            &self.response_id,
            &self.model,
            self.created_at,
            &compact,
            self.prompt_tokens,
            self.completion_tokens,
            self.cached_tokens,
        );
        if let Some(store) = &self.codex_history {
            store.record_response(&completed_response);
        }
        tracing::info!(
            response_id = %self.response_id,
            function_calls,
            text_blocks,
            body = %serde_json::to_string(&completed_response).unwrap_or_default(),
            "responses sse recorded"
        );
        out.push(self.emit(
            "response.completed",
            event_map! {
                "response".into() => completed_response,
            },
        ));

        out
    }
}

fn tool_call_item(state: &ToolCallState, completed: bool) -> Value {
    json!({
        "id": state.item_id,
        "type": "function_call",
        "status": if completed { "completed" } else { "in_progress" },
        "call_id": state.call_id,
        "name": state.name,
        "arguments": state.arguments,
    })
}

fn convert_responses_tools_to_chat(tools: &[Value]) -> Vec<Value> {
    let mut converted = Vec::new();
    for tool in tools {
        let Some(t) = tool.as_object() else { continue };
        if t.contains_key("function") {
            converted.push(Value::Object(t.clone()));
            continue;
        }
        if t.get("type").and_then(|v| v.as_str()) != Some("function") {
            continue;
        }
        let name = t.get("name").cloned().unwrap_or(json!(""));
        let description = t.get("description").cloned().unwrap_or(json!(""));
        let parameters = t.get("parameters").cloned().unwrap_or(json!({}));
        let mut function = Map::new();
        function.insert("name".into(), name);
        function.insert("description".into(), description);
        function.insert("parameters".into(), parameters);
        if let Some(strict) = t.get("strict") {
            function.insert("strict".into(), strict.clone());
        }
        converted.push(json!({"type": "function", "function": function}));
    }
    converted
}

fn convert_responses_tool_choice_to_chat(
    tool_choice: &Value,
    chat_tools: &mut Vec<Value>,
) -> Option<Value> {
    match tool_choice {
        Value::String(s) if !s.is_empty() => Some(Value::String(s.clone())),
        Value::Object(map) => {
            let choice_type = map.get("type").and_then(|v| v.as_str()).unwrap_or("");
            match choice_type {
                "function" => {
                    let name = map.get("name").and_then(|v| v.as_str()).unwrap_or("");
                    if name.is_empty() {
                        None
                    } else {
                        Some(json!({"type": "function", "function": {"name": name}}))
                    }
                }
                "allowed_tools" => {
                    *chat_tools = filter_chat_tools_by_allowed_names(chat_tools, map.get("tools"));
                    let mode = map.get("mode").and_then(|v| v.as_str()).unwrap_or("auto");
                    Some(json!(mode))
                }
                _ => Some(Value::Object(map.clone())),
            }
        }
        _ => None,
    }
}

fn filter_chat_tools_by_allowed_names(tools: &[Value], allowed: Option<&Value>) -> Vec<Value> {
    let Some(Value::Array(allowed_list)) = allowed else {
        return tools.to_vec();
    };
    if allowed_list.is_empty() {
        return tools.to_vec();
    }
    let names: std::collections::HashSet<String> = allowed_list
        .iter()
        .filter_map(|item| {
            item.get("name")
                .and_then(|v| v.as_str())
                .map(str::to_string)
        })
        .collect();
    if names.is_empty() {
        return tools.to_vec();
    }
    tools
        .iter()
        .filter(|tool| {
            tool.get("function")
                .and_then(|f| f.get("name"))
                .and_then(|v| v.as_str())
                .is_some_and(|n| names.contains(n))
        })
        .cloned()
        .collect()
}

fn stringify_function_output(output: Option<&Value>) -> String {
    match output {
        Some(Value::String(s)) => s.clone(),
        None => String::new(),
        Some(v) => serde_json::to_string(v).unwrap_or_default(),
    }
}

fn convert_chat_message_to_responses_output_items(
    chat_id: &Value,
    choice_index: usize,
    msg: &Value,
) -> Vec<Value> {
    let mut output = Vec::new();

    if let Some(reasoning) = msg
        .get("reasoning_content")
        .and_then(|v| v.as_str())
        .filter(|s| !s.trim().is_empty())
    {
        output.push(json!({
            "id": format!("reason_{chat_id}_{choice_index}"),
            "type": "reasoning",
            "status": "completed",
            "content": [{"type": "text", "text": reasoning, "annotations": []}],
        }));
    }

    if let Some(tool_calls) = msg.get("tool_calls").and_then(|t| t.as_array()) {
        for (i, tool_call) in tool_calls.iter().enumerate() {
            let call_id = tool_call.get("id").and_then(|v| v.as_str()).unwrap_or("");
            let function = tool_call.get("function");
            let name = function
                .and_then(|f| f.get("name"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let arguments = function
                .and_then(|f| f.get("arguments"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let cid = if call_id.is_empty() {
                format!("call_{chat_id}_{choice_index}_{i}")
            } else {
                call_id.to_string()
            };
            output.push(json!({
                "id": format!("fc_{cid}"),
                "type": "function_call",
                "status": "completed",
                "call_id": cid,
                "name": name,
                "arguments": arguments,
            }));
        }
    }

    let content_array = convert_chat_message_content_to_responses_content(msg.get("content"));
    if !content_array.is_empty() {
        let role = msg.get("role").and_then(|v| v.as_str()).unwrap_or("assistant");
        output.push(json!({
            "id": format!("msg_{chat_id}_{choice_index}"),
            "type": "message",
            "status": "completed",
            "role": role,
            "content": content_array,
        }));
    }
    output
}

fn convert_chat_message_content_to_responses_content(content: Option<&Value>) -> Vec<Value> {
    let mut out = Vec::new();
    match content {
        Some(Value::String(s)) if !s.is_empty() => {
            out.push(json!({"type": "output_text", "text": s, "annotations": []}));
        }
        Some(Value::Array(parts)) => {
            for part in parts {
                if part.get("type").and_then(|v| v.as_str()) == Some("text") {
                    if let Some(text) = part.get("text").and_then(|v| v.as_str()) {
                        if !text.is_empty() {
                            out.push(json!({"type": "output_text", "text": text, "annotations": []}));
                        }
                    }
                }
            }
        }
        _ => {}
    }
    out
}

fn responses_created_event(response_id: &str, model: &str, created_at: i64) -> Map<String, Value> {
    event_map! {
        "response".into() => json!({
            "id": response_id,
            "object": "response",
            "created_at": created_at,
            "model": model,
            "status": "in_progress",
            "output": [],
        }),
    }
}

fn responses_in_progress_event(
    response_id: &str,
    model: &str,
    created_at: i64,
) -> Map<String, Value> {
    responses_created_event(response_id, model, created_at)
}

fn responses_output_text_part(text: &str) -> Value {
    json!({"type": "output_text", "text": text, "annotations": []})
}

fn responses_message_item_with_status(
    message_id: &str,
    role: &str,
    content: &[Value],
    status: &str,
) -> Value {
    json!({
        "id": message_id,
        "type": "message",
        "status": status,
        "role": role,
        "content": content,
    })
}

fn responses_completed_response(
    response_id: &str,
    model: &str,
    created_at: i64,
    output: &[Value],
    input_tokens: i64,
    output_tokens: i64,
    cached_tokens: i64,
) -> Value {
    json!({
        "id": response_id,
        "object": "response",
        "created_at": created_at,
        "model": model,
        "status": "completed",
        "output": output,
        "usage": {
            "input_tokens": input_tokens,
            "output_tokens": output_tokens,
            "input_tokens_details": {"cached_tokens": cached_tokens},
            "output_tokens_details": {"reasoning_tokens": 0},
            "total_tokens": input_tokens + output_tokens,
        }
    })
}

fn as_i64(v: &Value) -> i64 {
    match v {
        Value::Number(n) => n.as_i64().unwrap_or(0),
        _ => 0,
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gateway::api::openai::Message;

    #[test]
    fn converts_instructions_and_input() {
        let body = json!({
            "model": "gpt-4",
            "stream": false,
            "instructions": "be concise",
            "input": "hi",
            "max_output_tokens": 123
        });
        let req = responses_request_to_openai(&body).unwrap();
        assert_eq!(req.model, "gpt-4");
        assert_eq!(req.max_tokens, Some(123));
        assert_eq!(req.messages[0].content.as_deref(), Some("be concise"));
        assert_eq!(req.messages[1].content.as_deref(), Some("hi"));
    }

    #[test]
    fn converts_tool_calls_in_non_stream_response() {
        let chat = json!({
            "id": "chatcmpl-tools",
            "created": 1710000000,
            "model": "upstream",
            "choices": [{
                "message": {
                    "role": "assistant",
                    "content": "",
                    "tool_calls": [{
                        "id": "call_1",
                        "type": "function",
                        "function": {"name": "exec_command", "arguments": "{\"cmd\":\"date\"}"}
                    }]
                }
            }],
            "usage": {"prompt_tokens": 3, "completion_tokens": 2}
        });
        let resp = chat_json_to_responses(&chat);
        assert_eq!(resp["id"], "resp_chatcmpl-tools");
        let output = resp["output"].as_array().unwrap();
        assert_eq!(output[0]["type"], "function_call");
        assert_eq!(output[0]["call_id"], "call_1");
    }

    #[test]
    fn chat_json_to_responses_adds_reasoning_item() {
        let chat = json!({
            "id": "chatcmpl-r",
            "model": "deepseek",
            "choices": [{
                "message": {
                    "role": "assistant",
                    "content": "Hello",
                    "reasoning_content": "Let me think..."
                }
            }]
        });
        let resp = chat_json_to_responses(&chat);
        let output = resp["output"].as_array().unwrap();
        assert_eq!(output[0]["type"], "reasoning");
        assert_eq!(output[0]["content"][0]["text"], "Let me think...");
        assert_eq!(output[1]["type"], "message");
        assert_eq!(output[1]["content"][0]["text"], "Hello");
    }

    #[test]
    fn chat_json_to_responses_reasoning_before_tool_calls() {
        let chat = json!({
            "id": "chatcmpl-rt",
            "model": "deepseek",
            "choices": [{
                "message": {
                    "role": "assistant",
                    "content": "",
                    "reasoning_content": "I need to search",
                    "tool_calls": [{
                        "id": "call_1",
                        "type": "function",
                        "function": {"name": "search", "arguments": "{}"}
                    }]
                }
            }]
        });
        let resp = chat_json_to_responses(&chat);
        let output = resp["output"].as_array().unwrap();
        assert_eq!(output[0]["type"], "reasoning");
        assert_eq!(output[1]["type"], "function_call");
    }

    #[test]
    fn response_id_gets_resp_prefix_from_chat() {
        let chat = json!({
            "id": "chatcmpl-abc123",
            "model": "gpt-4",
            "choices": [{"message": {"role": "assistant", "content": "ok"}}]
        });
        let resp = chat_json_to_responses(&chat);
        assert_eq!(resp["id"], "resp_chatcmpl-abc123");
    }

    #[test]
    fn instructions_strips_billing_header() {
        let body = json!({
            "model": "gpt-4",
            "instructions": "x-anthropic-billing-header: cch=abc\n\nBe concise.",
            "input": "hi"
        });
        let req = responses_request_to_openai(&body).unwrap();
        assert_eq!(req.messages[0].content.as_deref(), Some("Be concise."));
    }

    // --- DeepSeek reasoning preservation tests ---

    #[test]
    fn responses_deepseek_preserves_reasoning_effort_medium_maps_to_high() {
        let body = json!({
            "model": "deepseek-v4-flash",
            "input": "hello",
            "reasoning": {"effort": "medium"}
        });
        let req = responses_request_to_openai_with_base(&body, Some("https://api.deepseek.com/v1")).unwrap();
        assert_eq!(req.thinking, Some(json!({"type": "enabled"})));
        assert_eq!(req.reasoning_effort.as_deref(), Some("high"));
    }

    #[test]
    fn responses_deepseek_turn2_auto_enables_thinking() {
        let body = json!({
            "model": "deepseek-v4-flash",
            "input": [
                {"role": "assistant", "reasoning_content": "I think...", "content": "Answer"},
                {"role": "user", "content": "follow-up"}
            ]
        });
        let req = responses_request_to_openai_with_base(&body, Some("https://api.deepseek.com/v1")).unwrap();
        assert_eq!(req.thinking, Some(json!({"type": "enabled"})));
        assert_eq!(req.reasoning_effort.as_deref(), Some("high"));
    }

    #[test]
    fn responses_deepseek_explicit_disable_no_reasoning_effort() {
        let body = json!({
            "model": "deepseek-v4-flash",
            "input": "hello",
            "reasoning": {"effort": "none"}
        });
        let req = responses_request_to_openai_with_base(&body, Some("https://api.deepseek.com/v1")).unwrap();
        assert_eq!(req.thinking, Some(json!({"type": "disabled"})));
        assert!(req.reasoning_effort.is_none());
    }

    #[test]
    fn responses_deepseek_xhigh_effort_maps_to_max() {
        let body = json!({
            "model": "deepseek-v4-flash",
            "input": "hello",
            "reasoning": {"effort": "xhigh"}
        });
        let req = responses_request_to_openai_with_base(&body, Some("https://api.deepseek.com/v1")).unwrap();
        assert_eq!(req.thinking, Some(json!({"type": "enabled"})));
        assert_eq!(req.reasoning_effort.as_deref(), Some("max"));
    }

    #[test]
    fn responses_deepseek_serialized_upstream_includes_thinking() {
        let req = ChatCompletionRequest {
            model: "deepseek-v4-flash".into(),
            messages: vec![Message {
                role: Role::User,
                content: Some("hi".into()),
                content_parts: None,
                tool_calls: None,
                tool_call_id: None,
                reasoning_content: None,
            }],
            thinking: Some(json!({"type": "enabled"})),
            reasoning_effort: Some("high".into()),
            ..Default::default()
        };
        let value = serde_json::to_value(&req).unwrap();
        assert_eq!(value["thinking"], json!({"type": "enabled"}));
        assert_eq!(value["reasoning_effort"], "high");
    }

    #[test]
    fn responses_deepseek_no_reasoning_field_does_not_inject_thinking() {
        let body = json!({
            "model": "deepseek-v4-flash",
            "input": "hello"
        });
        let req = responses_request_to_openai_with_base(&body, Some("https://api.deepseek.com/v1")).unwrap();
        assert!(req.thinking.is_none());
        assert!(req.reasoning_effort.is_none());
    }

    #[test]
    fn converts_custom_tool_call_input() {
        let body = json!({
            "model": "gpt-4",
            "input": [
                {"type": "custom_tool_call", "call_id": "c1", "name": "exec", "arguments": "{}"},
                {"type": "custom_tool_call_output", "call_id": "c1", "output": "done"}
            ]
        });
        let req = responses_request_to_openai(&body).unwrap();
        let assistant = req.messages.iter().find(|m| m.role == Role::Assistant).unwrap();
        assert!(assistant.tool_calls.is_some());
        assert_eq!(assistant.tool_calls.as_ref().unwrap().len(), 1);
        let tool_msg = req.messages.iter().find(|m| m.role == Role::Tool).unwrap();
        assert_eq!(tool_msg.content.as_deref(), Some("done"));
    }
}
