use axum::{
    Json,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
};

use crate::config::auth_keys::{CreateGatewayAuthKeyRequest, UpdateGatewayAuthKeyRequest};
use crate::gateway::api::routes::AppState;
use crate::gateway::api::setup::require_admin;

pub async fn auth_keys_list(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Response {
    if let Some(resp) = require_admin(&state, &headers) {
        return resp;
    }
    match state.config_mgr.list_auth_keys() {
        Ok(keys) => Json(keys).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

pub async fn auth_keys_create(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<CreateGatewayAuthKeyRequest>,
) -> Response {
    if let Some(resp) = require_admin(&state, &headers) {
        return resp;
    }
    match state.config_mgr.create_auth_key(&body.name) {
        Ok((created, _config)) => {
            state.stats.upsert_auth_key_meta(
                &created.key.id,
                &created.key.name,
                &created.key.key_preview,
            );
            Json(created).into_response()
        }
        Err(e) => (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

pub async fn auth_keys_update(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(body): Json<UpdateGatewayAuthKeyRequest>,
) -> Response {
    if let Some(resp) = require_admin(&state, &headers) {
        return resp;
    }
    match state.config_mgr.update_auth_key_name(&id, &body.name) {
        Ok((updated, _config)) => {
            state.stats.update_auth_key_meta_name(&id, &updated.name);
            Json(updated).into_response()
        }
        Err(e) => (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

pub async fn auth_keys_delete(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Response {
    if let Some(resp) = require_admin(&state, &headers) {
        return resp;
    }
    match state.config_mgr.delete_auth_key(&id) {
        Ok(_config) => Json(serde_json::json!({"ok": true})).into_response(),
        Err(e) => (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}
