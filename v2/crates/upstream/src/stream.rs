//! 流式执行（P0b-2）：上游 SSE → canonical 事件，经 channel 交给编排层。
//!
//! 设计要点：
//! - 解码是增量的（`ChatStreamDecoder`），不缓存整段；
//! - 接收端丢弃（客户端断连）时，转发任务立即退出，进而释放上游连接；
//! - 错误只有两类：HTTP 非 2xx（握手失败）与流中途失败；两者都带 `ErrorClass`。

use futures_util::StreamExt;
use serde_json::Value;
use tokio::sync::mpsc;
use tr_contracts::{Candidate, CanonicalRequest, CanonicalStreamEvent, ErrorClass, ProviderKind};
use tr_proto::chat;
use tr_proto::sse::ChatStreamDecoder;

use crate::http::{
    AttemptCause, AttemptFailure, HttpUpstream, endpoint, physical_request, resolve_credential,
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

impl HttpUpstream {
    /// 建立流式请求。返回时只保证“上游已接受请求”，事件从 `events` 里读。
    pub async fn execute_stream(
        &self,
        candidate: &Candidate,
        request: &CanonicalRequest,
    ) -> Result<StreamAttempt, AttemptFailure> {
        if candidate.upstream.provider != ProviderKind::OpenAiCompatible {
            return Err(AttemptFailure {
                class: ErrorClass::Protocol,
                status: None,
                message: Some(format!(
                    "provider {:?} 尚未在 P0b 实现流式",
                    candidate.upstream.provider
                )),
                retry_after_ms: None,
                cause: AttemptCause::UnsupportedProvider,
                rate_limit: None,
            });
        }

        let mut wire = physical_request(candidate, request);
        wire.output.stream = true;
        let body = chat::encode_request(&wire);

        let mut builder = self
            .client
            .post(endpoint(&candidate.upstream.base_url))
            .header("accept", "text/event-stream")
            .json(&body);
        if let Some(reference) = &candidate.upstream.credential {
            builder = builder.bearer_auth(resolve_credential(reference)?);
        }

        let response = builder.send().await.map_err(|error| AttemptFailure {
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
            let mut decoder = ChatStreamDecoder::new();

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
