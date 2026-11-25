//! 流式执行（P0b-2）：上游 SSE → canonical 事件，经 channel 交给编排层。
//!
//! 设计要点：
//! - 解码是增量的（Chat 走 `ChatStreamDecoder`；Anthropic/Responses 走本文件的
//!   [`CanonicalStreamDecoder`]，按 `tr_proto::sse::canonicalize` 的同一套语义推进）；
//! - 接收端丢弃（客户端断连）时，转发任务立即退出，进而释放上游连接；
//! - 错误只有两类：HTTP 非 2xx（握手失败）与流中途失败；两者都带 `ErrorClass`。

use futures_util::StreamExt;
use serde_json::Value;
use tokio::sync::mpsc;
use tr_contracts::{
    Candidate, CanonicalRequest, CanonicalStreamEvent, ErrorClass, IngressProtocol,
};
use tr_proto::sse::{ChatStreamDecoder, RawChunk, StreamParseError, canonicalize, parse_sse_text};

use crate::http::{
    AttemptCause, AttemptFailure, HttpUpstream, apply_thinking_disable, encode_wire,
    physical_request, unsupported_provider, wire_protocol,
};

/// 流式 attempt 的条目。
#[derive(Debug)]
pub enum StreamItem {
    Event(CanonicalStreamEvent),
    Failed(AttemptFailure),
    /// 上游正常结束（收到 `[DONE]` 或连接关闭）。
    Ended,
}

/// 已建立的流式 attempt。
pub struct StreamAttempt {
    pub candidate_id: String,
    pub status: u16,
    pub events: mpsc::Receiver<StreamItem>,
    /// 建立连接时的限流头观测（v3 P5-B）。
    pub rate_limit: Option<Box<tr_contracts::RateLimitSnapshot>>,
}

fn now_millis() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as i64)
        .unwrap_or(0)
}

/// 按 wire 协议选增量解码器。Chat 沿用既有的线性解码器；其余协议共用
/// [`CanonicalStreamDecoder`]（语义以 `tr_proto::sse::canonicalize` 为准）。
enum StreamCodec {
    Chat(ChatStreamDecoder),
    Canonical(CanonicalStreamDecoder),
}

impl StreamCodec {
    fn new(protocol: IngressProtocol) -> Self {
        match protocol {
            IngressProtocol::ChatCompletions => Self::Chat(ChatStreamDecoder::new()),
            other => Self::Canonical(CanonicalStreamDecoder::new(other)),
        }
    }

    fn feed(&mut self, bytes: &[u8]) -> Result<Vec<CanonicalStreamEvent>, StreamParseError> {
        match self {
            Self::Chat(decoder) => decoder.feed(bytes),
            Self::Canonical(decoder) => decoder.feed(bytes),
        }
    }

    fn is_done(&self) -> bool {
        match self {
            Self::Chat(decoder) => decoder.is_done(),
            Self::Canonical(decoder) => decoder.is_done(),
        }
    }

    fn finish(&mut self) -> Result<Vec<CanonicalStreamEvent>, StreamParseError> {
        match self {
            Self::Chat(decoder) => decoder.finish(),
            Self::Canonical(decoder) => decoder.finish(),
        }
    }
}

/// Anthropic / Responses 的增量解码。
///
/// `tr_proto::sse::canonicalize` 是「整段 chunk 列表 → 事件序列」的纯函数，
/// 且跨 chunk 状态只有起始事件（Anthropic 的 `message_start` 用来说明
/// input/cache tokens；Responses 的 `response.created` 只标记 started）。
/// 因此保留起始前缀 `seed`，每次只对 `seed + 新 chunk` 重新归一化并跳过
/// seed 已产生的事件——总复杂度与 chunk 数线性相关，不缓存整条流。
struct CanonicalStreamDecoder {
    protocol: IngressProtocol,
    buffer: String,
    /// 起始事件之前的 chunk（Anthropic 的 `ping` 等）。
    head: Vec<RawChunk>,
    /// 起始前缀（含 `message_start` / `response.created`）。
    seed: Vec<RawChunk>,
    /// 上次归一化之后新到的 chunk。
    pending: Vec<RawChunk>,
    seed_events: usize,
    done: bool,
}

impl CanonicalStreamDecoder {
    fn new(protocol: IngressProtocol) -> Self {
        Self {
            protocol,
            buffer: String::new(),
            head: Vec::new(),
            seed: Vec::new(),
            pending: Vec::new(),
            seed_events: 0,
            done: false,
        }
    }

    fn is_done(&self) -> bool {
        self.done
    }

    fn feed(&mut self, bytes: &[u8]) -> Result<Vec<CanonicalStreamEvent>, StreamParseError> {
        self.buffer.push_str(&String::from_utf8_lossy(bytes));
        if self.buffer.contains('\r') {
            self.buffer = self.buffer.replace("\r\n", "\n");
        }

        let mut chunks = Vec::new();
        while let Some(index) = self.buffer.find("\n\n") {
            let block: String = self.buffer.drain(..index + 2).collect();
            chunks.extend(parse_sse_text(&block));
        }
        self.push_chunks(chunks)
    }

    fn finish(&mut self) -> Result<Vec<CanonicalStreamEvent>, StreamParseError> {
        let remaining = std::mem::take(&mut self.buffer);
        let chunks = if remaining.trim().is_empty() {
            Vec::new()
        } else {
            parse_sse_text(&remaining)
        };
        let mut out = self.push_chunks(chunks)?;
        if self.seed.is_empty() {
            return Err(StreamParseError::MissingField("start event"));
        }
        out.extend(self.flush_pending()?);
        Ok(out)
    }

    fn push_chunks(
        &mut self,
        chunks: Vec<RawChunk>,
    ) -> Result<Vec<CanonicalStreamEvent>, StreamParseError> {
        let mut out = Vec::new();
        for chunk in chunks {
            if self.seed.is_empty() {
                let started = is_start_chunk(self.protocol, &chunk);
                self.head.push(chunk);
                if !started {
                    continue;
                }
                self.seed = std::mem::take(&mut self.head);
                let events = canonicalize(self.protocol, &self.seed)?;
                self.seed_events = events.len();
                self.done |= self
                    .seed
                    .iter()
                    .any(|chunk| is_terminal(self.protocol, chunk));
                out.extend(events);
                continue;
            }
            self.done |= is_terminal(self.protocol, &chunk);
            self.pending.push(chunk);
        }
        out.extend(self.flush_pending()?);
        Ok(out)
    }

    fn flush_pending(&mut self) -> Result<Vec<CanonicalStreamEvent>, StreamParseError> {
        if self.pending.is_empty() {
            return Ok(Vec::new());
        }
        let mut all = Vec::with_capacity(self.seed.len() + self.pending.len());
        all.extend(self.seed.iter().cloned());
        all.extend(self.pending.iter().cloned());
        let events = canonicalize(self.protocol, &all)?;
        self.pending.clear();
        Ok(events.into_iter().skip(self.seed_events).collect())
    }
}

fn chunk_kind(chunk: &RawChunk) -> Option<&str> {
    chunk
        .data
        .get("type")
        .and_then(Value::as_str)
        .or(chunk.event.as_deref())
}

fn is_start_chunk(protocol: IngressProtocol, chunk: &RawChunk) -> bool {
    match protocol {
        IngressProtocol::AnthropicMessages => chunk_kind(chunk) == Some("message_start"),
        IngressProtocol::Responses => chunk_kind(chunk) == Some("response.created"),
        IngressProtocol::ChatCompletions => true,
    }
}

fn is_terminal(protocol: IngressProtocol, chunk: &RawChunk) -> bool {
    match protocol {
        IngressProtocol::AnthropicMessages => {
            matches!(chunk_kind(chunk), Some("message_stop" | "error"))
        }
        IngressProtocol::Responses => matches!(
            chunk_kind(chunk),
            Some("response.completed" | "response.failed" | "error")
        ),
        IngressProtocol::ChatCompletions => false,
    }
}

impl HttpUpstream {
    /// 建立流式请求。返回时只保证“上游已接受请求”，事件从 `events` 里读。
    pub async fn execute_stream(
        &self,
        candidate: &Candidate,
        request: &CanonicalRequest,
    ) -> Result<StreamAttempt, AttemptFailure> {
        let protocol = wire_protocol(&candidate.upstream.provider)
            .ok_or_else(|| unsupported_provider(&candidate.upstream.provider, "流式"))?;

        let mut wire = physical_request(candidate, request);
        wire.output.stream = true;
        let mut body = encode_wire(protocol, &wire);
        apply_thinking_disable(candidate, request, &mut body);

        let response = self
            .post_for(candidate)?
            .header("accept", "text/event-stream")
            .json(&body)
            .send()
            .await
            .map_err(|error| AttemptFailure {
                class: if error.is_timeout() {
                    ErrorClass::Timeout
                } else {
                    ErrorClass::Transport
                },
                status: None,
                message: Some(error.to_string()),
                retry_after_ms: None,
                cause: AttemptCause::Transport,
                rate_limit: None,
            })?;

        let status = response.status();
        if !status.is_success() {
            let headers: std::collections::BTreeMap<String, String> = response
                .headers()
                .iter()
                .filter_map(|(name, value)| {
                    value
                        .to_str()
                        .ok()
                        .map(|value| (name.as_str().to_string(), value.to_string()))
                })
                .collect();
            let text = response.text().await.unwrap_or_default();
            let body: Value = serde_json::from_str(&text)
                .unwrap_or_else(|_| serde_json::json!({"error": {"message": text}}));
            let classified = tr_proto::errors::classify(status.as_u16(), &headers, &body);
            let rate_limit =
                crate::rate_limit::parse_rate_limit(&headers, status.as_u16(), now_millis())
                    .map(Box::new);
            return Err(AttemptFailure {
                class: classified.class,
                status: Some(status.as_u16()),
                message: classified.message,
                retry_after_ms: classified.retry_after_ms,
                cause: AttemptCause::HttpStatus,
                rate_limit,
            });
        }

        let headers: std::collections::BTreeMap<String, String> = response
            .headers()
            .iter()
            .filter_map(|(name, value)| {
                value
                    .to_str()
                    .ok()
                    .map(|value| (name.as_str().to_string(), value.to_string()))
            })
            .collect();
        let rate_limit =
            crate::rate_limit::parse_rate_limit(&headers, status.as_u16(), now_millis())
                .map(Box::new);

        let (tx, rx) = mpsc::channel(64);
        tokio::spawn(async move {
            let mut stream = response.bytes_stream();
            let mut decoder = StreamCodec::new(protocol);

            while let Some(chunk) = stream.next().await {
                match chunk {
                    Ok(bytes) => match decoder.feed(&bytes) {
                        Ok(events) => {
                            for event in events {
                                if tx.send(StreamItem::Event(event)).await.is_err() {
                                    return;
                                }
                            }
                        }
                        Err(error) => {
                            let _ = tx
                                .send(StreamItem::Failed(AttemptFailure {
                                    class: ErrorClass::Protocol,
                                    status: None,
                                    message: Some(error.to_string()),
                                    retry_after_ms: None,
                                    cause: AttemptCause::MalformedResponse,
                                    rate_limit: None,
                                }))
                                .await;
                            return;
                        }
                    },
                    Err(error) => {
                        let _ = tx
                            .send(StreamItem::Failed(AttemptFailure {
                                class: if error.is_timeout() {
                                    ErrorClass::Timeout
                                } else {
                                    ErrorClass::Transport
                                },
                                status: None,
                                message: Some(error.to_string()),
                                retry_after_ms: None,
                                cause: AttemptCause::Transport,
                                rate_limit: None,
                            }))
                            .await;
                        return;
                    }
                }

                if decoder.is_done() {
                    break;
                }
            }

            if let Ok(events) = decoder.finish() {
                for event in events {
                    if tx.send(StreamItem::Event(event)).await.is_err() {
                        return;
                    }
                }
            }
            let _ = tx.send(StreamItem::Ended).await;
        });

        Ok(StreamAttempt {
            candidate_id: candidate.id.clone(),
            status: status.as_u16(),
            events: rx,
            rate_limit,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sse(event: &str, data: serde_json::Value) -> String {
        format!("event: {event}\ndata: {data}\n\n")
    }

    #[test]
    fn anthropic_stream_decodes_incrementally() {
        let text = [
            sse(
                "message_start",
                serde_json::json!({
                    "type": "message_start",
                    "message": {
                        "id": "m1",
                        "model": "claude",
                        "usage": {"input_tokens": 10}
                    }
                }),
            ),
            sse(
                "content_block_delta",
                serde_json::json!({
                    "type": "content_block_delta",
                    "index": 0,
                    "delta": {"type": "text_delta", "text": "hi"}
                }),
            ),
            sse(
                "message_delta",
                serde_json::json!({
                    "type": "message_delta",
                    "delta": {"stop_reason": "end_turn"},
                    "usage": {"output_tokens": 2}
                }),
            ),
            sse("message_stop", serde_json::json!({"type": "message_stop"})),
        ]
        .concat();

        let mut decoder = CanonicalStreamDecoder::new(IngressProtocol::AnthropicMessages);
        let mut events = Vec::new();
        for chunk in text.as_bytes().chunks(17) {
            events.extend(decoder.feed(chunk).expect("decode chunk"));
        }
        events.extend(decoder.finish().expect("finish"));

        assert_eq!(
            events
                .iter()
                .filter(|event| matches!(event, CanonicalStreamEvent::Started { .. }))
                .count(),
            1,
            "起始事件只发一次"
        );
        assert!(events.iter().any(|event| matches!(
            event,
            CanonicalStreamEvent::ContentDelta { text } if text == "hi"
        )));
        assert!(events.iter().any(|event| matches!(
            event,
            CanonicalStreamEvent::Usage {
                input: 10,
                output: 2,
                ..
            }
        )));
        assert!(matches!(
            events.last(),
            Some(CanonicalStreamEvent::Finished { .. })
        ));
        assert!(decoder.is_done());
    }

    #[test]
    fn responses_stream_decodes_incrementally() {
        let text = [
            sse(
                "response.created",
                serde_json::json!({
                    "type": "response.created",
                    "response": {"id": "r1", "model": "gpt", "usage": null}
                }),
            ),
            sse(
                "response.output_text.delta",
                serde_json::json!({"type": "response.output_text.delta", "delta": "yo"}),
            ),
            sse(
                "response.completed",
                serde_json::json!({
                    "type": "response.completed",
                    "response": {
                        "usage": {"input_tokens": 5, "output_tokens": 1},
                        "output": []
                    }
                }),
            ),
        ]
        .concat();

        let mut decoder = CanonicalStreamDecoder::new(IngressProtocol::Responses);
        let mut events = Vec::new();
        for chunk in text.as_bytes().chunks(11) {
            events.extend(decoder.feed(chunk).expect("decode chunk"));
        }
        events.extend(decoder.finish().expect("finish"));

        assert_eq!(
            events
                .iter()
                .filter(|event| matches!(event, CanonicalStreamEvent::Started { .. }))
                .count(),
            1
        );
        assert!(events.iter().any(|event| matches!(
            event,
            CanonicalStreamEvent::ContentDelta { text } if text == "yo"
        )));
        assert!(events.iter().any(|event| matches!(
            event,
            CanonicalStreamEvent::Usage {
                input: 5,
                output: 1,
                ..
            }
        )));
        assert!(matches!(
            events.last(),
            Some(CanonicalStreamEvent::Finished { .. })
        ));
        assert!(decoder.is_done());
    }
}
