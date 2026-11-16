//! axum 组装、鉴权、模型端点与决策头（P0a-13/14/15）。
//!
//! 边界：不直接调用 `core`/`decide`，一律经 `agent::Pipeline`。
//! 客户端只通过 HTTP API 访问内核，不直接读 SQLite。

#![forbid(unsafe_code)]

use std::net::SocketAddr;
use std::sync::Arc;

use axum::extract::State;
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde_json::{Value, json};
use tr_agent::pipeline::{DecisionHeaders, Pipeline, PipelineError, RequestContext};
use tr_proto::chat;

/// 鉴权方式（P0 只有入站 key）。
#[derive(Clone)]
pub enum AuthMode {
    Disabled,
    InboundKey(String),
}

/// HTTP 应用状态。
#[derive(Clone)]
pub struct AppState {
    pipeline: Arc<Pipeline>,
    auth: AuthMode,
}

impl AppState {
    pub fn new(pipeline: Pipeline, auth: AuthMode) -> Self {
        Self {
            pipeline: Arc::new(pipeline),
            auth,
        }
    }

    /// 共享同一个 `Pipeline`（测试与后续 embed 入口使用）。
    pub fn new_shared(pipeline: Arc<Pipeline>, auth: AuthMode) -> Self {
        Self { pipeline, auth }
    }
}

/// 路由表。
pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/health", get(health))
        .route("/v1/models", get(models))
        .route("/v1/chat/completions", post(chat_completions))
        .with_state(state)
}

/// 启动服务。
pub async fn serve(state: AppState, addr: SocketAddr) -> std::io::Result<()> {
    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, router(state)).await
}

async fn health() -> Json<Value> {
    Json(json!({"status": "ok"}))
}

async fn models(State(state): State<AppState>) -> Json<Value> {
    let catalog = state.pipeline.catalog();
    let data: Vec<Value> = catalog
        .virtual_models
        .values()
        .map(|model| {
            json!({
                "id": model.id,
                "object": "model",
                "owned_by": model.provider,
                "name": model.name,
            })
        })
        .collect();
    Json(json!({"object": "list", "data": data}))
}

async fn chat_completions(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Response {
    if !authorized(&state.auth, &headers) {
        return error_response(
            StatusCode::UNAUTHORIZED,
            "authentication_error",
            "invalid or missing inbound key",
        );
    }

    let request = match chat::decode_request(&body) {
        Ok(request) => request,
        Err(err) => {
            return error_response(
                StatusCode::BAD_REQUEST,
                "invalid_request_error",
                &err.to_string(),
            );
        }
    };

    let context = RequestContext {
        server_session_id: header_string(&headers, "x-token-router-session-id"),
        request_reason: header_string(&headers, "x-token-router-request-reason")
            .as_deref()
            .and_then(tr_agent::reason::parse_header),
    };

    match state.pipeline.handle(request, context).await {
        Ok(result) => {
            let body = chat::encode_response(&result.response, now_secs());
            let mut response = (StatusCode::OK, Json(body)).into_response();
            apply_decision_headers(response.headers_mut(), &result.headers);
            response
        }
        Err(err) => map_error(err),
    }
}

fn map_error(error: PipelineError) -> Response {
    match error {
        PipelineError::ModelNotFound(model) => error_response(
            StatusCode::BAD_REQUEST,
            "invalid_request_error",
            &format!("model not found: {model}"),
        ),
        PipelineError::NoEligibleCandidate { reason_codes, .. } => {
            let codes = reason_codes
                .iter()
                .map(|reason| reason.code.as_str())
                .collect::<Vec<_>>()
                .join(",");
            error_response(
                StatusCode::SERVICE_UNAVAILABLE,
                "no_eligible_candidate",
                &format!("no eligible candidate ({codes})"),
            )
        }
        PipelineError::Upstream {
            class,
            status,
            message,
            request_id,
        } => {
            eprintln!("upstream failure request_id={request_id} class={class}");
            let status = status
                .and_then(|code| StatusCode::from_u16(code).ok())
                .unwrap_or(StatusCode::BAD_GATEWAY);
            error_response(
                status,
                class.as_str(),
                message.as_deref().unwrap_or("upstream error"),
            )
        }
        PipelineError::Store(err) => {
            eprintln!("storage failure: {err}");
            error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "internal_error",
                "storage failure",
            )
        }
    }
}

fn error_response(status: StatusCode, kind: &str, message: &str) -> Response {
    (
        status,
        Json(json!({"error": {"type": kind, "message": message}})),
    )
        .into_response()
}

fn apply_decision_headers(headers: &mut HeaderMap, decision: &DecisionHeaders) {
    let pairs = [
        (
            "x-token-router-selected-model",
            decision.selected_model.as_str(),
        ),
        (
            "x-token-router-served-model",
            decision.served_model.as_str(),
        ),
        (
            "x-token-router-thinking-level",
            decision.thinking_level.as_str(),
        ),
        ("x-token-router-request-reason", decision.request_reason),
        ("x-token-router-dispatch-source", decision.dispatch_source),
        ("x-token-router-plan-id", decision.plan_id.as_str()),
        ("x-token-router-served-slot", decision.served_slot.as_str()),
        ("x-token-router-mode", decision.mode),
        (
            "x-token-router-reason-codes",
            decision.reason_codes.as_str(),
        ),
        ("x-token-router-judge", decision.judge),
    ];
    for (name, value) in pairs {
        if let Ok(value) = HeaderValue::from_str(value) {
            headers.insert(name, value);
        }
    }
}

fn authorized(auth: &AuthMode, headers: &HeaderMap) -> bool {
    let AuthMode::InboundKey(expected) = auth else {
        return true;
    };
    if expected.is_empty() {
        return false;
    }

    let provided = headers
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
        .or_else(|| {
            headers
                .get("x-api-key")
                .and_then(|value| value.to_str().ok())
        });

    provided.is_some_and(|provided| constant_time_eq(provided.as_bytes(), expected.as_bytes()))
}

/// 常数时间比较，避免按字节提前返回泄漏密钥前缀。
fn constant_time_eq(left: &[u8], right: &[u8]) -> bool {
    if left.len() != right.len() {
        return false;
    }
    let mut diff = 0u8;
    for (a, b) in left.iter().zip(right.iter()) {
        diff |= a ^ b;
    }
    diff == 0
}

fn header_string(headers: &HeaderMap, name: &str) -> Option<String> {
    headers
        .get(name)
        .and_then(|value| value.to_str().ok())
        .map(str::to_string)
}

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn constant_time_eq_matches() {
        assert!(constant_time_eq(b"abc", b"abc"));
        assert!(!constant_time_eq(b"abc", b"abd"));
        assert!(!constant_time_eq(b"abc", b"ab"));
    }

    #[test]
    fn auth_disabled_allows_anything() {
        let headers = HeaderMap::new();
        assert!(authorized(&AuthMode::Disabled, &headers));
    }

    #[test]
    fn auth_accepts_bearer_and_api_key() {
        let mut headers = HeaderMap::new();
        headers.insert(
            header::AUTHORIZATION,
            HeaderValue::from_static("Bearer secret"),
        );
        assert!(authorized(&AuthMode::InboundKey("secret".into()), &headers));

        let mut headers = HeaderMap::new();
        headers.insert("x-api-key", HeaderValue::from_static("secret"));
        assert!(authorized(&AuthMode::InboundKey("secret".into()), &headers));

        let headers = HeaderMap::new();
        assert!(!authorized(
            &AuthMode::InboundKey("secret".into()),
            &headers
        ));
    }
}
