use serde_json::{json, Map, Value};

use crate::gateway::api::openai::{Message, Role};

pub const TOOL_CALL_REASONING_PLACEHOLDER: &str = "tool call";

#[derive(Debug, Clone, Default)]
pub struct ChatReasoningConfig {
    pub supports_thinking: bool,
    pub thinking_param: Option<String>,
    pub effort_param: Option<String>,
    pub effort_value_mode: Option<String>,
}

pub fn infer_reasoning_config(model: &str, upstream_base: Option<&str>) -> Option<ChatReasoningConfig> {
    let model = model.to_ascii_lowercase();
    let base = upstream_base.unwrap_or("").to_ascii_lowercase();
    let haystack = format!("{model} {base}");

    if haystack.contains("deepseek") {
        return Some(ChatReasoningConfig {
            supports_thinking: true,
            thinking_param: Some("thinking".into()),
            effort_param: Some("reasoning_effort".into()),
            effort_value_mode: Some("deepseek".into()),
        });
    }
    if haystack.contains("kimi") || haystack.contains("moonshot") {
        return Some(ChatReasoningConfig {
            supports_thinking: true,
            thinking_param: Some("thinking".into()),
            effort_param: None,
            effort_value_mode: None,
        });
    }
    if haystack.contains("mimo") {
        return Some(ChatReasoningConfig {
            supports_thinking: true,
            thinking_param: Some("enable_thinking".into()),
            effort_param: None,
            effort_value_mode: None,
        });
    }
    if haystack.contains("qwen") || haystack.contains("dashscope") {
        return Some(ChatReasoningConfig {
            supports_thinking: true,
            thinking_param: Some("enable_thinking".into()),
            effort_param: None,
            effort_value_mode: None,
        });
    }
    None
}

pub fn model_requires_reasoning_replay(model: &str, upstream_base: Option<&str>) -> bool {
    infer_reasoning_config(model, upstream_base).is_some_and(|c| c.supports_thinking)
}

/// Determine whether the body explicitly requests reasoning.
/// Returns `Some(true)` for explicit enable, `Some(false)` for explicit disable,
/// or `None` when no reasoning preference is expressed.
fn reasoning_requested(body: &Value) -> Option<bool> {
    match body.pointer("/reasoning/effort").and_then(|v| v.as_str()) {
        Some(e) if matches!(e, "none" | "off" | "disabled") => Some(false),
        Some(_) => Some(true),
        None => {
            if body.get("reasoning").is_some_and(|v| !v.is_null()) {
                Some(true)
            } else {
                None
            }
        }
    }
}

fn set_thinking_param(result: &mut Map<String, Value>, config: &ChatReasoningConfig, enabled: bool) {
    let Some(param) = config.thinking_param.as_deref() else {
        return;
    };
    match param {
        "thinking" => {
            result.insert(
                "thinking".into(),
                json!({"type": if enabled { "enabled" } else { "disabled" }}),
            );
        }
        "enable_thinking" => {
            result.insert("enable_thinking".into(), json!(enabled));
        }
        _ => {}
    }
}

fn set_reasoning_effort(result: &mut Map<String, Value>, body: &Value, config: &ChatReasoningConfig) {
    let Some(effort) = body.pointer("/reasoning/effort").and_then(|v| v.as_str()) else {
        return;
    };
    let Some(mapped) = map_reasoning_effort(effort, config.effort_value_mode.as_deref()) else {
        return;
    };
    if config.effort_param.as_deref() == Some("reasoning_effort") {
        result.insert("reasoning_effort".into(), json!(mapped));
    }
}

/// Check if any message in the list already has non-empty `reasoning_content`,
/// indicating this is a multi-turn request requiring thinking to be enabled.
fn messages_require_thinking_enabled(messages: &[Value]) -> bool {
    messages.iter().any(|msg| {
        msg.get("reasoning_content")
            .and_then(|v| v.as_str())
            .is_some_and(|text| !text.trim().is_empty())
    })
}

pub fn apply_reasoning_options(
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

    let requested = reasoning_requested(body);

    if let Some(enabled) = requested {
        set_thinking_param(result, &config, enabled);
        if enabled {
            set_reasoning_effort(result, body, &config);
        }
    }

    // Auto-enable thinking when messages already contain reasoning_content (multi-turn).
    if let Some(messages) = result.get("messages").and_then(|v| v.as_array()) {
        if messages_require_thinking_enabled(messages) {
            set_thinking_param(result, &config, true);
            if config.effort_param.as_deref() == Some("reasoning_effort") {
                if !result.contains_key("reasoning_effort") {
                    result.insert("reasoning_effort".into(), json!("high"));
                }
            }
        }
    }
}

fn map_reasoning_effort(effort: &str, mode: Option<&str>) -> Option<&'static str> {
    match mode {
        Some("deepseek") => match effort {
            "max" | "xhigh" => Some("max"),
            _ => Some("high"),
        },
        Some("low_high") => match effort {
            "low" | "minimal" => Some("low"),
            "high" | "medium" | "standard" | "xhigh" => Some("high"),
            _ => None,
        },
        _ => match effort {
            "minimal" | "low" => Some("low"),
            "medium" | "standard" => Some("medium"),
            "high" | "xhigh" => Some("high"),
            _ => None,
        },
    }
}

pub fn ensure_tool_call_reasoning_content_value(message: &mut Value) {
    let Some(obj) = message.as_object_mut() else {
        return;
    };
    let is_assistant_tool_call = obj.get("role").and_then(|v| v.as_str()) == Some("assistant")
        && obj
            .get("tool_calls")
            .and_then(|v| v.as_array())
            .is_some_and(|calls| !calls.is_empty());
    if !is_assistant_tool_call {
        return;
    }
    let has_reasoning = obj
        .get("reasoning_content")
        .and_then(|v| v.as_str())
        .is_some_and(|text| !text.trim().is_empty());
    if !has_reasoning {
        obj.insert(
            "reasoning_content".into(),
            json!(TOOL_CALL_REASONING_PLACEHOLDER),
        );
    }
}

pub fn backfill_tool_call_reasoning_placeholders(messages: &mut [Value]) {
    for message in messages.iter_mut() {
        ensure_tool_call_reasoning_content_value(message);
    }
}

pub fn ensure_tool_call_reasoning_content_message(msg: &mut Message) {
    let has_tool_calls = msg
        .tool_calls
        .as_ref()
        .is_some_and(|calls| !calls.is_empty());
    if msg.role != Role::Assistant || !has_tool_calls {
        return;
    }
    let has_reasoning = msg
        .reasoning_content
        .as_deref()
        .is_some_and(|text| !text.trim().is_empty());
    if !has_reasoning {
        msg.reasoning_content = Some(TOOL_CALL_REASONING_PLACEHOLDER.into());
    }
    if msg.content.as_deref().unwrap_or("").is_empty() {
        msg.content = None;
    }
}

pub fn apply_reasoning_to_upstream_messages(
    messages: &mut [Message],
    model: &str,
    upstream_base: Option<&str>,
) {
    if !model_requires_reasoning_replay(model, upstream_base) {
        return;
    }
    for msg in messages.iter_mut() {
        ensure_tool_call_reasoning_content_message(msg);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deepseek_models_require_reasoning_replay() {
        assert!(model_requires_reasoning_replay(
            "deepseek-v4-flash",
            Some("https://opencode.ai/zen/go/v1")
        ));
    }

    #[test]
    fn injects_reasoning_placeholder_on_tool_call_message() {
        let mut msg = json!({
            "role": "assistant",
            "content": null,
            "tool_calls": [{"id": "c1", "type": "function", "function": {"name": "read", "arguments": "{}"}}]
        });
        ensure_tool_call_reasoning_content_value(&mut msg);
        assert_eq!(
            msg.get("reasoning_content").and_then(|v| v.as_str()),
            Some(TOOL_CALL_REASONING_PLACEHOLDER)
        );
    }

    // --- reasoning_requested tests ---

    #[test]
    fn reasoning_requested_some_true_when_effort_present() {
        let body = json!({"reasoning": {"effort": "high"}});
        assert_eq!(reasoning_requested(&body), Some(true));
    }

    #[test]
    fn reasoning_requested_some_false_when_effort_none() {
        let body = json!({"reasoning": {"effort": "none"}});
        assert_eq!(reasoning_requested(&body), Some(false));
    }

    #[test]
    fn reasoning_requested_some_false_when_effort_disabled() {
        let body = json!({"reasoning": {"effort": "disabled"}});
        assert_eq!(reasoning_requested(&body), Some(false));
    }

    #[test]
    fn reasoning_requested_some_true_when_reasoning_present_no_effort() {
        let body = json!({"reasoning": {}});
        assert_eq!(reasoning_requested(&body), Some(true));
    }

    #[test]
    fn reasoning_requested_none_when_no_reasoning() {
        let body = json!({"model": "deepseek"});
        assert_eq!(reasoning_requested(&body), None);
    }

    // --- messages_require_thinking_enabled tests ---

    #[test]
    fn messages_require_thinking_true_when_reasoning_content_present() {
        let msgs = vec![
            json!({"role": "assistant", "reasoning_content": "let me think", "content": "answer"}),
        ];
        assert!(messages_require_thinking_enabled(&msgs));
    }

    #[test]
    fn messages_require_thinking_false_when_no_reasoning_content() {
        let msgs = vec![
            json!({"role": "assistant", "content": "answer"}),
        ];
        assert!(!messages_require_thinking_enabled(&msgs));
    }

    #[test]
    fn messages_require_thinking_false_when_empty_reasoning_content() {
        let msgs = vec![
            json!({"role": "assistant", "reasoning_content": "", "content": "answer"}),
        ];
        assert!(!messages_require_thinking_enabled(&msgs));
    }

    #[test]
    fn messages_require_thinking_checks_all_messages() {
        let msgs = vec![
            json!({"role": "user", "content": "hi"}),
            json!({"role": "assistant", "reasoning_content": "thinking...", "content": "hello"}),
        ];
        assert!(messages_require_thinking_enabled(&msgs));
    }

    // --- map_reasoning_effort — DeepSeek mode ---

    #[test]
    fn deepseek_effort_maps_high_to_high() {
        assert_eq!(map_reasoning_effort("high", Some("deepseek")), Some("high"));
    }

    #[test]
    fn deepseek_effort_maps_max_to_max() {
        assert_eq!(map_reasoning_effort("max", Some("deepseek")), Some("max"));
    }

    #[test]
    fn deepseek_effort_maps_xhigh_to_max() {
        assert_eq!(map_reasoning_effort("xhigh", Some("deepseek")), Some("max"));
    }

    #[test]
    fn deepseek_effort_maps_low_to_high() {
        assert_eq!(map_reasoning_effort("low", Some("deepseek")), Some("high"));
    }

    #[test]
    fn deepseek_effort_maps_medium_to_high() {
        assert_eq!(map_reasoning_effort("medium", Some("deepseek")), Some("high"));
    }

    // --- apply_reasoning_options integration ---

    #[test]
    fn apply_deepseek_reasoning_with_medium_effort() {
        let mut result = Map::new();
        result.insert("model".into(), json!("deepseek"));
        result.insert("messages".into(), json!([]));
        let body = json!({"model": "deepseek", "reasoning": {"effort": "medium"}});
        apply_reasoning_options(&mut result, &body, "deepseek", Some("https://api.deepseek.com"));
        assert_eq!(result["thinking"], json!({"type": "enabled"}));
        assert_eq!(result["reasoning_effort"], json!("high"));
    }

    #[test]
    fn apply_deepseek_reasoning_disabled_none() {
        let mut result = Map::new();
        result.insert("model".into(), json!("deepseek"));
        result.insert("messages".into(), json!([]));
        let body = json!({"model": "deepseek", "reasoning": {"effort": "none"}});
        apply_reasoning_options(&mut result, &body, "deepseek", Some("https://api.deepseek.com"));
        assert_eq!(result["thinking"], json!({"type": "disabled"}));
        assert!(!result.contains_key("reasoning_effort"));
    }

    #[test]
    fn apply_deepseek_reasoning_xhigh_effort() {
        let mut result = Map::new();
        result.insert("model".into(), json!("deepseek"));
        result.insert("messages".into(), json!([]));
        let body = json!({"model": "deepseek", "reasoning": {"effort": "xhigh"}});
        apply_reasoning_options(&mut result, &body, "deepseek", Some("https://api.deepseek.com"));
        assert_eq!(result["reasoning_effort"], json!("max"));
    }

    #[test]
    fn apply_deepseek_reasoning_auto_enables_from_messages() {
        let mut result = Map::new();
        result.insert("model".into(), json!("deepseek"));
        result.insert("messages".into(), json!([
            {"role": "assistant", "reasoning_content": "prior reasoning", "content": "answer"},
            {"role": "user", "content": "follow-up"}
        ]));
        let body = json!({"model": "deepseek", "input": [{"role": "user", "content": "follow-up"}]});
        apply_reasoning_options(&mut result, &body, "deepseek", Some("https://api.deepseek.com"));
        assert_eq!(result["thinking"], json!({"type": "enabled"}));
        assert_eq!(result["reasoning_effort"], json!("high"));
    }

    #[test]
    fn apply_deepseek_reasoning_does_not_inject_without_reasoning_context() {
        let mut result = Map::new();
        result.insert("model".into(), json!("deepseek"));
        result.insert("messages".into(), json!([
            {"role": "user", "content": "hi"}
        ]));
        let body = json!({"model": "deepseek", "input": "hi"});
        apply_reasoning_options(&mut result, &body, "deepseek", Some("https://api.deepseek.com"));
        assert!(!result.contains_key("thinking"));
        assert!(!result.contains_key("reasoning_effort"));
    }
}
