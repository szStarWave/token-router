use serde_json::{json, Map, Value};

use crate::gateway::api::compat::reasoning::{infer_reasoning_config, map_reasoning_effort};

const ANTHROPIC_BILLING_HEADER_PREFIX: &str = "x-anthropic-billing-header:";

pub const ANTHROPIC_REDACTED_THINKING_PLACEHOLDER: &str = "[redacted thinking]";

/// Strip only a leading Claude Code billing attribution line from system text.
pub fn strip_leading_anthropic_billing_header(text: &str) -> &str {
    if !text.starts_with(ANTHROPIC_BILLING_HEADER_PREFIX) {
        return text;
    }

    let Some(line_end) = text
        .as_bytes()
        .iter()
        .position(|byte| *byte == b'\n' || *byte == b'\r')
    else {
        return "";
    };

    let bytes = text.as_bytes();
    let mut rest_start = line_end + 1;
    if bytes[line_end] == b'\r' && bytes.get(line_end + 1) == Some(&b'\n') {
        rest_start += 1;
    }

    let rest = &text[rest_start..];
    if let Some(stripped) = rest.strip_prefix("\r\n") {
        stripped
    } else if let Some(stripped) = rest.strip_prefix('\n') {
        stripped
    } else if let Some(stripped) = rest.strip_prefix('\r') {
        stripped
    } else {
        rest
    }
}

/// Map Anthropic `output_config.effort` / `thinking` to OpenAI-style effort strings.
pub fn resolve_anthropic_reasoning_effort(body: &Value) -> Option<&'static str> {
    if let Some(effort) = body.pointer("/output_config/effort").and_then(|v| v.as_str()) {
        return match effort {
            "low" => Some("low"),
            "medium" => Some("medium"),
            "high" => Some("high"),
            "max" => Some("xhigh"),
            _ => None,
        };
    }

    let thinking = body.get("thinking")?;
    match thinking.get("type").and_then(|t| t.as_str()) {
        Some("adaptive") => Some("xhigh"),
        Some("enabled") => {
            let budget = thinking.get("budget_tokens").and_then(|b| b.as_u64());
            match budget {
                Some(b) if b < 4_000 => Some("low"),
                Some(b) if b < 16_000 => Some("medium"),
                Some(_) => Some("high"),
                None => Some("high"),
            }
        }
        _ => None,
    }
}

fn anthropic_thinking_enabled(body: &Value) -> bool {
    match body.get("thinking").and_then(|t| t.get("type")).and_then(|v| v.as_str()) {
        Some("enabled" | "adaptive") => true,
        Some("disabled") => false,
        _ => body.get("thinking").is_some(),
    }
}

/// Apply DeepSeek/Kimi-style thinking params from an Anthropic request body.
pub fn apply_anthropic_upstream_options(
    result: &mut Map<String, Value>,
    body: &Value,
    model: &str,
    upstream_base: Option<&str>,
) {
    let Some(config) = infer_reasoning_config(model, upstream_base) else {
        return;
    };
    if !config.supports_thinking {
        return;
    }

    let reasoning_enabled = anthropic_thinking_enabled(body);
    if let Some(param) = config.thinking_param.as_deref() {
        match param {
            "thinking" => {
                result.insert(
                    "thinking".into(),
                    json!({"type": if reasoning_enabled { "enabled" } else { "disabled" }}),
                );
            }
            "enable_thinking" => {
                result.insert("enable_thinking".into(), json!(reasoning_enabled));
            }
            "reasoning_split" => {
                result.insert("reasoning_split".into(), json!(reasoning_enabled));
            }
            _ => {}
        }
    }

    let Some(effort) = resolve_anthropic_reasoning_effort(body) else {
        return;
    };
    let Some(mapped) = map_reasoning_effort(effort, config.effort_value_mode.as_deref()) else {
        return;
    };
    match config.effort_param.as_deref() {
        Some("reasoning_effort") => {
            result.insert("reasoning_effort".into(), json!(mapped));
        }
        Some("reasoning.effort") => {
            result.insert("reasoning".into(), json!({"effort": mapped}));
        }
        _ => {}
    }
}

pub fn inject_stream_include_usage(result: &mut Map<String, Value>) {
    if result.get("stream").and_then(|v| v.as_bool()) != Some(true) {
        return;
    }
    result.insert("stream_options".into(), json!({"include_usage": true}));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_leading_billing_header_only() {
        let text = "x-anthropic-billing-header: cch=abc\n\nYou are helpful.";
        assert_eq!(
            strip_leading_anthropic_billing_header(text),
            "You are helpful."
        );
    }

    #[test]
    fn maps_thinking_enabled_to_deepseek_params() {
        let body = json!({
            "thinking": {"type": "enabled", "budget_tokens": 8000},
            "model": "deepseek-v4-flash"
        });
        let mut map = Map::new();
        map.insert("model".into(), json!("deepseek-v4-flash"));
        apply_anthropic_upstream_options(&mut map, &body, "deepseek-v4-flash", Some("https://api.deepseek.com"));
        assert_eq!(map.get("thinking").and_then(|v| v.get("type")), Some(&json!("enabled")));
        assert_eq!(map.get("reasoning_effort").and_then(|v| v.as_str()), Some("high"));
    }
}
