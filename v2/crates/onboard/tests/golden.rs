//! golden fixture 驱动的合并器测试（P4-072）。
//!
//! 每个 Agent 一个 before fixture；断言合并后行为（幂等、不覆盖用户配置、
//! toml_edit 保留注释）。after 不是逐字节 golden（时间戳/格式无关），
//! 而是结构断言 —— fixture 描述"用户既有配置"，测试描述"合并语义"。

use std::path::PathBuf;

use tr_onboard::{AgentKind, SetupError, SetupRequest, merge::PROVIDER_KEY, merge::merge_document};

fn fixture(name: &str) -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name);
    std::fs::read_to_string(&path).unwrap_or_else(|error| panic!("fixture {name}: {error}"))
}

fn request() -> SetupRequest {
    SetupRequest {
        gateway_url: "http://127.0.0.1:17621".to_string(),
        credential_ref: "env:TR_TOKEN".to_string(),
        model: "auto".to_string(),
        force_model: false,
    }
}

fn json(text: &str) -> serde_json::Value {
    serde_json::from_str(text).expect("valid merged json")
}

// ---------------------------------------------------------------------------
// openclaw
// ---------------------------------------------------------------------------

#[test]
fn openclaw_merges_provider_and_keeps_user_config() {
    let before = fixture("openclaw.before.json");
    let after = merge_document(AgentKind::Openclaw, "json", &before, &request()).unwrap();
    let value = json(&after);

    // provider 块写入。
    let provider = &value["models"]["providers"][PROVIDER_KEY];
    assert_eq!(provider["baseUrl"], "http://127.0.0.1:17621/v1");
    assert_eq!(provider["apiKey"], "env:TR_TOKEN");
    assert_eq!(provider["models"]["auto"], serde_json::json!({}));

    // 用户既有配置保留：默认模型不被覆盖（已设置）。
    assert_eq!(
        value["agents"]["defaults"]["model"]["primary"],
        "my-favorite-model"
    );
    assert_eq!(value["agents"]["defaults"]["theme"], "dark");
    assert_eq!(value["mcpServers"]["existing"]["command"], "uvx");
}

#[test]
fn openclaw_sets_default_model_only_when_unset() {
    let before = r#"{"agents": {"defaults": {}}}"#;
    let after = merge_document(AgentKind::Openclaw, "json", before, &request()).unwrap();
    let value = json(&after);
    assert_eq!(value["agents"]["defaults"]["model"]["primary"], "auto");
}

#[test]
fn openclaw_is_idempotent() {
    let before = fixture("openclaw.before.json");
    let once = merge_document(AgentKind::Openclaw, "json", &before, &request()).unwrap();
    let twice = merge_document(AgentKind::Openclaw, "json", &once, &request()).unwrap();
    // 幂等：第二次合并不再改变内容（provider 值一致）。
    let first = json(&once)["models"]["providers"][PROVIDER_KEY].clone();
    let second = json(&twice)["models"]["providers"][PROVIDER_KEY].clone();
    assert_eq!(first, second);
}

// ---------------------------------------------------------------------------
// claude-code
// ---------------------------------------------------------------------------

#[test]
fn claude_code_merges_env_keys_and_keeps_user_env() {
    let before = fixture("claude-code.before.json");
    let after = merge_document(AgentKind::ClaudeCode, "json", &before, &request()).unwrap();
    let value = json(&after);

    assert_eq!(
        value["env"]["ANTHROPIC_BASE_URL"],
        "http://127.0.0.1:17621/anthropic"
    );
    assert_eq!(value["env"]["ANTHROPIC_AUTH_TOKEN"], "env:TR_TOKEN");
    // 用户 env 保留。
    assert_eq!(value["env"]["CUSTOM_FLAG"], "1");
    assert_eq!(value["permissions"]["allow"][0], "Bash(git *)");
}

// ---------------------------------------------------------------------------
// opencode
// ---------------------------------------------------------------------------

#[test]
fn opencode_merges_provider_without_forcing_root_model() {
    let before = fixture("opencode.before.json");
    let after = merge_document(AgentKind::Opencode, "json", &before, &request()).unwrap();
    let value = json(&after);

    let provider = &value["provider"][PROVIDER_KEY];
    assert_eq!(provider["baseURL"], "http://127.0.0.1:17621/v1");
    // 用户既有 provider 保留。
    assert_eq!(
        value["provider"]["existing-provider"]["baseURL"],
        "https://api.example.com/v1"
    );
    assert_eq!(value["keybinds"]["leader"], "ctrl-a");
    // 不强制根 model。
    assert!(value.get("model").is_none());
}

// ---------------------------------------------------------------------------
// codebuddy / workbuddy
// ---------------------------------------------------------------------------

#[test]
fn buddy_appends_auto_model_and_keeps_existing() {
    let before = fixture("codebuddy.before.json");
    let after = merge_document(AgentKind::Codebuddy, "json", &before, &request()).unwrap();
    let value = json(&after);

    let models = value["models"].as_array().expect("models array");
    assert_eq!(models.len(), 2, "existing + appended");
    assert_eq!(models[0]["id"], "legacy", "existing entry untouched");
    let auto = models
        .iter()
        .find(|m| m["id"] == "auto")
        .expect("auto appended");
    assert_eq!(auto["url"], "http://127.0.0.1:17621/v1/chat/completions");
    assert_eq!(auto["capabilities"]["streaming"], true);
    assert_eq!(value["settings"]["language"], "zh");
}

#[test]
fn buddy_updates_existing_auto_entry() {
    let before = r#"{"models": [{"id": "auto", "url": "http://stale:1/v1/chat/completions"}]}"#;
    let after = merge_document(AgentKind::Codebuddy, "json", before, &request()).unwrap();
    let value = json(&after);
    let models = value["models"].as_array().unwrap();
    assert_eq!(models.len(), 1, "updated in place, not appended");
    assert_eq!(
        models[0]["url"],
        "http://127.0.0.1:17621/v1/chat/completions"
    );
}

// ---------------------------------------------------------------------------
// hermes（YAML）
// ---------------------------------------------------------------------------

#[test]
fn hermes_merges_yaml_provider_and_keeps_user_model() {
    let before = fixture("hermes.before.yaml");
    let after = merge_document(AgentKind::Hermes, "yaml", &before, &request()).unwrap();
    let value: serde_json::Value = serde_yaml::from_str(&after).expect("valid merged yaml");

    let provider = &value["providers"][PROVIDER_KEY];
    assert_eq!(provider["api"], "http://127.0.0.1:17621/v1");
    assert_eq!(provider["api_key"], "env:TR_TOKEN");
    assert_eq!(provider["default_model"], "auto");

    // 用户已选择 model：不覆盖。
    assert_eq!(value["model"], "my-favorite-model");
    // 用户 providers/plugins 保留。
    assert_eq!(
        value["providers"]["existing-provider"]["api"],
        "https://api.example.com/v1"
    );
    assert_eq!(value["plugins"][0], "lint");
}

#[test]
fn hermes_sets_model_only_when_unset() {
    let before = "providers: {}\n";
    let after = merge_document(AgentKind::Hermes, "yaml", before, &request()).unwrap();
    let value: serde_json::Value = serde_yaml::from_str(&after).unwrap();
    assert_eq!(value["model"], "auto");
}

// ---------------------------------------------------------------------------
// codex（TOML 保留注释 + auth.json）
// ---------------------------------------------------------------------------

#[test]
fn codex_toml_preserves_comments_and_user_tables() {
    let before = fixture("codex.before.toml");
    let after = merge_document(AgentKind::Codex, "toml", &before, &request()).unwrap();

    // 注释保留（toml_edit 格式保留）。
    assert!(after.contains("# 用户自定义注释行"));
    assert!(after.contains("# Codex 配置（注释必须保留）"));
    // 用户表保留。
    assert!(after.contains("[mcp_servers.docs]"));
    assert!(after.contains("[model_providers.openai]"));
    assert!(after.contains("model_reasoning_effort = \"high\""));

    // 注释必须保留的 JSON 无注释场景由 toml 覆盖；provider 块写入：
    assert!(after.contains("[model_providers.token-router]"));
    assert!(after.contains("base_url = \"http://127.0.0.1:17621/v1\""));
    assert!(after.contains("api_key_ref = \"env:TR_TOKEN\""));
    assert!(after.contains("wire_api = \"responses\""));
    // 用户顶层 model 已设置：不覆盖。
    assert!(after.contains("model = \"gpt-5-codex\""));
}

#[test]
fn codex_toml_sets_model_when_unset() {
    let before = "# only a comment\n";
    let after = merge_document(AgentKind::Codex, "toml", before, &request()).unwrap();
    assert!(after.contains("model = \"auto\""));
    assert!(after.contains("# only a comment"));
}

#[test]
fn codex_auth_json_writes_key() {
    let before = fixture("codex-auth.before.json");
    let after = merge_document(AgentKind::Codex, "json", &before, &request()).unwrap();
    let value = json(&after);
    assert_eq!(value["OPENAI_API_KEY"], "env:TR_TOKEN");
    assert_eq!(value["tokens"]["existing"], "keep");
}

#[test]
fn codex_catalog_appends_auto() {
    let before = r#"{"models": [{"id": "existing-model"}]}"#;
    let after = tr_onboard::merge::merge_codex_catalog(before, &request()).unwrap();
    let value = json(&after);
    let models = value["models"].as_array().unwrap();
    assert_eq!(models.len(), 2);
    assert_eq!(models[1]["id"], "auto");
}

// ---------------------------------------------------------------------------
// 语义守卫：解析错误不半写
// ---------------------------------------------------------------------------

#[test]
fn malformed_json_returns_parse_error() {
    let result = merge_document(AgentKind::ClaudeCode, "json", "{not json", &request());
    assert!(matches!(result, Err(SetupError::Parse(_))));
}

#[test]
fn malformed_yaml_returns_parse_error() {
    let result = merge_document(AgentKind::Hermes, "yaml", "a: [unclosed", &request());
    assert!(matches!(result, Err(SetupError::Parse(_))));
}

#[test]
fn malformed_toml_returns_parse_error() {
    let result = merge_document(AgentKind::Codex, "toml", "key = [unclosed", &request());
    assert!(matches!(result, Err(SetupError::Parse(_))));
}
