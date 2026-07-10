use std::fs;
use std::path::{Path, PathBuf};

use token_router::gateway::api::codex_catalog::{
    build_codex_model_catalog, catalog_path_in_codex_dir, codex_catalog_specs_from_config,
    merge_router_models_into_models_cache,
};
use token_router::gateway::AppConfig;

pub use token_router::gateway::api::codex_catalog::CodexCatalogModelSpec;
pub use token_router::gateway::api::codex_catalog::CODEX_CATALOG_MODEL_ID;
pub use token_router::gateway::api::codex_catalog::TOKEN_ROUTER_CODEX_MODEL_CATALOG_FILENAME;

pub fn codex_catalog_specs_for_agent(config: &AppConfig, context_window: u64) -> Vec<CodexCatalogModelSpec> {
    let mut specs = codex_catalog_specs_from_config(config, None);
    for spec in &mut specs {
        spec.context_window = context_window;
    }
    specs
}

pub fn write_token_router_codex_catalog(home: &Path, specs: &[CodexCatalogModelSpec]) -> Result<PathBuf, String> {
    let codex_dir = home.join(".codex");
    fs::create_dir_all(&codex_dir).map_err(|e| e.to_string())?;

    let catalog = build_codex_model_catalog(specs);
    let path = catalog_path_in_codex_dir(home);
    let text = serde_json::to_string_pretty(&catalog).map_err(|e| e.to_string())?;
    fs::write(&path, text).map_err(|e| e.to_string())?;
    merge_router_models_into_models_cache(home, &catalog)?;

    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{json, Value};
    use token_router::config::ConfigFile;

    fn test_config(edge_model: Option<&str>, cloud_model: Option<&str>) -> AppConfig {
        let mut file = ConfigFile::default();
        file.gateway.ctx_edge_max_tokens = 131_072;
        if let Some(model) = edge_model {
            file.upstream.edge = Some(token_router::config::UpstreamEndpoint {
                base_url: "http://127.0.0.1:8080/v1".into(),
                api_key: None,
                model: Some(model.to_string()),
            });
        }
        if let Some(model) = cloud_model {
            file.upstream.cloud = Some(token_router::config::UpstreamEndpoint {
                base_url: "https://example.com".into(),
                api_key: Some("key".into()),
                model: Some(model.to_string()),
            });
        }
        AppConfig::from_file(file, std::env::temp_dir()).unwrap()
    }

    #[test]
    fn codex_catalog_specs_expose_token_router_model() {
        let config = test_config(Some("deepseek-v4-flash"), None);
        let specs = codex_catalog_specs_for_agent(&config, 1_000_000);
        assert_eq!(specs.len(), 1);
        assert_eq!(specs[0].model, "token-router");
        assert_eq!(specs[0].display_name, "TokenRouter");
        assert_eq!(specs[0].context_window, 1_000_000);
    }

    #[test]
    fn write_token_router_codex_catalog_writes_file_without_touching_models_cache() {
        let dir = std::env::temp_dir().join(format!("codex-catalog-test-{}", uuid::Uuid::new_v4()));
        let cache_path = dir.join(".codex").join("models_cache.json");
        fs::create_dir_all(cache_path.parent().unwrap()).unwrap();
        fs::write(
            &cache_path,
            r#"{"models":[{"slug":"gpt-5.4","display_name":"GPT-5.4"}]}"#,
        )
        .unwrap();

        let specs = vec![CodexCatalogModelSpec {
            model: "token-router".into(),
            display_name: "TokenRouter".into(),
            context_window: 1_000_000,
            visibility: token_router::gateway::api::codex_catalog::CodexCatalogVisibility::List,
        }];
        let path = write_token_router_codex_catalog(&dir, &specs).unwrap();
        assert!(path.is_file());
        let text = fs::read_to_string(&path).unwrap();
        let catalog: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(catalog["models"][0]["slug"], "token-router");
        assert_eq!(catalog["models"][0]["visibility"], "list");
        assert_eq!(catalog["models"][0]["display_name"], "TokenRouter");
        assert_eq!(catalog["models"][0]["service_tiers"], json!([]));

        let cache_text = fs::read_to_string(&cache_path).unwrap();
        assert!(
            cache_text.contains("gpt-5.4"),
            "models_cache.json must keep built-in GPT models"
        );
        assert!(
            cache_text.contains("token-router"),
            "models_cache.json must include TokenRouter entry"
        );
        let _ = fs::remove_dir_all(dir);
    }
}
