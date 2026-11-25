//! models.dev 目录元数据补全（推理档位 / 上下文窗口 / 输出上限 / 价格）。
//!
//! 设计约束：
//! - **只填空**：用户已经填过的字段（窗口/输出/价格/档位）一律不覆盖；
//! - 网络失败不影响目录本身：补全失败只记录日志，模型列表照常可用；
//! - `api.json` 落在数据目录并缓存 24h，避免每次刷新目录都下载 ~7MB；
//! - 供应商匹配依次尝试：provider id 前缀（如 `siliconflow-bccdda58` → `siliconflow`）、
//!   展示名、base_url host；匹配不到就返回 0，让用户手工填。
//!
//! 推理档位来自 models.dev 的 `reasoning_options[].values`（`effort` 类型），
//! 归一到我们支持的 5 档：low / medium / high / xhigh / max（`minimal`→`low`，
//! `none` 视为关闭、丢弃）；`toggle` / `budget_tokens` 不产生离散档位。

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::{Duration, SystemTime};

use serde::Deserialize;
use tr_contracts::{PriceDto, ProviderKindDto, ProviderTemplate};

use crate::error::AdminApiError;
use crate::provider_service;
use crate::state::ControlState;

/// models.dev 全量目录。
const API_URL: &str = "https://models.dev/api.json";
/// 本机缓存有效期。
const CACHE_TTL: Duration = Duration::from_secs(24 * 60 * 60);
/// 请求超时（目录约 7MB，给足时间）。
const FETCH_TIMEOUT: Duration = Duration::from_secs(30);
/// 我们支持的推理档位（升序）。
const SUPPORTED_LEVELS: &[&str] = &["low", "medium", "high", "xhigh", "max"];

#[derive(Debug, Deserialize)]
struct ProviderEntry {
    #[serde(default)]
    name: String,
    #[serde(default)]
    api: Option<String>,
    /// AI SDK 包名：决定协议（`@ai-sdk/openai-compatible` / `@ai-sdk/anthropic` …）。
    #[serde(default)]
    npm: Option<String>,
    #[serde(default)]
    models: BTreeMap<String, ModelEntry>,
}

#[derive(Debug, Default, Deserialize)]
struct ModelEntry {
    /// models.dev 的模型类型：chat / image / embedding / video / transcription / speech / reranking / realtime。
    #[serde(default, rename = "type")]
    kind: Option<String>,
    #[serde(default)]
    modalities: Option<Modalities>,
    #[serde(default)]
    reasoning_options: Vec<ReasoningOption>,
    #[serde(default)]
    limit: Option<Limit>,
    #[serde(default)]
    cost: Option<Cost>,
}

#[derive(Debug, Default, Deserialize)]
struct Modalities {
    #[serde(default)]
    input: Vec<String>,
    #[serde(default)]
    output: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct ReasoningOption {
    #[serde(default, rename = "type")]
    kind: String,
    #[serde(default)]
    values: Vec<String>,
}

#[derive(Debug, Default, Deserialize)]
struct Limit {
    #[serde(default)]
    context: Option<u64>,
    #[serde(default)]
    output: Option<u64>,
}

#[derive(Debug, Default, Deserialize)]
struct Cost {
    #[serde(default)]
    input: Option<f64>,
    #[serde(default)]
    output: Option<f64>,
    #[serde(default)]
    cache_read: Option<f64>,
    #[serde(default)]
    cache_write: Option<f64>,
}

/// 补全结果（给 UI 的反馈：匹配到哪个供应商、填了多少字段）。
#[derive(Debug, Default, Clone, serde::Serialize)]
pub struct EnrichReport {
    /// models.dev 里匹配到的供应商 slug。
    pub source_provider: Option<String>,
    /// 目录里有同名模型的个数。
    pub matched: u32,
    /// 实际补全了至少一个字段的模型个数。
    pub filled: u32,
}

/// 从 models.dev 补全某个供应商的目录元数据（只填空）。
pub async fn enrich_provider(
    state: &ControlState,
    provider_id: &str,
) -> Result<EnrichReport, AdminApiError> {
    let provider = provider_service::find_provider(state, provider_id)?;
    let catalog = provider_service::catalog(state, provider_id)?;
    let remote = load_catalog(state).await?;

    let Some((slug, entry)) = pick_provider(
        &remote,
        &provider.id,
        &provider.display_name,
        &provider.base_url,
    ) else {
        return Ok(EnrichReport::default());
    };

    let db = state.pipeline.db();
    let mut report = EnrichReport {
        source_provider: Some(slug.to_string()),
        ..EnrichReport::default()
    };
    for model in &catalog.models {
        let Some(remote_model) = entry.models.get(&model.model_id) else {
            continue;
        };
        report.matched += 1;

        let mut touched = false;
        let window = remote_model
            .limit
            .as_ref()
            .and_then(|limit| limit.context)
            .and_then(|value| u32::try_from(value).ok())
            .filter(|value| *value > 0);
        let max_output = remote_model
            .limit
            .as_ref()
            .and_then(|limit| limit.output)
            .and_then(|value| u32::try_from(value).ok())
            .filter(|value| *value > 0);

        // 只补空缺：已有值视为用户选择，不覆盖。
        let patch = tr_agent::store::providers::ModelMetadataPatch {
            purpose: None,
            caps: None,
            context_window: if model.context_window.is_none() {
                window
            } else {
                None
            },
            max_output: if model.max_output.is_none() {
                max_output
            } else {
                None
            },
            price: if model.price.is_none() {
                remote_model.cost.as_ref().map(price_from_cost)
            } else {
                None
            },
        };
        if patch.context_window.is_some() || patch.max_output.is_some() || patch.price.is_some() {
            touched |= tr_agent::store::providers::update_model_metadata(
                db,
                provider_id,
                &model.model_id,
                &patch,
            )?;
        }

        if model.thinking_levels.is_empty() {
            let levels = levels_from(remote_model);
            if !levels.is_empty() {
                touched |= tr_agent::store::providers::set_model_thinking_levels(
                    db,
                    provider_id,
                    &model.model_id,
                    &levels,
                )?;
            }
        }

        if touched {
            report.filled += 1;
            let _ = tr_agent::store::providers::set_model_source(
                db,
                provider_id,
                &model.model_id,
                "models.dev",
            );
        }
    }
    Ok(report)
}

/// 目录刷新 / 补全时用来过滤不可用于文本对话的模型（语音、图像生成、embedding…）。
///
/// 判定依据：
/// - `type` 明确为 image / embedding / video / transcription / speech / reranking / realtime；
/// - 或 `modalities.output` 非空且不含 `text`。
///
/// 没有 `type` / `modalities` 的条目视为对话模型（保持旧行为）。
pub async fn non_chat_models(
    state: &ControlState,
    provider_id: &str,
) -> Result<std::collections::BTreeSet<String>, AdminApiError> {
    let provider = provider_service::find_provider(state, provider_id)?;
    let remote = load_catalog(state).await?;
    let Some((_, entry)) = pick_provider(
        &remote,
        &provider.id,
        &provider.display_name,
        &provider.base_url,
    ) else {
        return Ok(std::collections::BTreeSet::new());
    };
    Ok(entry
        .models
        .iter()
        .filter(|(_, model)| !is_chat_capable(model))
        .map(|(id, _)| id.clone())
        .collect())
}

fn is_chat_capable(model: &ModelEntry) -> bool {
    if let Some(kind) = model.kind.as_deref() {
        if matches!(
            kind,
            "image" | "embedding" | "video" | "transcription" | "speech" | "reranking" | "realtime"
        ) {
            return false;
        }
    }
    match &model.modalities {
        Some(modalities) => {
            let text_out =
                modalities.output.is_empty() || modalities.output.iter().any(|item| item == "text");
            let text_in =
                modalities.input.is_empty() || modalities.input.iter().any(|item| item == "text");
            text_out && text_in
        }
        None => true,
    }
}

/// models.dev 供应商 → 添加供应商模板（仅内核支持的两类协议）。
///
/// 过滤规则：
/// - 必须有 `api`（base_url），否则无法直连；
/// - `npm` 必须是 OpenAI 兼容或 Anthropic SDK（Google/Bedrock 等协议内核不支持）；
/// - 目录端点按约定推导（OpenAI 兼容 `{base}/models`、Anthropic `{base}/v1/models`），
///   与 probe 的默认回退一致。
pub async fn provider_templates(
    state: &ControlState,
) -> Result<Vec<ProviderTemplate>, AdminApiError> {
    let providers = load_catalog(state).await?;
    let mut out = Vec::new();
    for (slug, entry) in &providers {
        let Some(base_url) = entry.api.as_deref().filter(|api| !api.is_empty()) else {
            continue;
        };
        let Some(kind) = kind_from_npm(entry.npm.as_deref()) else {
            continue;
        };
        let base = base_url.trim_end_matches('/');
        let catalog_url = match kind {
            ProviderKindDto::Anthropic => format!("{base}/v1/models"),
            ProviderKindDto::OpenAiCompatible | ProviderKindDto::Responses => {
                format!("{base}/models")
            }
        };
        out.push(ProviderTemplate {
            id: slug.clone(),
            display_name: if entry.name.is_empty() {
                slug.clone()
            } else {
                entry.name.clone()
            },
            kind,
            base_url: base.to_string(),
            catalog_url: Some(catalog_url),
            default_price: None,
            default_window: None,
            suggested_tier: "cloud".to_string(),
        });
    }
    out.sort_by_key(|template| template.display_name.to_lowercase());
    Ok(out)
}

fn kind_from_npm(npm: Option<&str>) -> Option<ProviderKindDto> {
    match npm? {
        "@ai-sdk/anthropic" => Some(ProviderKindDto::Anthropic),
        "@ai-sdk/openai" | "@ai-sdk/openai-compatible" => Some(ProviderKindDto::OpenAiCompatible),
        _ => None,
    }
}

/// 目录刷新后的尽力补全（网络失败只记日志，不阻断刷新）。
pub async fn enrich_after_refresh(state: &ControlState, provider_id: &str) {
    match enrich_provider(state, provider_id).await {
        Ok(report) if report.filled > 0 => {
            eprintln!(
                "models.dev enriched provider {provider_id}: {}/{} models (source {:?})",
                report.filled, report.matched, report.source_provider
            );
        }
        Ok(_) => {}
        Err(error) => eprintln!("models.dev enrich skipped for {provider_id}: {error:?}"),
    }
}

fn pick_provider<'a>(
    providers: &'a BTreeMap<String, ProviderEntry>,
    our_id: &str,
    display_name: &str,
    base_url: &str,
) -> Option<(&'a str, &'a ProviderEntry)> {
    // 1. slug 是 provider id 的前缀（最长匹配）：siliconflow-bccdda58 → siliconflow。
    let mut best: Option<&str> = None;
    for slug in providers.keys() {
        if our_id.starts_with(slug.as_str())
            && best.is_none_or(|current| slug.len() > current.len())
        {
            best = Some(slug.as_str());
        }
    }
    if let Some(slug) = best {
        if let Some((slug, entry)) = providers.get_key_value(slug) {
            return Some((slug.as_str(), entry));
        }
    }
    // 2. 展示名忽略大小写相等。
    for (slug, entry) in providers {
        if !entry.name.is_empty() && entry.name.eq_ignore_ascii_case(display_name) {
            return Some((slug, entry));
        }
    }
    // 3. base_url host 与 models.dev 的 api 地址同源。
    if let Some(host) = url_host(base_url) {
        for (slug, entry) in providers {
            if entry.api.as_deref().and_then(url_host).as_deref() == Some(host.as_str()) {
                return Some((slug, entry));
            }
        }
    }
    None
}

fn url_host(url: &str) -> Option<String> {
    let rest = url.split_once("://").map(|(_, rest)| rest).unwrap_or(url);
    let host = rest.split('/').next().unwrap_or_default();
    let host = host.rsplit('@').next().unwrap_or(host);
    let host = host.split(':').next().unwrap_or(host);
    (!host.is_empty()).then(|| host.to_ascii_lowercase())
}

fn price_from_cost(cost: &Cost) -> PriceDto {
    fn micros(value: Option<f64>) -> u64 {
        value
            .filter(|value| *value > 0.0)
            .map(|value| (value * 1_000_000.0).round() as u64)
            .unwrap_or(0)
    }
    PriceDto {
        currency: "USD".to_string(),
        input_micros_per_mtok: micros(cost.input),
        output_micros_per_mtok: micros(cost.output),
        cache_read_micros_per_mtok: cost
            .cache_read
            .filter(|value| *value > 0.0)
            .map(|value| (value * 1_000_000.0).round() as u64),
        cache_write_micros_per_mtok: cost
            .cache_write
            .filter(|value| *value > 0.0)
            .map(|value| (value * 1_000_000.0).round() as u64),
        request_micros: 0,
    }
}

/// `reasoning_options` → 支持的档位（有序、去重）。
fn levels_from(model: &ModelEntry) -> Vec<String> {
    let mut levels: Vec<String> = Vec::new();
    for option in &model.reasoning_options {
        if option.kind != "effort" {
            continue;
        }
        for value in &option.values {
            let normalized = match value.as_str() {
                "minimal" => "low",
                other => other,
            };
            if SUPPORTED_LEVELS.contains(&normalized)
                && !levels.iter().any(|item| item == normalized)
            {
                levels.push(normalized.to_string());
            }
        }
    }
    levels.sort_by_key(|level| {
        SUPPORTED_LEVELS
            .iter()
            .position(|item| *item == level.as_str())
            .unwrap_or(0)
    });
    levels
}

fn cache_path(state: &ControlState) -> Option<PathBuf> {
    state
        .data_dir
        .as_ref()
        .map(|dir| dir.join("modelsdev.json"))
}

async fn load_catalog(
    state: &ControlState,
) -> Result<BTreeMap<String, ProviderEntry>, AdminApiError> {
    if let Some(path) = cache_path(state) {
        if let Ok(metadata) = std::fs::metadata(&path) {
            let fresh = metadata
                .modified()
                .ok()
                .and_then(|modified| SystemTime::now().duration_since(modified).ok())
                .map(|age| age < CACHE_TTL)
                .unwrap_or(false);
            if fresh {
                if let Ok(text) = std::fs::read_to_string(&path) {
                    if let Ok(parsed) = serde_json::from_str(&text) {
                        return Ok(parsed);
                    }
                }
            }
        }
    }

    let client = reqwest::Client::builder()
        .timeout(FETCH_TIMEOUT)
        .build()
        .map_err(|error| AdminApiError::internal(format!("models.dev client: {error}")))?;
    let response = client.get(API_URL).send().await.map_err(|error| {
        AdminApiError::new(
            crate::error::ApiErrorType::ServerUnavailable,
            format!("models.dev unavailable: {error}"),
        )
    })?;
    if !response.status().is_success() {
        return Err(AdminApiError::new(
            crate::error::ApiErrorType::ServerUnavailable,
            format!("models.dev returned {}", response.status()),
        ));
    }
    let text = response
        .text()
        .await
        .map_err(|error| AdminApiError::internal(format!("models.dev body: {error}")))?;
    let parsed: BTreeMap<String, ProviderEntry> = serde_json::from_str(&text)
        .map_err(|error| AdminApiError::internal(format!("models.dev schema changed: {error}")))?;
    if let Some(path) = cache_path(state) {
        let _ = std::fs::write(&path, &text);
    }
    Ok(parsed)
}

/// 供测试：纯函数形态的匹配与档位派生。
#[cfg(test)]
mod tests {
    use super::*;

    fn provider(name: &str, api: &str) -> ProviderEntry {
        ProviderEntry {
            name: name.to_string(),
            api: Some(api.to_string()),
            npm: None,
            models: BTreeMap::new(),
        }
    }

    #[test]
    fn provider_matching_prefers_slug_prefix() {
        let mut providers = BTreeMap::new();
        providers.insert(
            "siliconflow-cn".to_string(),
            provider("SiliconFlow CN", "https://api.siliconflow.cn/v1"),
        );
        providers.insert(
            "siliconflow".to_string(),
            provider("SiliconFlow", "https://api.siliconflow.com/v1"),
        );
        let (slug, _) = pick_provider(
            &providers,
            "siliconflow-bccdda58",
            "SiliconFlow",
            "https://api.siliconflow.com/v1",
        )
        .expect("slug prefix");
        assert_eq!(slug, "siliconflow");
    }

    #[test]
    fn provider_matching_falls_back_to_host() {
        let mut providers = BTreeMap::new();
        providers.insert(
            "custom".to_string(),
            provider("Custom", "https://api.example.com/v1"),
        );
        let (slug, _) = pick_provider(
            &providers,
            "my-provider",
            "Unknown",
            "https://api.example.com/v1",
        )
        .expect("host match");
        assert_eq!(slug, "custom");
    }

    #[test]
    fn levels_normalize_and_keep_order() {
        let model = ModelEntry {
            reasoning_options: vec![ReasoningOption {
                kind: "effort".to_string(),
                values: vec![
                    "none".to_string(),
                    "minimal".to_string(),
                    "low".to_string(),
                    "medium".to_string(),
                    "high".to_string(),
                    "xhigh".to_string(),
                    "max".to_string(),
                ],
            }],
            ..ModelEntry::default()
        };
        assert_eq!(
            levels_from(&model),
            vec!["low", "medium", "high", "xhigh", "max"]
        );
    }

    #[test]
    fn levels_empty_for_toggle_or_budget() {
        let model = ModelEntry {
            reasoning_options: vec![
                ReasoningOption {
                    kind: "toggle".to_string(),
                    values: Vec::new(),
                },
                ReasoningOption {
                    kind: "budget_tokens".to_string(),
                    values: Vec::new(),
                },
            ],
            ..ModelEntry::default()
        };
        assert!(levels_from(&model).is_empty());
    }

    #[test]
    fn template_kind_only_supports_known_sdks() {
        assert!(matches!(
            kind_from_npm(Some("@ai-sdk/anthropic")),
            Some(ProviderKindDto::Anthropic)
        ));
        assert!(matches!(
            kind_from_npm(Some("@ai-sdk/openai-compatible")),
            Some(ProviderKindDto::OpenAiCompatible)
        ));
        assert!(matches!(
            kind_from_npm(Some("@ai-sdk/openai")),
            Some(ProviderKindDto::OpenAiCompatible)
        ));
        assert!(kind_from_npm(Some("@ai-sdk/google")).is_none());
        assert!(kind_from_npm(None).is_none());
    }

    #[test]
    fn cost_converts_to_micros() {
        let price = price_from_cost(&Cost {
            input: Some(0.75),
            output: Some(1.0),
            cache_read: Some(0.25),
            cache_write: None,
        });
        assert_eq!(price.input_micros_per_mtok, 750_000);
        assert_eq!(price.output_micros_per_mtok, 1_000_000);
        assert_eq!(price.cache_read_micros_per_mtok, Some(250_000));
        assert_eq!(price.cache_write_micros_per_mtok, None);
    }

    #[test]
    fn chat_capability_filters_audio_and_image_models() {
        let audio = ModelEntry {
            kind: Some("speech".to_string()),
            modalities: None,
            ..ModelEntry::default()
        };
        assert!(!is_chat_capable(&audio));

        let image_out = ModelEntry {
            modalities: Some(Modalities {
                input: vec!["text".to_string()],
                output: vec!["image".to_string()],
            }),
            ..ModelEntry::default()
        };
        assert!(!is_chat_capable(&image_out));

        let audio_in = ModelEntry {
            modalities: Some(Modalities {
                input: vec!["audio".to_string()],
                output: vec!["text".to_string()],
            }),
            ..ModelEntry::default()
        };
        assert!(!is_chat_capable(&audio_in));

        let vision_chat = ModelEntry {
            modalities: Some(Modalities {
                input: vec!["text".to_string(), "image".to_string()],
                output: vec!["text".to_string()],
            }),
            ..ModelEntry::default()
        };
        assert!(is_chat_capable(&vision_chat));

        let unknown = ModelEntry::default();
        assert!(is_chat_capable(&unknown));
    }
}
