use std::sync::Arc;

use axum::{
    Json, Router,
    extract::State,
    routing::{delete, get, patch, post},
};
use serde::Serialize;

use crate::gateway::agent_usage::AgentCloudUsageStore;
use crate::gateway::api::auth_keys;
use crate::gateway::api::admin;
use crate::gateway::api::anthropic::anthropic_messages_handler;
use crate::gateway::api::chat::chat_completions;
use crate::gateway::api::responses::responses_handler;
use crate::gateway::api::setup;
use crate::gateway::classifier::ClassifierStore;
use crate::gateway::config_manager::ConfigManager;
use crate::gateway::routing_log::RoutingLogStore;
use crate::gateway::server::GatewayRuntime;
use crate::gateway::edge_load::EdgeInferenceTracker;
use crate::gateway::experience::ExperienceStore;
use crate::gateway::multimodal::MultimodalStore;
use crate::gateway::routing::{AdaptiveTuner, WordFreqStore};
use crate::gateway::session::SessionStore;
use crate::gateway::stats::GatewayStats;
use crate::gateway::upstream::UpstreamClient;

#[derive(Clone)]
pub struct AppState {
    pub config_mgr: Arc<ConfigManager>,
    pub sessions: Arc<SessionStore>,
    pub experience: Arc<ExperienceStore>,
    pub classifier: Arc<ClassifierStore>,
    pub multimodal: Arc<MultimodalStore>,
    pub upstream: UpstreamClient,
    pub runtime: Arc<GatewayRuntime>,
    pub stats: Arc<GatewayStats>,
    pub adaptive_tuner: Arc<AdaptiveTuner>,
    pub edge_load: Arc<EdgeInferenceTracker>,
    pub agent_usage: Arc<AgentCloudUsageStore>,
    pub wordfreq: Arc<WordFreqStore>,
    pub routing_logs: Arc<RoutingLogStore>,
}

impl AppState {
    pub fn config(&self) -> crate::gateway::config::AppConfig {
        self.config_mgr.get()
    }

    /// Apply hot-reloaded gateway settings to in-memory stores (experience, adaptive tuner).
    pub fn apply_runtime_config(&self, config: &crate::gateway::config::AppConfig) {
        self.experience
            .update_settings(config.experience.clone());
        self.classifier
            .update_settings(config.classifier.clone());
        self.wordfreq.update_settings(config.wordfreq.clone());
        self.adaptive_tuner.recompute(config, self.experience.as_ref(), &self.stats);
    }
}

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/health", get(health))
        .route("/setup", get(setup::setup_page))
        .route("/v1/admin/status", get(admin::status))
        .route("/v1/admin/stats", get(admin::stats))
        .route("/v1/admin/stats/timeline", get(admin::stats_timeline))
        .route("/v1/admin/logs", get(admin::logs))
        .route("/v1/admin/routing-logs", get(admin::routing_logs))
        .route("/v1/admin/setup", get(setup::setup_get).post(setup::setup_post))
        .route("/v1/admin/setup/init", post(setup::setup_init))
        .route("/v1/admin/auth-keys", get(auth_keys::auth_keys_list).post(auth_keys::auth_keys_create))
        .route(
            "/v1/admin/auth-keys/{id}",
            patch(auth_keys::auth_keys_update).delete(auth_keys::auth_keys_delete),
        )
        .route("/v1/admin/shutdown", post(admin::shutdown))
        .route("/v1/admin/restart", post(admin::restart))
        .route("/v1/chat/completions", post(chat_completions_handler))
        .route("/v1/responses", post(responses_handler))
        .route("/anthropic/v1/messages", post(anthropic_messages_handler))
        .route("/v1/messages", post(anthropic_messages_handler))
        .with_state(state)
}

#[derive(Serialize)]
struct HealthResponse {
    service: &'static str,
    status: &'static str,
    edge_configured: bool,
    cloud_configured: bool,
}

async fn health(State(state): State<AppState>) -> Json<HealthResponse> {
    let config = state.config();
    Json(HealthResponse {
        service: "token-router",
        status: "ok",
        edge_configured: config.edge_base_url.is_some(),
        cloud_configured: config.cloud_base_url.is_some(),
    })
}

pub fn extract_agent_id(headers: &axum::http::HeaderMap) -> Option<String> {
    headers
        .get("x-agent-id")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

async fn chat_completions_handler(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
    Json(req): Json<crate::gateway::api::openai::ChatCompletionRequest>,
) -> crate::gateway::error::AppResult<impl axum::response::IntoResponse> {
    let agent_id = extract_agent_id(&headers);
    chat_completions(state, headers, agent_id, req).await
}
