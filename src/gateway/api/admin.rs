use axum::{
    Json,
    extract::{Query, State},
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
};
use serde::{Deserialize, Serialize};

use crate::daemon_ctl;
use crate::gateway::api::routes::AppState;
use crate::gateway::stats::AgentBudgetSnapshot;
use crate::gateway::routing::Profile;

#[derive(Serialize)]
pub struct GatewayStatus {
    pub service: &'static str,
    pub status: &'static str,
    pub version: &'static str,
    pub listen: String,
    pub pid: u32,
    pub uptime_secs: u64,
    pub edge_configured: bool,
    pub cloud_configured: bool,
    pub default_profile: String,
    pub pid_file: String,
    pub data_dir: String,
}

#[derive(Debug, Deserialize)]
pub struct StatsQuery {
    /// `session` (default) = current gateway process; `global` = cumulative persisted totals.
    #[serde(default)]
    pub scope: Option<String>,
}

pub async fn stats(
    State(state): State<AppState>,
    Query(query): Query<StatsQuery>,
) -> Result<Json<crate::gateway::stats::StatsSnapshot>, (StatusCode, Json<serde_json::Value>)> {
    let scope = match query.scope.as_deref() {
        None | Some("session") => crate::gateway::stats::StatsScope::Session,
        Some("global") => crate::gateway::stats::StatsScope::Global,
        Some(other) => {
            return Err((
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({
                    "error": format!("invalid stats scope `{other}` (use session or global)")
                })),
            ));
        }
    };
    let _ = state.stats.flush_if_dirty();
    let _ = state.experience.flush_if_dirty();
    let _ = state.classifier.flush_if_dirty();
    let _ = state.sessions.flush_if_dirty();
    let uptime = state.stats.session_uptime_secs();
    let experience = Some(state.experience.snapshot());
    let classifier = Some(state.classifier.snapshot());
    let effective_routing = Some(state.adaptive_tuner.snapshot());
    let config = state.config();
    let usage = state.agent_usage.snapshot();
    let agent_budgets = AgentBudgetSnapshot::from_config_and_usage(&config.agents, &usage);
    let agent_budgets = if agent_budgets.is_empty() { None } else { Some(agent_budgets) };
    Ok(Json(state.stats.snapshot(
        scope,
        uptime,
        experience,
        classifier,
        effective_routing,
        agent_budgets,
    )))
}

pub async fn status(State(state): State<AppState>) -> Json<GatewayStatus> {
    let config = state.config();
    Json(GatewayStatus {
        service: "token-router",
        status: "running",
        version: env!("CARGO_PKG_VERSION"),
        listen: config.listen_addr.clone(),
        pid: std::process::id(),
        uptime_secs: state.runtime.started_at.elapsed().as_secs(),
        edge_configured: config.edge_base_url.is_some(),
        cloud_configured: config.cloud_base_url.is_some(),
        default_profile: profile_name(config.default_profile).to_string(),
        pid_file: config.pid_file.display().to_string(),
        data_dir: config.data_dir.display().to_string(),
    })
}

pub async fn shutdown(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> impl IntoResponse {
    match check_admin_token(&headers, state.config().admin_token.as_ref()) {
        Ok(()) => {}
        Err(resp) => return resp,
    }

    flush_before_exit(&state);
    state.runtime.trigger_shutdown();
    (
        StatusCode::OK,
        Json(serde_json::json!({"status": "shutting_down"})),
    )
        .into_response()
}

pub async fn restart(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> impl IntoResponse {
    match check_admin_token(&headers, state.config().admin_token.as_ref()) {
        Ok(()) => {}
        Err(resp) => return resp,
    }

    let config = state.config();
    let old_pid = std::process::id();
    if let Err(e) = daemon_ctl::schedule_daemon_restart(&config.config_path, old_pid) {
        tracing::error!(error = %e, "schedule daemon restart failed");
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": e.to_string()})),
        )
            .into_response();
    }

    flush_before_exit(&state);
    state.runtime.trigger_shutdown();
    (
        StatusCode::OK,
        Json(serde_json::json!({"status": "restarting"})),
    )
        .into_response()
}

fn check_admin_token(
    headers: &HeaderMap,
    expected: Option<&String>,
) -> Result<(), axum::response::Response> {
    let Some(expected) = expected else {
        return Ok(());
    };
    let provided = headers
        .get("x-token-router-admin-token")
        .and_then(|v| v.to_str().ok());
    if provided == Some(expected.as_str()) {
        Ok(())
    } else {
        Err((
            StatusCode::UNAUTHORIZED,
            Json(serde_json::json!({"error": "invalid admin token"})),
        )
            .into_response())
    }
}

fn flush_before_exit(state: &AppState) {
    if let Err(e) = state.stats.flush() {
        tracing::warn!(error = %e, "stats flush before exit failed");
    }
    if let Err(e) = state.experience.flush() {
        tracing::warn!(error = %e, "experience flush before exit failed");
    }
    if let Err(e) = state.classifier.flush() {
        tracing::warn!(error = %e, "classifier flush before exit failed");
    }
    if let Err(e) = state.sessions.flush() {
        tracing::warn!(error = %e, "session flush before exit failed");
    }
}

fn profile_name(p: Profile) -> &'static str {
    match p {
        Profile::Economy => "economy",
        Profile::Balanced => "balanced",
        Profile::Premium => "premium",
        Profile::Privacy => "privacy",
    }
}
