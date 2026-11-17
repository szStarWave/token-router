//! 内置厂商模板库（v3 P5-A）。
//!
//! 纯数据，零 IO。模板只给默认值（base_url / 目录端点 / 价格 / 档位），
//! 用户实例化后一切以 `providers` 表为准。

use tr_contracts::{PriceDto, ProviderKindDto, ProviderTemplate};

/// micros / MTok 简写：`per_mtok(2.5) == 2.5 美元 / 百万 token`。
const fn usd_per_mtok(dollars: u64) -> u64 {
    dollars * 1_000_000
}

/// 内置模板（id 稳定，前端依赖）。
pub fn builtin_templates() -> Vec<ProviderTemplate> {
    vec![
        ProviderTemplate {
            id: "deepseek".into(),
            display_name: "DeepSeek".into(),
            kind: ProviderKindDto::OpenAiCompatible,
            base_url: "https://api.deepseek.com/v1".into(),
            catalog_url: Some("https://api.deepseek.com/v1/models".into()),
            default_price: Some(PriceDto {
                currency: "USD".into(),
                input_micros_per_mtok: usd_per_mtok(0),
                output_micros_per_mtok: usd_per_mtok(0),
                cache_read_micros_per_mtok: None,
                cache_write_micros_per_mtok: None,
                request_micros: 0,
            }),
            default_window: Some(128_000),
            suggested_tier: "cloud".into(),
        },
        ProviderTemplate {
            id: "openai".into(),
            display_name: "OpenAI".into(),
            kind: ProviderKindDto::OpenAiCompatible,
            base_url: "https://api.openai.com/v1".into(),
            catalog_url: Some("https://api.openai.com/v1/models".into()),
            default_price: None,
            default_window: Some(128_000),
            suggested_tier: "cloud".into(),
        },
        ProviderTemplate {
            id: "anthropic".into(),
            display_name: "Anthropic".into(),
            kind: ProviderKindDto::Anthropic,
            base_url: "https://api.anthropic.com".into(),
            catalog_url: Some("https://api.anthropic.com/v1/models".into()),
            default_price: None,
            default_window: Some(200_000),
            suggested_tier: "cloud".into(),
        },
        ProviderTemplate {
            id: "kimi".into(),
            display_name: "Moonshot Kimi".into(),
            kind: ProviderKindDto::OpenAiCompatible,
            base_url: "https://api.moonshot.cn/v1".into(),
            catalog_url: Some("https://api.moonshot.cn/v1/models".into()),
            default_price: None,
            default_window: Some(128_000),
            suggested_tier: "cloud".into(),
        },
        ProviderTemplate {
            id: "zhipu-glm".into(),
            display_name: "智谱 GLM".into(),
            kind: ProviderKindDto::OpenAiCompatible,
            base_url: "https://open.bigmodel.cn/api/paas/v4".into(),
            catalog_url: Some("https://open.bigmodel.cn/api/paas/v4/models".into()),
            default_price: None,
            default_window: Some(128_000),
            suggested_tier: "cloud".into(),
        },
        ProviderTemplate {
            id: "qwen".into(),
            display_name: "阿里云百炼 Qwen".into(),
            kind: ProviderKindDto::OpenAiCompatible,
            base_url: "https://dashscope.aliyuncs.com/compatible-mode/v1".into(),
            catalog_url: Some("https://dashscope.aliyuncs.com/compatible-mode/v1/models".into()),
            default_price: None,
            default_window: Some(128_000),
            suggested_tier: "cloud".into(),
        },
        ProviderTemplate {
            id: "openrouter".into(),
            display_name: "OpenRouter".into(),
            kind: ProviderKindDto::OpenAiCompatible,
            base_url: "https://openrouter.ai/api/v1".into(),
            catalog_url: Some("https://openrouter.ai/api/v1/models".into()),
            default_price: None,
            default_window: Some(128_000),
            suggested_tier: "cloud".into(),
        },
        ProviderTemplate {
            id: "groq".into(),
            display_name: "Groq".into(),
            kind: ProviderKindDto::OpenAiCompatible,
            base_url: "https://api.groq.com/openai/v1".into(),
            catalog_url: Some("https://api.groq.com/openai/v1/models".into()),
            default_price: None,
            default_window: Some(128_000),
            suggested_tier: "cloud".into(),
        },
        ProviderTemplate {
            id: "xai".into(),
            display_name: "xAI Grok".into(),
            kind: ProviderKindDto::OpenAiCompatible,
            base_url: "https://api.x.ai/v1".into(),
            catalog_url: Some("https://api.x.ai/v1/models".into()),
            default_price: None,
            default_window: Some(131_072),
            suggested_tier: "cloud".into(),
        },
        ProviderTemplate {
            id: "siliconflow".into(),
            display_name: "硅基流动 SiliconFlow".into(),
            kind: ProviderKindDto::OpenAiCompatible,
            base_url: "https://api.siliconflow.cn/v1".into(),
            catalog_url: Some("https://api.siliconflow.cn/v1/models".into()),
            default_price: None,
            default_window: Some(128_000),
            suggested_tier: "cloud".into(),
        },
        ProviderTemplate {
            id: "volces-ark".into(),
            display_name: "火山方舟 Ark".into(),
            kind: ProviderKindDto::OpenAiCompatible,
            base_url: "https://ark.cn-beijing.volces.com/api/v3".into(),
            catalog_url: None,
            default_price: None,
            default_window: Some(128_000),
            suggested_tier: "cloud".into(),
        },
        ProviderTemplate {
            id: "minimax".into(),
            display_name: "MiniMax".into(),
            kind: ProviderKindDto::OpenAiCompatible,
            base_url: "https://api.minimax.chat/v1".into(),
            catalog_url: None,
            default_price: None,
            default_window: Some(128_000),
            suggested_tier: "cloud".into(),
        },
        ProviderTemplate {
            id: "stepfun".into(),
            display_name: "阶跃星辰 StepFun".into(),
            kind: ProviderKindDto::OpenAiCompatible,
            base_url: "https://api.stepfun.com/v1".into(),
            catalog_url: None,
            default_price: None,
            default_window: Some(128_000),
            suggested_tier: "cloud".into(),
        },
        ProviderTemplate {
            id: "mistral".into(),
            display_name: "Mistral".into(),
            kind: ProviderKindDto::OpenAiCompatible,
            base_url: "https://api.mistral.ai/v1".into(),
            catalog_url: Some("https://api.mistral.ai/v1/models".into()),
            default_price: None,
            default_window: Some(128_000),
            suggested_tier: "cloud".into(),
        },
        ProviderTemplate {
            id: "together".into(),
            display_name: "Together".into(),
            kind: ProviderKindDto::OpenAiCompatible,
            base_url: "https://api.together.xyz/v1".into(),
            catalog_url: Some("https://api.together.xyz/v1/models".into()),
            default_price: None,
            default_window: Some(128_000),
            suggested_tier: "cloud".into(),
        },
        ProviderTemplate {
            id: "fireworks".into(),
            display_name: "Fireworks".into(),
            kind: ProviderKindDto::OpenAiCompatible,
            base_url: "https://api.fireworks.ai/inference/v1".into(),
            catalog_url: Some("https://api.fireworks.ai/inference/v1/models".into()),
            default_price: None,
            default_window: Some(128_000),
            suggested_tier: "cloud".into(),
        },
        ProviderTemplate {
            id: "ollama".into(),
            display_name: "Ollama（本地）".into(),
            kind: ProviderKindDto::OpenAiCompatible,
            base_url: "http://127.0.0.1:11434/v1".into(),
            catalog_url: Some("http://127.0.0.1:11434/v1/models".into()),
            default_price: Some(PriceDto {
                currency: "USD".into(),
                input_micros_per_mtok: 0,
                output_micros_per_mtok: 0,
                cache_read_micros_per_mtok: None,
                cache_write_micros_per_mtok: None,
                request_micros: 0,
            }),
            default_window: Some(32_768),
            suggested_tier: "local".into(),
        },
        ProviderTemplate {
            id: "lmstudio".into(),
            display_name: "LM Studio（本地）".into(),
            kind: ProviderKindDto::OpenAiCompatible,
            base_url: "http://127.0.0.1:1234/v1".into(),
            catalog_url: Some("http://127.0.0.1:1234/v1/models".into()),
            default_price: Some(PriceDto {
                currency: "USD".into(),
                input_micros_per_mtok: 0,
                output_micros_per_mtok: 0,
                cache_read_micros_per_mtok: None,
                cache_write_micros_per_mtok: None,
                request_micros: 0,
            }),
            default_window: Some(32_768),
            suggested_tier: "local".into(),
        },
        ProviderTemplate {
            id: "custom-openai".into(),
            display_name: "自定义 OpenAI 兼容".into(),
            kind: ProviderKindDto::OpenAiCompatible,
            base_url: String::new(),
            catalog_url: None,
            default_price: None,
            default_window: Some(128_000),
            suggested_tier: "cloud".into(),
        },
        ProviderTemplate {
            id: "custom-anthropic".into(),
            display_name: "自定义 Anthropic 兼容".into(),
            kind: ProviderKindDto::Anthropic,
            base_url: String::new(),
            catalog_url: None,
            default_price: None,
            default_window: Some(200_000),
            suggested_tier: "cloud".into(),
        },
    ]
}

/// 按 id 查模板。
pub fn find_template(id: &str) -> Option<ProviderTemplate> {
    builtin_templates().into_iter().find(|t| t.id == id)
}

/// 候选编译时按 provider 推断 tier：模板 id 与 provider id 一致用其 suggested_tier，
/// 否则按 id 前缀（`ollama`/`lmstudio` 开头 = local）兜底 cloud。
pub fn find_template_id_default(provider_id: &str) -> String {
    if let Some(template) = find_template(provider_id) {
        return template.suggested_tier;
    }
    if provider_id.starts_with("ollama") || provider_id.starts_with("lmstudio") {
        return "local".to_string();
    }
    "cloud".to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn templates_have_unique_ids() {
        let templates = builtin_templates();
        assert!(templates.len() >= 20, "至少 20 个厂商模板");
        let mut ids: Vec<_> = templates.iter().map(|t| t.id.clone()).collect();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), templates.len(), "模板 id 必须唯一");
    }

    #[test]
    fn find_template_works() {
        assert!(find_template("deepseek").is_some());
        assert!(find_template("nope").is_none());
    }
}
