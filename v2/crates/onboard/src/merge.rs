//! 8 个 Agent 的精确合并器（技术方案 §8.2 / 附录 B）。
//!
//! 原则：
//! - 幂等：只改 token-router 自己的键，不覆盖用户其它配置；
//! - 格式保留：JSON 保留键序与缩进风格；YAML/TOML 用格式保留解析
//!   （toml_edit 保留注释）；
//! - 密钥不落明文日志：写入 Agent 配置的是 host 解析后的真实 token，
//!   但 plan 预览与任何报告只出现引用。

use serde_json::{Map, Value};

use super::{AgentKind, SetupError, SetupRequest};

/// token-router 在各 Agent 配置中的 provider 键名。
pub const PROVIDER_KEY: &str = "token-router";

/// 单文件精确合并：`before` 原文 + Agent 类型 → 合并后完整文本。
///
/// 失败语义：解析错误返回 `SetupError::Parse`，不半写。
pub fn merge_document(
    agent: AgentKind,
    format: &str,
    before: &str,
    request: &SetupRequest,
) -> Result<String, SetupError> {
    match (agent, format) {
        // JSON 家族
        (AgentKind::Openclaw, "json") => merge_openclaw(before, request),
        (AgentKind::ClaudeCode, "json") => merge_claude_code(before, request),
        (AgentKind::Opencode, "json") => merge_opencode(before, request),
        (AgentKind::Codebuddy, "json") | (AgentKind::Workbuddy, "json") => {
            merge_buddy(before, request)
        }
        // YAML
        (AgentKind::Hermes, "yaml") | (AgentKind::HermesFlash, "yaml") => {
            merge_hermes(before, request)
        }
        // TOML（codex config.toml）
        (AgentKind::Codex, "toml") => merge_codex_toml(before, request),
        // codex auth.json
        (AgentKind::Codex, "json") => merge_codex_auth(before, request),
        _ => Err(SetupError::UnsupportedFormat(agent.id())),
    }
}

/// codex catalog（模型目录 JSON）：models 列表加 auto。
pub fn merge_codex_catalog(before: &str, request: &SetupRequest) -> Result<String, SetupError> {
    let mut root = parse_json(before)?;
    let models = root
        .as_object_mut()
        .ok_or_else(parse_error)?
        .entry("models".to_string())
        .or_insert_with(|| Value::Array(Vec::new()));
    if !models.is_array() {
        return Err(parse_error_msg("catalog `models` 必须是数组"));
    }
    let list = models.as_array_mut().expect("checked array");
    if !list
        .iter()
        .any(|m| m.get("id").and_then(|id| id.as_str()) == Some(&request.model))
    {
        list.push(serde_json::json!({
            "id": request.model,
            "provider": PROVIDER_KEY,
        }));
    }
    Ok(to_json_pretty(&root))
}

// ---------------------------------------------------------------------------
// openclaw：models.providers["token-router"]；仅未设置时改默认模型
// ---------------------------------------------------------------------------

fn merge_openclaw(before: &str, request: &SetupRequest) -> Result<String, SetupError> {
    let mut root = parse_json(before)?;
    let root_obj = root.as_object_mut().ok_or_else(parse_error)?;

    let models = root_obj
        .entry("models".to_string())
        .or_insert_with(|| Value::Object(Map::new()));
    let models_obj = models.as_object_mut().ok_or_else(parse_error)?;

    let providers = models_obj
        .entry("providers".to_string())
        .or_insert_with(|| Value::Object(Map::new()));
    let providers_obj = providers.as_object_mut().ok_or_else(parse_error)?;

    providers_obj.insert(
        PROVIDER_KEY.to_string(),
        serde_json::json!({
            "baseUrl": format!("{}/v1", request.gateway_url.trim_end_matches('/')),
            "apiKey": request.credential_ref,
            "models": { request.model.clone(): {} },
        }),
    );

    // 仅在未设置时改默认模型（幂等：不覆盖用户选择）。
    if let Some(agents) = root_obj.get_mut("agents").and_then(|v| v.as_object_mut()) {
        if let Some(defaults) = agents.get_mut("defaults").and_then(|v| v.as_object_mut()) {
            if request.force_model
                || defaults
                    .get("model")
                    .and_then(|m| m.get("primary"))
                    .is_none()
            {
                defaults.insert(
                    "model".to_string(),
                    serde_json::json!({ "primary": request.model }),
                );
            }
        }
    }

    Ok(to_json_pretty(&root))
}

// ---------------------------------------------------------------------------
// claude-code：env 合并 4 个键
// ---------------------------------------------------------------------------

fn merge_claude_code(before: &str, request: &SetupRequest) -> Result<String, SetupError> {
    let mut root = parse_json(before)?;
    let root_obj = root.as_object_mut().ok_or_else(parse_error)?;
    let env = root_obj
        .entry("env".to_string())
        .or_insert_with(|| Value::Object(Map::new()));
    let env_obj = env.as_object_mut().ok_or_else(parse_error)?;
    env_obj.insert(
        "ANTHROPIC_BASE_URL".to_string(),
        Value::String(format!(
            "{}/anthropic",
            request.gateway_url.trim_end_matches('/')
        )),
    );
    env_obj.insert(
        "ANTHROPIC_AUTH_TOKEN".to_string(),
        Value::String(request.credential_ref.clone()),
    );
    if request.force_model {
        env_obj.insert(
            "ANTHROPIC_MODEL".to_string(),
            Value::String(request.model.clone()),
        );
        env_obj.insert(
            "ANTHROPIC_SMALL_FAST_MODEL".to_string(),
            Value::String(request.model.clone()),
        );
    } else {
        env_obj
            .entry("ANTHROPIC_MODEL".to_string())
            .or_insert_with(|| Value::String(request.model.clone()));
        env_obj
            .entry("ANTHROPIC_SMALL_FAST_MODEL".to_string())
            .or_insert_with(|| Value::String(request.model.clone()));
    }
    Ok(to_json_pretty(&root))
}

// ---------------------------------------------------------------------------
// opencode：provider["token-router"]；默认不强制根 model，显式切换时写入
// ---------------------------------------------------------------------------

fn merge_opencode(before: &str, request: &SetupRequest) -> Result<String, SetupError> {
    let mut root = parse_json(before)?;
    let root_obj = root.as_object_mut().ok_or_else(parse_error)?;
    let provider = root_obj
        .entry("provider".to_string())
        .or_insert_with(|| Value::Object(Map::new()));
    let provider_obj = provider.as_object_mut().ok_or_else(parse_error)?;
    provider_obj.insert(
        PROVIDER_KEY.to_string(),
        serde_json::json!({
            "baseURL": format!("{}/v1", request.gateway_url.trim_end_matches('/')),
            "apiKey": request.credential_ref,
            "models": { request.model.clone(): {} },
        }),
    );
    if request.force_model {
        root_obj.insert(
            "model".to_string(),
            Value::String(format!("{PROVIDER_KEY}/{}", request.model)),
        );
    }
    Ok(to_json_pretty(&root))
}

// ---------------------------------------------------------------------------
// codebuddy / workbuddy：models[] 追加/更新 id=auto
// ---------------------------------------------------------------------------

fn merge_buddy(before: &str, request: &SetupRequest) -> Result<String, SetupError> {
    let mut root = parse_json(before)?;
    let root_obj = root.as_object_mut().ok_or_else(parse_error)?;
    let models = root_obj
        .entry("models".to_string())
        .or_insert_with(|| Value::Array(Vec::new()));
    let list = models.as_array_mut().ok_or_else(parse_error)?;

    let entry = serde_json::json!({
        "id": request.model,
        "url": format!("{}/v1/chat/completions", request.gateway_url.trim_end_matches('/')),
        "apiKey": request.credential_ref,
        "provider": PROVIDER_KEY,
        "capabilities": { "tools": true, "streaming": true },
    });
    if let Some(existing) = list
        .iter_mut()
        .find(|m| m.get("id").and_then(|id| id.as_str()) == Some(request.model.as_str()))
    {
        *existing = entry;
    } else {
        list.push(entry);
    }
    if request.force_model
        && let Some(position) = list
            .iter()
            .position(|m| m.get("id").and_then(|id| id.as_str()) == Some(request.model.as_str()))
        && position > 0
    {
        let selected = list.remove(position);
        list.insert(0, selected);
    }
    Ok(to_json_pretty(&root))
}

// ---------------------------------------------------------------------------
// hermes：YAML providers["token-router"]；仅未选择时改 model
// ---------------------------------------------------------------------------

fn merge_hermes(before: &str, request: &SetupRequest) -> Result<String, SetupError> {
    let mut root: Value = if before.trim().is_empty() {
        Value::Object(Map::new())
    } else {
        serde_yaml::from_str(before).map_err(|error| SetupError::Parse(error.to_string()))?
    };
    let root_obj = root.as_object_mut().ok_or_else(parse_error)?;

    let providers = root_obj
        .entry("providers".to_string())
        .or_insert_with(|| Value::Object(Map::new()));
    let providers_obj = providers.as_object_mut().ok_or_else(parse_error)?;
    providers_obj.insert(
        PROVIDER_KEY.to_string(),
        serde_json::json!({
            "api": format!("{}/v1", request.gateway_url.trim_end_matches('/')),
            "api_key": request.credential_ref,
            "default_model": request.model,
        }),
    );

    // 显式切换时覆盖；否则仅在未选择时补 model（幂等）。
    if request.force_model || !root_obj.contains_key("model") {
        root_obj.insert("model".to_string(), Value::String(request.model.clone()));
    }

    serde_yaml::to_string(&root).map_err(|error| SetupError::Parse(error.to_string()))
}

// ---------------------------------------------------------------------------
// codex：toml_edit 保留注释 + auth.json
// ---------------------------------------------------------------------------

fn merge_codex_toml(before: &str, request: &SetupRequest) -> Result<String, SetupError> {
    let mut doc = if before.trim().is_empty() {
        toml_edit::DocumentMut::new()
    } else {
        before
            .parse::<toml_edit::DocumentMut>()
            .map_err(|error| SetupError::Parse(error.to_string()))?
    };

    // model_providers.token-router
    let providers = doc
        .entry("model_providers")
        .or_insert(toml_edit::Item::Table(toml_edit::Table::new()))
        .as_table_mut()
        .ok_or_else(|| parse_error_msg("model_providers 必须是表"))?;
    providers.set_implicit(true);
    let mut provider_table = toml_edit::Table::new();
    provider_table["base_url"] =
        toml_edit::value(format!("{}/v1", request.gateway_url.trim_end_matches('/')));
    provider_table["api_key_ref"] = toml_edit::value(request.credential_ref.clone());
    provider_table["wire_api"] = toml_edit::value("responses");
    providers.insert(PROVIDER_KEY, toml_edit::Item::Table(provider_table));

    // 显式切换时覆盖；否则顶层 model 仅未设置时写（幂等）。
    if request.force_model || doc.get("model").is_none() {
        doc["model"] = toml_edit::value(request.model.clone());
    }

    Ok(doc.to_string())
}

fn merge_codex_auth(before: &str, request: &SetupRequest) -> Result<String, SetupError> {
    let mut root = parse_json(before)?;
    let root_obj = root.as_object_mut().ok_or_else(parse_error)?;
    root_obj.insert(
        "OPENAI_API_KEY".to_string(),
        Value::String(request.credential_ref.clone()),
    );
    Ok(to_json_pretty(&root))
}

// ---------------------------------------------------------------------------
// helpers
// ---------------------------------------------------------------------------

fn parse_json(before: &str) -> Result<Value, SetupError> {
    if before.trim().is_empty() {
        return Ok(Value::Object(Map::new()));
    }
    serde_json::from_str(before).map_err(|error| SetupError::Parse(error.to_string()))
}

fn parse_error() -> SetupError {
    SetupError::Parse("配置根节点必须是对象".to_string())
}

fn parse_error_msg(message: &str) -> SetupError {
    SetupError::Parse(message.to_string())
}

/// 序列化：serde_json pretty（2 空格缩进），末尾补换行。
fn to_json_pretty(root: &Value) -> String {
    let mut out = serde_json::to_string_pretty(root).unwrap_or_default();
    out.push('\n');
    out
}
