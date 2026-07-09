use std::path::Path;

use serde_json::{json, Value};

use crate::gateway::api::models::{build_models, DEFAULT_CLOUD_MAX_CONTEXT_LENGTH, ModelObject};
use crate::gateway::config::AppConfig;

pub const TOKEN_ROUTER_CODEX_MODEL_CATALOG_FILENAME: &str = "token-router-model-catalog.json";
pub const CODEX_MODELS_CACHE_FILENAME: &str = "models_cache.json";
pub const CODEX_CATALOG_MODEL_ID: &str = "token-router";
pub const CODEX_CATALOG_PROVIDER_DISPLAY_NAME: &str = "TokenRouter";
pub const CODEX_CATALOG_TIER_ID: &str = "auto";
pub const CODEX_CATALOG_TIER_DISPLAY_NAME: &str = "Auto";
const ROUTER_AUTO_MODEL_ID: &str = "auto";

pub fn is_router_auto_model(model: &str) -> bool {
    let model = model.trim();
    model.is_empty()
        || model.eq_ignore_ascii_case(ROUTER_AUTO_MODEL_ID)
        || model.eq_ignore_ascii_case(CODEX_CATALOG_MODEL_ID)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CodexCatalogVisibility {
    List,
    Hide,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodexCatalogModelSpec {
    pub model: String,
    pub display_name: String,
    pub context_window: u64,
    pub visibility: CodexCatalogVisibility,
}

fn auto_model_context_window(models: &[ModelObject]) -> u64 {
    models
        .iter()
        .find(|model| model.id.eq_ignore_ascii_case(ROUTER_AUTO_MODEL_ID))
        .map(|model| model.max_context_length as u64)
        .unwrap_or(DEFAULT_CLOUD_MAX_CONTEXT_LENGTH as u64)
}

pub fn codex_catalog_specs_from_config(config: &AppConfig, agent_id: Option<&str>) -> Vec<CodexCatalogModelSpec> {
    let models = build_models(config, agent_id);
    if models.is_empty() {
        return Vec::new();
    }

    let context_window = auto_model_context_window(&models);
    vec![
        CodexCatalogModelSpec {
            model: CODEX_CATALOG_MODEL_ID.to_string(),
            display_name: CODEX_CATALOG_PROVIDER_DISPLAY_NAME.to_string(),
            context_window,
            visibility: CodexCatalogVisibility::Hide,
        },
        CodexCatalogModelSpec {
            model: ROUTER_AUTO_MODEL_ID.to_string(),
            display_name: CODEX_CATALOG_TIER_DISPLAY_NAME.to_string(),
            context_window,
            visibility: CodexCatalogVisibility::List,
        },
    ]
}

fn load_codex_native_responses_template() -> Value {
    serde_json::from_str(include_str!("resources/codex_native_responses_template.json"))
        .expect("bundled codex_native_responses_template.json must be valid JSON")
}

fn codex_catalog_model_entry(template: &Value, spec: &CodexCatalogModelSpec, priority: usize) -> Value {
    let mut entry = template.clone();
    let Some(entry_obj) = entry.as_object_mut() else {
        return json!({});
    };

    entry_obj.insert("slug".to_string(), json!(spec.model));
    entry_obj.insert("model".to_string(), json!(spec.model));
    entry_obj.insert("display_name".to_string(), json!(spec.display_name));
    entry_obj.insert("description".to_string(), json!(spec.display_name));
    entry_obj.insert(
        "visibility".to_string(),
        json!(match spec.visibility {
            CodexCatalogVisibility::List => "list",
            CodexCatalogVisibility::Hide => "hide",
        }),
    );
    entry_obj.insert("context_window".to_string(), json!(spec.context_window));
    entry_obj.insert("max_context_window".to_string(), json!(spec.context_window));
    entry_obj.insert("priority".to_string(), json!(1000 + priority));
    entry_obj.insert("additional_speed_tiers".to_string(), json!([]));
    entry_obj.insert("service_tiers".to_string(), json!([]));
    entry_obj.insert("availability_nux".to_string(), Value::Null);
    entry_obj.insert("upgrade".to_string(), Value::Null);

    for key in [
        "apply_patch_tool_type",
        "web_search_tool_type",
        "tools",
        "model_messages",
    ] {
        entry_obj.remove(key);
    }
    entry_obj.insert("shell_type".to_string(), json!("shell_command"));

    entry
}

pub fn build_codex_model_catalog(specs: &[CodexCatalogModelSpec]) -> Value {
    let template = load_codex_native_responses_template();
    let entries: Vec<Value> = specs
        .iter()
        .enumerate()
        .map(|(index, spec)| codex_catalog_model_entry(&template, spec, index))
        .collect();
    json!({ "models": entries })
}

pub fn build_codex_model_catalog_from_config(config: &AppConfig, agent_id: Option<&str>) -> Value {
    let specs = codex_catalog_specs_from_config(config, agent_id);
    build_codex_model_catalog(&specs)
}

pub fn catalog_path_in_codex_dir(home: &Path) -> std::path::PathBuf {
    home.join(".codex").join(TOKEN_ROUTER_CODEX_MODEL_CATALOG_FILENAME)
}

pub fn models_cache_path_in_codex_dir(home: &Path) -> std::path::PathBuf {
    home.join(".codex").join(CODEX_MODELS_CACHE_FILENAME)
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
    fn codex_catalog_specs_expose_hidden_parent_and_visible_auto() {
        let config = test_config(true, true, Some("edge-model"), Some("cloud-model"));
        let specs = codex_catalog_specs_from_config(&config, None);
        assert_eq!(specs.len(), 2);
        assert_eq!(specs[0].model, CODEX_CATALOG_MODEL_ID);
        assert_eq!(specs[0].display_name, CODEX_CATALOG_PROVIDER_DISPLAY_NAME);
        assert_eq!(specs[0].visibility, CodexCatalogVisibility::Hide);
        assert_eq!(specs[1].model, ROUTER_AUTO_MODEL_ID);
        assert_eq!(specs[1].display_name, CODEX_CATALOG_TIER_DISPLAY_NAME);
        assert_eq!(specs[1].visibility, CodexCatalogVisibility::List);
    }

    #[test]
    fn is_router_auto_model_matches_codex_catalog_and_legacy_auto() {
        assert!(is_router_auto_model("auto"));
        assert!(is_router_auto_model("token-router"));
        assert!(!is_router_auto_model("deepseek-v4-flash"));
    }

    #[test]
    fn codex_catalog_entry_has_required_fields() {
        let spec = CodexCatalogModelSpec {
            model: ROUTER_AUTO_MODEL_ID.into(),
            display_name: CODEX_CATALOG_TIER_DISPLAY_NAME.into(),
            context_window: 1_000_000,
            visibility: CodexCatalogVisibility::List,
        };
        let catalog = build_codex_model_catalog(&[spec]);
        let entry = &catalog["models"][0];
        assert_eq!(entry["slug"], ROUTER_AUTO_MODEL_ID);
        assert_eq!(entry["model"], ROUTER_AUTO_MODEL_ID);
        assert_eq!(entry["display_name"], CODEX_CATALOG_TIER_DISPLAY_NAME);
        assert_eq!(entry["visibility"], "list");
        assert_eq!(entry["additional_speed_tiers"], json!([]));
        assert_eq!(entry["service_tiers"], json!([]));
        assert_eq!(entry["context_window"], 1_000_000);
        assert!(entry.get("base_instructions").is_some());
        assert_eq!(entry["shell_type"], "shell_command");
    }
}
