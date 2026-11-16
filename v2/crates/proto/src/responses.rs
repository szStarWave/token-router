//! OpenAI Responses 协议编解码（P0b-5）。
//!
//! Responses 的输入是“item 列表”而不是消息数组：`message` / `function_call` /
//! `function_call_output` 三类 item 映射到 canonical 的 assistant/tool 消息。
//! `instructions` 映射到 `system[]`；`previous_response_id` 只进 meta，不做历史重放。

use serde_json::{Map, Value, json};
use tr_contracts::{
    CanonicalMessage, CanonicalRequest, CanonicalResponse, CanonicalRole, ContentBlock,
    FinishReason, IngressProtocol, MediaSource, OutputParams, ReasoningParams, RequestMeta,
    SamplingParams, SystemBlock, ToolSpec, Usage,
};

use crate::CodecError;

/// 解码 Responses 请求。
pub fn decode_request(value: &Value) -> Result<CanonicalRequest, CodecError> {
    let object = value
        .as_object()
        .ok_or(CodecError::Invalid("request: not an object"))?;

    let mut request = CanonicalRequest {
        meta: RequestMeta {
            ingress_protocol: IngressProtocol::Responses,
            virtual_model: string(object, "model"),
            previous_response_id: string(object, "previous_response_id"),
            ..RequestMeta::default()
        },
        ..CanonicalRequest::default()
    };

    if let Some(instructions) = object.get("instructions").and_then(Value::as_str)
        && !instructions.is_empty()
    {
        request.system.push(SystemBlock {
            text: instructions.to_string(),
            cache_control: None,
        });
    }

    match object.get("input") {
        None | Some(Value::Null) => {}
        Some(Value::String(text)) => request.messages.push(CanonicalMessage {
            role: CanonicalRole::User,
            blocks: vec![ContentBlock::Text {
                text: text.clone(),
                cache_control: None,
            }],
            ..CanonicalMessage::default()
        }),
        Some(Value::Array(items)) => {
            for item in items {
                decode_item(item, &mut request)?;
            }
        }
        Some(_) => return Err(CodecError::Invalid("input: expected string or array")),
    }

    if let Some(tools) = object.get("tools").and_then(Value::as_array) {
        for tool in tools {
            if let Some(spec) = decode_tool(tool)? {
                request.tools.push(spec);
            }
        }
    }

    request.sampling = SamplingParams {
        temperature: number_f32(object, "temperature"),
        top_p: number_f32(object, "top_p"),
        ..SamplingParams::default()
    };
    request.output = OutputParams {
        max_tokens: number_u32(object, "max_output_tokens"),
        stream: object
            .get("stream")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        response_format: object
            .get("text")
            .and_then(Value::as_object)
            .and_then(|text| text.get("format"))
            .cloned(),
        tool_choice: object.get("tool_choice").cloned(),
        parallel_tool_calls: object.get("parallel_tool_calls").and_then(Value::as_bool),
        ..OutputParams::default()
    };
    request.reasoning = object
        .get("reasoning")
        .and_then(Value::as_object)
        .and_then(|reasoning| reasoning.get("effort"))
        .and_then(Value::as_str)
        .map(|effort| ReasoningParams {
            effort: Some(effort.to_string()),
            ..ReasoningParams::default()
        });

    for (key, value) in object {
        if !matches!(
            key.as_str(),
            "model"
                | "instructions"
                | "input"
                | "tools"
                | "tool_choice"
                | "parallel_tool_calls"
                | "max_output_tokens"
                | "temperature"
                | "top_p"
                | "stream"
                | "previous_response_id"
                | "reasoning"
                | "text"
        ) {
            request.passthrough.insert(key.clone(), value.clone());
        }
    }

    Ok(request)
}

fn decode_item(item: &Value, request: &mut CanonicalRequest) -> Result<(), CodecError> {
    let object = item
        .as_object()
        .ok_or(CodecError::Invalid("input item: not an object"))?;

    match object.get("type").and_then(Value::as_str) {
        Some("function_call") => {
            request.messages.push(CanonicalMessage {
                role: CanonicalRole::Assistant,
                blocks: vec![ContentBlock::ToolUse {
                    id: string(object, "call_id")
                        .or_else(|| string(object, "id"))
                        .ok_or(CodecError::MissingField("function_call.call_id"))?,
                    name: string(object, "name")
                        .ok_or(CodecError::MissingField("function_call.name"))?,
                    arguments: object
                        .get("arguments")
                        .and_then(Value::as_str)
                        .map(|raw| {
                            serde_json::from_str(raw)
                                .unwrap_or_else(|_| Value::String(raw.to_string()))
                        })
                        .unwrap_or(Value::Null),
                }],
                ..CanonicalMessage::default()
            });
            Ok(())
        }
        Some("function_call_output") => {
            request.messages.push(CanonicalMessage {
                role: CanonicalRole::Tool,
                blocks: vec![ContentBlock::ToolResult {
                    tool_use_id: string(object, "call_id")
                        .ok_or(CodecError::MissingField("function_call_output.call_id"))?,
                    content: decode_output_content(object.get("output"))?,
                    is_error: false,
                }],
                ..CanonicalMessage::default()
            });
            Ok(())
        }
        Some("reasoning") => Ok(()),
        Some("message") | None => decode_message_item(object, request),
        Some(other) => Err(CodecError::Unsupported(other.to_string())),
    }
}

fn decode_message_item(
    object: &Map<String, Value>,
    request: &mut CanonicalRequest,
) -> Result<(), CodecError> {
    let role = match object.get("role").and_then(Value::as_str) {
        Some("assistant") => CanonicalRole::Assistant,
        // Responses 的 developer message 等价于 system（o 系列/Codex 会发）。
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
                message.blocks.push(decode_part(part)?);
            }
        }
        Some(_) => return Err(CodecError::Invalid("content: expected string or array")),
        None => {}
    }

    request.messages.push(message);
    Ok(())
}

fn decode_part(value: &Value) -> Result<ContentBlock, CodecError> {
    let object = value
        .as_object()
        .ok_or(CodecError::Invalid("content part: not an object"))?;
    match object.get("type").and_then(Value::as_str) {
        Some("input_text") | Some("output_text") | Some("text") => Ok(ContentBlock::Text {
            text: object
                .get("text")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            cache_control: None,
        }),
        Some("input_image") => {
            let url = object
                .get("image_url")
                .and_then(Value::as_str)
                .ok_or(CodecError::MissingField("input_image.image_url"))?;
            Ok(ContentBlock::Image {
                source: MediaSource::Url {
                    url: url.to_string(),
                },
                detail: string(object, "detail"),
            })
        }
        Some(other) => Err(CodecError::Unsupported(other.to_string())),
        None => Err(CodecError::MissingField("content[].type")),
    }
}

fn decode_output_content(value: Option<&Value>) -> Result<Vec<ContentBlock>, CodecError> {
    match value {
        None | Some(Value::Null) => Ok(Vec::new()),
        Some(Value::String(text)) => Ok(vec![ContentBlock::Text {
            text: text.clone(),
            cache_control: None,
        }]),
        Some(Value::Array(parts)) => parts.iter().map(decode_part).collect(),
        Some(other) => Ok(vec![ContentBlock::Text {
            text: other.to_string(),
            cache_control: None,
        }]),
    }
}

/// 解码一个 tool。只接受 function 工具；内置工具（web_search / local_shell 等）
/// 无法映射到 canonical，直接跳过（对话仍可正常进行，只是这些工具不由路由转发）。
fn decode_tool(value: &Value) -> Result<Option<ToolSpec>, CodecError> {
    let object = value
        .as_object()
        .ok_or(CodecError::Invalid("tool: not an object"))?;
    let kind = object
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or("function");
    if kind != "function" {
        return Ok(None);
    }
    let name = string(object, "name")
        .or_else(|| {
            object
                .get("function")
                .and_then(Value::as_object)
                .and_then(|function| function.get("name"))
                .and_then(Value::as_str)
                .map(str::to_string)
        })
        .ok_or(CodecError::MissingField("tool.name"))?;
    Ok(Some(ToolSpec {
        name,
        description: string(object, "description").or_else(|| {
            object
                .get("function")
                .and_then(Value::as_object)
                .and_then(|function| function.get("description"))
                .and_then(Value::as_str)
                .map(str::to_string)
        }),
        parameters: object
            .get("parameters")
            .or_else(|| {
                object
                    .get("function")
                    .and_then(Value::as_object)
                    .and_then(|function| function.get("parameters"))
            })
            .cloned()
            .unwrap_or(Value::Null),
        strict: object.get("strict").and_then(Value::as_bool),
    }))
}

// ---------------------------------------------------------------------------
// 编码
// ---------------------------------------------------------------------------

/// 编码为 Responses wire 请求。
pub fn encode_request(request: &CanonicalRequest) -> Value {
    let mut object = Map::new();

    if let Some(model) = &request.meta.virtual_model {
        object.insert("model".to_string(), Value::String(model.clone()));
    }

    let instructions = request
        .system
        .iter()
        .map(|block| block.text.as_str())
        .collect::<Vec<_>>()
        .join("\n\n");
    if !instructions.is_empty() {
        object.insert("instructions".to_string(), Value::String(instructions));
    }

    let mut input: Vec<Value> = Vec::new();
    for message in &request.messages {
        match message.role {
            CanonicalRole::System => {
                let text: String = message
                    .blocks
                    .iter()
                    .filter_map(|block| match block {
                        ContentBlock::Text { text, .. } => Some(text.as_str()),
                        _ => None,
                    })
                    .collect();
                if !text.is_empty() {
                    input.push(json!({
                        "type": "message",
                        "role": "user",
                        "content": [{"type": "input_text", "text": text}]
                    }));
                }
            }
            CanonicalRole::Tool => {
                for block in &message.blocks {
                    if let ContentBlock::ToolResult {
                        tool_use_id,
                        content,
                        is_error,
                    } = block
                    {
                        let output = content
                            .iter()
                            .filter_map(|block| match block {
                                ContentBlock::Text { text, .. } => Some(text.as_str()),
                                _ => None,
                            })
                            .collect::<String>();
                        input.push(json!({
                            "type": "function_call_output",
                            "call_id": tool_use_id,
                            "output": if *is_error { format!("error: {output}") } else { output }
                        }));
                    }
                }
            }
            CanonicalRole::Assistant => {
                let text: String = message
                    .blocks
                    .iter()
                    .filter_map(|block| match block {
                        ContentBlock::Text { text, .. } => Some(text.as_str()),
                        _ => None,
                    })
                    .collect();
                if !text.is_empty() {
                    input.push(json!({
                        "type": "message",
                        "role": "assistant",
                        "content": [{"type": "output_text", "text": text}]
                    }));
                }
                for block in &message.blocks {
                    if let ContentBlock::ToolUse {
                        id,
                        name,
                        arguments,
                    } = block
                    {
                        input.push(json!({
                            "type": "function_call",
                            "call_id": id,
                            "name": name,
                            "arguments": arguments.to_string()
                        }));
                    }
                }
            }
            CanonicalRole::User => {
                let content: Vec<Value> = message
                    .blocks
                    .iter()
                    .map(|block| match block {
                        ContentBlock::Text { text, .. } => {
                            json!({"type": "input_text", "text": text})
                        }
                        ContentBlock::Image { source, detail } => {
                            let url = match source {
                                MediaSource::Url { url } => url.clone(),
                                MediaSource::Base64 { media_type, data } => {
                                    format!("data:{media_type};base64,{data}")
                                }
                                MediaSource::File { file_id } => file_id.clone(),
                            };
                            json!({"type": "input_image", "image_url": url, "detail": detail})
                        }
                        other => json!({"type": "input_text", "text": format!("{other:?}")}),
                    })
                    .collect();
                input.push(json!({"type": "message", "role": "user", "content": content}));
            }
        }
    }
    object.insert("input".to_string(), Value::Array(input));

    if !request.tools.is_empty() {
        let tools = request
            .tools
            .iter()
            .map(|tool| {
                let mut value = json!({
                    "type": "function",
                    "name": tool.name,
                    "parameters": tool.parameters
                });
                if let Some(description) = &tool.description {
                    value["description"] = Value::String(description.clone());
                }
                if let Some(strict) = tool.strict {
                    value["strict"] = Value::Bool(strict);
                }
                value
            })
            .collect();
        object.insert("tools".to_string(), Value::Array(tools));
    }

    insert_opt_u32(&mut object, "max_output_tokens", request.output.max_tokens);
    insert_opt_f32(&mut object, "temperature", request.sampling.temperature);
    insert_opt_f32(&mut object, "top_p", request.sampling.top_p);
    object.insert("stream".to_string(), Value::Bool(request.output.stream));

    if let Some(choice) = &request.output.tool_choice {
        object.insert("tool_choice".to_string(), choice.clone());
    }
    if let Some(parallel) = request.output.parallel_tool_calls {
        object.insert("parallel_tool_calls".to_string(), Value::Bool(parallel));
    }
    if let Some(previous) = &request.meta.previous_response_id {
        object.insert(
            "previous_response_id".to_string(),
            Value::String(previous.clone()),
        );
    }
    if let Some(reasoning) = &request.reasoning
        && let Some(effort) = &reasoning.effort
    {
        object.insert("reasoning".to_string(), json!({"effort": effort}));
    }
    if let Some(format) = &request.output.response_format {
        object.insert("text".to_string(), json!({"format": format}));
    }

    for (key, value) in &request.passthrough {
        object.entry(key.clone()).or_insert_with(|| value.clone());
    }

    Value::Object(object)
}

/// 解码非流式响应。
pub fn decode_response(value: &Value) -> Result<CanonicalResponse, CodecError> {
    let object = value
        .as_object()
        .ok_or(CodecError::Invalid("response: not an object"))?;

    let mut blocks = Vec::new();
    let mut has_tool_call = false;
    if let Some(output) = object.get("output").and_then(Value::as_array) {
        for item in output {
            let Some(item) = item.as_object() else {
                continue;
            };
            match item.get("type").and_then(Value::as_str) {
                Some("message") => {
                    if let Some(content) = item.get("content").and_then(Value::as_array) {
                        for part in content {
                            blocks.push(decode_part(part)?);
                        }
                    }
                }
                Some("function_call") => {
                    has_tool_call = true;
                    blocks.push(ContentBlock::ToolUse {
                        id: string(item, "call_id")
                            .or_else(|| string(item, "id"))
                            .ok_or(CodecError::MissingField("function_call.call_id"))?,
                        name: string(item, "name")
                            .ok_or(CodecError::MissingField("function_call.name"))?,
                        arguments: item
                            .get("arguments")
                            .and_then(Value::as_str)
                            .map(|raw| {
                                serde_json::from_str(raw)
                                    .unwrap_or_else(|_| Value::String(raw.to_string()))
                            })
                            .unwrap_or(Value::Null),
                    });
                }
                _ => {}
            }
        }
    }

    let status = object
        .get("status")
        .and_then(Value::as_str)
        .unwrap_or("completed");
    let finish_reason = if has_tool_call {
        FinishReason::ToolUse
    } else if status == "incomplete" {
        FinishReason::Length
    } else {
        FinishReason::Stop
    };

    Ok(CanonicalResponse {
        id: string(object, "id").unwrap_or_default(),
        model: string(object, "model").unwrap_or_default(),
        blocks,
        finish_reason,
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
            .get("input_tokens_details")
            .and_then(|details| details.get("cached_tokens"))
            .and_then(Value::as_u64)
            .map(|value| value as u32),
        cache_write_tokens: None,
    }
}

/// 编码非流式响应。
pub fn encode_response(response: &CanonicalResponse, created_at: u64) -> Value {
    let mut output: Vec<Value> = Vec::new();

    let text: String = response
        .blocks
        .iter()
        .filter_map(|block| match block {
            ContentBlock::Text { text, .. } => Some(text.as_str()),
            _ => None,
        })
        .collect();
    if !text.is_empty() {
        output.push(json!({
            "type": "message",
            "id": format!("msg_{}", response.id),
            "status": "completed",
            "role": "assistant",
            "content": [{"type": "output_text", "text": text, "annotations": []}]
        }));
    }
    for block in &response.blocks {
        if let ContentBlock::ToolUse {
            id,
            name,
            arguments,
        } = block
        {
            output.push(json!({
                "type": "function_call",
                "id": format!("fc_{id}"),
                "call_id": id,
                "name": name,
                "arguments": arguments.to_string(),
                "status": "completed"
            }));
        }
    }

    json!({
        "id": response.id,
        "object": "response",
        "created_at": created_at,
        "status": "completed",
        "model": response.model,
        "output": output,
        "usage": {
            "input_tokens": response.usage.input_tokens,
            "output_tokens": response.usage.output_tokens,
            "total_tokens": response.usage.input_tokens + response.usage.output_tokens,
            "input_tokens_details": {"cached_tokens": response.usage.cached_tokens.unwrap_or(0)}
        }
    })
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
    fn developer_role_maps_to_system() {
        // Codex/o 系列会用 developer message；它等价于 system，不能拒绝。
        let request = decode_request(&serde_json::json!({
            "model": "auto",
            "input": [
                {
                    "type": "message",
                    "role": "developer",
                    "content": [{"type": "input_text", "text": "follow the rules"}]
                },
                {
                    "type": "message",
                    "role": "user",
                    "content": [{"type": "input_text", "text": "hi"}]
                }
            ]
        }))
        .expect("decode developer message");

        assert_eq!(request.messages[0].role, CanonicalRole::System);
        assert_eq!(request.messages[1].role, CanonicalRole::User);
    }
}

#[cfg(test)]
mod tool_tests {
    use super::*;

    #[test]
    fn builtin_tools_are_skipped_and_function_names_are_read() {
        let request = decode_request(&serde_json::json!({
            "model": "auto",
            "input": "hi",
            "tools": [
                {"type": "web_search_preview"},
                {"type": "local_shell"},
                {"type": "function", "function": {"name": "shell_exec", "parameters": {"type": "object"}}}
            ]
        }))
        .expect("decode tools");

        assert_eq!(request.tools.len(), 1);
        assert_eq!(request.tools[0].name, "shell_exec");
    }
}
