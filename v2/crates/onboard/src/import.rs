//! 从本机 agent 配置反向导入 provider/account 草稿（v3 P5-A，roadmap §0.3 吸收项）。
//!
//! 只读扫描：读取已装 agent（Claude Code / Codex / OpenCode 等）的配置文件，
//! 提取上游 base_url 与 key **引用**（env 名 / 文件路径），生成 Provider+Account 草稿。
//! 明文 key 不复制、不落库——能拿到引用就足够编译出可用候选。

use std::collections::BTreeMap;
use std::path::Path;

use serde::{Deserialize, Serialize};

/// 反向导入出的 provider 草稿（管理面落库前用户可编辑）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImportedProvider {
    /// 建议 display_name（来源 agent + host）。
    pub display_name: String,
    pub base_url: String,
    /// key 引用（`env:NAME` 或 `file:path`）；拿不到就为空（用户手填）。
    #[serde(default)]
    pub credential_ref: Option<String>,
    /// 来源说明（agent id + 配置文件）。
    pub source: String,
}

/// 扫描单个 home 目录下已知 agent 配置，收集上游端点。
///
/// 返回按 base_url 去重后的草稿；`home` 不存在返回空。
pub fn scan_home(home: &Path) -> Vec<ImportedProvider> {
    let mut found: BTreeMap<String, ImportedProvider> = BTreeMap::new();
    for (config_relative, format) in KNOWN_CONFIGS {
        let path = home.join(config_relative);
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        for extracted in extract_endpoints(&text, *format) {
            found
                .entry(extracted.base_url.clone())
                .or_insert(ImportedProvider {
                    display_name: extracted.display_name,
                    base_url: extracted.base_url,
                    credential_ref: extracted.credential_ref,
                    source: format!("{} ({config_relative})", extracted.agent),
                });
        }
    }
    found.into_values().collect()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ConfigFormat {
    Json,
    Toml,
    Yaml,
}

struct Extracted {
    agent: &'static str,
    display_name: String,
    base_url: String,
    credential_ref: Option<String>,
}

/// 已知 agent 配置文件与其承载上游端点的字段名。
const KNOWN_CONFIGS: &[(&str, ConfigFormat)] = &[
    (".claude/settings.json", ConfigFormat::Json),
    (".codex/config.toml", ConfigFormat::Toml),
    (".config/opencode/opencode.json", ConfigFormat::Json),
    (".openclaw/openclaw.json", ConfigFormat::Json),
    (".codebuddy/models.json", ConfigFormat::Json),
    (".workbuddy/models.json", ConfigFormat::Json),
    (".hermes/config.yaml", ConfigFormat::Yaml),
];

#[derive(Debug, Deserialize)]
struct GenericJsonConfig {
    #[serde(flatten)]
    fields: serde_json::Map<String, serde_json::Value>,
}

fn extract_endpoints(text: &str, format: ConfigFormat) -> Vec<Extracted> {
    match format {
        ConfigFormat::Json => extract_json(text),
        ConfigFormat::Toml => extract_toml(text),
        ConfigFormat::Yaml => extract_yaml(text),
    }
}

fn extract_json(text: &str) -> Vec<Extracted> {
    let Ok(config) = serde_json::from_str::<GenericJsonConfig>(text) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    collect_from_value(&serde_json::Value::Object(config.fields), None, &mut out, 0);
    out
}

/// 递归找 `base_url` / `baseUrl` 键；兄弟位置的 `api_key`/`apiKey`/`api_key_env` 作凭据引用。
fn collect_from_value(
    value: &serde_json::Value,
    agent: Option<&'static str>,
    out: &mut Vec<Extracted>,
    depth: usize,
) {
    const MAX_DEPTH: usize = 6;
    if depth > MAX_DEPTH {
        return;
    }
    match value {
        serde_json::Value::Object(map) => {
            let base_url = map
                .get("base_url")
                .or_else(|| map.get("baseUrl"))
                .and_then(|value| value.as_str())
                .filter(|url| url.starts_with("http"));
            let credential_ref = map
                .get("api_key_env")
                .or_else(|| map.get("apiKeyEnv"))
                .and_then(|value| value.as_str())
                .map(|name| format!("env:{name}"))
                .or_else(|| {
                    map.get("api_key")
                        .or_else(|| map.get("apiKey"))
                        .and_then(|value| value.as_str())
                        .filter(|key| looks_like_reference(key))
                        .map(str::to_string)
                });
            if let Some(base_url) = base_url {
                out.push(Extracted {
                    agent: agent.unwrap_or("agent"),
                    display_name: format!("导入（{}）", host_of(base_url)),
                    base_url: base_url.to_string(),
                    credential_ref: credential_ref.clone(),
                });
            }
            for value in map.values() {
                collect_from_value(value, agent, out, depth + 1);
            }
        }
        serde_json::Value::Array(items) => {
            for item in items {
                collect_from_value(item, agent, out, depth + 1);
            }
        }
        _ => {}
    }
}

fn extract_toml(text: &str) -> Vec<Extracted> {
    let Ok(value) = toml::from_str::<toml::Value>(text) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    collect_from_value(&value_to_json(&value), Some("codex"), &mut out, 0);
    out
}

fn extract_yaml(text: &str) -> Vec<Extracted> {
    // 轻量做法：不引入 serde_yaml，只在行级找 `base_url:`/`api_key_env:`。
    let mut base_url = None;
    let mut key_env = None;
    for line in text.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix("base_url:") {
            let value = rest.trim().trim_matches(['"', '\'']);
            if value.starts_with("http") {
                base_url = Some(value.to_string());
            }
        }
        if let Some(rest) = trimmed.strip_prefix("api_key_env:") {
            key_env = Some(format!("env:{}", rest.trim().trim_matches(['"', '\''])));
        }
    }
    match base_url {
        Some(base_url) => vec![Extracted {
            agent: "hermes",
            display_name: format!("导入（{}）", host_of(&base_url)),
            base_url,
            credential_ref: key_env,
        }],
        None => Vec::new(),
    }
}

fn value_to_json(value: &toml::Value) -> serde_json::Value {
    match value {
        toml::Value::String(text) => serde_json::Value::String(text.clone()),
        toml::Value::Integer(number) => serde_json::json!(number),
        toml::Value::Float(number) => serde_json::json!(number),
        toml::Value::Boolean(flag) => serde_json::Value::Bool(*flag),
        toml::Value::Datetime(text) => serde_json::Value::String(text.to_string()),
        toml::Value::Array(items) => {
            serde_json::Value::Array(items.iter().map(value_to_json).collect())
        }
        toml::Value::Table(map) => {
            let mut object = serde_json::Map::new();
            for (key, value) in map {
                object.insert(key.clone(), value_to_json(value));
            }
            serde_json::Value::Object(object)
        }
    }
}

/// key 看起来是引用而不是明文：env 名 / `file:` / `keyring:` 引用。
/// 明文 key（`sk-…` 等）一律**不**导入——只留空让用户手填。
fn looks_like_reference(key: &str) -> bool {
    if key.starts_with("file:") || key.starts_with("keyring:") || key.starts_with("env:") {
        return true;
    }
    // 全大写下划线 = env 名约定。
    !key.is_empty()
        && key
            .chars()
            .all(|c| c.is_ascii_uppercase() || c == '_' || c.is_ascii_digit())
}

fn host_of(base_url: &str) -> String {
    base_url
        .trim_start_matches("http://")
        .trim_start_matches("https://")
        .split('/')
        .next()
        .unwrap_or(base_url)
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_config_with_env_reference() {
        let text = r#"{
            "providers": {
                "deepseek": {
                    "base_url": "https://api.deepseek.com/v1",
                    "api_key_env": "DEEPSEEK_API_KEY"
                }
            }
        }"#;
        let out = extract_json(text);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].base_url, "https://api.deepseek.com/v1");
        assert_eq!(
            out[0].credential_ref.as_deref(),
            Some("env:DEEPSEEK_API_KEY")
        );
    }

    #[test]
    fn plaintext_key_is_never_imported() {
        let text = r#"{
            "p": { "base_url": "https://x.example.com/v1", "api_key": "sk-live-abcdef123456" }
        }"#;
        let out = extract_json(text);
        assert_eq!(out.len(), 1);
        assert!(out[0].credential_ref.is_none(), "明文 key 不得进入草稿");
    }

    #[test]
    fn toml_codex_style() {
        let text = r#"
[model_providers.custom]
base_url = "https://api.custom.com/v1"
api_key_env = "CUSTOM_API_KEY"
"#;
        let out = extract_toml(text);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].base_url, "https://api.custom.com/v1");
        assert_eq!(out[0].credential_ref.as_deref(), Some("env:CUSTOM_API_KEY"));
    }

    #[test]
    fn yaml_hermes_style() {
        let text = "base_url: \"https://api.hermes.dev/v1\"\napi_key_env: HERMES_KEY\n";
        let out = extract_yaml(text);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].credential_ref.as_deref(), Some("env:HERMES_KEY"));
    }

    #[test]
    fn scan_home_dedupes_by_base_url() {
        let home = std::env::temp_dir().join(format!("tr2-import-{}", std::process::id()));
        std::fs::create_dir_all(home.join(".claude")).unwrap();
        std::fs::write(
            home.join(".claude").join("settings.json"),
            r#"{ "p": { "base_url": "https://same.example.com/v1", "api_key_env": "K1" } }"#,
        )
        .unwrap();
        let out = scan_home(&home);
        assert_eq!(out.len(), 1);
        assert!(out[0].source.contains(".claude/settings.json"));
        let _ = std::fs::remove_dir_all(&home);
    }
}
