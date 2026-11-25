//! 本地 mock 上游（替代 S0-3 的 v1 重放）。
//!
//! 用 fixture 形状驱动 P0a e2e：测试按顺序给出一组回复（200 / 503 / 超时），
//! `agent::Pipeline` 照着真实 HTTP 路径执行，不需要任何 API key。

#![forbid(unsafe_code)]

use std::collections::VecDeque;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::body::{Body, Bytes};
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde_json::{Value, json};

/// 一次上游回复。
#[derive(Debug, Clone)]
pub enum Reply {
    /// 直接返回 JSON。
    Json { status: u16, body: Value },
    /// 延迟后返回 200。
    Delay { millis: u64, body: Value },
    /// 原样返回 SSE 文本。
    RawSse { body: String },
    /// 按固定间隔逐帧返回 SSE（用于测试客户端断开/取消）。
    SlowSse {
        delay_millis: u64,
        frames: Vec<String>,
    },
}

impl Reply {
    /// 成功回复（Chat Completions 形状）。
    pub fn ok(content: &str) -> Self {
        Self::Json {
            status: 200,
            body: json!({
                "id": "chatcmpl-mock",
                "object": "chat.completion",
                "created": 0,
                "model": "mock-physical",
                "choices": [{
                    "index": 0,
                    "message": {"role": "assistant", "content": content},
                    "finish_reason": "stop"
                }],
                "usage": {
                    "prompt_tokens": 12,
                    "completion_tokens": 4,
                    "total_tokens": 16,
                    "prompt_tokens_details": {"cached_tokens": 0}
                }
            }),
        }
    }

    /// 错误回复（OpenAI 兼容错误体）。
    pub fn error(status: u16, kind: &str, message: &str) -> Self {
        Self::Json {
            status,
            body: json!({"error": {"type": kind, "message": message, "code": kind}}),
        }
    }

    /// 成功回复（Anthropic Messages 形状）。
    pub fn anthropic_ok(content: &str) -> Self {
        Self::Json {
            status: 200,
            body: json!({
                "id": "msg_mock",
                "type": "message",
                "role": "assistant",
                "model": "mock-physical",
                "content": [{"type": "text", "text": content}],
                "stop_reason": "end_turn",
                "stop_sequence": null,
                "usage": {"input_tokens": 12, "output_tokens": 4}
            }),
        }
    }

    /// 成功回复（Responses 形状）。
    pub fn responses_ok(content: &str) -> Self {
        Self::Json {
            status: 200,
            body: json!({
                "id": "resp_mock",
                "object": "response",
                "created_at": 0,
                "model": "mock-physical",
                "status": "completed",
                "output": [{
                    "type": "message",
                    "id": "msg_mock",
                    "role": "assistant",
                    "content": [{"type": "output_text", "text": content}]
                }],
                "usage": {"input_tokens": 12, "output_tokens": 4}
            }),
        }
    }

    /// Anthropic Messages 流：message_start → text delta → message_delta → stop。
    pub fn anthropic_stream(parts: &[&str]) -> Self {
        let mut body = String::new();
        body.push_str(&format!(
            "event: message_start\ndata: {}\n\n",
            json!({
                "type": "message_start",
                "message": {
                    "id": "msg_mock",
                    "model": "mock-physical",
                    "usage": {"input_tokens": 12}
                }
            })
        ));
        for part in parts {
            body.push_str(&format!(
                "event: content_block_delta\ndata: {}\n\n",
                json!({
                    "type": "content_block_delta",
                    "index": 0,
                    "delta": {"type": "text_delta", "text": part}
                })
            ));
        }
        body.push_str(&format!(
            "event: message_delta\ndata: {}\n\n",
            json!({
                "type": "message_delta",
                "delta": {"stop_reason": "end_turn"},
                "usage": {"output_tokens": parts.len() as u32}
            })
        ));
        body.push_str(&format!(
            "event: message_stop\ndata: {}\n\n",
            json!({"type": "message_stop"})
        ));
        Self::RawSse { body }
    }

    /// Responses 流：response.created → text delta → response.completed。
    pub fn responses_stream(parts: &[&str]) -> Self {
        let mut body = String::new();
        body.push_str(&format!(
            "event: response.created\ndata: {}\n\n",
            json!({
                "type": "response.created",
                "response": {"id": "resp_mock", "model": "mock-physical"}
            })
        ));
        for part in parts {
            body.push_str(&format!(
                "event: response.output_text.delta\ndata: {}\n\n",
                json!({"type": "response.output_text.delta", "delta": part})
            ));
        }
        body.push_str(&format!(
            "event: response.completed\ndata: {}\n\n",
            json!({
                "type": "response.completed",
                "response": {
                    "usage": {"input_tokens": 12, "output_tokens": parts.len() as u32},
                    "output": []
                }
            })
        ));
        Self::RawSse { body }
    }

    /// Chat Completions 流：逐片 content + finish + usage + `[DONE]`。
    pub fn chat_stream(parts: &[&str]) -> Self {
        let id = "chatcmpl-mock-stream";
        let model = "mock-physical";
        let mut body = String::new();

        let mut push = |delta: Value, finish: Value| {
            body.push_str(&format!(
                "data: {}\n\n",
                json!({
                    "id": id,
                    "object": "chat.completion.chunk",
                    "created": 0,
                    "model": model,
                    "choices": [{"index": 0, "delta": delta, "finish_reason": finish}]
                })
            ));
        };

        push(json!({"role": "assistant", "content": ""}), Value::Null);
        for part in parts {
            push(json!({"content": part}), Value::Null);
        }
        push(json!({}), json!("stop"));
        body.push_str(&format!(
            "data: {}\n\n",
            json!({
                "id": id,
                "object": "chat.completion.chunk",
                "model": model,
                "choices": [],
                "usage": {
                    "prompt_tokens": 12,
                    "completion_tokens": parts.len() as u32,
                    "total_tokens": 12 + parts.len() as u32,
                    "prompt_tokens_details": {"cached_tokens": 0}
                }
            })
        ));
        body.push_str("data: [DONE]\n\n");

        Self::RawSse { body }
    }

    /// 慢速 Chat Completions 流：每帧间隔 `delay_millis`，用于取消语义测试。
    pub fn slow_chat_stream(parts: &[&str], delay_millis: u64) -> Self {
        let id = "chatcmpl-mock-slow";
        let model = "mock-physical";
        let mut frames = Vec::new();

        let mut frame = |delta: Value, finish: Value| {
            frames.push(format!(
                "data: {}\n\n",
                json!({
                    "id": id,
                    "object": "chat.completion.chunk",
                    "created": 0,
                    "model": model,
                    "choices": [{"index": 0, "delta": delta, "finish_reason": finish}]
                })
            ));
        };

        frame(json!({"role": "assistant", "content": ""}), Value::Null);
        for part in parts {
            frame(json!({"content": part}), Value::Null);
        }
        frame(json!({}), json!("stop"));
        frames.push(format!(
            "data: {}\n\n",
            json!({
                "id": id,
                "object": "chat.completion.chunk",
                "created": 0,
                "model": model,
                "choices": [],
                "usage": {
                    "prompt_tokens": 12,
                    "completion_tokens": parts.len() as u32,
                    "total_tokens": 12 + parts.len() as u32,
                    "prompt_tokens_details": {"cached_tokens": 0}
                }
            })
        ));
        frames.push("data: [DONE]\n\n".to_string());

        Self::SlowSse {
            delay_millis,
            frames,
        }
    }
}

#[derive(Clone)]
struct MockState {
    replies: Arc<Mutex<VecDeque<Reply>>>,
    /// 收到的 JSON 请求体（按到达顺序），供测试断言提示词内容。
    requests: Arc<Mutex<Vec<Value>>>,
    /// 收到的请求路径（按到达顺序），供测试断言 wire 协议端点。
    paths: Arc<Mutex<Vec<String>>>,
}

/// 跑在后台的 mock 上游。
pub struct MockUpstream {
    addr: SocketAddr,
    replies: Arc<Mutex<VecDeque<Reply>>>,
    requests: Arc<Mutex<Vec<Value>>>,
    paths: Arc<Mutex<Vec<String>>>,
    handle: tokio::task::JoinHandle<()>,
}

impl MockUpstream {
    /// 启动并绑定随机端口。
    pub async fn start(replies: Vec<Reply>) -> Self {
        let replies = Arc::new(Mutex::new(replies.into()));
        let requests = Arc::new(Mutex::new(Vec::new()));
        let paths = Arc::new(Mutex::new(Vec::new()));
        let state = MockState {
            replies: replies.clone(),
            requests: requests.clone(),
            paths: paths.clone(),
        };

        let app = Router::new()
            .route("/v1/chat/completions", post(chat))
            .route("/v1/messages", post(chat))
            .route("/v1/responses", post(chat))
            .route("/v1/systemone", post(chat))
            .route("/v1/models", get(models))
            .with_state(state);

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind mock upstream");
        let addr = listener.local_addr().expect("local addr");

        let handle = tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });

        Self {
            addr,
            replies,
            requests,
            paths,
            handle,
        }
    }

    pub fn base_url(&self) -> String {
        format!("http://{}/v1", self.addr)
    }

    pub fn remaining(&self) -> usize {
        self.replies.lock().expect("mock mutex").len()
    }

    /// 收到的 JSON 请求体快照（按到达顺序）。
    pub fn requests(&self) -> Vec<Value> {
        self.requests.lock().expect("mock mutex").clone()
    }

    /// 收到的请求路径快照（按到达顺序）。
    pub fn paths(&self) -> Vec<String> {
        self.paths.lock().expect("mock mutex").clone()
    }

    /// 停止服务。
    pub fn shutdown(self) {
        self.handle.abort();
    }
}

async fn chat(
    State(state): State<MockState>,
    uri: axum::http::Uri,
    body: Bytes,
) -> impl IntoResponse {
    state
        .paths
        .lock()
        .expect("mock mutex")
        .push(uri.path().to_string());
    if let Ok(value) = serde_json::from_slice::<Value>(&body) {
        state.requests.lock().expect("mock mutex").push(value);
    }
    let reply = state
        .replies
        .lock()
        .expect("mock mutex")
        .pop_front()
        .unwrap_or(Reply::Json {
            status: 500,
            body: json!({"error": {"type": "server_error", "message": "mock exhausted"}}),
        });

    match reply {
        Reply::Json { status, body } => (
            StatusCode::from_u16(status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR),
            Json(body),
        )
            .into_response(),
        Reply::Delay { millis, body } => {
            tokio::time::sleep(Duration::from_millis(millis)).await;
            (StatusCode::OK, Json(body)).into_response()
        }
        Reply::RawSse { body } => (
            StatusCode::OK,
            [(axum::http::header::CONTENT_TYPE, "text/event-stream")],
            body,
        )
            .into_response(),
        Reply::SlowSse {
            delay_millis,
            frames,
        } => {
            let stream = async_stream::stream! {
                for frame in frames {
                    tokio::time::sleep(Duration::from_millis(delay_millis)).await;
                    yield Ok::<Bytes, std::io::Error>(Bytes::from(frame));
                }
            };
            Response::builder()
                .status(StatusCode::OK)
                .header(axum::http::header::CONTENT_TYPE, "text/event-stream")
                .body(Body::from_stream(stream))
                .expect("sse response")
        }
    }
}

async fn models() -> Json<Value> {
    Json(json!({"object": "list", "data": []}))
}
