use std::collections::HashSet;

use axum::{
    Json,
    extract::{Path, State},
    http::HeaderMap,
};
use serde::Serialize;
use serde_json::Value;

use crate::gateway::api::auth::require_gateway_api_key;
use crate::gateway::api::codex_catalog::build_codex_model_catalog_from_config;
use crate::gateway::api::routes::{extract_agent_id, AppState};
use crate::gateway::config::AppConfig;
use crate::gateway::error::{AppError, AppResult};

/// Default max context for cloud / Flowy Auto models (1M tokens).
pub const DEFAULT_CLOUD_MAX_CONTEXT_LENGTH: u32 = 1_000_000;

/// Fixed `created` epoch for synthetic router models (OpenAI list shape).
const MODEL_CREATED_EPOCH: i64 = 1_704_067_200;

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ModelObject {
    pub id: String,
    pub object: &'static str,
    pub created: i64,
    pub owned_by: String,
    pub max_context_length: u32,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ModelsListResponse {
    pub object: &'static str,
    pub data: Vec<ModelObject>,
    /// Codex `/v1/models` probe expects this catalog-shaped field (cc-switch compatible).
    pub models: Vec<Value>,
}

pub async fn list_models(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> AppResult<Json<ModelsListResponse>> {
    require_gateway_api_key(&headers, &state.config().inbound_api_keys)?;
    let agent_id = extract_agent_id(&headers);
    let data = build_models(&state.config(), agent_id.as_deref());
    let catalog = build_codex_model_catalog_from_config(&state.config(), agent_id.as_deref());
    let models = catalog
        .get("models")
        .and_then(|value| value.as_array())
        .cloned()
        .unwrap_or_default();
    Ok(Json(ModelsListResponse {
        object: "list",
        data,
        models,
    }))
}

pub async fn get_model(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(model_id): Path<String>,
) -> AppResult<Json<ModelObject>> {
    require_gateway_api_key(&headers, &state.config().inbound_api_keys)?;
    let agent_id = extract_agent_id(&headers);
    let data = build_models(&state.config(), agent_id.as_deref());
    data.into_iter()
        .find(|m| m.id == model_id)
        .map(Json)
        .ok_or_else(|| AppError::NotFound(format!("The model `{model_id}` does not exist")))
}

pub fn build_models(config: &AppConfig, agent_id: Option<&str>) -> Vec<ModelObject> {
    let edge = config.resolve_upstream(agent_id, "edge");
    let cloud = config.resolve_upstream(agent_id, "cloud");

    let edge_on = edge
        .base_url
        .as_deref()
        .is_some_and(|url| !url.trim().is_empty());
    let cloud_on = cloud
        .base_url
        .as_deref()
        .is_some_and(|url| !url.trim().is_empty());

    let edge_ctx = config.ctx_edge_max_tokens;
    let cloud_ctx = DEFAULT_CLOUD_MAX_CONTEXT_LENGTH;

    let mut out = Vec::new();
    let mut seen = HashSet::new();

    if edge_on || cloud_on {
        let auto_ctx = match (edge_on, cloud_on) {
            (true, true) => edge_ctx.max(cloud_ctx),
            (true, false) => edge_ctx,
            (false, true) => cloud_ctx,
            (false, false) => 0,
        };
        push_model(
            &mut out,
            &mut seen,
            model_object("auto", auto_ctx, "token-router"),
        );
    }

    if edge_on {
        if let Some(id) = explicit_model_id(edge.model.as_deref()) {
            push_model(
                &mut out,
                &mut seen,
                model_object(&id, edge_ctx, "edge"),
            );
        }
    }

    if cloud_on {
        if let Some(id) = explicit_model_id(cloud.model.as_deref()) {
            push_model(
                &mut out,
                &mut seen,
                model_object(&id, cloud_ctx, "cloud"),
            );
        }
    }

    out
}

fn model_object(id: &str, max_context_length: u32, owned_by: &str) -> ModelObject {
    ModelObject {
        id: id.to_string(),
        object: "model",
        created: MODEL_CREATED_EPOCH,
        owned_by: owned_by.to_string(),
        max_context_length,
    }
}

fn explicit_model_id(model: Option<&str>) -> Option<String> {
    let model = model?.trim();
    if model.is_empty() || model.eq_ignore_ascii_case("auto") {
        None
    } else {
        Some(model.to_string())
    }
}

fn push_model(out: &mut Vec<ModelObject>, seen: &mut HashSet<String>, model: ModelObject) {
    if seen.insert(model.id.clone()) {
        out.push(model);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::ConfigFile;
    use crate::gateway::config::AppConfig;

    fn test_config(edge: bool, cloud: bool, edge_model: Option<&str>, cloud_model: Option<&str>) -> AppConfig {
        let mut file = ConfigFile::default();
        file.gateway.ctx_edge_max_tokens = 131_072;
        if edge {
            file.upstream.edge = Some(crate::config::UpstreamEndpoint {
                base_url: "http://127.0.0.1:8080/v1".into(),
                api_key: None,
                model: edge_model.map(str::to_string),
            });
        }
        if cloud {
            file.upstream.cloud = Some(crate::config::UpstreamEndpoint {
                base_url: "https://api.flowy.test/claw/v1".into(),
                api_key: Some("token".into()),
                model: cloud_model.map(str::to_string),
            });
        }
        AppConfig::from_file(file, std::env::temp_dir()).unwrap()
    }

    #[test]
    fn lists_auto_when_edge_and_cloud_configured() {
        let cfg = test_config(true, true, None, None);
        let models = build_models(&cfg, None);
        assert_eq!(models.len(), 1);
        assert_eq!(models[0].id, "auto");
        assert_eq!(models[0].max_context_length, DEFAULT_CLOUD_MAX_CONTEXT_LENGTH);
        assert_eq!(models[0].owned_by, "token-router");
    }

    #[test]
    fn lists_explicit_edge_and_cloud_models() {
        let cfg = test_config(true, true, Some("qwen-local"), Some("gpt-4o"));
        let models = build_models(&cfg, None);
        assert_eq!(models.len(), 3);
        assert_eq!(models[0].id, "auto");
        assert!(models.iter().any(|m| m.id == "qwen-local" && m.max_context_length == 131_072));
        assert!(models.iter().any(|m| m.id == "gpt-4o" && m.max_context_length == DEFAULT_CLOUD_MAX_CONTEXT_LENGTH));
    }

    #[test]
    fn edge_only_auto_uses_edge_context() {
        let cfg = test_config(true, false, None, None);
        let models = build_models(&cfg, None);
        assert_eq!(models.len(), 1);
        assert_eq!(models[0].id, "auto");
        assert_eq!(models[0].max_context_length, 131_072);
    }

    #[test]
    fn empty_when_no_upstream() {
        let cfg = test_config(false, false, None, None);
        assert!(build_models(&cfg, None).is_empty());
    }

    #[test]
    fn deduplicates_same_model_id() {
        let cfg = test_config(true, false, Some("auto"), None);
        let models = build_models(&cfg, None);
        assert_eq!(models.len(), 1);
        assert_eq!(models[0].id, "auto");
    }

    #[test]
    fn codex_catalog_only_lists_token_router_model() {
        use crate::gateway::api::codex_catalog::{
            build_codex_model_catalog_from_config, CODEX_CATALOG_PROVIDER_DISPLAY_NAME,
            CODEX_CATALOG_MODEL_ID,
        };

        let cfg = test_config(true, true, Some("deepseek-v4-flash"), None);
        let catalog = build_codex_model_catalog_from_config(&cfg, None);
        let entries = catalog["models"].as_array().unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0]["slug"], CODEX_CATALOG_MODEL_ID);
        assert_eq!(entries[0]["display_name"], CODEX_CATALOG_PROVIDER_DISPLAY_NAME);
        assert_eq!(entries[0]["visibility"], "list");
        assert_eq!(entries[0]["service_tiers"], serde_json::json!([]));
    }
}
