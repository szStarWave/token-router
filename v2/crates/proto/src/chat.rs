//! Chat Completions 类型化编解码（P0a-12）。
//!
//! 只做形状映射，不做策略；未知顶层字段进 `passthrough` 并原样带回。
//! 三种协议共用 `Canonical*`，因此这里必须“无损”：fixture 里出现过的字段
//! 都必须能编码回去（字段覆盖探针会盯着）。

use serde_json::{Map, Value, json};
use tr_contracts::{
    CanonicalMessage, CanonicalRequest, CanonicalResponse, CanonicalRole, ContentBlock,
    FinishReason, IngressProtocol, MediaSource, OutputParams, ReasoningParams, RequestMeta,
    SamplingParams, StreamOptions, ToolSpec, Usage,
};

use crate::CodecError;

const KNOWN_REQUEST_KEYS: &[&str] = &[
    "model",
    "messages",
    "tools",
    "tool_choice",
    "parallel_tool_calls",
    "temperature",
    "top_p",
    "top_k",
    "seed",
    "stop",
    "presence_penalty",
    "frequency_penalty",
    "logprobs",
    "top_logprobs",
    "logit_bias",
    "max_tokens",
    "max_completion_tokens",
    "n",
    "response_format",
    "stream",
    "stream_options",
    "reasoning_effort",
    "user",
    "metadata",
    "service_tier",
    "store",
];

/// 解码 Chat Completions 请求。
pub fn decode_request(value: &Value) -> Result<CanonicalRequest, CodecError> {
    let object = value
        .as_object()
        .ok_or(CodecError::Invalid("request: not an object"))?;

    let mut request = CanonicalRequest {
        meta: RequestMeta {
            ingress_protocol: IngressProtocol::ChatCompletions,
            virtual_model: string(object, "model"),
            ..RequestMeta::default()
        },
        ..CanonicalRequest::default()
    };

    match object.get("messages") {
        Some(Value::Array(messages)) => {
            for message in messages {
                request.messages.push(decode_message(message)?);
            }
        }
        Some(Value::Null) | None => {}
        Some(_) => return Err(CodecError::Invalid("messages: expected array")),
    }

    if let Some(tools) = object.get("tools").and_then(Value::as_array) {
        for tool in tools {
            request.tools.push(decode_tool(tool)?);
        }
    }

    request.sampling = decode_sampling(object);
    request.output = decode_output(object);
    request.reasoning = object
        .get("reasoning_effort")
        .and_then(Value::as_str)
        .map(|effort| ReasoningParams {
            effort: Some(effort.to_string()),
            ..ReasoningParams::default()
        });

    for (key, value) in object {
        if !KNOWN_REQUEST_KEYS.contains(&key.as_str()) {
            request.passthrough.insert(key.clone(), value.clone());
        }
    }

    Ok(request)
}

fn decode_message(value: &Value) -> Result<CanonicalMessage, CodecError> {
    let object = value
        .as_object()
        .ok_or(CodecError::Invalid("message: not an object"))?;
    let role = match object.get("role").and_then(Value::as_str) {
        Some("system") => CanonicalRole::System,
        Some("assistant") => CanonicalRole::Assistant,
        Some("tool") => CanonicalRole::Tool,
        Some("user") | None => CanonicalRole::User,
        Some(other) => return Err(CodecError::Unsupported(other.to_string())),
    };

    let mut message = CanonicalMessage {
        role,
        tool_call_id: string(object, "tool_call_id"),
        name: string(object, "name"),
        ..CanonicalMessage::default()
    };

    match object.get("content") {
        Some(Value::String(text)) => message.blocks.push(ContentBlock::Text {
            text: text.clone(),
            cache_control: None,
        }),
        Some(Value::Array(parts)) => {
            for part in parts {
                message.blocks.push(decode_content_part(part)?);
            }
        }
        _ => {}
    }

    if let Some(tool_calls) = object.get("tool_calls").and_then(Value::as_array) {
        for call in tool_calls {
            message.blocks.push(decode_tool_call(call)?);
        }
    }

    Ok(message)
}

fn decode_content_part(value: &Value) -> Result<ContentBlock, CodecError> {
    let object = value
        .as_object()
        .ok_or(CodecError::Invalid("content part: not an object"))?;
    match object.get("type").and_then(Value::as_str) {
        Some("text") => Ok(ContentBlock::Text {
            text: object
                .get("text")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            cache_control: None,
        }),
        Some("image_url") => {
            let image = object
                .get("image_url")
                .and_then(Value::as_object)
                .ok_or(CodecError::MissingField("image_url"))?;
            let url = image
                .get("url")
                .and_then(Value::as_str)
                .ok_or(CodecError::MissingField("image_url.url"))?;
            Ok(ContentBlock::Image {
                source: parse_media(url),
                detail: string(image, "detail"),
            })
        }
        Some(other) => Err(CodecError::Unsupported(other.to_string())),
        None => Err(CodecError::MissingField("content[].type")),
    }
}

fn decode_tool_call(value: &Value) -> Result<ContentBlock, CodecError> {
    let object = value
        .as_object()
        .ok_or(CodecError::Invalid("tool_call: not an object"))?;
    let function = object
        .get("function")
        .and_then(Value::as_object)
        .ok_or(CodecError::MissingField("tool_call.function"))?;
    let arguments = function
        .get("arguments")
        .and_then(Value::as_str)
        .map(|raw| serde_json::from_str(raw).unwrap_or_else(|_| Value::String(raw.to_string())))
        .unwrap_or(Value::Null);

    Ok(ContentBlock::ToolUse {
        id: string(object, "id").ok_or(CodecError::MissingField("tool_call.id"))?,
        name: string(function, "name")
            .ok_or(CodecError::MissingField("tool_call.function.name"))?,
        arguments,
    })
}

fn decode_tool(value: &Value) -> Result<ToolSpec, CodecError> {
    let object = value
        .as_object()
        .ok_or(CodecError::Invalid("tool: not an object"))?;
    let function = object
        .get("function")
        .and_then(Value::as_object)
        .ok_or(CodecError::MissingField("tool.function"))?;
    Ok(ToolSpec {
        name: string(function, "name").ok_or(CodecError::MissingField("tool.function.name"))?,
        description: string(function, "description"),
        parameters: function.get("parameters").cloned().unwrap_or(Value::Null),
        strict: function.get("strict").and_then(Value::as_bool),
    })
}

fn decode_sampling(object: &Map<String, Value>) -> SamplingParams {
    SamplingParams {
        temperature: number_f32(object, "temperature"),
        top_p: number_f32(object, "top_p"),
        top_k: number_u32(object, "top_k"),
        seed: number_u64(object, "seed"),
        stop: stop_sequences(object.get("stop")),
        presence_penalty: number_f32(object, "presence_penalty"),
        frequency_penalty: number_f32(object, "frequency_penalty"),
        logprobs: object.get("logprobs").and_then(Value::as_bool),
        top_logprobs: number_u32(object, "top_logprobs"),
    }
}

fn decode_output(object: &Map<String, Value>) -> OutputParams {
    OutputParams {
        max_tokens: number_u32(object, "max_tokens")
            .or_else(|| number_u32(object, "max_completion_tokens")),
        stream: object
            .get("stream")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        stream_options: object
            .get("stream_options")
            .and_then(Value::as_object)
            .map(|options| StreamOptions {
                include_usage: options
                    .get("include_usage")
                    .and_then(Value::as_bool)
                    .unwrap_or(false),
            }),
        response_format: object.get("response_format").cloned(),
        n: number_u32(object, "n"),
        tool_choice: object.get("tool_choice").cloned(),
        parallel_tool_calls: object.get("parallel_tool_calls").and_then(Value::as_bool),
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

fn parse_media(url: &str) -> MediaSource {
    if let Some(rest) = url.strip_prefix("data:")
        && let Some((meta, data)) = rest.split_once(",")
    {
        return MediaSource::Base64 {
            media_type: meta.trim_end_matches(";base64").to_string(),
            data: data.to_string(),
        };
    }
    MediaSource::Url {
        url: url.to_string(),
    }
}

// ---------------------------------------------------------------------------
// 编码
// ---------------------------------------------------------------------------

/// 编码为 wire 请求。`created` 只影响响应，不影响请求。
pub fn encode_request(request: &CanonicalRequest) -> Value {
    let mut object = Map::new();

    if let Some(model) = &request.meta.virtual_model {
        object.insert("model".to_string(), Value::String(model.clone()));
    }

    let messages: Vec<Value> = request.messages.iter().map(encode_message).collect();
    object.insert("messages".to_string(), Value::Array(messages));

    if !request.tools.is_empty() {
        let tools = request.tools.iter().map(encode_tool).collect();
        object.insert("tools".to_string(), Value::Array(tools));
    }

    let sampling = &request.sampling;
    insert_opt_f32(&mut object, "temperature", sampling.temperature);
    insert_opt_f32(&mut object, "top_p", sampling.top_p);
    insert_opt_u32(&mut object, "top_k", sampling.top_k);
    insert_opt_u64(&mut object, "seed", sampling.seed);
    if !sampling.stop.is_empty() {
        object.insert(
            "stop".to_string(),
            Value::Array(
                sampling
                    .stop
                    .iter()
                    .map(|text| Value::String(text.clone()))
                    .collect(),
            ),
        );
    }
    insert_opt_f32(&mut object, "presence_penalty", sampling.presence_penalty);
    insert_opt_f32(&mut object, "frequency_penalty", sampling.frequency_penalty);
    if let Some(logprobs) = sampling.logprobs {
        object.insert("logprobs".to_string(), Value::Bool(logprobs));
    }
    insert_opt_u32(&mut object, "top_logprobs", sampling.top_logprobs);

    let output = &request.output;
    insert_opt_u32(&mut object, "max_tokens", output.max_tokens);
    object.insert("stream".to_string(), Value::Bool(output.stream));
    if let Some(options) = &output.stream_options {
        object.insert(
            "stream_options".to_string(),
            json!({ "include_usage": options.include_usage }),
        );
    }
    if let Some(format) = &output.response_format {
        object.insert("response_format".to_string(), format.clone());
    }
    insert_opt_u32(&mut object, "n", output.n);
    if let Some(choice) = &output.tool_choice {
        object.insert("tool_choice".to_string(), choice.clone());
    }
    if let Some(parallel) = output.parallel_tool_calls {
        object.insert("parallel_tool_calls".to_string(), Value::Bool(parallel));
    }

    if let Some(reasoning) = &request.reasoning
        && let Some(effort) = &reasoning.effort
    {
        object.insert(
            "reasoning_effort".to_string(),
            Value::String(effort.clone()),
        );
    }

    // 透传字段不覆盖已知字段：canonical 的显式值优先。
    for (key, value) in &request.passthrough {
        object.entry(key.clone()).or_insert_with(|| value.clone());
    }

    Value::Object(object)
}

fn encode_message(message: &CanonicalMessage) -> Value {
    let mut object = Map::new();
    object.insert(
        "role".to_string(),
        Value::String(
            match message.role {
                CanonicalRole::System => "system",
                CanonicalRole::User => "user",
                CanonicalRole::Assistant => "assistant",
                CanonicalRole::Tool => "tool",
            }
            .to_string(),
        ),
    );

    let tool_calls: Vec<Value> = message
        .blocks
        .iter()
        .filter_map(|block| match block {
            ContentBlock::ToolUse {
                id,
                name,
                arguments,
            } => Some(json!({
                "id": id,
                "type": "function",
                "function": {"name": name, "arguments": arguments.to_string()}
            })),
            _ => None,
        })
        .collect();

    let text_blocks: Vec<&ContentBlock> = message
        .blocks
        .iter()
        .filter(|block| !matches!(block, ContentBlock::ToolUse { .. }))
        .collect();

    let content = if text_blocks.is_empty() {
        Value::Null
    } else if text_blocks.len() == 1
        && let ContentBlock::Text { text, .. } = text_blocks[0]
    {
        Value::String(text.clone())
    } else {
        Value::Array(text_blocks.into_iter().map(encode_content_block).collect())
    };
    object.insert("content".to_string(), content);

    if !tool_calls.is_empty() {
        object.insert("tool_calls".to_string(), Value::Array(tool_calls));
    }
    if let Some(name) = &message.name {
        object.insert("name".to_string(), Value::String(name.clone()));
    }
    if let Some(tool_call_id) = &message.tool_call_id {
        object.insert(
            "tool_call_id".to_string(),
            Value::String(tool_call_id.clone()),
        );
    }

    Value::Object(object)
}

fn encode_content_block(block: &ContentBlock) -> Value {
    match block {
        ContentBlock::Text { text, .. } => json!({"type": "text", "text": text}),
        ContentBlock::Image { source, detail } => {
            let url = match source {
                MediaSource::Url { url } => url.clone(),
                MediaSource::Base64 { media_type, data } => {
                    format!("data:{media_type};base64,{data}")
                }
                MediaSource::File { file_id } => file_id.clone(),
            };
            json!({"type": "image_url", "image_url": {"url": url, "detail": detail}})
        }
        ContentBlock::Audio { source } => json!({"type": "input_audio", "input_audio": source}),
        ContentBlock::ToolResult {
            tool_use_id,
            content,
            is_error,
        } => json!({
            "type": "tool_result",
            "tool_use_id": tool_use_id,
            "is_error": is_error,
            "content": content.iter().map(encode_content_block).collect::<Vec<_>>()
        }),
        ContentBlock::ToolUse {
            id,
            name,
            arguments,
        } => json!({
            "type": "tool_use",
            "id": id,
            "name": name,
            "input": arguments
        }),
        ContentBlock::Reasoning { text, .. } => json!({"type": "text", "text": text}),
    }
}

fn encode_tool(tool: &ToolSpec) -> Value {
    let mut function = Map::new();
    function.insert("name".to_string(), Value::String(tool.name.clone()));
    if let Some(description) = &tool.description {
        function.insert(
            "description".to_string(),
            Value::String(description.clone()),
        );
    }
    function.insert("parameters".to_string(), tool.parameters.clone());
    if let Some(strict) = tool.strict {
        function.insert("strict".to_string(), Value::Bool(strict));
    }
    json!({"type": "function", "function": function})
}

/// 解码非流式响应。
pub fn decode_response(value: &Value) -> Result<CanonicalResponse, CodecError> {
    let object = value
        .as_object()
        .ok_or(CodecError::Invalid("response: not an object"))?;
    let choice = object
        .get("choices")
        .and_then(Value::as_array)
        .and_then(|choices| choices.first())
        .ok_or(CodecError::MissingField("choices[0]"))?;
    let message = choice
        .get("message")
        .and_then(Value::as_object)
        .ok_or(CodecError::MissingField("choices[0].message"))?;

    let mut blocks = Vec::new();
    match message.get("content") {
        Some(Value::String(text)) => blocks.push(ContentBlock::Text {
            text: text.clone(),
            cache_control: None,
        }),
        Some(Value::Array(parts)) => {
            for part in parts {
                blocks.push(decode_content_part(part)?);
            }
        }
        _ => {}
    }
    if let Some(tool_calls) = message.get("tool_calls").and_then(Value::as_array) {
        for call in tool_calls {
            blocks.push(decode_tool_call(call)?);
        }
    }

    Ok(CanonicalResponse {
        id: string(object, "id").unwrap_or_default(),
        model: string(object, "model").unwrap_or_default(),
        blocks,
        finish_reason: choice
            .get("finish_reason")
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
            .get("prompt_tokens")
            .and_then(Value::as_u64)
            .unwrap_or(0) as u32,
        output_tokens: object
            .get("completion_tokens")
            .and_then(Value::as_u64)
            .unwrap_or(0) as u32,
        cached_tokens: object
            .get("prompt_tokens_details")
            .and_then(|details| details.get("cached_tokens"))
            .and_then(Value::as_u64)
            .map(|value| value as u32),
        cache_write_tokens: None,
    }
}

/// 编码非流式响应。`created` 由调用方给（服务器用当前时间，测试用固定值）。
pub fn encode_response(response: &CanonicalResponse, created: u64) -> Value {
    let mut message = Map::new();
    message.insert("role".to_string(), Value::String("assistant".to_string()));

    let tool_calls: Vec<Value> = response
        .blocks
        .iter()
        .filter_map(|block| match block {
            ContentBlock::ToolUse {
                id,
                name,
                arguments,
            } => Some(json!({
                "id": id,
                "type": "function",
                "function": {"name": name, "arguments": arguments.to_string()}
            })),
            _ => None,
        })
        .collect();

    let text: String = response
        .blocks
        .iter()
        .filter_map(|block| match block {
            ContentBlock::Text { text, .. } => Some(text.as_str()),
            _ => None,
        })
        .collect();

    if text.is_empty() && !tool_calls.is_empty() {
        message.insert("content".to_string(), Value::Null);
    } else {
        message.insert("content".to_string(), Value::String(text));
    }
    if !tool_calls.is_empty() {
        message.insert("tool_calls".to_string(), Value::Array(tool_calls));
    }

    json!({
        "id": response.id,
        "object": "chat.completion",
        "created": created,
        "model": response.model,
        "choices": [{
            "index": 0,
            "message": message,
            "finish_reason": finish_reason_str(&response.finish_reason)
        }],
        "usage": {
            "prompt_tokens": response.usage.input_tokens,
            "completion_tokens": response.usage.output_tokens,
            "total_tokens": response.usage.input_tokens + response.usage.output_tokens,
            "prompt_tokens_details": {
                "cached_tokens": response.usage.cached_tokens.unwrap_or(0)
            }
        }
    })
}

fn finish_reason(value: &str) -> FinishReason {
    match value {
        "stop" => FinishReason::Stop,
        "tool_calls" => FinishReason::ToolUse,
        "length" => FinishReason::Length,
        "content_filter" => FinishReason::ContentFilter,
        other => FinishReason::Other(other.to_string()),
    }
}

fn finish_reason_str(reason: &FinishReason) -> &'static str {
    match reason {
        FinishReason::Stop => "stop",
        FinishReason::ToolUse => "tool_calls",
        FinishReason::Length => "length",
        FinishReason::ContentFilter => "content_filter",
        FinishReason::Error | FinishReason::Other(_) => "stop",
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

fn number_u64(object: &Map<String, Value>, key: &str) -> Option<u64> {
    object.get(key).and_then(Value::as_u64)
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

fn insert_opt_u64(object: &mut Map<String, Value>, key: &str, value: Option<u64>) {
    if let Some(value) = value {
        object.insert(key.to_string(), json!(value));
    }
}
