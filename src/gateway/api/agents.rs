use axum::{
    Json,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
};
use serde::Deserialize;
use serde_json::json;

use crate::agent_setup::{
    self, configure_named_agent, is_known_host_agent, list_host_agents,
};
use crate::gateway::api::routes::AppState;
use crate::gateway::api::setup::require_admin;
use crate::wsl_setup;

#[derive(Debug, Deserialize)]
pub struct ConfigureAgentBody {
    #[serde(default)]
    pub api_key: Option<String>,
    #[serde(default)]
    pub context_window: Option<u64>,
}

fn map_agent_err(err: String) -> Response {
    if err.starts_with(agent_setup::ERR_AGENT_NOT_INITIALIZED)
        || err.starts_with("agent_not_initialized:")
    {
        let parts: Vec<&str> = err.splitn(3, ':').collect();
        let (agent, config_path) = if parts.len() >= 3 {
            (parts[1].to_string(), parts[2].to_string())
        } else {
            (String::new(), String::new())
        };
        return (
            StatusCode::CONFLICT,
            Json(json!({
                "error": "agent_not_initialized",
                "agent": agent,
                "config_path": config_path,
                "message": err,
            })),
        )
            .into_response();
    }
    if err.starts_with("unknown agent:") {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": err })),
        )
            .into_response();
    }
    (
        StatusCode::BAD_REQUEST,
        Json(json!({ "error": err })),
    )
        .into_response()
}

pub async fn agents_list(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Response {
    if let Some(resp) = require_admin(&state, &headers) {
        return resp;
    }
    match list_host_agents() {
        Ok(agents) => Json(json!({ "agents": agents })).into_response(),
        Err(err) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": err })),
        )
            .into_response(),
    }
}

pub async fn agents_get(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(agent): Path<String>,
) -> Response {
    if let Some(resp) = require_admin(&state, &headers) {
        return resp;
    }
    if !is_known_host_agent(agent.trim()) {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": format!("unknown agent: {agent}") })),
        )
            .into_response();
    }
    match agent_setup::agent_status(agent.trim()) {
        Ok(status) => Json(status).into_response(),
        Err(err) => map_agent_err(err),
    }
}

pub async fn agents_configure(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(agent): Path<String>,
    Json(body): Json<ConfigureAgentBody>,
) -> Response {
    if let Some(resp) = require_admin(&state, &headers) {
        return resp;
    }
    let agent = agent.trim().to_string();
    if !is_known_host_agent(&agent) {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": format!("unknown agent: {agent}") })),
        )
            .into_response();
    }
    // Use the running gateway home + default inbound key (not ~/.token-router).
    let config = state.config();
    let data_dir = config.data_dir.clone();
    let api_key = body
        .api_key
        .map(|k| k.trim().to_string())
        .filter(|k| !k.is_empty())
        .or_else(|| config.default_api_key.clone());
    let context_window = body.context_window;
    match tokio::task::spawn_blocking(move || {
        crate::config::paths::set_runtime_app_home(data_dir);
        configure_named_agent(&agent, api_key, context_window)
    })
    .await
    {
        Ok(Ok(result)) => Json(result).into_response(),
        Ok(Err(err)) => map_agent_err(err),
        Err(err) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": format!("configure task failed: {err}") })),
        )
            .into_response(),
    }
}

pub async fn wsl_detect(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Response {
    if let Some(resp) = require_admin(&state, &headers) {
        return resp;
    }
    match tokio::task::spawn_blocking(wsl_setup::detect).await {
        Ok(Ok(result)) => Json(result).into_response(),
        Ok(Err(err)) => (
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": err })),
        )
            .into_response(),
        Err(err) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": format!("WSL detect task failed: {err}") })),
        )
            .into_response(),
    }
}

pub async fn wsl_configure(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((distro, agent)): Path<(String, String)>,
    Json(body): Json<ConfigureAgentBody>,
) -> Response {
    if let Some(resp) = require_admin(&state, &headers) {
        return resp;
    }
    #[cfg(not(windows))]
    {
        let _ = (state, distro, agent, body);
        return (
            StatusCode::NOT_IMPLEMENTED,
            Json(json!({ "error": "WSL agent configure is only available on Windows" })),
        )
            .into_response();
    }
    #[cfg(windows)]
    {
        let config = state.config();
        let data_dir = config.data_dir.clone();
        let api_key = body
            .api_key
            .map(|k| k.trim().to_string())
            .filter(|k| !k.is_empty())
            .or_else(|| config.default_api_key.clone());
        let distro = distro.trim().to_string();
        let agent = agent.trim().to_string();
        match tokio::task::spawn_blocking(move || {
            crate::config::paths::set_runtime_app_home(data_dir);
            wsl_setup::configure_agent(&distro, &agent, api_key)
        })
        .await
        {
            Ok(Ok(result)) => Json(result).into_response(),
            Ok(Err(err)) => map_agent_err(err),
            Err(err) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": format!("WSL configure task failed: {err}") })),
            )
                .into_response(),
        }
    }
}
