use serde_json::{json, Map, Value};

use crate::gateway::api::openai::{ChatCompletionRequest, Message, Role};

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
    if haystack.contains("glm") || haystack.contains("zhipu") || haystack.contains("z.ai") {
        return Some(ChatReasoningConfig {
            supports_thinking: true,
            thinking_param: Some("thinking".into()),
            effort_param: None,
            effort_value_mode: None,
        });
    }
    if haystack.contains("qwen") || haystack.contains("dashscope") || haystack.contains("bailian") {
        return Some(ChatReasoningConfig {
            supports_thinking: true,
            thinking_param: Some("enable_thinking".into()),
            effort_param: None,
            effort_value_mode: None,
        });
    }
    if haystack.contains("minimax") {
        return Some(ChatReasoningConfig {
            supports_thinking: true,
            thinking_param: Some("reasoning_split".into()),
            effort_param: None,
            effort_value_mode: None,
        });
    }
    if haystack.contains("mimo") {
        return Some(ChatReasoningConfig {
            supports_thinking: true,
            thinking_param: Some("thinking".into()),
            effort_param: None,
            effort_value_mode: None,
        });
    }
    if haystack.contains("stepfun") || haystack.contains("step-3.5-flash-2603") {
        return Some(ChatReasoningConfig {
            supports_thinking: true,
            thinking_param: Some("thinking".into()),
            effort_param: Some("reasoning_effort".into()),
            effort_value_mode: Some("low_high".into()),
        });
    }
    if haystack.contains("openrouter") {
        return Some(ChatReasoningConfig {
            supports_thinking: false,
            thinking_param: None,
            effort_param: Some("reasoning.effort".into()),
            effort_value_mode: Some("openrouter".into()),
        });
    }
    None
}

pub fn model_requires_reasoning_replay(model: &str, upstream_base: Option<&str>) -> bool {
    infer_reasoning_config(model, upstream_base).is_some_and(|c| c.supports_thinking)
}

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
        "reasoning_split" => {
            result.insert("reasoning_split".into(), json!(enabled));
        }
        _ => {}
    }
}

fn set_reasoning_effort(
    result: &mut Map<String, Value>,
    body: &Value,
    config: &ChatReasoningConfig,
    stored_effort: Option<&str>,
) {
    let effort = body
        .pointer("/reasoning/effort")
        .and_then(|v| v.as_str())
        .or(stored_effort);
    let Some(effort) = effort else {
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
    stored_effort: Option<&str>,
) {
    let Some(config) = infer_reasoning_config(model, upstream_base) else {
        return;
    };
    if !config.supports_thinking && config.effort_param.as_deref() != Some("reasoning.effort") {
        return;
    }

    let requested = reasoning_requested(body);

    if let Some(enabled) = requested {
        if config.supports_thinking {
            set_thinking_param(result, &config, enabled);
        }
        if enabled {
            set_reasoning_effort(result, body, &config, stored_effort);
        }
    } else if config.effort_param.as_deref() == Some("reasoning.effort") {
        set_reasoning_effort(result, body, &config, stored_effort);
    }

    if let Some(messages) = result.get("messages").and_then(|v| v.as_array()) {
        if messages_require_thinking_enabled(messages) {
            if config.supports_thinking {
                set_thinking_param(result, &config, true);
            }
            if config.effort_param.is_some() && !result.contains_key("reasoning_effort") {
                let effort = stored_effort
                    .or_else(|| body.pointer("/reasoning/effort").and_then(|v| v.as_str()))
                    .unwrap_or("high");
                set_reasoning_effort(
                    result,
                    &json!({"reasoning": {"effort": effort}}),
                    &config,
                    stored_effort,
                );
            }
        }
    }
}

pub fn map_reasoning_effort(effort: &str, mode: Option<&str>) -> Option<&'static str> {
    let effort = effort.trim().to_ascii_lowercase();
    if matches!(effort.as_str(), "none" | "off" | "disabled") {
        return None;
    }

    match mode {
        Some("deepseek") => match effort.as_str() {
            "max" | "xhigh" => Some("max"),
            _ => Some("high"),
        },
        Some("low_high") => match effort.as_str() {
            "minimal" | "low" => Some("low"),
            _ => Some("high"),
        },
        Some("openrouter") => match effort.as_str() {
            "max" | "xhigh" => Some("xhigh"),
            "high" => Some("high"),
            "medium" => Some("medium"),
            "low" => Some("low"),
            "minimal" => Some("minimal"),
            _ => None,
        },
        _ => match effort.as_str() {
            "minimal" | "low" => Some("low"),
            "medium" | "standard" => Some("medium"),
            "high" | "xhigh" => Some("high"),
            "max" => Some("max"),
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

/// Apply top-level thinking/reasoning params for chat-completions upstream calls.
pub fn apply_reasoning_options_to_chat_request(
    req: &mut ChatCompletionRequest,
    upstream_base: Option<&str>,
) {
    let body = serde_json::to_value(&*req).unwrap_or(json!({}));
    let mut result = match &body {
        Value::Object(map) => map.clone(),
        _ => return,
    };
    apply_reasoning_options(&mut result, &body, &req.model, upstream_base, None);

    if let Some(v) = result.get("thinking") {
        req.thinking = Some(v.clone());
    }
    if let Some(v) = result.get("reasoning_effort").and_then(|v| v.as_str()) {
        req.reasoning_effort = Some(v.to_string());
    }
    if let Some(v) = result.get("enable_thinking").and_then(|v| v.as_bool()) {
        req.enable_thinking = Some(v);
    }
    if let Some(v) = result.get("reasoning_split").and_then(|v| v.as_bool()) {
        req.reasoning_split = Some(v);
    }
    if let Some(v) = result.get("reasoning") {
        req.reasoning = Some(v.clone());
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
    fn deepseek_effort_maps_high_to_high() {
        assert_eq!(map_reasoning_effort("high", Some("deepseek")), Some("high"));
    }

    #[test]
    fn deepseek_effort_maps_max_to_max() {
        assert_eq!(map_reasoning_effort("max", Some("deepseek")), Some("max"));
    }

    #[test]
    fn deepseek_effort_maps_medium_to_high() {
        assert_eq!(map_reasoning_effort("medium", Some("deepseek")), Some("high"));
    }

    #[test]
    fn apply_deepseek_reasoning_with_medium_effort() {
        let mut result = Map::new();
        result.insert("model".into(), json!("deepseek"));
        result.insert("messages".into(), json!([]));
        let body = json!({"model": "deepseek", "reasoning": {"effort": "medium"}});
        apply_reasoning_options(&mut result, &body, "deepseek", Some("https://api.deepseek.com"), None);
        assert_eq!(result["thinking"], json!({"type": "enabled"}));
        assert_eq!(result["reasoning_effort"], json!("high"));
    }

    #[test]
    fn apply_deepseek_reasoning_auto_enables_from_messages_with_stored_max() {
        let mut result = Map::new();
        result.insert("model".into(), json!("deepseek"));
        result.insert("messages".into(), json!([
            {"role": "assistant", "reasoning_content": "prior reasoning", "content": "answer"},
            {"role": "user", "content": "follow-up"}
        ]));
        let body = json!({"model": "deepseek", "input": [{"role": "user", "content": "follow-up"}]});
        apply_reasoning_options(
            &mut result,
            &body,
            "deepseek",
            Some("https://api.deepseek.com"),
            Some("max"),
        );
        assert_eq!(result["thinking"], json!({"type": "enabled"}));
        assert_eq!(result["reasoning_effort"], json!("max"));
    }

    #[test]
    fn apply_qwen_reasoning_sets_enable_thinking() {
        let mut result = Map::new();
        result.insert("model".into(), json!("qwen-plus"));
        result.insert("messages".into(), json!([]));
        let body = json!({"model": "qwen-plus", "reasoning": {"effort": "high"}});
        apply_reasoning_options(
            &mut result,
            &body,
            "qwen-plus",
            Some("https://dashscope.aliyuncs.com"),
            None,
        );
        assert_eq!(result["enable_thinking"], json!(true));
    }

    #[test]
    fn apply_reasoning_options_to_chat_request_enables_deepseek_thinking() {
        let mut req = ChatCompletionRequest {
            model: "deepseek-v4-flash".into(),
            messages: vec![Message {
                role: Role::Assistant,
                content: Some("ok".into()),
                content_parts: None,
                tool_calls: None,
                tool_call_id: None,
                reasoning_content: Some("planning".into()),
            }],
            ..Default::default()
        };
        apply_reasoning_options_to_chat_request(&mut req, Some("https://api.deepseek.com/v1"));
        assert_eq!(req.thinking, Some(json!({"type": "enabled"})));
        assert_eq!(req.reasoning_effort.as_deref(), Some("high"));
    }
}
