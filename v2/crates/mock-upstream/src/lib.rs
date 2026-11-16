//! 本地 mock 上游（替代 S0-3 的 v1 重放）。
//!
//! 用 fixture 形状驱动 P0a e2e：测试按顺序给出一组回复（200 / 503 / 超时），
//! `agent::Pipeline` 照着真实 HTTP 路径执行，不需要任何 API key。

#![forbid(unsafe_code)]

use std::collections::VecDeque;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::IntoResponse;
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
}

#[derive(Clone)]
struct MockState {
    replies: Arc<Mutex<VecDeque<Reply>>>,
}

/// 跑在后台的 mock 上游。
pub struct MockUpstream {
    addr: SocketAddr,
    replies: Arc<Mutex<VecDeque<Reply>>>,
    handle: tokio::task::JoinHandle<()>,
}

impl MockUpstream {
    /// 启动并绑定随机端口。
    pub async fn start(replies: Vec<Reply>) -> Self {
        let replies = Arc::new(Mutex::new(replies.into()));
        let state = MockState {
            replies: replies.clone(),
        };

        let app = Router::new()
            .route("/v1/chat/completions", post(chat))
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
            handle,
        }
    }

    pub fn base_url(&self) -> String {
        format!("http://{}/v1", self.addr)
    }

    pub fn remaining(&self) -> usize {
        self.replies.lock().expect("mock mutex").len()
    }

    /// 停止服务。
    pub fn shutdown(self) {
        self.handle.abort();
    }
}

async fn chat(State(state): State<MockState>) -> impl IntoResponse {
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
    }
}

async fn models() -> Json<Value> {
    Json(json!({"object": "list", "data": []}))
}
