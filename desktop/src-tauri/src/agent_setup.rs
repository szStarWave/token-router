use std::fs;
use std::path::{Path, PathBuf};

use serde::Serialize;
use serde_json::{json, Value};
use token_router::config::auth_keys::{collect_inbound_api_keys, default_gateway_auth_key_value};
use token_router::config::{ensure_initialized, load_from_path};
use token_router::gateway::AppConfig;

use crate::codex_catalog::{
    codex_catalog_specs_for_agent, write_token_router_codex_catalog,
    CODEX_CATALOG_TIER_ID, TOKEN_ROUTER_CODEX_MODEL_CATALOG_FILENAME,
};

const OPENCLAW_PROVIDER: &str = "token-router";
const OPENCLAW_MODEL_DISPLAY: &str = "Token Router Auto Route";
const OPENCLAW_CONTEXT_WINDOW: u64 = 1_000_000;
const OPENCLAW_TIMEOUT_SECONDS: u64 = 300;
const CODEX_PROVIDER: &str = "token_router";
const CODEX_PROVIDER_NAME: &str = "TokenRouter";
const OPENCODE_PROVIDER: &str = "token-router";
const OPENCODE_PROVIDER_NAME: &str = "Token Router";
const OPENCODE_MODEL_DISPLAY: &str = "Token Router Auto Route";
const MODELS_JSON_VENDOR: &str = "Token Router";
const MODELS_JSON_MODEL_DISPLAY: &str = "Token Router Auto Route";
const MODELS_JSON_MAX_OUTPUT_TOKENS: u64 = 8192;
const DEFAULT_MODEL: &str = "auto";
const CONTEXT_WINDOW_MIN: u64 = 4096;
const CONTEXT_WINDOW_MAX: u64 = 2_000_000;
pub const ERR_AGENT_NOT_INITIALIZED: &str = "agent_not_initialized";

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentInitStatus {
    pub initialized: bool,
    pub config_path: String,
    pub agent: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentDeployStatus {
    pub deployed: bool,
    pub config_path: String,
    pub agent: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentSetupResult {
    pub path: String,
    pub model: String,
    pub base_url: String,
    pub agent: String,
}

fn home_dir() -> Result<PathBuf, String> {
    dirs::home_dir().ok_or_else(|| "home directory not found".to_string())
}

fn agent_config_path(agent: &str) -> Result<PathBuf, String> {
    match agent {
        "openclaw" => openclaw_config_path(),
        "hermes" => hermes_config_path(),
        "hermes-flash" => hermes_flash_config_path(),
        "claude-code" => claude_code_settings_path(),
        "codex" => codex_config_path(),
        "opencode" => opencode_config_path(),
        "codebuddy" => codebuddy_config_path(),
        "workbuddy" => workbuddy_config_path(),
        _ => Err(format!("unknown agent: {agent}")),
    }
}

fn agent_init_state(agent: &str) -> Result<(bool, PathBuf), String> {
    match agent {
        "claude-code" => {
            let settings = claude_code_settings_path()?;
            let legacy = home_dir()?.join(".claude.json");
            Ok((settings.is_file() || legacy.is_file(), settings))
        }
        "opencode" | "codebuddy" | "workbuddy" => {
            let path = agent_config_path(agent)?;
            Ok((true, path))
        }
        _ => {
            let path = agent_config_path(agent)?;
            Ok((path.is_file(), path))
        }
    }
}

fn agent_init_status(agent: &str) -> Result<AgentInitStatus, String> {
    let (initialized, path) = agent_init_state(agent)?;
    Ok(AgentInitStatus {
        initialized,
        config_path: path.display().to_string(),
        agent: agent.to_string(),
    })
}

fn openclaw_home_dir() -> Result<PathBuf, String> {
    Ok(home_dir()?.join(".openclaw"))
}

fn hermes_home_dir() -> Result<PathBuf, String> {
    Ok(home_dir()?.join(".hermes"))
}

fn hermes_flash_home_dir() -> Result<PathBuf, String> {
    Ok(home_dir()?.join(".hermes-flash"))
}

fn openclaw_config_path() -> Result<PathBuf, String> {
    Ok(openclaw_home_dir()?.join("openclaw.json"))
}

fn hermes_config_path() -> Result<PathBuf, String> {
    Ok(hermes_home_dir()?.join("config.yaml"))
}

fn hermes_flash_config_path() -> Result<PathBuf, String> {
    Ok(hermes_flash_home_dir()?.join("config.yaml"))
}

fn claude_code_home_dir() -> Result<PathBuf, String> {
    Ok(home_dir()?.join(".claude"))
}

fn claude_code_settings_path() -> Result<PathBuf, String> {
    Ok(claude_code_home_dir()?.join("settings.json"))
}

fn codex_home_dir() -> Result<PathBuf, String> {
    Ok(home_dir()?.join(".codex"))
}

fn codex_config_path() -> Result<PathBuf, String> {
    Ok(codex_home_dir()?.join("config.toml"))
}

fn opencode_config_dir() -> Result<PathBuf, String> {
    Ok(home_dir()?.join(".config").join("opencode"))
}

fn opencode_config_path() -> Result<PathBuf, String> {
    Ok(opencode_config_dir()?.join("opencode.json"))
}

fn codebuddy_config_dir() -> Result<PathBuf, String> {
    Ok(home_dir()?.join(".codebuddy"))
}

fn codebuddy_config_path() -> Result<PathBuf, String> {
    Ok(codebuddy_config_dir()?.join("models.json"))
}

fn workbuddy_config_dir() -> Result<PathBuf, String> {
    Ok(home_dir()?.join(".workbuddy"))
}

fn workbuddy_config_path() -> Result<PathBuf, String> {
    Ok(workbuddy_config_dir()?.join("models.json"))
}

fn agent_config_path_at(home: &Path, agent: &str) -> Result<PathBuf, String> {
    match agent {
        "openclaw" => Ok(home.join(".openclaw").join("openclaw.json")),
        "hermes" => Ok(home.join(".hermes").join("config.yaml")),
        "hermes-flash" => Ok(home.join(".hermes-flash").join("config.yaml")),
        "claude-code" => Ok(home.join(".claude").join("settings.json")),
        "codex" => Ok(home.join(".codex").join("config.toml")),
        "opencode" => Ok(home.join(".config").join("opencode").join("opencode.json")),
        "codebuddy" => Ok(home.join(".codebuddy").join("models.json")),
        "workbuddy" => Ok(home.join(".workbuddy").join("models.json")),
        _ => Err(format!("unknown agent: {agent}")),
    }
}

fn agent_init_state_at(home: &Path, agent: &str) -> Result<(bool, PathBuf), String> {
    match agent {
        "claude-code" => {
            let settings = agent_config_path_at(home, agent)?;
            let legacy = home.join(".claude.json");
            Ok((settings.is_file() || legacy.is_file(), settings))
        }
        "opencode" | "codebuddy" | "workbuddy" => {
            let path = agent_config_path_at(home, agent)?;
            Ok((true, path))
        }
        _ => {
            let path = agent_config_path_at(home, agent)?;
            Ok((path.is_file(), path))
        }
    }
}

pub fn agent_init_status_at(home: &Path, agent: &str) -> Result<AgentInitStatus, String> {
    let (initialized, path) = agent_init_state_at(home, agent)?;
    Ok(AgentInitStatus {
        initialized,
        config_path: path.display().to_string(),
        agent: agent.to_string(),
    })
}

fn claude_code_deployed_at(home: &Path) -> Result<bool, String> {
    let settings = agent_config_path_at(home, "claude-code")?;
    if settings.is_file() && claude_code_has_token_router(&settings)? {
        return Ok(true);
    }
    let legacy = home.join(".claude.json");
    if legacy.is_file() {
        return claude_code_has_token_router(&legacy);
    }
    Ok(false)
}

pub fn agent_deploy_state_at(home: &Path, agent: &str) -> Result<AgentDeployStatus, String> {
    let (initialized, path) = agent_init_state_at(home, agent)?;
    let config_path = path.display().to_string();
    if !initialized {
        return Ok(AgentDeployStatus {
            deployed: false,
            config_path,
            agent: agent.to_string(),
        });
    }

    let deployed = match agent {
        "openclaw" => openclaw_has_token_router(&path)?,
        "hermes" | "hermes-flash" => hermes_has_token_router(&path)?,
        "claude-code" => claude_code_deployed_at(home)?,
        "codex" => codex_has_token_router(&path)?,
        "opencode" => opencode_has_token_router(&path)?,
        "codebuddy" | "workbuddy" => models_json_has_token_router(&path)?,
        _ => return Err(format!("unknown agent: {agent}")),
    };

    Ok(AgentDeployStatus {
        deployed,
        config_path,
        agent: agent.to_string(),
    })
}

fn ensure_agent_initialized_at(home: &Path, agent: &str) -> Result<PathBuf, String> {
    let (initialized, path) = agent_init_state_at(home, agent)?;
    if initialized {
        return Ok(path);
    }
    Err(format!(
        "{ERR_AGENT_NOT_INITIALIZED}:{agent}:{}",
        path.display()
    ))
}

pub fn configure_openclaw_at(
    home: &Path,
    openai_v1_base: &str,
    api_key: Option<String>,
) -> Result<AgentSetupResult, String> {
    let (auth_enabled, default_key) = load_gateway_auth_state()?;
    let path = ensure_agent_initialized_at(home, "openclaw")?;
    let model = DEFAULT_MODEL.to_string();
    let key = resolve_api_key(auth_enabled, api_key, default_key);

    let mut doc = read_json_file(&path)?;
    merge_openclaw_config(&mut doc, openai_v1_base, &model, &key);
    write_json_file(&path, &doc)?;

    Ok(AgentSetupResult {
        path: path.display().to_string(),
        model: model.clone(),
        base_url: openai_v1_base.to_string(),
        agent: "openclaw".to_string(),
    })
}

pub fn configure_hermes_at(
    home: &Path,
    agent: &str,
    openai_v1_base: &str,
    api_key: Option<String>,
) -> Result<AgentSetupResult, String> {
    let (auth_enabled, default_key) = load_gateway_auth_state()?;
    let path = ensure_agent_initialized_at(home, agent)?;
    let config = load_app_config()?;
    let model = resolved_model(&config);
    let key = resolve_api_key(auth_enabled, api_key, default_key);

    let mut doc = read_yaml_file(&path)?;
    merge_hermes_config(&mut doc, openai_v1_base, &model, &key);
    write_yaml_file(&path, &doc)?;

    Ok(AgentSetupResult {
        path: path.display().to_string(),
        model: model.clone(),
        base_url: openai_v1_base.to_string(),
        agent: agent.to_string(),
    })
}

pub fn configure_claude_code_at(
    home: &Path,
    anthropic_base: &str,
    api_key: Option<String>,
) -> Result<AgentSetupResult, String> {
    let (auth_enabled, default_key) = load_gateway_auth_state()?;
    let path = ensure_agent_initialized_at(home, "claude-code")?;
    let config = load_app_config()?;
    let model = resolved_model(&config);
    let key = resolve_api_key(auth_enabled, api_key, default_key);

    let mut doc = if path.is_file() {
        read_json_file(&path)?
    } else {
        json!({})
    };
    let context_window = resolve_agent_context_window(&config, None);
    merge_claude_code_settings(&mut doc, anthropic_base, &key, context_window);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    write_json_file(&path, &doc)?;

    Ok(AgentSetupResult {
        path: path.display().to_string(),
        model: model.clone(),
        base_url: anthropic_base.to_string(),
        agent: "claude-code".to_string(),
    })
}

pub fn configure_codex_at(
    home: &Path,
    openai_v1_base: &str,
    api_key: Option<String>,
) -> Result<AgentSetupResult, String> {
    let (auth_enabled, default_key) = load_gateway_auth_state()?;
    let path = ensure_agent_initialized_at(home, "codex")?;
    let config = load_app_config()?;
    let key = resolve_api_key(auth_enabled, api_key, default_key);

    let mut doc = read_toml_file(&path)?;
    let context_window = resolve_agent_context_window(&config, None);
    merge_codex_config(&mut doc, openai_v1_base, CODEX_CATALOG_TIER_ID, &key, context_window);
    write_toml_file(&path, &doc)?;
    let specs = codex_catalog_specs_for_agent(&config, context_window);
    write_token_router_codex_catalog(home, &specs)?;

    Ok(AgentSetupResult {
        path: path.display().to_string(),
        model: CODEX_CATALOG_TIER_ID.to_string(),
        base_url: openai_v1_base.to_string(),
        agent: "codex".to_string(),
    })
}

pub fn configure_opencode_at(
    home: &Path,
    openai_v1_base: &str,
    api_key: Option<String>,
) -> Result<AgentSetupResult, String> {
    let (auth_enabled, default_key) = load_gateway_auth_state()?;
    let path = agent_config_path_at(home, "opencode")?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let config = load_app_config()?;
    let model = resolved_model(&config);
    let key = resolve_api_key(auth_enabled, api_key, default_key);

    let mut doc = read_json_file(&path)?;
    merge_opencode_config(&mut doc, openai_v1_base, &model, &key);
    write_json_file(&path, &doc)?;

    Ok(AgentSetupResult {
        path: path.display().to_string(),
        model: model.clone(),
        base_url: openai_v1_base.to_string(),
        agent: "opencode".to_string(),
    })
}

pub fn configure_codebuddy_at(
    home: &Path,
    openai_v1_base: &str,
    api_key: Option<String>,
) -> Result<AgentSetupResult, String> {
    configure_models_json_at(home, "codebuddy", openai_v1_base, api_key)
}

pub fn configure_workbuddy_at(
    home: &Path,
    openai_v1_base: &str,
    api_key: Option<String>,
) -> Result<AgentSetupResult, String> {
    configure_models_json_at(home, "workbuddy", openai_v1_base, api_key)
}

fn configure_models_json_at(
    home: &Path,
    agent: &str,
    openai_v1_base: &str,
    api_key: Option<String>,
) -> Result<AgentSetupResult, String> {
    let (auth_enabled, default_key) = load_gateway_auth_state()?;
    let path = agent_config_path_at(home, agent)?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let config = load_app_config()?;
    let model = resolved_model(&config);
    let key = resolve_api_key(auth_enabled, api_key, default_key);
    let context_window = resolve_agent_context_window(&config, None);
    let chat_url = models_json_chat_completions_url(openai_v1_base);

    let mut doc = read_json_file(&path)?;
    merge_models_json_config(&mut doc, &chat_url, &model, &key, context_window);
    write_json_file(&path, &doc)?;

    Ok(AgentSetupResult {
        path: path.display().to_string(),
        model: model.clone(),
        base_url: chat_url,
        agent: agent.to_string(),
    })
}

fn ensure_agent_initialized(agent: &str) -> Result<PathBuf, String> {
    let (initialized, path) = agent_init_state(agent)?;
    if initialized {
        return Ok(path);
    }
    Err(format!(
        "{ERR_AGENT_NOT_INITIALIZED}:{agent}:{}",
        path.display()
    ))
}

fn load_app_config() -> Result<AppConfig, String> {
    AppConfig::load().map_err(|e| e.to_string())
}

fn gateway_agent_base_url(config: &AppConfig) -> String {
    format!(
        "{}/v1",
        token_router::config::setup::client_gateway_http_url(&config.listen_addr)
    )
}

fn gateway_anthropic_base_url(config: &AppConfig) -> String {
    format!(
        "{}/anthropic",
        token_router::config::setup::client_gateway_http_url(&config.listen_addr)
    )
}

fn resolved_model(config: &AppConfig) -> String {
    config
        .cloud_model
        .as_ref()
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .unwrap_or_else(|| DEFAULT_MODEL.to_string())
}

fn normalize_context_window(value: u64) -> u64 {
    value.clamp(CONTEXT_WINDOW_MIN, CONTEXT_WINDOW_MAX)
}

fn resolve_edge_context_window(config: &AppConfig) -> Option<u64> {
    if let Some(model_id) = config
        .edge_model
        .as_ref()
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
    {
        let snapshot = crate::herdsman::herdsman_get_status();
        for model in &snapshot.models {
            if model.id == model_id || model.name == model_id {
                if let Some(ctx) = model.context_window.filter(|&v| v > 0) {
                    return Some(normalize_context_window(ctx));
                }
            }
        }
    }
    (config.ctx_edge_max_tokens > 0).then(|| {
        normalize_context_window(config.ctx_edge_max_tokens as u64)
    })
}

fn resolve_cloud_context_window(_config: &AppConfig) -> Option<u64> {
    Some(normalize_context_window(
        token_router::gateway::api::models::DEFAULT_CLOUD_MAX_CONTEXT_LENGTH as u64,
    ))
}

fn resolve_agent_context_window(config: &AppConfig, override_ctx: Option<u64>) -> u64 {
    if let Some(value) = override_ctx.filter(|&v| v > 0) {
        return normalize_context_window(value);
    }
    let edge = resolve_edge_context_window(config);
    let cloud = resolve_cloud_context_window(config);
    match (edge, cloud) {
        (Some(edge_ctx), Some(cloud_ctx)) => edge_ctx.max(cloud_ctx),
        (Some(edge_ctx), None) => edge_ctx,
        (None, Some(cloud_ctx)) => cloud_ctx,
        (None, None) => normalize_context_window(
            token_router::gateway::api::models::DEFAULT_CLOUD_MAX_CONTEXT_LENGTH as u64,
        ),
    }
}

fn generate_placeholder_key() -> String {
    format!("placeholder-{}", uuid::Uuid::new_v4().simple())
}

fn resolve_api_key(auth_enabled: bool, override_key: Option<String>, default_key: Option<String>) -> String {
    if let Some(key) = override_key.map(|k| k.trim().to_string()).filter(|k| !k.is_empty()) {
        return key;
    }
    if auth_enabled {
        if let Some(key) = default_key {
            return key;
        }
    }
    generate_placeholder_key()
}

pub fn read_default_auth_key() -> Result<Option<String>, String> {
    let (path, _) = ensure_initialized(None).map_err(|e| e.to_string())?;
    let (file, _) = load_from_path(&path).map_err(|e| e.to_string())?;
    Ok(default_gateway_auth_key_value(&file.gateway))
}

pub fn read_inbound_auth_key(preferred_name: Option<String>) -> Result<Option<String>, String> {
    let (path, _) = ensure_initialized(None).map_err(|e| e.to_string())?;
    let (file, _) = load_from_path(&path).map_err(|e| e.to_string())?;

    if !file.gateway.auth_enabled {
        return Ok(None);
    }

    if let Some(name) = preferred_name.as_ref().map(|n| n.trim()).filter(|n| !n.is_empty()) {
        if let Some(entry) = file
            .gateway
            .api_keys
            .iter()
            .find(|entry| entry.name.eq_ignore_ascii_case(name))
        {
            return Ok(Some(entry.key.clone()));
        }
    }

    let keys = collect_inbound_api_keys(&file.gateway);
    Ok(keys.into_iter().next())
}

fn backup_file(path: &Path) -> Result<(), String> {
    if !path.exists() {
        return Ok(());
    }
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let file_name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("config");
    let backup = path.with_file_name(format!("{file_name}.bak-{ts}"));
    fs::copy(path, &backup).map_err(|e| e.to_string())?;
    Ok(())
}

fn read_json_file(path: &Path) -> Result<Value, String> {
    if !path.exists() {
        return Ok(json!({}));
    }
    let raw = fs::read_to_string(path).map_err(|e| e.to_string())?;
    if raw.trim().is_empty() {
        return Ok(json!({}));
    }
    serde_json::from_str(&raw).map_err(|e| format!("invalid JSON at {}: {e}", path.display()))
}

fn write_json_file(path: &Path, value: &Value) -> Result<(), String> {
    backup_file(path)?;
    let pretty = serde_json::to_string_pretty(value).map_err(|e| e.to_string())?;
    fs::write(path, pretty + "\n").map_err(|e| e.to_string())
}

fn upsert_model_entry(models: &mut Value, model_id: &str, display_name: &str, context_window: Option<u64>) {
    if !models.is_array() {
        *models = json!([]);
    }
    let arr = models.as_array_mut().unwrap();

    if let Some(entry) = arr.iter_mut().find(|item| {
        item.get("id")
            .and_then(|v| v.as_str())
            .is_some_and(|id| id == model_id)
    }) {
        if let Some(obj) = entry.as_object_mut() {
            obj.insert("id".to_string(), json!(model_id));
            obj.insert("name".to_string(), json!(display_name));
            if let Some(cw) = context_window {
                obj.insert("contextWindow".to_string(), json!(cw));
            }
        }
        return;
    }

    let mut entry = json!({
        "id": model_id,
        "name": display_name,
    });
    if let Some(cw) = context_window {
        entry
            .as_object_mut()
            .unwrap()
            .insert("contextWindow".to_string(), json!(cw));
    }
    arr.push(entry);
}

fn merge_openclaw_config(existing: &mut Value, base_url: &str, model_id: &str, api_key: &str) {
    if !existing.is_object() {
        *existing = json!({});
    }
    let root = existing.as_object_mut().unwrap();

    let models = root.entry("models").or_insert_with(|| json!({}));
    if !models.is_object() {
        *models = json!({});
    }
    let providers = models
        .as_object_mut()
        .unwrap()
        .entry("providers")
        .or_insert_with(|| json!({}));
    if !providers.is_object() {
        *providers = json!({});
    }

    let flowy = providers
        .as_object_mut()
        .unwrap()
        .entry(OPENCLAW_PROVIDER)
        .or_insert_with(|| json!({}));
    if !flowy.is_object() {
        *flowy = json!({});
    }
    let flowy_obj = flowy.as_object_mut().unwrap();
    flowy_obj.insert("baseUrl".to_string(), json!(base_url));
    flowy_obj.insert("apiKey".to_string(), json!(api_key));
    flowy_obj.insert("timeoutSeconds".to_string(), json!(OPENCLAW_TIMEOUT_SECONDS));

    let model_list = flowy_obj
        .entry("models")
        .or_insert_with(|| json!([]));
    upsert_model_entry(
        model_list,
        model_id,
        OPENCLAW_MODEL_DISPLAY,
        Some(OPENCLAW_CONTEXT_WINDOW),
    );

    let agents = root.entry("agents").or_insert_with(|| json!({}));
    if !agents.is_object() {
        *agents = json!({});
    }
    let defaults = agents
        .as_object_mut()
        .unwrap()
        .entry("defaults")
        .or_insert_with(|| json!({}));
    if !defaults.is_object() {
        *defaults = json!({});
    }
    let model = defaults
        .as_object_mut()
        .unwrap()
        .entry("model")
        .or_insert_with(|| json!({}));
    if !model.is_object() {
        *model = json!({});
    }
    model
        .as_object_mut()
        .unwrap()
        .insert(
            "primary".to_string(),
            json!(format!("{OPENCLAW_PROVIDER}/{model_id}")),
        );
}

fn read_yaml_file(path: &Path) -> Result<Value, String> {
    if !path.exists() {
        return Ok(json!({}));
    }
    let raw = fs::read_to_string(path).map_err(|e| e.to_string())?;
    if raw.trim().is_empty() {
        return Ok(json!({}));
    }
    serde_yaml::from_str(&raw).map_err(|e| format!("invalid YAML at {}: {e}", path.display()))
}

fn write_yaml_file(path: &Path, value: &Value) -> Result<(), String> {
    backup_file(path)?;
    let content = serde_yaml::to_string(value).map_err(|e| e.to_string())?;
    fs::write(path, content).map_err(|e| e.to_string())
}

fn merge_hermes_config(existing: &mut Value, base_url: &str, model_id: &str, api_key: &str) {
    if !existing.is_object() {
        *existing = json!({});
    }
    let root = existing.as_object_mut().unwrap();

    let model = root.entry("model").or_insert_with(|| json!({}));
    if !model.is_object() {
        *model = json!({});
    }
    let model_obj = model.as_object_mut().unwrap();
    model_obj.insert("default".to_string(), json!(model_id));
    model_obj.insert("provider".to_string(), json!("custom"));
    model_obj.insert("base_url".to_string(), json!(base_url));
    model_obj.insert("api_key".to_string(), json!(api_key));
}

fn merge_claude_code_settings(
    existing: &mut Value,
    base_url: &str,
    api_key: &str,
    context_window: u64,
) {
    if !existing.is_object() {
        *existing = json!({});
    }
    let root = existing.as_object_mut().unwrap();
    let env = root.entry("env").or_insert_with(|| json!({}));
    if !env.is_object() {
        *env = json!({});
    }
    let env_obj = env.as_object_mut().unwrap();
    env_obj.insert("ANTHROPIC_BASE_URL".to_string(), json!(base_url));
    env_obj.insert("ANTHROPIC_AUTH_TOKEN".to_string(), json!(api_key));
    let context = context_window.to_string();
    env_obj.insert("CLAUDE_CODE_MAX_CONTEXT_TOKENS".to_string(), json!(context));
    env_obj.insert("CLAUDE_CODE_AUTO_COMPACT_WINDOW".to_string(), json!(context));
}

fn read_toml_file(path: &Path) -> Result<toml::Value, String> {
    if !path.exists() {
        return Ok(toml::Value::Table(toml::map::Map::new()));
    }
    let raw = fs::read_to_string(path).map_err(|e| e.to_string())?;
    if raw.trim().is_empty() {
        return Ok(toml::Value::Table(toml::map::Map::new()));
    }
    toml::from_str(&raw).map_err(|e| format!("invalid TOML at {}: {e}", path.display()))
}

fn write_toml_file(path: &Path, value: &toml::Value) -> Result<(), String> {
    backup_file(path)?;
    let content = toml::to_string_pretty(value).map_err(|e| e.to_string())?;
    fs::write(path, content).map_err(|e| e.to_string())
}

fn merge_opencode_config(existing: &mut Value, base_url: &str, model_id: &str, api_key: &str) {
    if !existing.is_object() {
        *existing = json!({});
    }
    let root = existing.as_object_mut().unwrap();
    root.insert(
        "$schema".to_string(),
        json!("https://opencode.ai/config.json"),
    );

    let providers = root.entry("provider").or_insert_with(|| json!({}));
    if !providers.is_object() {
        *providers = json!({});
    }

    let provider = providers
        .as_object_mut()
        .unwrap()
        .entry(OPENCODE_PROVIDER)
        .or_insert_with(|| json!({}));
    if !provider.is_object() {
        *provider = json!({});
    }
    let provider_obj = provider.as_object_mut().unwrap();
    provider_obj.insert("npm".to_string(), json!("@ai-sdk/openai-compatible"));
    provider_obj.insert("name".to_string(), json!(OPENCODE_PROVIDER_NAME));

    let options = provider_obj
        .entry("options")
        .or_insert_with(|| json!({}));
    if !options.is_object() {
        *options = json!({});
    }
    let options_obj = options.as_object_mut().unwrap();
    options_obj.insert("baseURL".to_string(), json!(base_url));
    options_obj.insert("apiKey".to_string(), json!(api_key));

    let models = provider_obj
        .entry("models")
        .or_insert_with(|| json!({}));
    if !models.is_object() {
        *models = json!({});
    }
    let model_entry = models
        .as_object_mut()
        .unwrap()
        .entry(model_id.to_string())
        .or_insert_with(|| json!({}));
    if !model_entry.is_object() {
        *model_entry = json!({});
    }
    model_entry
        .as_object_mut()
        .unwrap()
        .insert("name".to_string(), json!(OPENCODE_MODEL_DISPLAY));

    root.insert(
        "model".to_string(),
        json!(format!("{OPENCODE_PROVIDER}/{model_id}")),
    );
}

fn models_json_chat_completions_url(openai_v1_base: &str) -> String {
    format!(
        "{}/chat/completions",
        openai_v1_base.trim_end_matches('/')
    )
}

fn merge_models_json_config(
    existing: &mut Value,
    chat_url: &str,
    model_id: &str,
    api_key: &str,
    max_input_tokens: u64,
) {
    if !existing.is_object() {
        *existing = json!({});
    }
    let root = existing.as_object_mut().unwrap();

    let models = root.entry("models").or_insert_with(|| json!([]));
    if !models.is_array() {
        *models = json!([]);
    }
    let models_arr = models.as_array_mut().unwrap();

    let new_model = json!({
        "id": model_id,
        "name": MODELS_JSON_MODEL_DISPLAY,
        "vendor": MODELS_JSON_VENDOR,
        "apiKey": api_key,
        "maxInputTokens": max_input_tokens,
        "maxOutputTokens": MODELS_JSON_MAX_OUTPUT_TOKENS,
        "url": chat_url,
        "supportsToolCall": true,
        "supportsImages": true
    });

    if let Some(idx) = models_arr
        .iter()
        .position(|m| m.get("id").and_then(|v| v.as_str()) == Some(model_id))
    {
        models_arr[idx] = new_model;
    } else {
        models_arr.push(new_model);
    }

    let available = root.entry("availableModels").or_insert_with(|| json!([]));
    if !available.is_array() {
        *available = json!([]);
    }
    let available_arr = available.as_array_mut().unwrap();
    if !available_arr
        .iter()
        .any(|v| v.as_str() == Some(model_id))
    {
        available_arr.insert(0, json!(model_id));
    }
}

fn merge_codex_config(
    existing: &mut toml::Value,
    base_url: &str,
    model_id: &str,
    api_key: &str,
    context_window: u64,
) {
    if !existing.is_table() {
        *existing = toml::Value::Table(toml::map::Map::new());
    }
    let root = existing.as_table_mut().unwrap();
    root.insert("model".into(), toml::Value::String(model_id.to_string()));
    root.insert(
        "model_provider".into(),
        toml::Value::String(CODEX_PROVIDER.to_string()),
    );
    root.insert(
        "model_context_window".into(),
        toml::Value::Integer(context_window as i64),
    );
    root.insert(
        "model_catalog_json".into(),
        toml::Value::String(TOKEN_ROUTER_CODEX_MODEL_CATALOG_FILENAME.to_string()),
    );
    root.insert(
        "disable_response_storage".into(),
        toml::Value::Boolean(true),
    );
    root.insert(
        "model_reasoning_effort".into(),
        toml::Value::String("medium".to_string()),
    );

    let mut provider = toml::map::Map::new();
    provider.insert(
        "name".into(),
        toml::Value::String(CODEX_PROVIDER_NAME.to_string()),
    );
    provider.insert(
        "base_url".into(),
        toml::Value::String(base_url.to_string()),
    );
    provider.insert(
        "experimental_bearer_token".into(),
        toml::Value::String(api_key.to_string()),
    );
    provider.insert(
        "wire_api".into(),
        toml::Value::String("responses".to_string()),
    );
    provider.insert(
        "requires_openai_auth".into(),
        toml::Value::Boolean(true),
    );

    let providers = root
        .entry("model_providers")
        .or_insert_with(|| toml::Value::Table(toml::map::Map::new()));
    if !providers.is_table() {
        *providers = toml::Value::Table(toml::map::Map::new());
    }
    providers
        .as_table_mut()
        .unwrap()
        .insert(CODEX_PROVIDER.to_string(), toml::Value::Table(provider));
}

fn load_gateway_auth_state() -> Result<(bool, Option<String>), String> {
    let (path, _) = ensure_initialized(None).map_err(|e| e.to_string())?;
    let (file, _) = load_from_path(&path).map_err(|e| e.to_string())?;
    let default_key = default_gateway_auth_key_value(&file.gateway);
    Ok((file.gateway.auth_enabled, default_key))
}

fn configure_hermes_for(agent: &str, api_key: Option<String>) -> Result<AgentSetupResult, String> {
    let config = load_app_config()?;
    let (auth_enabled, default_key) = load_gateway_auth_state()?;
    let path = ensure_agent_initialized(agent)?;
    let base_url = gateway_agent_base_url(&config);
    let model = resolved_model(&config);
    let key = resolve_api_key(auth_enabled, api_key, default_key);

    let mut doc = read_yaml_file(&path)?;
    merge_hermes_config(&mut doc, &base_url, &model, &key);
    write_yaml_file(&path, &doc)?;

    Ok(AgentSetupResult {
        path: path.display().to_string(),
        model: model.clone(),
        base_url,
        agent: agent.to_string(),
    })
}

fn configure_openclaw(api_key: Option<String>) -> Result<AgentSetupResult, String> {
    let config = load_app_config()?;
    let (auth_enabled, default_key) = load_gateway_auth_state()?;
    let path = ensure_agent_initialized("openclaw")?;
    let base_url = gateway_agent_base_url(&config);
    let model = DEFAULT_MODEL.to_string();
    let key = resolve_api_key(auth_enabled, api_key, default_key);

    let mut doc = read_json_file(&path)?;
    merge_openclaw_config(&mut doc, &base_url, &model, &key);
    write_json_file(&path, &doc)?;

    Ok(AgentSetupResult {
        path: path.display().to_string(),
        model: model.clone(),
        base_url,
        agent: "openclaw".to_string(),
    })
}

fn configure_hermes(api_key: Option<String>) -> Result<AgentSetupResult, String> {
    configure_hermes_for("hermes", api_key)
}

fn configure_hermes_flash(api_key: Option<String>) -> Result<AgentSetupResult, String> {
    configure_hermes_for("hermes-flash", api_key)
}

fn configure_claude_code(
    api_key: Option<String>,
    context_window: Option<u64>,
) -> Result<AgentSetupResult, String> {
    let config = load_app_config()?;
    let (auth_enabled, default_key) = load_gateway_auth_state()?;
    let path = ensure_agent_initialized("claude-code")?;
    let base_url = gateway_anthropic_base_url(&config);
    let model = resolved_model(&config);
    let key = resolve_api_key(auth_enabled, api_key, default_key);

    let mut doc = if path.is_file() {
        read_json_file(&path)?
    } else {
        json!({})
    };
    let context_window = resolve_agent_context_window(&config, context_window);
    merge_claude_code_settings(&mut doc, &base_url, &key, context_window);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    write_json_file(&path, &doc)?;

    Ok(AgentSetupResult {
        path: path.display().to_string(),
        model: model.clone(),
        base_url,
        agent: "claude-code".to_string(),
    })
}

fn configure_codex(api_key: Option<String>, context_window: Option<u64>) -> Result<AgentSetupResult, String> {
    let config = load_app_config()?;
    let (auth_enabled, default_key) = load_gateway_auth_state()?;
    let path = ensure_agent_initialized("codex")?;
    let base_url = gateway_agent_base_url(&config);
    let key = resolve_api_key(auth_enabled, api_key, default_key);

    let mut doc = read_toml_file(&path)?;
    let context_window = resolve_agent_context_window(&config, context_window);
    merge_codex_config(&mut doc, &base_url, CODEX_CATALOG_TIER_ID, &key, context_window);
    write_toml_file(&path, &doc)?;
    let home = home_dir()?;
    let specs = codex_catalog_specs_for_agent(&config, context_window);
    write_token_router_codex_catalog(&home, &specs)?;

    Ok(AgentSetupResult {
        path: path.display().to_string(),
        model: CODEX_CATALOG_TIER_ID.to_string(),
        base_url,
        agent: "codex".to_string(),
    })
}

fn configure_opencode(api_key: Option<String>) -> Result<AgentSetupResult, String> {
    let config = load_app_config()?;
    let (auth_enabled, default_key) = load_gateway_auth_state()?;
    let path = opencode_config_path()?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let base_url = gateway_agent_base_url(&config);
    let model = resolved_model(&config);
    let key = resolve_api_key(auth_enabled, api_key, default_key);

    let mut doc = read_json_file(&path)?;
    merge_opencode_config(&mut doc, &base_url, &model, &key);
    write_json_file(&path, &doc)?;

    Ok(AgentSetupResult {
        path: path.display().to_string(),
        model: model.clone(),
        base_url,
        agent: "opencode".to_string(),
    })
}

fn configure_codebuddy(api_key: Option<String>, context_window: Option<u64>) -> Result<AgentSetupResult, String> {
    configure_models_json("codebuddy", api_key, context_window)
}

fn configure_workbuddy(api_key: Option<String>, context_window: Option<u64>) -> Result<AgentSetupResult, String> {
    configure_models_json("workbuddy", api_key, context_window)
}

fn configure_models_json(
    agent: &str,
    api_key: Option<String>,
    context_window: Option<u64>,
) -> Result<AgentSetupResult, String> {
    let config = load_app_config()?;
    let (auth_enabled, default_key) = load_gateway_auth_state()?;
    let path = agent_config_path(agent)?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let openai_v1_base = gateway_agent_base_url(&config);
    let model = resolved_model(&config);
    let key = resolve_api_key(auth_enabled, api_key, default_key);
    let context_window = resolve_agent_context_window(&config, context_window);
    let chat_url = models_json_chat_completions_url(&openai_v1_base);

    let mut doc = read_json_file(&path)?;
    merge_models_json_config(&mut doc, &chat_url, &model, &key, context_window);
    write_json_file(&path, &doc)?;

    Ok(AgentSetupResult {
        path: path.display().to_string(),
        model: model.clone(),
        base_url: chat_url,
        agent: agent.to_string(),
    })
}

fn non_empty_str(value: Option<&str>) -> bool {
    value.is_some_and(|s| !s.trim().is_empty())
}

fn openclaw_has_token_router(path: &Path) -> Result<bool, String> {
    let doc = read_json_file(path)?;
    let provider = doc
        .get("models")
        .and_then(|m| m.get("providers"))
        .and_then(|p| p.get(OPENCLAW_PROVIDER));
    let has_provider = provider
        .and_then(|f| f.get("baseUrl"))
        .and_then(|v| v.as_str())
        .is_some_and(|s| !s.trim().is_empty());
    let primary = doc
        .get("agents")
        .and_then(|a| a.get("defaults"))
        .and_then(|d| d.get("model"))
        .and_then(|m| m.get("primary"))
        .and_then(|v| v.as_str())
        .map(|s| s.starts_with(&format!("{OPENCLAW_PROVIDER}/")))
        .unwrap_or(false);
    Ok(has_provider || primary)
}

fn hermes_has_token_router(path: &Path) -> Result<bool, String> {
    let doc = read_yaml_file(path)?;
    let model = doc.get("model");
    Ok(non_empty_str(
        model
            .and_then(|m| m.get("base_url"))
            .and_then(|v| v.as_str()),
    ) && non_empty_str(
        model
            .and_then(|m| m.get("api_key"))
            .and_then(|v| v.as_str()),
    ))
}

fn claude_code_has_token_router(path: &Path) -> Result<bool, String> {
    let doc = read_json_file(path)?;
    Ok(non_empty_str(
        doc.get("env")
            .and_then(|e| e.get("ANTHROPIC_BASE_URL"))
            .and_then(|v| v.as_str()),
    ) && non_empty_str(
        doc.get("env")
            .and_then(|e| e.get("ANTHROPIC_AUTH_TOKEN"))
            .and_then(|v| v.as_str()),
    ))
}

fn codex_has_token_router(path: &Path) -> Result<bool, String> {
    let doc = read_toml_file(path)?;
    let model_provider = doc
        .get("model_provider")
        .and_then(|v| v.as_str());
    let has_provider = doc
        .get("model_providers")
        .and_then(|p| p.get(CODEX_PROVIDER))
        .and_then(|p| p.get("base_url"))
        .and_then(|v| v.as_str())
        .is_some_and(|s| !s.trim().is_empty());
    Ok(model_provider == Some(CODEX_PROVIDER) && has_provider)
}

fn opencode_has_token_router(path: &Path) -> Result<bool, String> {
    if !path.is_file() {
        return Ok(false);
    }
    let doc = read_json_file(path)?;
    let provider = doc.get("provider").and_then(|p| p.get(OPENCODE_PROVIDER));
    let has_base = provider
        .and_then(|p| p.get("options"))
        .and_then(|o| o.get("baseURL"))
        .and_then(|v| v.as_str())
        .is_some_and(|s| !s.trim().is_empty());
    let model = doc
        .get("model")
        .and_then(|v| v.as_str())
        .map(|s| s.starts_with(&format!("{OPENCODE_PROVIDER}/")))
        .unwrap_or(false);
    Ok(has_base || model)
}

fn models_json_has_token_router(path: &Path) -> Result<bool, String> {
    if !path.is_file() {
        return Ok(false);
    }
    let doc = read_json_file(path)?;
    let models = doc.get("models").and_then(|m| m.as_array());
    Ok(models.is_some_and(|arr| {
        arr.iter().any(|m| {
            m.get("vendor")
                .and_then(|v| v.as_str())
                .is_some_and(|v| v == MODELS_JSON_VENDOR)
                && m.get("url")
                    .and_then(|v| v.as_str())
                    .is_some_and(|s| s.contains("/chat/completions") && !s.trim().is_empty())
        })
    }))
}

fn claude_code_deployed() -> Result<bool, String> {
    let settings = claude_code_settings_path()?;
    if settings.is_file() && claude_code_has_token_router(&settings)? {
        return Ok(true);
    }
    let legacy = home_dir()?.join(".claude.json");
    if legacy.is_file() {
        return claude_code_has_token_router(&legacy);
    }
    Ok(false)
}

fn agent_deploy_state(agent: &str) -> Result<AgentDeployStatus, String> {
    let (initialized, path) = agent_init_state(agent)?;
    let config_path = path.display().to_string();
    if !initialized {
        return Ok(AgentDeployStatus {
            deployed: false,
            config_path,
            agent: agent.to_string(),
        });
    }

    let deployed = match agent {
        "openclaw" => openclaw_has_token_router(&path)?,
        "hermes" | "hermes-flash" => hermes_has_token_router(&path)?,
        "claude-code" => claude_code_deployed()?,
        "codex" => codex_has_token_router(&path)?,
        "opencode" => opencode_has_token_router(&path)?,
        "codebuddy" | "workbuddy" => models_json_has_token_router(&path)?,
        _ => return Err(format!("unknown agent: {agent}")),
    };

    Ok(AgentDeployStatus {
        deployed,
        config_path,
        agent: agent.to_string(),
    })
}

#[tauri::command]
pub fn check_agent_initialized(agent: String) -> Result<AgentInitStatus, String> {
    agent_init_status(agent.trim())
}

#[tauri::command]
pub fn check_agent_deployed(agent: String) -> Result<AgentDeployStatus, String> {
    agent_deploy_state(agent.trim())
}

#[tauri::command]
pub fn configure_openclaw_agent(api_key: Option<String>) -> Result<AgentSetupResult, String> {
    configure_openclaw(api_key)
}

#[tauri::command]
pub fn configure_hermes_agent(api_key: Option<String>) -> Result<AgentSetupResult, String> {
    configure_hermes(api_key)
}

#[tauri::command]
pub fn configure_hermes_flash_agent(api_key: Option<String>) -> Result<AgentSetupResult, String> {
    configure_hermes_flash(api_key)
}

#[tauri::command]
pub fn configure_claude_code_agent(
    api_key: Option<String>,
    context_window: Option<u64>,
) -> Result<AgentSetupResult, String> {
    configure_claude_code(api_key, context_window)
}

#[tauri::command]
pub fn configure_codex_agent(
    api_key: Option<String>,
    context_window: Option<u64>,
) -> Result<AgentSetupResult, String> {
    configure_codex(api_key, context_window)
}

#[tauri::command]
pub fn configure_opencode_agent(api_key: Option<String>) -> Result<AgentSetupResult, String> {
    configure_opencode(api_key)
}

#[tauri::command]
pub fn configure_codebuddy_agent(
    api_key: Option<String>,
    context_window: Option<u64>,
) -> Result<AgentSetupResult, String> {
    configure_codebuddy(api_key, context_window)
}

#[tauri::command]
pub fn configure_workbuddy_agent(
    api_key: Option<String>,
    context_window: Option<u64>,
) -> Result<AgentSetupResult, String> {
    configure_workbuddy(api_key, context_window)
}

#[tauri::command]
pub fn read_inbound_auth_key_cmd(preferred_name: Option<String>) -> Result<Option<String>, String> {
    read_inbound_auth_key(preferred_name)
}

#[tauri::command]
pub fn read_default_auth_key_cmd() -> Result<Option<String>, String> {
    read_default_auth_key()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn merge_openclaw_preserves_other_providers() {
        let mut doc = json!({
            "models": {
                "providers": {
                    "anthropic": { "baseUrl": "https://api.anthropic.com" }
                }
            }
        });
        merge_openclaw_config(&mut doc, "http://127.0.0.1:11080/v1", "auto", "test-key");
        let providers = &doc["models"]["providers"];
        assert!(providers.get("anthropic").is_some());
        assert_eq!(providers["token-router"]["baseUrl"], "http://127.0.0.1:11080/v1");
        assert_eq!(
            providers["token-router"]["models"][0]["contextWindow"],
            1_000_000
        );
        assert_eq!(providers["token-router"]["timeoutSeconds"], 300);
        assert_eq!(doc["agents"]["defaults"]["model"]["primary"], "token-router/auto");
        let dir = std::env::temp_dir().join(format!("agent-setup-test-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("openclaw.json");
        write_json_file(&path, &doc).unwrap();
        assert!(openclaw_has_token_router(&path).unwrap());
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn merge_hermes_preserves_other_sections() {
        let mut doc = json!({
            "tools": { "web_search": true }
        });
        merge_hermes_config(&mut doc, "http://127.0.0.1:11080/v1", "auto", "test-key");
        assert_eq!(doc["tools"]["web_search"], true);
        assert_eq!(doc["model"]["provider"], "custom");
        assert_eq!(doc["model"]["default"], "auto");
    }

    #[test]
    fn hermes_flash_config_path_uses_home_dir() {
        let path = hermes_flash_config_path().unwrap();
        assert!(path.to_string_lossy().contains(".hermes-flash"));
        assert!(path.to_string_lossy().ends_with("config.yaml"));
    }

    #[test]
    fn merge_claude_code_settings_updates_env() {
        let mut doc = json!({ "permissions": { "allow": [] } });
        merge_claude_code_settings(
            &mut doc,
            "http://127.0.0.1:11080/anthropic",
            "test-key",
            262_144,
        );
        assert_eq!(doc["permissions"]["allow"], json!([]));
        assert_eq!(
            doc["env"]["ANTHROPIC_BASE_URL"],
            "http://127.0.0.1:11080/anthropic"
        );
        assert_eq!(doc["env"]["ANTHROPIC_AUTH_TOKEN"], "test-key");
        assert_eq!(doc["env"]["CLAUDE_CODE_MAX_CONTEXT_TOKENS"], "262144");
        assert_eq!(doc["env"]["CLAUDE_CODE_AUTO_COMPACT_WINDOW"], "262144");
    }

    #[test]
    fn merge_opencode_config_sets_token_router_provider() {
        let mut doc = json!({
            "theme": "dark"
        });
        merge_opencode_config(&mut doc, "http://127.0.0.1:11080/v1", "auto", "test-key");
        assert_eq!(doc["theme"], "dark");
        assert_eq!(
            doc["provider"]["token-router"]["options"]["baseURL"],
            "http://127.0.0.1:11080/v1"
        );
        assert_eq!(doc["model"], "token-router/auto");
        let dir = std::env::temp_dir().join(format!("agent-setup-test-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("opencode.json");
        write_json_file(&path, &doc).unwrap();
        assert!(opencode_has_token_router(&path).unwrap());
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn merge_codex_config_sets_token_router_provider() {
        let mut doc = toml::Value::Table(toml::map::Map::new());
        merge_codex_config(
            &mut doc,
            "http://127.0.0.1:11080/v1",
            "auto",
            "test-key",
            1_000_000,
        );
        assert_eq!(doc["model"], toml::Value::String("auto".into()));
        assert_eq!(doc["model_provider"], toml::Value::String("token_router".into()));
        assert_eq!(doc["model_context_window"], toml::Value::Integer(1_000_000));
        assert_eq!(
            doc["model_catalog_json"],
            toml::Value::String("token-router-model-catalog.json".into())
        );
        assert_eq!(
            doc["model_providers"]["token_router"]["wire_api"],
            toml::Value::String("responses".into())
        );
        assert_eq!(
            doc["model_providers"]["token_router"]["requires_openai_auth"],
            toml::Value::Boolean(true)
        );
        assert_eq!(
            doc["disable_response_storage"],
            toml::Value::Boolean(true)
        );
    }

    #[test]
    fn resolve_agent_context_window_uses_max_of_edge_and_cloud() {
        let mut file = token_router::config::ConfigFile::default();
        file.gateway.ctx_edge_max_tokens = 131_072;
        let config = AppConfig::from_file(file, std::env::temp_dir()).unwrap();
        assert_eq!(
            resolve_agent_context_window(&config, None),
            token_router::gateway::api::models::DEFAULT_CLOUD_MAX_CONTEXT_LENGTH as u64
        );
        assert_eq!(resolve_agent_context_window(&config, Some(262_144)), 262_144);
    }

    #[test]
    fn merge_models_json_config_preserves_other_models() {
        let mut doc = json!({
            "models": [
                { "id": "deepseek-chat", "name": "DeepSeek", "vendor": "DeepSeek" }
            ],
            "availableModels": ["deepseek-chat"]
        });
        merge_models_json_config(
            &mut doc,
            "http://127.0.0.1:11080/v1/chat/completions",
            "auto",
            "test-key",
            262_144,
        );
        assert_eq!(doc["models"].as_array().unwrap().len(), 2);
        assert_eq!(doc["models"][0]["id"], "deepseek-chat");
        assert_eq!(doc["models"][1]["id"], "auto");
        assert_eq!(doc["models"][1]["vendor"], MODELS_JSON_VENDOR);
        assert_eq!(
            doc["models"][1]["url"],
            "http://127.0.0.1:11080/v1/chat/completions"
        );
        assert_eq!(doc["availableModels"][0], "auto");
        assert_eq!(doc["availableModels"][1], "deepseek-chat");
    }

    #[test]
    fn merge_models_json_config_overwrites_same_id() {
        let mut doc = json!({
            "models": [
                { "id": "auto", "name": "Old", "vendor": "Other", "url": "https://old.example.com/v1/chat/completions" }
            ]
        });
        merge_models_json_config(
            &mut doc,
            "http://127.0.0.1:11080/v1/chat/completions",
            "auto",
            "new-key",
            131_072,
        );
        assert_eq!(doc["models"].as_array().unwrap().len(), 1);
        assert_eq!(doc["models"][0]["vendor"], MODELS_JSON_VENDOR);
        assert_eq!(doc["models"][0]["apiKey"], "new-key");
        let dir = std::env::temp_dir().join(format!("agent-setup-test-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("models.json");
        write_json_file(&path, &doc).unwrap();
        assert!(models_json_has_token_router(&path).unwrap());
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn models_json_chat_completions_url_appends_path() {
        assert_eq!(
            models_json_chat_completions_url("http://127.0.0.1:11080/v1"),
            "http://127.0.0.1:11080/v1/chat/completions"
        );
        assert_eq!(
            models_json_chat_completions_url("http://127.0.0.1:11080/v1/"),
            "http://127.0.0.1:11080/v1/chat/completions"
        );
    }

    #[test]
    fn codebuddy_and_workbuddy_config_paths_differ() {
        let codebuddy = codebuddy_config_path().unwrap();
        let workbuddy = workbuddy_config_path().unwrap();
        assert!(codebuddy.to_string_lossy().contains(".codebuddy"));
        assert!(workbuddy.to_string_lossy().contains(".workbuddy"));
        assert_ne!(codebuddy, workbuddy);
    }
}
