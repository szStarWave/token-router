//! canonical 事件 → wire SSE 编码（P0b-1）。
//!
//! 与 [`crate::sse`] 互为逆操作。编码器带有每协议状态（消息 id、块索引、
//! 工具调用累积），因为 wire 协议要求先发 `content_block_start` / `response.created`
//! 之类的结构性事件，而 canonical 序列里没有它们。
//!
//! 已知取舍（P0b）：
//! - Anthropic `message_start` 的 `input_tokens` 先发 0；真实用量在后续
//!   `message_delta` 里补，客户端 SDK 会累加。
//! - Responses 的 `response.completed` 在 `finish()` 时补齐（需要先收齐 output）。

use std::collections::BTreeMap;

use serde_json::{Value, json};
use tr_contracts::{CanonicalStreamEvent, FinishReason, IngressProtocol};

/// 一帧 SSE。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WireFrame {
    pub event: Option<String>,
    pub data: String,
}

impl WireFrame {
    /// 渲染成 SSE 文本。
    pub fn to_sse(&self) -> String {
        match &self.event {
            Some(event) => format!("event: {event}\ndata: {}\n\n", self.data),
            None => format!("data: {}\n\n", self.data),
        }
    }
}

/// 流式编码器。
#[derive(Debug)]
pub struct StreamEncoder {
    protocol: IngressProtocol,
    state: State,
}

#[derive(Debug, Default)]
struct State {
    id: String,
    model: String,
    started: bool,
    // Chat / Responses 共用：已见到的 usage。
    usage: Option<(u32, u32, Option<u32>)>,
    // Anthropic
    next_block: u32,
    open_block: Option<(u32, BlockKind)>,
    stop_sent: bool,
    // Responses
    output_started: bool,
    tool_calls: BTreeMap<u32, ToolCall>,
    text: String,
    completed: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BlockKind {
    Text,
    Thinking,
    ToolUse,
}

#[derive(Debug, Default, Clone)]
struct ToolCall {
    id: Option<String>,
    name: Option<String>,
    arguments: String,
}

impl StreamEncoder {
    pub fn new(protocol: IngressProtocol) -> Self {
        Self {
            protocol,
            state: State::default(),
        }
    }

    /// 编码一个 canonical 事件。
    pub fn push(&mut self, event: &CanonicalStreamEvent) -> Vec<WireFrame> {
        match self.protocol {
            IngressProtocol::ChatCompletions => self.push_chat(event),
            IngressProtocol::AnthropicMessages => self.push_anthropic(event),
            IngressProtocol::Responses => self.push_responses(event),
        }
    }

    /// 流结束：补齐协议终止帧。
    pub fn finish(&mut self) -> Vec<WireFrame> {
        match self.protocol {
            IngressProtocol::ChatCompletions => vec![WireFrame {
                event: None,
                data: "[DONE]".to_string(),
            }],
            IngressProtocol::AnthropicMessages => {
                if self.state.stop_sent {
                    return Vec::new();
                }
                self.state.stop_sent = true;
                let mut frames = Vec::new();
                if let Some((index, _)) = self.state.open_block.take() {
                    frames.push(anthropic_frame(
                        "content_block_stop",
                        json!({"type": "content_block_stop", "index": index}),
                    ));
                }
                frames.push(anthropic_frame(
                    "message_stop",
                    json!({"type": "message_stop"}),
                ));
                frames
            }
            IngressProtocol::Responses => {
                if self.state.completed {
                    return Vec::new();
                }
                self.state.completed = true;

                let mut output: Vec<Value> = Vec::new();
                if !self.state.text.is_empty() {
                    output.push(json!({
                        "type": "message",
                        "id": format!("msg_{}", self.state.id),
                        "status": "completed",
                        "role": "assistant",
                        "content": [{"type": "output_text", "text": self.state.text, "annotations": []}]
                    }));
                }
                for (index, call) in &self.state.tool_calls {
                    output.push(json!({
                        "type": "function_call",
                        "id": format!("fc_{}_{}", self.state.id, index),
                        "call_id": call.id.clone().unwrap_or_default(),
                        "name": call.name.clone().unwrap_or_default(),
                        "arguments": call.arguments,
                        "status": "completed"
                    }));
                }

                let usage = self
                    .state
                    .usage
                    .map_or(Value::Null, |(input, output, cached)| {
                        json!({
                            "input_tokens": input,
                            "output_tokens": output,
                            "total_tokens": input + output,
                            "input_tokens_details": {"cached_tokens": cached.unwrap_or(0)}
                        })
                    });

                vec![WireFrame {
                    event: Some("response.completed".to_string()),
                    data: json!({
                        "type": "response.completed",
                        "response": {
                            "id": self.state.id,
                            "object": "response",
                            "status": "completed",
                            "model": self.state.model,
                            "output": output,
                            "usage": usage
                        }
                    })
                    .to_string(),
                }]
            }
        }
    }

    fn push_chat(&mut self, event: &CanonicalStreamEvent) -> Vec<WireFrame> {
        match event {
            CanonicalStreamEvent::Started { id, model } => {
                self.state.id = id.clone();
                self.state.model = model.clone();
                self.state.started = true;
                vec![chat_frame(json!({
                    "id": id,
                    "object": "chat.completion.chunk",
                    "created": 0,
                    "model": model,
                    "choices": [{"index": 0, "delta": {"role": "assistant", "content": ""}, "finish_reason": null}]
                }))]
            }
            CanonicalStreamEvent::ContentDelta { text } => vec![chat_frame(json!({
                "id": self.state.id,
                "object": "chat.completion.chunk",
                "created": 0,
                "model": self.state.model,
                "choices": [{"index": 0, "delta": {"content": text}, "finish_reason": null}]
            }))],
            CanonicalStreamEvent::ReasoningDelta { text } => vec![chat_frame(json!({
                "id": self.state.id,
                "object": "chat.completion.chunk",
                "created": 0,
                "model": self.state.model,
                "choices": [{"index": 0, "delta": {"reasoning_content": text}, "finish_reason": null}]
            }))],
            CanonicalStreamEvent::ToolCallDelta {
                index,
                id,
                name,
                arguments,
            } => vec![chat_frame(json!({
                "id": self.state.id,
                "object": "chat.completion.chunk",
                "created": 0,
                "model": self.state.model,
                "choices": [{
                    "index": 0,
                    "delta": {"tool_calls": [{
                        "index": index,
                        "id": id,
                        "type": "function",
                        "function": {"name": name, "arguments": arguments}
                    }]},
                    "finish_reason": null
                }]
            }))],
            CanonicalStreamEvent::Usage {
                input,
                output,
                cached,
            } => {
                self.state.usage = Some((*input, *output, *cached));
                vec![chat_frame(json!({
                    "id": self.state.id,
                    "object": "chat.completion.chunk",
                    "created": 0,
                    "model": self.state.model,
                    "choices": [],
                    "usage": {
                        "prompt_tokens": input,
                        "completion_tokens": output,
                        "total_tokens": input + output,
                        "prompt_tokens_details": {"cached_tokens": cached.unwrap_or(0)}
                    }
                }))]
            }
            CanonicalStreamEvent::Finished { reason } => {
                vec![chat_frame(json!({
                    "id": self.state.id,
                    "object": "chat.completion.chunk",
                    "created": 0,
                    "model": self.state.model,
                    "choices": [{"index": 0, "delta": {}, "finish_reason": chat_finish(reason)}]
                }))]
            }
            CanonicalStreamEvent::Error { message } => vec![chat_frame(json!({
                "error": {"message": message}
            }))],
        }
    }

    fn push_anthropic(&mut self, event: &CanonicalStreamEvent) -> Vec<WireFrame> {
        match event {
            CanonicalStreamEvent::Started { id, model } => {
                self.state.id = id.clone();
                self.state.model = model.clone();
                self.state.started = true;
                vec![anthropic_frame(
                    "message_start",
                    json!({
                        "type": "message_start",
                        "message": {
                            "id": id,
                            "type": "message",
                            "role": "assistant",
                            "model": model,
                            "content": [],
                            "stop_reason": null,
                            "stop_sequence": null,
                            "usage": {"input_tokens": 0, "output_tokens": 0}
                        }
                    }),
                )]
            }
            CanonicalStreamEvent::ContentDelta { text } => {
                let mut frames = self.ensure_anthropic_block(BlockKind::Text);
                let index = self.state.next_block.saturating_sub(1);
                frames.push(anthropic_frame(
                    "content_block_delta",
                    json!({
                        "type": "content_block_delta",
                        "index": index,
                        "delta": {"type": "text_delta", "text": text}
                    }),
                ));
                frames
            }
            CanonicalStreamEvent::ReasoningDelta { text } => {
                let mut frames = self.ensure_anthropic_block(BlockKind::Thinking);
                let index = self.state.next_block.saturating_sub(1);
                frames.push(anthropic_frame(
                    "content_block_delta",
                    json!({
                        "type": "content_block_delta",
                        "index": index,
                        "delta": {"type": "thinking_delta", "thinking": text}
                    }),
                ));
                frames
            }
            CanonicalStreamEvent::ToolCallDelta {
                index: _,
                id,
                name,
                arguments,
            } => {
                let mut frames = Vec::new();
                if self.state.open_block.map(|(_, kind)| kind) != Some(BlockKind::ToolUse)
                    && id.is_some()
                {
                    // 关闭上一个块，开启 tool_use 块。
                    if let Some((open_index, _)) = self.state.open_block.take() {
                        frames.push(anthropic_frame(
                            "content_block_stop",
                            json!({"type": "content_block_stop", "index": open_index}),
                        ));
                    }
                    let block_index = self.state.next_block;
                    self.state.next_block += 1;
                    self.state.open_block = Some((block_index, BlockKind::ToolUse));
                    frames.push(anthropic_frame(
                        "content_block_start",
                        json!({
                            "type": "content_block_start",
                            "index": block_index,
                            "content_block": {
                                "type": "tool_use",
                                "id": id.clone().unwrap_or_default(),
                                "name": name.clone().unwrap_or_default(),
                                "input": {}
                            }
                        }),
                    ));
                }
                let index = self.state.open_block.map_or(0, |(index, _)| index);
                frames.push(anthropic_frame(
                    "content_block_delta",
                    json!({
                        "type": "content_block_delta",
                        "index": index,
                        "delta": {"type": "input_json_delta", "partial_json": arguments}
                    }),
                ));
                frames
            }
            CanonicalStreamEvent::Usage {
                input,
                output,
                cached,
            } => {
                self.state.usage = Some((*input, *output, *cached));
                vec![anthropic_frame(
                    "message_delta",
                    json!({
                        "type": "message_delta",
                        "delta": {},
                        "usage": {"output_tokens": output}
                    }),
                )]
            }
            CanonicalStreamEvent::Finished { reason } => {
                let mut frames = Vec::new();
                if let Some((index, _)) = self.state.open_block.take() {
                    frames.push(anthropic_frame(
                        "content_block_stop",
                        json!({"type": "content_block_stop", "index": index}),
                    ));
                }
                frames.push(anthropic_frame(
                    "message_delta",
                    json!({
                        "type": "message_delta",
                        "delta": {"stop_reason": anthropic_finish(reason), "stop_sequence": null},
                        "usage": {"output_tokens": self.state.usage.map_or(0, |(_, out, _)| out)}
                    }),
                ));
                frames
            }
            CanonicalStreamEvent::Error { message } => vec![anthropic_frame(
                "error",
                json!({"type": "error", "error": {"type": "api_error", "message": message}}),
            )],
        }
    }

    fn ensure_anthropic_block(&mut self, kind: BlockKind) -> Vec<WireFrame> {
        if self.state.open_block.map(|(_, open)| open) == Some(kind) {
            return Vec::new();
        }
        let mut frames = Vec::new();
        if let Some((index, _)) = self.state.open_block.take() {
            frames.push(anthropic_frame(
                "content_block_stop",
                json!({"type": "content_block_stop", "index": index}),
            ));
        }
        let index = self.state.next_block;
        self.state.next_block += 1;
        self.state.open_block = Some((index, kind));
        let block = match kind {
            BlockKind::Text => json!({"type": "text", "text": ""}),
            BlockKind::Thinking => json!({"type": "thinking", "thinking": ""}),
            BlockKind::ToolUse => json!({"type": "tool_use", "id": "", "name": "", "input": {}}),
        };
        frames.push(anthropic_frame(
            "content_block_start",
            json!({"type": "content_block_start", "index": index, "content_block": block}),
        ));
        frames
    }

    fn push_responses(&mut self, event: &CanonicalStreamEvent) -> Vec<WireFrame> {
        match event {
            CanonicalStreamEvent::Started { id, model } => {
                self.state.id = id.clone();
                self.state.model = model.clone();
                self.state.started = true;
                vec![responses_frame(
                    "response.created",
                    json!({
                        "type": "response.created",
                        "response": {
                            "id": id,
                            "object": "response",
                            "status": "in_progress",
                            "model": model,
                            "output": []
                        }
                    }),
                )]
            }
            CanonicalStreamEvent::ContentDelta { text } => {
                self.state.text.push_str(text);
                let mut frames = Vec::new();
                if !self.state.output_started {
                    self.state.output_started = true;
                    frames.push(responses_frame(
                        "response.output_item.added",
                        json!({
                            "type": "response.output_item.added",
                            "output_index": 0,
                            "item": {
                                "type": "message",
                                "id": format!("msg_{}", self.state.id),
                                "status": "in_progress",
                                "role": "assistant",
                                "content": []
                            }
                        }),
                    ));
                }
                frames.push(responses_frame(
                    "response.output_text.delta",
                    json!({
                        "type": "response.output_text.delta",
                        "item_id": format!("msg_{}", self.state.id),
                        "output_index": 0,
                        "content_index": 0,
                        "delta": text
                    }),
                ));
                frames
            }
            CanonicalStreamEvent::ReasoningDelta { text } => vec![responses_frame(
                "response.reasoning_summary_text.delta",
                json!({
                    "type": "response.reasoning_summary_text.delta",
                    "item_id": format!("rs_{}", self.state.id),
                    "output_index": 0,
                    "delta": text
                }),
            )],
            CanonicalStreamEvent::ToolCallDelta {
                index,
                id,
                name,
                arguments,
            } => {
                let mut frames = Vec::new();
                let call = self.state.tool_calls.entry(*index).or_default();
                if id.is_some() || name.is_some() {
                    if let Some(id) = id {
                        call.id = Some(id.clone());
                    }
                    if let Some(name) = name {
                        call.name = Some(name.clone());
                    }
                    self.state.output_started = true;
                    frames.push(responses_frame(
                        "response.output_item.added",
                        json!({
                            "type": "response.output_item.added",
                            "output_index": index,
                            "item": {
                                "type": "function_call",
                                "id": format!("fc_{}_{}", self.state.id, index),
                                "call_id": call.id.clone().unwrap_or_default(),
                                "name": call.name.clone().unwrap_or_default(),
                                "arguments": "",
                                "status": "in_progress"
                            }
                        }),
                    ));
                }
                let call = self.state.tool_calls.get_mut(index).expect("just inserted");
                call.arguments.push_str(arguments);
                frames.push(responses_frame(
                    "response.function_call_arguments.delta",
                    json!({
                        "type": "response.function_call_arguments.delta",
                        "item_id": format!("fc_{}_{}", self.state.id, index),
                        "output_index": index,
                        "delta": arguments
                    }),
                ));
                frames
            }
            CanonicalStreamEvent::Usage {
                input,
                output,
                cached,
            } => {
                self.state.usage = Some((*input, *output, *cached));
                Vec::new()
            }
            CanonicalStreamEvent::Finished { reason: _ } => Vec::new(),
            CanonicalStreamEvent::Error { message } => vec![responses_frame(
                "error",
                json!({"type": "error", "error": {"message": message}}),
            )],
        }
    }
}

fn chat_frame(data: Value) -> WireFrame {
    WireFrame {
        event: None,
        data: data.to_string(),
    }
}

fn anthropic_frame(event: &str, data: Value) -> WireFrame {
    WireFrame {
        event: Some(event.to_string()),
        data: data.to_string(),
    }
}

fn responses_frame(event: &str, data: Value) -> WireFrame {
    WireFrame {
        event: Some(event.to_string()),
        data: data.to_string(),
    }
}

fn chat_finish(reason: &FinishReason) -> &'static str {
    match reason {
        FinishReason::Stop => "stop",
        FinishReason::ToolUse => "tool_calls",
        FinishReason::Length => "length",
        FinishReason::ContentFilter => "content_filter",
        FinishReason::Error | FinishReason::Other(_) => "stop",
    }
}

fn anthropic_finish(reason: &FinishReason) -> &'static str {
    match reason {
        FinishReason::Stop => "end_turn",
        FinishReason::ToolUse => "tool_use",
        FinishReason::Length => "max_tokens",
        FinishReason::ContentFilter => "refusal",
        FinishReason::Error | FinishReason::Other(_) => "end_turn",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sse::canonicalize;

    #[test]
    fn chat_encode_then_decode_round_trips() {
        let events = vec![
            CanonicalStreamEvent::Started {
                id: "c1".to_string(),
                model: "m".to_string(),
            },
            CanonicalStreamEvent::ContentDelta {
                text: "hello".to_string(),
            },
            CanonicalStreamEvent::Finished {
                reason: FinishReason::Stop,
            },
            CanonicalStreamEvent::Usage {
                input: 3,
                output: 2,
                cached: Some(1),
            },
        ];

        let mut encoder = StreamEncoder::new(IngressProtocol::ChatCompletions);
        let mut text = String::new();
        for event in &events {
            for frame in encoder.push(event) {
                text.push_str(&frame.to_sse());
            }
        }
        for frame in encoder.finish() {
            text.push_str(&frame.to_sse());
        }

        let chunks = crate::sse::parse_sse_text(&text);
        let decoded = canonicalize(IngressProtocol::ChatCompletions, &chunks).expect("decode");
        assert_eq!(decoded, events);
    }

    #[test]
    fn anthropic_starts_blocks_lazily() {
        let mut encoder = StreamEncoder::new(IngressProtocol::AnthropicMessages);
        let mut frames = encoder.push(&CanonicalStreamEvent::Started {
            id: "m1".to_string(),
            model: "claude".to_string(),
        });
        frames.extend(encoder.push(&CanonicalStreamEvent::ContentDelta {
            text: "hi".to_string(),
        }));
        let events: Vec<_> = frames
            .iter()
            .filter_map(|frame| frame.event.clone())
            .collect();
        assert_eq!(
            events,
            vec![
                "message_start".to_string(),
                "content_block_start".to_string(),
                "content_block_delta".to_string()
            ]
        );
    }

    #[test]
    fn responses_completes_with_assembled_output() {
        let mut encoder = StreamEncoder::new(IngressProtocol::Responses);
        encoder.push(&CanonicalStreamEvent::Started {
            id: "r1".to_string(),
            model: "gpt".to_string(),
        });
        encoder.push(&CanonicalStreamEvent::ContentDelta {
            text: "done".to_string(),
        });
        encoder.push(&CanonicalStreamEvent::Usage {
            input: 5,
            output: 1,
            cached: None,
        });
        encoder.push(&CanonicalStreamEvent::Finished {
            reason: FinishReason::Stop,
        });

        let frames = encoder.finish();
        assert_eq!(frames.len(), 1);
        let value: Value = serde_json::from_str(&frames[0].data).expect("json");
        assert_eq!(value["response"]["output"][0]["content"][0]["text"], "done");
        assert_eq!(value["response"]["usage"]["input_tokens"], 5);
    }
}
