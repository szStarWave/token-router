use axum::{
    Json,
    body::Body,
    http::{HeaderMap, Response, StatusCode},
    response::IntoResponse,
};
use tracing::info;

use crate::gateway::api::auth::require_gateway_api_key;
use crate::gateway::api::meta::{build_token_router_meta, token_router_meta_headers};
use crate::gateway::api::openai::ChatCompletionRequest;
use crate::gateway::api::routes::AppState;
use crate::gateway::experience::RequestOutcome;
use crate::gateway::error::{AppError, AppResult};

pub async fn chat_completions(
    state: AppState,
    headers: HeaderMap,
    agent_id: Option<String>,
    req: ChatCompletionRequest,
) -> AppResult<impl IntoResponse> {
    let stream = req.stream;
    state.stats.record_request(stream);

    if let Err(e) = require_gateway_api_key(&headers, &state.config().api_key) {
        state.stats.record_error(&e);
        return Err(e);
    }

    if let Err(e) = crate::gateway::routing::require_any_upstream(&state.config()) {
        state.stats.record_error(&e);
        return Err(e);
    }

    let routing = state.adaptive_tuner.refresh(
        &state.config(),
        state.experience.as_ref(),
        state.stats.as_ref(),
    );
    let decision = crate::gateway::routing::decide(
        &state.config(),
        &req,
        state.sessions.as_ref(),
        Some(state.experience.as_ref()),
        Some(state.multimodal.as_ref()),
        &routing,
        Some(state.edge_load.as_ref()),
        Some(state.classifier.as_ref()),
    );
    state.stats.record_decision(&decision);

    let conv_key = decision.conversation_key.clone();
    let assistant_failed = decision.assistant_failed_recent;

    info!(
        route = ?decision.route,
        step = ?decision.step_kind,
        difficulty = decision.difficulty,
        stream = stream,
        tok_in = decision.tokens_in_estimate,
        reasons = ?decision.reason_codes,
        "routing decision"
    );

    if stream {
        match state.upstream.stream(&req, &decision, agent_id.as_deref()).await {
            Ok((byte_stream, fallback)) => {
                let outcome = RequestOutcome::success(&decision, fallback);
                record_learning(&state, &decision, &conv_key, outcome, assistant_failed);
                let mut resp = Response::builder()
                    .status(StatusCode::OK)
                    .body(Body::from_stream(byte_stream))
                    .map_err(|e| AppError::Internal(e.into()))?;
                let headers = resp.headers_mut();
                headers.extend(token_router_meta_headers(&decision, fallback));
                apply_sse_headers(headers);
                Ok(resp.into_response())
            }
            Err(e) => {
                state.stats.record_error(&e);
                record_learning(
                    &state,
                    &decision,
                    &conv_key,
                    RequestOutcome::upstream_error(),
                    assistant_failed,
                );
                Err(e)
            }
        }
    } else {
        match state.upstream.complete(&req, &decision, agent_id.as_deref()).await {
            Ok(mut resp) => {
                let fallback = resp.token_router_meta.as_ref().is_some_and(|m| m.fallback);
                let outcome = RequestOutcome::success(&decision, fallback);
                record_learning(&state, &decision, &conv_key, outcome, assistant_failed);
                resp.token_router_meta = Some(build_token_router_meta(&decision, fallback, &resp));
                Ok(Json(resp).into_response())
            }
            Err(e) => {
                state.stats.record_error(&e);
                record_learning(
                    &state,
                    &decision,
                    &conv_key,
                    RequestOutcome::upstream_error(),
                    assistant_failed,
                );
                Err(e)
            }
        }
    }
}

fn record_learning(
    state: &AppState,
    decision: &crate::gateway::routing::RouteDecision,
    conv_key: &str,
    outcome: RequestOutcome,
    assistant_failed_signal: bool,
) {
    state
        .experience
        .record_outcome(decision.step_kind, outcome);
    if let Some(features) = decision.classifier_features.as_ref() {
        state.classifier.record(
            features,
            outcome,
            decision.route,
            decision.work_strategy,
        );
    }
    state.sessions.apply_outcome(
        conv_key,
        decision,
        outcome,
        state.config().cloud_sticky_ttl_secs,
        assistant_failed_signal,
    );
}

fn apply_sse_headers(headers: &mut HeaderMap) {
    use axum::http::header::{CACHE_CONTROL, CONNECTION, CONTENT_TYPE};
    headers.insert(
        CONTENT_TYPE,
        axum::http::HeaderValue::from_static("text/event-stream; charset=utf-8"),
    );
    headers.insert(CACHE_CONTROL, axum::http::HeaderValue::from_static("no-cache"));
    headers.insert(CONNECTION, axum::http::HeaderValue::from_static("keep-alive"));
}
