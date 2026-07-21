use axum::{
    Json,
    body::Body,
    http::{HeaderMap, Response, StatusCode},
    response::IntoResponse,
};
use crate::gateway::api::anthropic::{chat_json_to_anthropic, AnthropicSseTransform};
use crate::gateway::api::auth::require_gateway_api_key;
use crate::gateway::api::meta::{build_token_router_meta, log_request_error, log_route_decision, token_router_meta_headers};
use crate::gateway::routing::last_message_text;
use crate::gateway::api::openai::ChatCompletionRequest;
use crate::gateway::api::responses::{chat_response_to_responses, ResponsesSseTransform};
use crate::gateway::api::routes::AppState;
use crate::gateway::api::sse_transform::wrap_sse_transform;
use crate::gateway::experience::RequestOutcome;
use crate::gateway::served_outcome::{record_request_completion, CloudCacheSettings, ServedOutcome};
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
    }

    if let Err(e) = crate::gateway::routing::require_any_upstream(&state.config()) {
        state.stats.record_error(&e);
        if let Some(ref ctx) = auth_ctx {
            state.stats.record_error_for_auth_key(ctx, &e);
        }
        return Err(e);
    }

    let routing = state.adaptive_tuner.refresh(&state.config());
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
        Some(state.routing_logs.as_ref()),
        state.wordfreq.as_ref(),
    );
    // Count only after a route decision so requests_total == edge+cloud+cascade.
    state.stats.record_request(stream);
    state.stats.record_decision(&decision);
    let auth_key_ref = auth_ctx.as_ref();
    if let Some(ctx) = auth_key_ref {
        state.stats.record_request_for_auth_key(ctx, stream);
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
                record_learning(&state, &decision, &req, outcome, assistant_failed, None);

                let byte_stream = match &output {
                    ChatOutputFormat::OpenAi => byte_stream,
                    ChatOutputFormat::Responses => {
                        wrap_sse_transform(
                            byte_stream,
                            ResponsesSseTransform::with_codex_history(Some(
                                state.codex_history.clone(),
                            )),
                        )
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
                log_request_error(
                    Some(state.routing_logs.as_ref()),
                    decision.routing_log_id,
                    &e,
                );
                record_learning(
                    &state,
                    &decision,
                    &req,
                    RequestOutcome::upstream_error(),
                    assistant_failed,
                    None,
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
                let served = ServedOutcome::from_non_stream(&decision, &resp, fallback);
                let outcome = served.outcome;
                record_learning(
                    &state,
                    &decision,
                    &req,
                    outcome,
                    assistant_failed,
                    Some(&served),
                );

                match output {
                    ChatOutputFormat::OpenAi => {
                        resp.token_router_meta =
                            Some(build_token_router_meta(&decision, fallback, &resp));
                        Ok(Json(resp).into_response())
                    }
                    ChatOutputFormat::Responses => {
                        let original_json = serde_json::to_value(&resp).unwrap_or_default();
                        let body = chat_response_to_responses(&resp);
                        tracing::info!(
                            original = %serde_json::to_string(&original_json).unwrap_or_default(),
                            "responses raw response"
                        );
                        tracing::info!(
                            converted = %serde_json::to_string(&body).unwrap_or_default(),
                            "responses converted response"
                        );
                        state.codex_history.record_response(&body);
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
                log_request_error(
                    Some(state.routing_logs.as_ref()),
                    decision.routing_log_id,
                    &e,
                );
                record_learning(
                    &state,
                    &decision,
                    &req,
                    RequestOutcome::upstream_error(),
                    assistant_failed,
                    None,
                );
                Err(e)
            }
        }
    }
}

fn record_learning(
    state: &AppState,
    decision: &crate::gateway::routing::RouteDecision,
    req: &ChatCompletionRequest,
    outcome: RequestOutcome,
    assistant_failed_signal: bool,
    served: Option<&ServedOutcome>,
) {
    state
        .experience
        .record_outcome(decision.step_kind, outcome);
    if decision.consecutive_tool_error_streak > 0 {
        state.experience.record_tool_failure(
            decision.step_kind,
            decision.consecutive_tool_error_streak,
        );
    }
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
                decision.consecutive_tool_error_streak,
            );
        }
    }
    if let Some(served) = served {
        let settings = CloudCacheSettings::from_config(&state.config());
        record_request_completion(
            state.sessions.as_ref(),
            state.routing_logs.as_ref(),
            &settings,
            req,
            decision,
            served,
            assistant_failed_signal,
        );
    } else if outcome.upstream_error {
        let settings = CloudCacheSettings::from_config(&state.config());
        let served = ServedOutcome {
            outcome,
            served_tier: "edge".to_string(),
            served_model: String::new(),
            cached_tokens: 0,
            prompt_tokens: decision.tokens_in_estimate,
        };
        state.sessions.apply_served_outcome(
            &decision.conversation_key,
            decision,
            &served,
            &settings,
            assistant_failed_signal,
        );
    }
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
