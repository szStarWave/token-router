use std::collections::HashSet;
use std::fs;
use std::path::Path;

use serde_json::{json, Value};

use crate::gateway::api::models::{build_models, DEFAULT_CLOUD_MAX_CONTEXT_LENGTH, ModelObject};
use crate::gateway::config::AppConfig;

pub const TOKEN_ROUTER_CODEX_MODEL_CATALOG_FILENAME: &str = "token-router-model-catalog.json";
pub const CODEX_MODELS_CACHE_FILENAME: &str = "models_cache.json";
pub const CODEX_CATALOG_MODEL_ID: &str = "token-router";
pub const CODEX_CATALOG_PROVIDER_DISPLAY_NAME: &str = "TokenRouter";
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
    vec![CodexCatalogModelSpec {
        model: CODEX_CATALOG_MODEL_ID.to_string(),
        display_name: CODEX_CATALOG_PROVIDER_DISPLAY_NAME.to_string(),
        context_window,
        visibility: CodexCatalogVisibility::List,
    }]
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

fn strip_utf8_bom(text: &str) -> &str {
    text.strip_prefix('\u{FEFF}').unwrap_or(text)
}

fn is_router_model_slug(slug: &str) -> bool {
    slug.eq_ignore_ascii_case(CODEX_CATALOG_MODEL_ID)
        || slug.eq_ignore_ascii_case(ROUTER_AUTO_MODEL_ID)
}

fn is_native_codex_model_slug(slug: &str) -> bool {
    slug.starts_with("gpt-")
        || slug.starts_with("codex-")
        || slug.starts_with("o1")
        || slug.starts_with("o3")
        || slug.starts_with("o4")
}

fn cache_has_native_codex_models(cache: &Value) -> bool {
    cache
        .get("models")
        .and_then(|value| value.as_array())
        .map(|models| {
            models.iter().any(|entry| {
                entry
                    .get("slug")
                    .and_then(|slug| slug.as_str())
                    .is_some_and(is_native_codex_model_slug)
            })
        })
        .unwrap_or(false)
}

fn cache_only_has_router_models(cache: &Value) -> bool {
    let Some(models) = cache.get("models").and_then(|value| value.as_array()) else {
        return false;
    };
    !models.is_empty()
        && models.iter().all(|entry| {
            entry
                .get("slug")
                .and_then(|slug| slug.as_str())
                .is_some_and(is_router_model_slug)
        })
}

fn read_json_value(path: &Path) -> Result<Value, String> {
    let text = fs::read_to_string(path).map_err(|e| e.to_string())?;
    let text = strip_utf8_bom(text.trim_start());
    if text.is_empty() {
        return Err(format!("empty JSON file: {}", path.display()));
    }
    serde_json::from_str(text).map_err(|e| e.to_string())
}

/// Upsert TokenRouter catalog entries into Codex's `models_cache.json` without
/// removing built-in GPT models. Codex Desktop reads the picker list from this
/// cache; `model_catalog_json` alone is not enough for the UI.
///
/// If Codex has not populated native models yet, this is a no-op so we do not
/// create a router-only cache that blocks GPT refetch. A router-only cache is
/// removed when detected.
pub fn merge_router_models_into_models_cache(home: &Path, router_catalog: &Value) -> Result<(), String> {
    let router_models = router_catalog
        .get("models")
        .and_then(|value| value.as_array())
        .ok_or_else(|| "router catalog missing models array".to_string())?;
    if router_models.is_empty() {
        return Ok(());
    }

    let router_slugs: HashSet<String> = router_models
        .iter()
        .filter_map(|entry| {
            entry
                .get("slug")
                .and_then(|slug| slug.as_str())
                .map(str::to_string)
        })
        .collect();

    let cache_path = models_cache_path_in_codex_dir(home);
    if !cache_path.is_file() {
        return Ok(());
    }

    let cache = match read_json_value(&cache_path) {
        Ok(cache) => cache,
        Err(_) => {
            let _ = fs::remove_file(&cache_path);
            return Ok(());
        }
    };

    if cache_only_has_router_models(&cache) {
        let _ = fs::remove_file(&cache_path);
        return Ok(());
    }

    if !cache_has_native_codex_models(&cache) {
        return Ok(());
    }

    let mut cache = cache;
    if cache.get("models").and_then(|value| value.as_array()).is_none() {
        cache["models"] = json!([]);
    }

    let cache_models = cache["models"].as_array_mut().unwrap();
    cache_models.retain(|entry| {
        entry
            .get("slug")
            .and_then(|slug| slug.as_str())
            .is_none_or(|slug| !router_slugs.contains(slug))
    });
    cache_models.extend(router_models.iter().cloned());

    if let Some(parent) = cache_path.parent() {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let text = serde_json::to_string_pretty(&cache).map_err(|e| e.to_string())?;
    fs::write(&cache_path, text).map_err(|e| e.to_string())?;
    Ok(())
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
    fn codex_catalog_specs_expose_token_router_model() {
        let config = test_config(true, true, Some("edge-model"), Some("cloud-model"));
        let specs = codex_catalog_specs_from_config(&config, None);
        assert_eq!(specs.len(), 1);
        assert_eq!(specs[0].model, CODEX_CATALOG_MODEL_ID);
        assert_eq!(specs[0].display_name, CODEX_CATALOG_PROVIDER_DISPLAY_NAME);
        assert_eq!(specs[0].visibility, CodexCatalogVisibility::List);
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
            model: CODEX_CATALOG_MODEL_ID.into(),
            display_name: CODEX_CATALOG_PROVIDER_DISPLAY_NAME.into(),
            context_window: 1_000_000,
            visibility: CodexCatalogVisibility::List,
        };
        let catalog = build_codex_model_catalog(&[spec]);
        let entry = &catalog["models"][0];
        assert_eq!(entry["slug"], CODEX_CATALOG_MODEL_ID);
        assert_eq!(entry["model"], CODEX_CATALOG_MODEL_ID);
        assert_eq!(entry["display_name"], CODEX_CATALOG_PROVIDER_DISPLAY_NAME);
        assert_eq!(entry["visibility"], "list");
        assert_eq!(entry["additional_speed_tiers"], json!([]));
        assert_eq!(entry["service_tiers"], json!([]));
        assert_eq!(entry["context_window"], 1_000_000);
        assert!(entry.get("base_instructions").is_some());
        assert_eq!(entry["shell_type"], "shell_command");
    }

    #[test]
    fn merge_router_models_into_models_cache_preserves_native_models() {
        let dir = std::env::temp_dir().join(format!("codex-cache-merge-{}", uuid::Uuid::new_v4()));
        let codex_dir = dir.join(".codex");
        fs::create_dir_all(&codex_dir).unwrap();
        fs::write(
            codex_dir.join(CODEX_MODELS_CACHE_FILENAME),
            r#"{"models":[{"slug":"gpt-5.5","display_name":"GPT-5.5"}]}"#,
        )
        .unwrap();

        let specs = codex_catalog_specs_from_config(
            &test_config(true, true, Some("edge-model"), Some("cloud-model")),
            None,
        );
        let router_catalog = build_codex_model_catalog(&specs);
        merge_router_models_into_models_cache(&dir, &router_catalog).unwrap();

        let cache: Value =
            serde_json::from_str(&fs::read_to_string(codex_dir.join(CODEX_MODELS_CACHE_FILENAME)).unwrap())
                .unwrap();
        let slugs: Vec<_> = cache["models"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|entry| entry.get("slug").and_then(|slug| slug.as_str()))
            .collect();
        assert!(slugs.contains(&"gpt-5.5"));
        assert!(slugs.contains(&CODEX_CATALOG_MODEL_ID));
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn merge_router_models_into_models_cache_tolerates_utf8_bom() {
        let dir = std::env::temp_dir().join(format!("codex-cache-bom-{}", uuid::Uuid::new_v4()));
        let codex_dir = dir.join(".codex");
        fs::create_dir_all(&codex_dir).unwrap();
        fs::write(
            codex_dir.join(CODEX_MODELS_CACHE_FILENAME),
            "\u{FEFF}{\"models\":[{\"slug\":\"gpt-5.5\",\"display_name\":\"GPT-5.5\"}]}",
        )
        .unwrap();

        let specs = codex_catalog_specs_from_config(
            &test_config(true, true, Some("edge-model"), Some("cloud-model")),
            None,
        );
        let router_catalog = build_codex_model_catalog(&specs);
        merge_router_models_into_models_cache(&dir, &router_catalog).unwrap();

        let cache = read_json_value(&codex_dir.join(CODEX_MODELS_CACHE_FILENAME)).unwrap();
        let slugs: Vec<_> = cache["models"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|entry| entry.get("slug").and_then(|slug| slug.as_str()))
            .collect();
        assert!(slugs.contains(&"gpt-5.5"));
        assert!(slugs.contains(&CODEX_CATALOG_MODEL_ID));
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn merge_router_models_into_models_cache_skips_when_cache_missing() {
        let dir = std::env::temp_dir().join(format!("codex-cache-missing-{}", uuid::Uuid::new_v4()));
        let specs = codex_catalog_specs_from_config(
            &test_config(true, true, Some("edge-model"), Some("cloud-model")),
            None,
        );
        let router_catalog = build_codex_model_catalog(&specs);
        merge_router_models_into_models_cache(&dir, &router_catalog).unwrap();
        assert!(!models_cache_path_in_codex_dir(&dir).is_file());
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn merge_router_models_into_models_cache_removes_router_only_cache() {
        let dir = std::env::temp_dir().join(format!("codex-cache-router-only-{}", uuid::Uuid::new_v4()));
        let codex_dir = dir.join(".codex");
        fs::create_dir_all(&codex_dir).unwrap();
        fs::write(
            codex_dir.join(CODEX_MODELS_CACHE_FILENAME),
            r#"{"models":[{"slug":"token-router"},{"slug":"auto"}]}"#,
        )
        .unwrap();

        let specs = codex_catalog_specs_from_config(
            &test_config(true, true, Some("edge-model"), Some("cloud-model")),
            None,
        );
        let router_catalog = build_codex_model_catalog(&specs);
        merge_router_models_into_models_cache(&dir, &router_catalog).unwrap();
        assert!(!codex_dir.join(CODEX_MODELS_CACHE_FILENAME).is_file());
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn merge_router_models_into_models_cache_recovers_from_corrupt_cache() {
        let dir = std::env::temp_dir().join(format!("codex-cache-corrupt-{}", uuid::Uuid::new_v4()));
        let codex_dir = dir.join(".codex");
        fs::create_dir_all(&codex_dir).unwrap();
        fs::write(codex_dir.join(CODEX_MODELS_CACHE_FILENAME), "not json").unwrap();

        let specs = codex_catalog_specs_from_config(
            &test_config(true, true, Some("edge-model"), Some("cloud-model")),
            None,
        );
        let router_catalog = build_codex_model_catalog(&specs);
        merge_router_models_into_models_cache(&dir, &router_catalog).unwrap();

        assert!(!codex_dir.join(CODEX_MODELS_CACHE_FILENAME).is_file());
        let _ = fs::remove_dir_all(dir);
    }
}
