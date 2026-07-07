use axum::{
    Json,
    body::Body,
    http::{HeaderMap, Response, StatusCode},
    response::IntoResponse,
};
use crate::gateway::api::anthropic::{chat_json_to_anthropic, AnthropicSseTransform};
use crate::gateway::api::auth::require_gateway_api_key;
use crate::gateway::api::meta::{build_token_router_meta, log_route_decision, token_router_meta_headers};
use crate::gateway::routing::last_message_text;
use crate::gateway::api::openai::ChatCompletionRequest;
use crate::gateway::api::responses::{chat_response_to_responses, ResponsesSseTransform};
use crate::gateway::api::routes::AppState;
use crate::gateway::api::sse_transform::wrap_sse_transform;
use crate::gateway::experience::RequestOutcome;
use crate::gateway::error::{AppError, AppResult};
use crate::gateway::stats::AuthKeyContext;

pub enum ChatOutputFormat {
    OpenAi,
    Responses,
    Anthropic { model: String },
}

pub async fn chat_completions(
    state: AppState,
    headers: HeaderMap,
    agent_id: Option<String>,
    req: ChatCompletionRequest,
) -> AppResult<impl IntoResponse> {
    chat_completions_core(state, headers, agent_id, req, ChatOutputFormat::OpenAi).await
}

pub async fn chat_completions_core(
    state: AppState,
    headers: HeaderMap,
    agent_id: Option<String>,
    req: ChatCompletionRequest,
    output: ChatOutputFormat,
) -> AppResult<impl IntoResponse> {
    let stream = req.stream;
    state.stats.record_request(stream);

    let auth_ctx = match require_gateway_api_key(&headers, &state.config().inbound_api_keys) {
        Ok(matched) => matched.and_then(|key_value| {
            state.config().auth_key_by_value.get(&key_value).map(|resolved| AuthKeyContext {
                id: resolved.id.clone(),
                name: resolved.name.clone(),
                key_preview: resolved.key_preview.clone(),
            })
        }),
        Err(e) => {
            state.stats.record_error(&e);
            return Err(e);
        }
    };

    if let Some(ref ctx) = auth_ctx {
        state.stats.touch_auth_key_last_used(ctx);
        state.stats.record_request_for_auth_key(ctx, stream);
    }

    if let Err(e) = crate::gateway::routing::require_any_upstream(&state.config()) {
        state.stats.record_error(&e);
        if let Some(ref ctx) = auth_ctx {
            state.stats.record_error_for_auth_key(ctx, &e);
        }
        return Err(e);
    }

    let routing = state.adaptive_tuner.refresh(
        &state.config(),
        state.experience.as_ref(),
        state.stats.as_ref(),
    );
    let edge_tps = state.stats.session_edge_tps();
    let mut decision = crate::gateway::routing::decide(
        &state.config(),
        &req,
        state.sessions.as_ref(),
        Some(state.experience.as_ref()),
        Some(state.multimodal.as_ref()),
        &routing,
        Some(state.edge_load.as_ref()),
        edge_tps,
        Some(state.classifier.as_ref()),
        state.wordfreq.as_ref(),
    );
    state.stats.record_decision(&decision);
    let auth_key_ref = auth_ctx.as_ref();
    if let Some(ctx) = auth_key_ref {
        state.stats.record_decision_for_auth_key(ctx, &decision);
    }

    let conv_key = decision.conversation_key.clone();
    let assistant_failed = decision.assistant_failed_recent;

    decision.routing_log_id = log_route_decision(
        Some(state.routing_logs.as_ref()),
        &decision,
        &req.model,
        &last_message_text(&req),
        stream,
        agent_id.as_deref(),
    );

    if stream {
        match state
            .upstream
            .stream(&req, &decision, agent_id.as_deref(), auth_key_ref)
            .await
        {
            Ok((byte_stream, fallback)) => {
                let outcome = RequestOutcome::success(&decision, fallback);
                record_learning(&state, &decision, &conv_key, outcome, assistant_failed);

                let byte_stream = match &output {
                    ChatOutputFormat::OpenAi => byte_stream,
                    ChatOutputFormat::Responses => {
                        wrap_sse_transform(byte_stream, ResponsesSseTransform::new())
                    }
                    ChatOutputFormat::Anthropic { model } => {
                        wrap_sse_transform(byte_stream, AnthropicSseTransform::new(model.clone()))
                    }
                };

                let mut resp = Response::builder()
                    .status(StatusCode::OK)
                    .body(Body::from_stream(byte_stream))
                    .map_err(|e| AppError::Internal(e.into()))?;
                let headers = resp.headers_mut();
                if matches!(output, ChatOutputFormat::OpenAi) {
                    headers.extend(token_router_meta_headers(&decision, fallback));
                }
                apply_sse_headers(headers);
                Ok(resp.into_response())
            }
            Err(e) => {
                state.stats.record_error(&e);
                if let Some(ctx) = auth_key_ref {
                    state.stats.record_error_for_auth_key(ctx, &e);
                }
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
        match state
            .upstream
            .complete(&req, &decision, agent_id.as_deref(), auth_key_ref)
            .await
        {
            Ok(mut resp) => {
                let fallback = resp.token_router_meta.as_ref().is_some_and(|m| m.fallback);
                let outcome = RequestOutcome::success(&decision, fallback);
                record_learning(&state, &decision, &conv_key, outcome, assistant_failed);

                match output {
                    ChatOutputFormat::OpenAi => {
                        resp.token_router_meta =
                            Some(build_token_router_meta(&decision, fallback, &resp));
                        Ok(Json(resp).into_response())
                    }
                    ChatOutputFormat::Responses => {
                        let body = chat_response_to_responses(&resp);
                        Ok(Json(body).into_response())
                    }
                    ChatOutputFormat::Anthropic { model } => {
                        let oai_json = serde_json::to_vec(&resp)
                            .map_err(|e| AppError::Internal(e.into()))?;
                        let body = chat_json_to_anthropic(&oai_json, &model);
                        Ok(Json(body).into_response())
                    }
                }
            }
            Err(e) => {
                state.stats.record_error(&e);
                if let Some(ctx) = auth_key_ref {
                    state.stats.record_error_for_auth_key(ctx, &e);
                }
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
    state.wordfreq.reinforce_from_outcome(
        &decision.lexical_learn,
        decision.step_kind,
        outcome,
    );
    if let Some(features) = decision.classifier_features.as_ref() {
        let skip_classifier =
            decision.casual_quality_fallback && outcome.cascade_fallback;
        if !skip_classifier {
            state.classifier.record(
                features,
                outcome,
                decision.route,
                decision.work_strategy,
            );
        }
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
    headers.insert(
        CACHE_CONTROL,
        axum::http::HeaderValue::from_static("no-cache"),
    );
    headers.insert(
        CONNECTION,
        axum::http::HeaderValue::from_static("keep-alive"),
    );
}
