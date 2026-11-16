//! Anthropic Messages 类型化编解码（P0b-8）。
//!
//! 与 Chat 的差异都在这里吸收：`system` 单独字段、`content` 块数组、
//! `tool_use.input` 是对象而不是 JSON 字符串、thinking/redacted_thinking 块。
//!
//! 归一化决策：`tool_choice` 统一成 OpenAI 语义（`"auto"` / `"required"` /
//! `{"type":"function","function":{"name":..}}`），这样跨协议转发不必二次翻译。

use serde_json::{Map, Value, json};
use tr_contracts::{
    CanonicalMessage, CanonicalRequest, CanonicalResponse, CanonicalRole, ContentBlock,
    FinishReason, IngressProtocol, MediaSource, OutputParams, ReasoningParams, RequestMeta,
    SamplingParams, SystemBlock, ToolSpec, Usage,
};

use crate::CodecError;

/// 解码 Anthropic Messages 请求。
pub fn decode_request(value: &Value) -> Result<CanonicalRequest, CodecError> {
    let object = value
        .as_object()
        .ok_or(CodecError::Invalid("request: not an object"))?;

    let mut request = CanonicalRequest {
        meta: RequestMeta {
            ingress_protocol: IngressProtocol::AnthropicMessages,
            virtual_model: string(object, "model"),
            ..RequestMeta::default()
        },
        ..CanonicalRequest::default()
    };

    request.system = decode_system(object.get("system"))?;

    if let Some(messages) = object.get("messages").and_then(Value::as_array) {
        for message in messages {
            request.messages.push(decode_message(message)?);
        }
    } else if object.get("messages").is_some() {
        return Err(CodecError::Invalid("messages: expected array"));
    }

    if let Some(tools) = object.get("tools").and_then(Value::as_array) {
        for tool in tools {
            request.tools.push(decode_tool(tool)?);
        }
    }

    request.sampling = SamplingParams {
        temperature: number_f32(object, "temperature"),
        top_p: number_f32(object, "top_p"),
        top_k: number_u32(object, "top_k"),
        stop: stop_sequences(object.get("stop_sequences")),
        ..SamplingParams::default()
    };
    request.output = OutputParams {
        max_tokens: number_u32(object, "max_tokens"),
        stream: object
            .get("stream")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        tool_choice: object.get("tool_choice").map(normalize_tool_choice),
        parallel_tool_calls: object
            .get("tool_choice")
            .and_then(Value::as_object)
            .and_then(|choice| choice.get("disable_parallel_tool_use"))
            .and_then(Value::as_bool)
            .map(|disabled| !disabled),
        ..OutputParams::default()
    };
    request.reasoning = decode_thinking(object.get("thinking"));

    for (key, value) in object {
        if !matches!(
            key.as_str(),
            "model"
                | "system"
                | "messages"
                | "tools"
                | "tool_choice"
                | "thinking"
                | "temperature"
                | "top_p"
                | "top_k"
                | "stop_sequences"
                | "max_tokens"
                | "stream"
        ) {
            request.passthrough.insert(key.clone(), value.clone());
        }
    }

    Ok(request)
}

fn decode_system(value: Option<&Value>) -> Result<Vec<SystemBlock>, CodecError> {
    match value {
        None | Some(Value::Null) => Ok(Vec::new()),
        Some(Value::String(text)) => Ok(vec![SystemBlock {
            text: text.clone(),
            cache_control: None,
        }]),
        Some(Value::Array(blocks)) => blocks
            .iter()
            .map(|block| {
                let object = block
                    .as_object()
                    .ok_or(CodecError::Invalid("system block: not an object"))?;
                Ok(SystemBlock {
                    text: object
                        .get("text")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string(),
                    cache_control: object.get("cache_control").cloned().map(|_| {
                        // canonical 只保留标记存在性；具体形状随上游透传。
                        tr_contracts::CacheControl {
                            kind: "ephemeral".to_string(),
                            ttl: None,
                        }
                    }),
                })
            })
            .collect(),
        Some(_) => Err(CodecError::Invalid("system: expected string or array")),
    }
}

fn decode_message(value: &Value) -> Result<CanonicalMessage, CodecError> {
    let object = value
        .as_object()
        .ok_or(CodecError::Invalid("message: not an object"))?;
    let role = match object.get("role").and_then(Value::as_str) {
        Some("assistant") => CanonicalRole::Assistant,
        // 部分客户端（Claude Code）会把 system 作为消息发送；canonical 允许 System 消息。
        Some("system") | Some("developer") => CanonicalRole::System,
        Some("user") | None => CanonicalRole::User,
        Some(other) => return Err(CodecError::Unsupported(other.to_string())),
    };

    let mut message = CanonicalMessage {
        role,
        ..CanonicalMessage::default()
    };

    match object.get("content") {
        Some(Value::String(text)) => message.blocks.push(ContentBlock::Text {
            text: text.clone(),
            cache_control: None,
        }),
        Some(Value::Array(parts)) => {
            for part in parts {
                message.blocks.push(decode_block(part)?);
            }
        }
        Some(_) => return Err(CodecError::Invalid("content: expected string or array")),
        None => {}
    }

    Ok(message)
}

fn decode_block(value: &Value) -> Result<ContentBlock, CodecError> {
    let object = value
        .as_object()
        .ok_or(CodecError::Invalid("content block: not an object"))?;
    match object.get("type").and_then(Value::as_str) {
        Some("text") => Ok(ContentBlock::Text {
            text: object
                .get("text")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            cache_control: object
                .get("cache_control")
                .map(|_| tr_contracts::CacheControl {
                    kind: "ephemeral".to_string(),
                    ttl: None,
                }),
        }),
        Some("image") => {
            let source = object
                .get("source")
                .and_then(Value::as_object)
                .ok_or(CodecError::MissingField("image.source"))?;
            let media = match source.get("type").and_then(Value::as_str) {
                Some("base64") => MediaSource::Base64 {
                    media_type: source
                        .get("media_type")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string(),
                    data: source
                        .get("data")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string(),
                },
                Some("url") => MediaSource::Url {
                    url: source
                        .get("url")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string(),
                },
                other => {
                    return Err(CodecError::Unsupported(
                        other.unwrap_or("image.source.type").to_string(),
                    ));
                }
            };
            Ok(ContentBlock::Image {
                source: media,
                detail: None,
            })
        }
        Some("tool_use") => Ok(ContentBlock::ToolUse {
            id: string(object, "id").ok_or(CodecError::MissingField("tool_use.id"))?,
            name: string(object, "name").ok_or(CodecError::MissingField("tool_use.name"))?,
            arguments: object.get("input").cloned().unwrap_or(Value::Null),
        }),
        Some("tool_result") => Ok(ContentBlock::ToolResult {
            tool_use_id: string(object, "tool_use_id")
                .ok_or(CodecError::MissingField("tool_result.tool_use_id"))?,
            content: decode_tool_result_content(object.get("content"))?,
            is_error: object
                .get("is_error")
                .and_then(Value::as_bool)
                .unwrap_or(false),
        }),
        Some("thinking") => Ok(ContentBlock::Reasoning {
            text: object
                .get("thinking")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            signature: string(object, "signature"),
            redacted: false,
        }),
        Some("redacted_thinking") => Ok(ContentBlock::Reasoning {
            text: String::new(),
            // data 是加密载荷；canonical 用 signature 承载，保证原样回传。
            signature: string(object, "data"),
            redacted: true,
        }),
        Some(other) => Err(CodecError::Unsupported(other.to_string())),
        None => Err(CodecError::MissingField("content[].type")),
    }
}

fn decode_tool_result_content(value: Option<&Value>) -> Result<Vec<ContentBlock>, CodecError> {
    match value {
        None | Some(Value::Null) => Ok(Vec::new()),
        Some(Value::String(text)) => Ok(vec![ContentBlock::Text {
            text: text.clone(),
            cache_control: None,
        }]),
        Some(Value::Array(parts)) => parts.iter().map(decode_block).collect(),
        Some(_) => Err(CodecError::Invalid(
            "tool_result.content: expected string or array",
        )),
    }
}

fn decode_tool(value: &Value) -> Result<ToolSpec, CodecError> {
    let object = value
        .as_object()
        .ok_or(CodecError::Invalid("tool: not an object"))?;
    Ok(ToolSpec {
        name: string(object, "name").ok_or(CodecError::MissingField("tool.name"))?,
        description: string(object, "description"),
        parameters: object.get("input_schema").cloned().unwrap_or(Value::Null),
        strict: None,
    })
}

fn normalize_tool_choice(value: &Value) -> Value {
    match value.get("type").and_then(Value::as_str) {
        Some("auto") => json!("auto"),
        Some("any") => json!("required"),
        Some("tool") => value
            .get("name")
            .and_then(Value::as_str)
            .map(|name| json!({"type": "function", "function": {"name": name}}))
            .unwrap_or_else(|| json!("required")),
        _ => value.clone(),
    }
}

fn decode_thinking(value: Option<&Value>) -> Option<ReasoningParams> {
    let object = value?.as_object()?;
    match object.get("type").and_then(Value::as_str) {
        Some("enabled") => Some(ReasoningParams {
            effort: None,
            budget_tokens: object
                .get("budget_tokens")
                .and_then(Value::as_u64)
                .map(|value| value as u32),
            enabled: Some(true),
        }),
        Some("disabled") => Some(ReasoningParams {
            enabled: Some(false),
            ..ReasoningParams::default()
        }),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// 编码
// ---------------------------------------------------------------------------

/// 编码为 Anthropic wire 请求。
pub fn encode_request(request: &CanonicalRequest) -> Value {
    let mut object = Map::new();

    if let Some(model) = &request.meta.virtual_model {
        object.insert("model".to_string(), Value::String(model.clone()));
    }

    // Anthropic 要求 max_tokens；canonical 没有时给一个保守默认值。
    object.insert(
        "max_tokens".to_string(),
        json!(request.output.max_tokens.unwrap_or(4096)),
    );

    if !request.system.is_empty() {
        let system: Vec<Value> = request
            .system
            .iter()
            .map(|block| {
                let mut part = json!({"type": "text", "text": block.text});
                if block.cache_control.is_some() {
                    part["cache_control"] = json!({"type": "ephemeral"});
                }
                part
            })
            .collect();
        object.insert("system".to_string(), Value::Array(system));
    }

    let messages: Vec<Value> = request.messages.iter().map(encode_message).collect();
    object.insert("messages".to_string(), Value::Array(messages));

    if !request.tools.is_empty() {
        let tools = request
            .tools
            .iter()
            .map(|tool| {
                let mut value = json!({
                    "name": tool.name,
                    "input_schema": tool.parameters
                });
                if let Some(description) = &tool.description {
                    value["description"] = Value::String(description.clone());
                }
                value
            })
            .collect();
        object.insert("tools".to_string(), Value::Array(tools));
    }

    if let Some(choice) = &request.output.tool_choice {
        object.insert("tool_choice".to_string(), denormalize_tool_choice(choice));
        if let Some(parallel) = request.output.parallel_tool_calls
            && let Some(Value::Object(choice)) = object.get_mut("tool_choice")
        {
            choice.insert("disable_parallel_tool_use".to_string(), json!(!parallel));
        }
    }

    if let Some(reasoning) = &request.reasoning {
        if let Some(budget) = reasoning.budget_tokens {
            object.insert(
                "thinking".to_string(),
                json!({"type": "enabled", "budget_tokens": budget}),
            );
        } else if reasoning.enabled == Some(false) {
            object.insert("thinking".to_string(), json!({"type": "disabled"}));
        }
    }

    insert_opt_f32(&mut object, "temperature", request.sampling.temperature);
    insert_opt_f32(&mut object, "top_p", request.sampling.top_p);
    insert_opt_u32(&mut object, "top_k", request.sampling.top_k);
    if !request.sampling.stop.is_empty() {
        object.insert(
            "stop_sequences".to_string(),
            Value::Array(
                request
                    .sampling
                    .stop
                    .iter()
                    .map(|text| Value::String(text.clone()))
                    .collect(),
            ),
        );
    }
    object.insert("stream".to_string(), Value::Bool(request.output.stream));

    for (key, value) in &request.passthrough {
        object.entry(key.clone()).or_insert_with(|| value.clone());
    }

    Value::Object(object)
}

fn encode_message(message: &CanonicalMessage) -> Value {
    // Anthropic 没有 tool 角色：tool_result 必须作为 user 内容的块。
    let role = match message.role {
        CanonicalRole::Assistant => "assistant",
        _ => "user",
    };

    let blocks: Vec<Value> = message.blocks.iter().map(encode_block).collect();
    let content = if blocks.is_empty() {
        Value::String(String::new())
    } else {
        Value::Array(blocks)
    };

    json!({"role": role, "content": content})
}

fn encode_block(block: &ContentBlock) -> Value {
    match block {
        ContentBlock::Text {
            text,
            cache_control,
        } => {
            let mut value = json!({"type": "text", "text": text});
            if cache_control.is_some() {
                value["cache_control"] = json!({"type": "ephemeral"});
            }
            value
        }
        ContentBlock::Image { source, .. } => {
            let source = match source {
                MediaSource::Base64 { media_type, data } => {
                    json!({"type": "base64", "media_type": media_type, "data": data})
                }
                MediaSource::Url { url } => json!({"type": "url", "url": url}),
                MediaSource::File { file_id } => json!({"type": "url", "url": file_id}),
            };
            json!({"type": "image", "source": source})
        }
        ContentBlock::ToolUse {
            id,
            name,
            arguments,
        } => json!({"type": "tool_use", "id": id, "name": name, "input": arguments}),
        ContentBlock::ToolResult {
            tool_use_id,
            content,
            is_error,
        } => json!({
            "type": "tool_result",
            "tool_use_id": tool_use_id,
            "is_error": is_error,
            "content": content.iter().map(encode_block).collect::<Vec<_>>()
        }),
        ContentBlock::Reasoning {
            text,
            signature,
            redacted,
        } => {
            if *redacted {
                json!({"type": "redacted_thinking", "data": signature.clone().unwrap_or_default()})
            } else {
                json!({"type": "thinking", "thinking": text, "signature": signature})
            }
        }
        ContentBlock::Audio { .. } => json!({"type": "text", "text": "[unsupported audio]"}),
    }
}

fn denormalize_tool_choice(value: &Value) -> Value {
    match value {
        Value::String(kind) if kind == "auto" => json!({"type": "auto"}),
        Value::String(kind) if kind == "required" || kind == "any" => json!({"type": "any"}),
        Value::Object(object) if object.get("type").and_then(Value::as_str) == Some("function") => {
            let name = object
                .get("function")
                .and_then(|function| function.get("name"))
                .cloned()
                .unwrap_or(Value::Null);
            json!({"type": "tool", "name": name})
        }
        other => other.clone(),
    }
}

/// 解码非流式响应。
pub fn decode_response(value: &Value) -> Result<CanonicalResponse, CodecError> {
    let object = value
        .as_object()
        .ok_or(CodecError::Invalid("response: not an object"))?;

    let mut blocks = Vec::new();
    if let Some(content) = object.get("content").and_then(Value::as_array) {
        for block in content {
            blocks.push(decode_block(block)?);
        }
    }

    Ok(CanonicalResponse {
        id: string(object, "id").unwrap_or_default(),
        model: string(object, "model").unwrap_or_default(),
        blocks,
        finish_reason: object
            .get("stop_reason")
            .and_then(Value::as_str)
            .map(finish_reason)
            .unwrap_or(FinishReason::Stop),
        usage: decode_usage(object.get("usage")),
    })
}

fn decode_usage(value: Option<&Value>) -> Usage {
    let Some(object) = value.and_then(Value::as_object) else {
        return Usage::default();
    };
    Usage {
        input_tokens: object
            .get("input_tokens")
            .and_then(Value::as_u64)
            .unwrap_or(0) as u32,
        output_tokens: object
            .get("output_tokens")
            .and_then(Value::as_u64)
            .unwrap_or(0) as u32,
        cached_tokens: object
            .get("cache_read_input_tokens")
            .and_then(Value::as_u64)
            .map(|value| value as u32),
        cache_write_tokens: object
            .get("cache_creation_input_tokens")
            .and_then(Value::as_u64)
            .map(|value| value as u32),
    }
}

/// 编码非流式响应。
pub fn encode_response(response: &CanonicalResponse) -> Value {
    let content: Vec<Value> = response.blocks.iter().map(encode_block).collect();
    json!({
        "id": response.id,
        "type": "message",
        "role": "assistant",
        "model": response.model,
        "content": content,
        "stop_reason": finish_reason_str(&response.finish_reason),
        "stop_sequence": Value::Null,
        "usage": {
            "input_tokens": response.usage.input_tokens,
            "output_tokens": response.usage.output_tokens,
            "cache_read_input_tokens": response.usage.cached_tokens.unwrap_or(0),
            "cache_creation_input_tokens": response.usage.cache_write_tokens.unwrap_or(0)
        }
    })
}

fn finish_reason(value: &str) -> FinishReason {
    match value {
        "end_turn" | "stop_sequence" => FinishReason::Stop,
        "max_tokens" => FinishReason::Length,
        "tool_use" => FinishReason::ToolUse,
        "refusal" => FinishReason::ContentFilter,
        other => FinishReason::Other(other.to_string()),
    }
}

fn finish_reason_str(reason: &FinishReason) -> &'static str {
    match reason {
        FinishReason::Stop => "end_turn",
        FinishReason::ToolUse => "tool_use",
        FinishReason::Length => "max_tokens",
        FinishReason::ContentFilter => "refusal",
        FinishReason::Error | FinishReason::Other(_) => "end_turn",
    }
}

fn stop_sequences(value: Option<&Value>) -> Vec<String> {
    match value {
        Some(Value::String(text)) => vec![text.clone()],
        Some(Value::Array(items)) => items
            .iter()
            .filter_map(Value::as_str)
            .map(str::to_string)
            .collect(),
        _ => Vec::new(),
    }
}

fn string(object: &Map<String, Value>, key: &str) -> Option<String> {
    object.get(key).and_then(Value::as_str).map(str::to_string)
}

fn number_f32(object: &Map<String, Value>, key: &str) -> Option<f32> {
    object
        .get(key)
        .and_then(Value::as_f64)
        .map(|value| value as f32)
}

fn number_u32(object: &Map<String, Value>, key: &str) -> Option<u32> {
    object
        .get(key)
        .and_then(Value::as_u64)
        .map(|value| value as u32)
}

fn insert_opt_f32(object: &mut Map<String, Value>, key: &str, value: Option<f32>) {
    if let Some(value) = value {
        object.insert(key.to_string(), json!(value));
    }
}

fn insert_opt_u32(object: &mut Map<String, Value>, key: &str, value: Option<u32>) {
    if let Some(value) = value {
        object.insert(key.to_string(), json!(value));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn system_role_in_messages_is_accepted() {
        let request = decode_request(&serde_json::json!({
            "model": "auto",
            "max_tokens": 32,
            "messages": [
                {"role": "system", "content": "follow the rules"},
                {"role": "user", "content": "hi"}
            ]
        }))
        .expect("decode system message");
        assert_eq!(request.messages[0].role, CanonicalRole::System);
        assert_eq!(request.messages[1].role, CanonicalRole::User);
    }
}
