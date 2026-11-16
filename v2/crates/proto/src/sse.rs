//! 流式事件状态机（§10.3）。
//!
//! 三协议各自解析成同一个 [`CanonicalStreamEvent`] 序列。事件顺序必须与上游一致，
//! 不能提前发 `Finished`；工具调用 delta 保留 index/id/name/arguments 片段。
//!
//! 文本层（SSE 分帧）与语义层分开：`parse_sse_text` 只负责 `event:` / `data:` / 空行，
//! `canonicalize` 才做协议判断。

use serde_json::Value;
use tr_contracts::{CanonicalStreamEvent, FinishReason, IngressProtocol};

/// 一条已分帧的 SSE 数据块。
#[derive(Debug, Clone, PartialEq)]
pub struct RawChunk {
    /// `event:` 字段（Anthropic / Responses 使用）。
    pub event: Option<String>,
    /// `data:` 解析后的 JSON；`[DONE]` 解析为 `Value::String("[DONE]")`。
    pub data: Value,
}

/// 解析失败：宁可报错，也不静默丢事件。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StreamParseError {
    MissingField(&'static str),
    UnsupportedEvent(String),
    Malformed(String),
}

impl std::fmt::Display for StreamParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::MissingField(field) => write!(f, "missing field: {field}"),
            Self::UnsupportedEvent(event) => write!(f, "unsupported stream event: {event}"),
            Self::Malformed(detail) => write!(f, "malformed stream chunk: {detail}"),
        }
    }
}

impl std::error::Error for StreamParseError {}

/// 把 SSE 文本分成数据块。忽略注释与 `id:` / `retry:` 字段。
pub fn parse_sse_text(text: &str) -> Vec<RawChunk> {
    let mut chunks = Vec::new();
    let mut event: Option<String> = None;
    let mut data = String::new();

    let flush = |event: &mut Option<String>, data: &mut String, out: &mut Vec<RawChunk>| {
        if data.is_empty() && event.is_none() {
            return;
        }
        let payload = if data == "[DONE]" {
            Value::String("[DONE]".to_string())
        } else if data.is_empty() {
            Value::Null
        } else {
            serde_json::from_str(data).unwrap_or_else(|_| Value::String(std::mem::take(data)))
        };
        out.push(RawChunk {
            event: event.take(),
            data: payload,
        });
        data.clear();
    };

    for line in text.lines() {
        let line = line.trim_end_matches('\r');
        if line.is_empty() {
            flush(&mut event, &mut data, &mut chunks);
            continue;
        }
        if let Some(rest) = line.strip_prefix("event:") {
            event = Some(rest.trim().to_string());
        } else if let Some(rest) = line.strip_prefix("data:") {
            if !data.is_empty() {
                data.push('\n');
            }
            data.push_str(rest.trim_start());
        }
        // `id:` / `retry:` / `:comment` 与语义无关，忽略。
    }
    flush(&mut event, &mut data, &mut chunks);
    chunks
}

/// 把协议事件序列转换为 canonical 事件序列。
pub fn canonicalize(
    protocol: IngressProtocol,
    chunks: &[RawChunk],
) -> Result<Vec<CanonicalStreamEvent>, StreamParseError> {
    match protocol {
        IngressProtocol::ChatCompletions => chat(chunks),
        IngressProtocol::AnthropicMessages => anthropic(chunks),
        IngressProtocol::Responses => responses(chunks),
    }
}

/// 增量 Chat 流状态：批次与流式共用同一套推进逻辑，避免两套行为漂移。
#[derive(Debug, Default)]
pub struct ChatStreamState {
    started: bool,
}

impl ChatStreamState {
    pub fn is_started(&self) -> bool {
        self.started
    }

    /// 处理一个 SSE 数据块，返回本块产生的事件。
    pub fn push(
        &mut self,
        chunk: &RawChunk,
    ) -> Result<Vec<CanonicalStreamEvent>, StreamParseError> {
        let mut out = Vec::new();
        let Some(object) = chunk.data.as_object() else {
            return Ok(out);
        };
        if let Some(error) = object.get("error") {
            let message = error
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or("upstream stream error");
            out.push(CanonicalStreamEvent::Error {
                message: message.to_string(),
            });
            return Ok(out);
        }

        if !self.started {
            let id = object
                .get("id")
                .and_then(Value::as_str)
                .ok_or(StreamParseError::MissingField("id"))?;
            let model = object.get("model").and_then(Value::as_str).unwrap_or("");
            out.push(CanonicalStreamEvent::Started {
                id: id.to_string(),
                model: model.to_string(),
            });
            self.started = true;
        }

        if let Some(usage) = object.get("usage").filter(|usage| !usage.is_null()) {
            out.push(usage_event(usage));
        }

        for choice in object
            .get("choices")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            if let Some(delta) = choice.get("delta") {
                if let Some(text) = delta.get("content").and_then(Value::as_str) {
                    if !text.is_empty() {
                        out.push(CanonicalStreamEvent::ContentDelta {
                            text: text.to_string(),
                        });
                    }
                }
                if let Some(text) = delta.get("reasoning_content").and_then(Value::as_str) {
                    if !text.is_empty() {
                        out.push(CanonicalStreamEvent::ReasoningDelta {
                            text: text.to_string(),
                        });
                    }
                }
                for call in delta
                    .get("tool_calls")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                {
                    let index = call.get("index").and_then(Value::as_u64).unwrap_or(0) as u32;
                    let id = call.get("id").and_then(Value::as_str).map(str::to_string);
                    let function = call.get("function");
                    let name = function
                        .and_then(|function| function.get("name"))
                        .and_then(Value::as_str)
                        .map(str::to_string);
                    let arguments = function
                        .and_then(|function| function.get("arguments"))
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_string();
                    out.push(CanonicalStreamEvent::ToolCallDelta {
                        index,
                        id,
                        name,
                        arguments,
                    });
                }
            }

            if let Some(reason) = choice.get("finish_reason").and_then(Value::as_str) {
                out.push(CanonicalStreamEvent::Finished {
                    reason: chat_finish_reason(reason),
                });
            }
        }

        Ok(out)
    }
}

fn chat(chunks: &[RawChunk]) -> Result<Vec<CanonicalStreamEvent>, StreamParseError> {
    let mut state = ChatStreamState::default();
    let mut out = Vec::new();
    for chunk in chunks {
        out.extend(state.push(chunk)?);
    }
    if !state.is_started() {
        return Err(StreamParseError::MissingField("id"));
    }
    Ok(out)
}

/// 增量 SSE 解码器（上游 OpenAI 兼容流）。
///
/// 只做分帧 + 状态推进，不缓存整段响应；长流是 O(n)，不是 O(n²)。
#[derive(Debug, Default)]
pub struct ChatStreamDecoder {
    state: ChatStreamState,
    buffer: String,
    done: bool,
}

impl ChatStreamDecoder {
    pub fn new() -> Self {
        Self::default()
    }

    /// 是否已收到 `[DONE]`。
    pub fn is_done(&self) -> bool {
        self.done
    }

    /// 喂入一段字节，返回本段产生的事件。
    pub fn feed(&mut self, bytes: &[u8]) -> Result<Vec<CanonicalStreamEvent>, StreamParseError> {
        self.buffer.push_str(&String::from_utf8_lossy(bytes));
        if self.buffer.contains('\r') {
            self.buffer = self.buffer.replace("\r\n", "\n");
        }

        let mut out = Vec::new();
        while let Some(index) = self.buffer.find("\n\n") {
            let block: String = self.buffer.drain(..index + 2).collect();
            for chunk in parse_sse_text(&block) {
                if chunk.data == Value::String("[DONE]".to_string()) {
                    self.done = true;
                    continue;
                }
                out.extend(self.state.push(&chunk)?);
            }
        }
        Ok(out)
    }

    /// 流结束：处理残留在缓冲区里的最后一个块。
    pub fn finish(&mut self) -> Result<Vec<CanonicalStreamEvent>, StreamParseError> {
        let remaining = std::mem::take(&mut self.buffer);
        if remaining.trim().is_empty() {
            return Ok(Vec::new());
        }
        let mut out = Vec::new();
        for chunk in parse_sse_text(&remaining) {
            if chunk.data == Value::String("[DONE]".to_string()) {
                self.done = true;
                continue;
            }
            out.extend(self.state.push(&chunk)?);
        }
        Ok(out)
    }
}

fn anthropic(chunks: &[RawChunk]) -> Result<Vec<CanonicalStreamEvent>, StreamParseError> {
    let mut out = Vec::new();
    let mut input_tokens = 0;
    let mut cached_tokens: Option<u32> = None;
    let mut started = false;

    for chunk in chunks {
        let Some(object) = chunk.data.as_object() else {
            continue;
        };
        let kind = object
            .get("type")
            .and_then(Value::as_str)
            .map(str::to_string)
            .or_else(|| chunk.event.clone())
            .ok_or(StreamParseError::MissingField("type"))?;

        match kind.as_str() {
            "message_start" => {
                let message = object
                    .get("message")
                    .ok_or(StreamParseError::MissingField("message"))?;
                let id = message
                    .get("id")
                    .and_then(Value::as_str)
                    .ok_or(StreamParseError::MissingField("message.id"))?;
                let model = message.get("model").and_then(Value::as_str).unwrap_or("");
                if let Some(usage) = message.get("usage") {
                    input_tokens = usage
                        .get("input_tokens")
                        .and_then(Value::as_u64)
                        .unwrap_or(0) as u32;
                    cached_tokens = usage
                        .get("cache_read_input_tokens")
                        .and_then(Value::as_u64)
                        .map(|value| value as u32);
                }
                out.push(CanonicalStreamEvent::Started {
                    id: id.to_string(),
                    model: model.to_string(),
                });
                started = true;
            }
            "content_block_start" => {
                let block = object
                    .get("content_block")
                    .ok_or(StreamParseError::MissingField("content_block"))?;
                if block.get("type").and_then(Value::as_str) == Some("tool_use") {
                    let index = object.get("index").and_then(Value::as_u64).unwrap_or(0) as u32;
                    let id = block.get("id").and_then(Value::as_str).map(str::to_string);
                    let name = block
                        .get("name")
                        .and_then(Value::as_str)
                        .map(str::to_string);
                    out.push(CanonicalStreamEvent::ToolCallDelta {
                        index,
                        id,
                        name,
                        arguments: String::new(),
                    });
                }
            }
            "content_block_delta" => {
                let index = object.get("index").and_then(Value::as_u64).unwrap_or(0) as u32;
                let delta = object
                    .get("delta")
                    .ok_or(StreamParseError::MissingField("delta"))?;
                match delta.get("type").and_then(Value::as_str) {
                    Some("text_delta") => {
                        if let Some(text) = delta.get("text").and_then(Value::as_str) {
                            out.push(CanonicalStreamEvent::ContentDelta {
                                text: text.to_string(),
                            });
                        }
                    }
                    Some("thinking_delta") => {
                        if let Some(text) = delta.get("thinking").and_then(Value::as_str) {
                            out.push(CanonicalStreamEvent::ReasoningDelta {
                                text: text.to_string(),
                            });
                        }
                    }
                    Some("input_json_delta") => {
                        let arguments = delta
                            .get("partial_json")
                            .and_then(Value::as_str)
                            .unwrap_or("")
                            .to_string();
                        out.push(CanonicalStreamEvent::ToolCallDelta {
                            index,
                            id: None,
                            name: None,
                            arguments,
                        });
                    }
                    Some(other) => {
                        return Err(StreamParseError::UnsupportedEvent(format!(
                            "content_block_delta:{other}"
                        )));
                    }
                    None => return Err(StreamParseError::MissingField("delta.type")),
                }
            }
            "message_delta" => {
                if let Some(usage) = object.get("usage") {
                    if let Some(output) = usage.get("output_tokens").and_then(Value::as_u64) {
                        out.push(CanonicalStreamEvent::Usage {
                            input: input_tokens,
                            output: output as u32,
                            cached: cached_tokens,
                        });
                    }
                }
                if let Some(reason) = object
                    .get("delta")
                    .and_then(|delta| delta.get("stop_reason"))
                    .and_then(Value::as_str)
                {
                    out.push(CanonicalStreamEvent::Finished {
                        reason: anthropic_finish_reason(reason),
                    });
                }
            }
            "message_stop" | "content_block_stop" | "ping" => {}
            "error" => {
                let message = object
                    .get("error")
                    .and_then(|error| error.get("message"))
                    .and_then(Value::as_str)
                    .unwrap_or("upstream stream error");
                out.push(CanonicalStreamEvent::Error {
                    message: message.to_string(),
                });
            }
            other => return Err(StreamParseError::UnsupportedEvent(other.to_string())),
        }
    }

    if !started {
        return Err(StreamParseError::MissingField("message_start"));
    }
    Ok(out)
}

fn responses(chunks: &[RawChunk]) -> Result<Vec<CanonicalStreamEvent>, StreamParseError> {
    let mut out = Vec::new();
    let mut started = false;

    for chunk in chunks {
        let Some(object) = chunk.data.as_object() else {
            continue;
        };
        let kind = object
            .get("type")
            .and_then(Value::as_str)
            .map(str::to_string)
            .or_else(|| chunk.event.clone())
            .ok_or(StreamParseError::MissingField("type"))?;

        match kind.as_str() {
            "response.created" => {
                let response = object
                    .get("response")
                    .ok_or(StreamParseError::MissingField("response"))?;
                let id = response
                    .get("id")
                    .and_then(Value::as_str)
                    .ok_or(StreamParseError::MissingField("response.id"))?;
                let model = response.get("model").and_then(Value::as_str).unwrap_or("");
                out.push(CanonicalStreamEvent::Started {
                    id: id.to_string(),
                    model: model.to_string(),
                });
                started = true;
            }
            "response.output_item.added" => {
                let item = object
                    .get("item")
                    .ok_or(StreamParseError::MissingField("item"))?;
                if item.get("type").and_then(Value::as_str) == Some("function_call") {
                    let index = object
                        .get("output_index")
                        .and_then(Value::as_u64)
                        .unwrap_or(0) as u32;
                    out.push(CanonicalStreamEvent::ToolCallDelta {
                        index,
                        id: item
                            .get("call_id")
                            .and_then(Value::as_str)
                            .map(str::to_string),
                        name: item.get("name").and_then(Value::as_str).map(str::to_string),
                        arguments: String::new(),
                    });
                }
            }
            "response.output_text.delta" => {
                if let Some(text) = object.get("delta").and_then(Value::as_str) {
                    out.push(CanonicalStreamEvent::ContentDelta {
                        text: text.to_string(),
                    });
                }
            }
            "response.reasoning_text.delta" | "response.reasoning_summary_text.delta" => {
                if let Some(text) = object.get("delta").and_then(Value::as_str) {
                    out.push(CanonicalStreamEvent::ReasoningDelta {
                        text: text.to_string(),
                    });
                }
            }
            "response.function_call_arguments.delta" => {
                let index = object
                    .get("output_index")
                    .and_then(Value::as_u64)
                    .unwrap_or(0) as u32;
                let arguments = object
                    .get("delta")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string();
                out.push(CanonicalStreamEvent::ToolCallDelta {
                    index,
                    id: None,
                    name: None,
                    arguments,
                });
            }
            "response.completed" => {
                let response = object
                    .get("response")
                    .ok_or(StreamParseError::MissingField("response"))?;
                if let Some(usage) = response.get("usage").filter(|usage| !usage.is_null()) {
                    out.push(usage_event(usage));
                }
                let has_tool_call = response
                    .get("output")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .any(|item| item.get("type").and_then(Value::as_str) == Some("function_call"));
                out.push(CanonicalStreamEvent::Finished {
                    reason: if has_tool_call {
                        FinishReason::ToolUse
                    } else {
                        FinishReason::Stop
                    },
                });
            }
            "response.failed" | "error" => {
                let message = object
                    .get("error")
                    .and_then(|error| error.get("message"))
                    .and_then(Value::as_str)
                    .or_else(|| {
                        object
                            .get("response")
                            .and_then(|response| response.get("error"))
                            .and_then(|error| error.get("message"))
                            .and_then(Value::as_str)
                    })
                    .unwrap_or("upstream stream error");
                out.push(CanonicalStreamEvent::Error {
                    message: message.to_string(),
                });
            }
            "response.in_progress"
            | "response.output_item.done"
            | "response.content_part.added"
            | "response.content_part.done"
            | "response.output_text.done"
            | "response.function_call_arguments.done" => {}
            other => return Err(StreamParseError::UnsupportedEvent(other.to_string())),
        }
    }

    if !started {
        return Err(StreamParseError::MissingField("response.created"));
    }
    Ok(out)
}

fn usage_event(usage: &Value) -> CanonicalStreamEvent {
    let prompt_details = usage.get("prompt_tokens_details");
    CanonicalStreamEvent::Usage {
        input: usage
            .get("prompt_tokens")
            .or_else(|| usage.get("input_tokens"))
            .and_then(Value::as_u64)
            .unwrap_or(0) as u32,
        output: usage
            .get("completion_tokens")
            .or_else(|| usage.get("output_tokens"))
            .and_then(Value::as_u64)
            .unwrap_or(0) as u32,
        cached: prompt_details
            .and_then(|details| details.get("cached_tokens"))
            .or_else(|| {
                usage
                    .get("input_tokens_details")
                    .and_then(|details| details.get("cached_tokens"))
            })
            .and_then(Value::as_u64)
            .map(|value| value as u32),
    }
}

fn chat_finish_reason(reason: &str) -> FinishReason {
    match reason {
        "stop" => FinishReason::Stop,
        "tool_calls" => FinishReason::ToolUse,
        "length" => FinishReason::Length,
        "content_filter" => FinishReason::ContentFilter,
        other => FinishReason::Other(other.to_string()),
    }
}

fn anthropic_finish_reason(reason: &str) -> FinishReason {
    match reason {
        "end_turn" | "stop_sequence" => FinishReason::Stop,
        "max_tokens" => FinishReason::Length,
        "tool_use" => FinishReason::ToolUse,
        "refusal" => FinishReason::ContentFilter,
        other => FinishReason::Other(other.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sse_framing_keeps_event_and_data() {
        let text = "event: response.created\n\
                    data: {\"type\":\"response.created\",\"response\":{\"id\":\"r1\"}}\n\
                    \n\
                    data: [DONE]\n\n";
        let chunks = parse_sse_text(text);
        assert_eq!(chunks.len(), 2);
        assert_eq!(chunks[0].event.as_deref(), Some("response.created"));
        assert_eq!(chunks[0].data["response"]["id"], "r1");
        assert_eq!(chunks[1].data, Value::String("[DONE]".to_string()));
    }

    #[test]
    fn chat_orders_finish_before_usage_when_wire_says_so() {
        let chunks = vec![
            RawChunk {
                event: None,
                data: serde_json::json!({
                    "id": "c1", "model": "m",
                    "choices": [{"delta": {"content": "hi"}, "finish_reason": null}]
                }),
            },
            RawChunk {
                event: None,
                data: serde_json::json!({
                    "id": "c1", "model": "m",
                    "choices": [{"delta": {}, "finish_reason": "stop"}]
                }),
            },
            RawChunk {
                event: None,
                data: serde_json::json!({
                    "id": "c1", "choices": [],
                    "usage": {"prompt_tokens": 3, "completion_tokens": 2}
                }),
            },
        ];
        let events = canonicalize(IngressProtocol::ChatCompletions, &chunks).unwrap();
        assert!(matches!(events[0], CanonicalStreamEvent::Started { .. }));
        assert!(matches!(
            events[1],
            CanonicalStreamEvent::ContentDelta { .. }
        ));
        assert!(matches!(
            events[2],
            CanonicalStreamEvent::Finished {
                reason: FinishReason::Stop
            }
        ));
        assert!(matches!(events[3], CanonicalStreamEvent::Usage { .. }));
    }

    #[test]
    fn unknown_event_is_an_error() {
        let chunks = vec![RawChunk {
            event: None,
            data: serde_json::json!({"type": "response.mystery"}),
        }];
        assert_eq!(
            canonicalize(IngressProtocol::Responses, &chunks),
            Err(StreamParseError::UnsupportedEvent(
                "response.mystery".to_string()
            ))
        );
    }
}
