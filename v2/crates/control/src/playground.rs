//! Playground 执行（v3 P5-D）：管理面测试请求。
//!
//! 两种目标：
//! - `candidate:<id>`：**绕过路由**，直接对该物理候选执行（不计路由决策）；
//! - `virtual:<id>`：走该虚拟模型的正常路由（记录账本）。
//!
//! 计时：TTFT 取首事件，duration 取总耗时；上游流式事件在这里聚合成完整文本。

use std::time::Instant;

use tr_contracts::{
    CanonicalMessage, CanonicalRequest, CanonicalRole, CanonicalStreamEvent, ContentBlock,
    IngressProtocol, OutputParams, PlaygroundExecuteRequest, PlaygroundExecuteResponse,
    PlaygroundMessage, RequestReason, SamplingParams, SystemBlock,
};
use tr_upstream::stream::StreamItem;

use crate::error::AdminApiError;
use crate::state::ControlState;

fn role_of(message: &PlaygroundMessage) -> CanonicalRole {
    match message.role.as_str() {
        "assistant" => CanonicalRole::Assistant,
        "system" => CanonicalRole::System,
        "tool" => CanonicalRole::Tool,
        _ => CanonicalRole::User,
    }
}

fn build_request(body: &PlaygroundExecuteRequest) -> (CanonicalRequest, Vec<String>) {
    let mut system: Vec<SystemBlock> = Vec::new();
    let mut messages = Vec::new();
    if let Some(system_text) = &body.system {
        if !system_text.trim().is_empty() {
            system.push(SystemBlock {
                text: system_text.clone(),
                cache_control: None,
            });
        }
    }
    for message in body.messages.as_deref().unwrap_or(&[]) {
        messages.push(CanonicalMessage {
            role: role_of(message),
            blocks: vec![ContentBlock::Text {
                text: message.content.clone(),
                cache_control: None,
            }],
            tool_call_id: None,
            name: None,
        });
    }
    let sampling = SamplingParams {
        temperature: body.temperature,
        ..SamplingParams::default()
    };
    let output = OutputParams {
        max_tokens: body.max_tokens,
        stream: true,
        ..OutputParams::default()
    };
    let meta = tr_contracts::RequestMeta {
        ingress_protocol: IngressProtocol::ChatCompletions,
        ..tr_contracts::RequestMeta::default()
    };
    let request = CanonicalRequest {
        messages,
        tools: Vec::new(),
        system,
        sampling,
        output,
        reasoning: None,
        meta,
        passthrough: Default::default(),
    };
    (request, Vec::new())
}

/// 聚合流式事件为完整结果。
async fn collect_stream(
    started: Instant,
    served_model: String,
    mut events: tokio::sync::mpsc::Receiver<StreamItem>,
) -> PlaygroundExecuteResponse {
    let mut text = String::new();
    let mut reasoning = String::new();
    let mut ttft_ms: Option<u64> = None;
    let mut input_tokens = 0;
    let mut output_tokens = 0;
    let mut finish_reason = None;
    let mut error = None;

    while let Some(item) = events.recv().await {
        match item {
            StreamItem::Event(event) => {
                if ttft_ms.is_none() {
                    ttft_ms = Some(started.elapsed().as_millis() as u64);
                }
                match event {
                    CanonicalStreamEvent::ContentDelta { text: delta } => text.push_str(&delta),
                    CanonicalStreamEvent::ReasoningDelta { text: delta } => {
                        reasoning.push_str(&delta)
                    }
                    CanonicalStreamEvent::Usage {
                        input,
                        output,
                        cached: _,
                    } => {
                        input_tokens = input;
                        output_tokens = output;
                    }
                    CanonicalStreamEvent::Finished { reason } => {
                        finish_reason = Some(format!("{reason:?}").to_lowercase());
                    }
                    CanonicalStreamEvent::Error { message } => error = Some(message),
                    _ => {}
                }
            }
            StreamItem::Failed(failure) => {
                error = Some(
                    failure
                        .message
                        .unwrap_or_else(|| format!("upstream error ({:?})", failure.class)),
                );
            }
            StreamItem::Ended => break,
        }
    }

    PlaygroundExecuteResponse {
        ok: error.is_none(),
        served_model,
        text,
        reasoning,
        ttft_ms,
        duration_ms: started.elapsed().as_millis() as u64,
        input_tokens,
        output_tokens,
        finish_reason,
        error,
    }
}

/// 执行一次 playground 请求。
pub async fn execute(
    state: &ControlState,
    body: PlaygroundExecuteRequest,
) -> Result<PlaygroundExecuteResponse, AdminApiError> {
    let (kind, id) = body.parse_target().ok_or_else(|| {
        AdminApiError::invalid_request("target 需要 `candidate:<id>` 或 `virtual:<id>`")
    })?;
    let (request, _) = build_request(&body);

    if kind == "candidate" {
        let candidate = state
            .pipeline
            .catalog()
            .candidate(id)
            .cloned()
            .ok_or_else(|| AdminApiError::not_found(format!("candidate `{id}` not found")))?;
        let started = Instant::now();
        let attempt = tr_upstream::HttpUpstream::default()
            .execute_stream(&candidate, &request)
            .await
            .map_err(|failure| {
                AdminApiError::new(
                    crate::error::ApiErrorType::ServerUnavailable,
                    format!(
                        "upstream failed: {}",
                        failure.message.unwrap_or_else(|| "unknown".to_string())
                    ),
                )
            })?;
        Ok(collect_stream(started, candidate.id.clone(), attempt.events).await)
    } else {
        // 虚拟模型：走正常路由（记录账本），但标记为 Direct 原因。
        let context = tr_agent::pipeline::RequestContext {
            server_session_id: Some("playground".to_string()),
            request_reason: Some(RequestReason::Direct),
            branch_id: None,
            authority: Some(tr_contracts::RequestAuthority {
                authority_id: "playground".to_string(),
                ..Default::default()
            }),
        };
        let started = Instant::now();
        let streaming = state
            .pipeline
            .handle_stream(request, context)
            .await
            .map_err(|error| {
                AdminApiError::new(
                    crate::error::ApiErrorType::ServerUnavailable,
                    format!("playground route failed: {error}"),
                )
            })?;
        Ok(collect_pipeline_stream(started, id.to_string(), streaming.events).await)
    }
}

/// 聚合 `agent` 管线的流式事件（与上游 `StreamItem` 形状不同）。
async fn collect_pipeline_stream(
    started: Instant,
    served_model: String,
    mut events: tokio::sync::mpsc::Receiver<tr_agent::pipeline::StreamChunk>,
) -> PlaygroundExecuteResponse {
    let mut text = String::new();
    let mut reasoning = String::new();
    let mut ttft_ms: Option<u64> = None;
    let mut input_tokens = 0;
    let mut output_tokens = 0;
    let mut finish_reason = None;
    let mut error = None;

    while let Some(chunk) = events.recv().await {
        match chunk {
            tr_agent::pipeline::StreamChunk::Event(event) => {
                if ttft_ms.is_none() {
                    ttft_ms = Some(started.elapsed().as_millis() as u64);
                }
                match event {
                    CanonicalStreamEvent::ContentDelta { text: delta } => text.push_str(&delta),
                    CanonicalStreamEvent::ReasoningDelta { text: delta } => {
                        reasoning.push_str(&delta)
                    }
                    CanonicalStreamEvent::Usage {
                        input,
                        output,
                        cached: _,
                    } => {
                        input_tokens = input;
                        output_tokens = output;
                    }
                    CanonicalStreamEvent::Finished { reason } => {
                        finish_reason = Some(format!("{reason:?}").to_lowercase());
                    }
                    CanonicalStreamEvent::Error { message } => error = Some(message),
                    _ => {}
                }
            }
            tr_agent::pipeline::StreamChunk::Error { message } => error = Some(message),
            tr_agent::pipeline::StreamChunk::Done => break,
        }
    }

    PlaygroundExecuteResponse {
        ok: error.is_none(),
        served_model,
        text,
        reasoning,
        ttft_ms,
        duration_ms: started.elapsed().as_millis() as u64,
        input_tokens,
        output_tokens,
        finish_reason,
        error,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn target_parsing() {
        let mut request = PlaygroundExecuteRequest {
            target: "candidate:deepseek-r1-a".to_string(),
            messages: None,
            system: None,
            max_tokens: None,
            temperature: None,
        };
        assert_eq!(request.parse_target(), Some(("candidate", "deepseek-r1-a")));
        request.target = "virtual:auto".to_string();
        assert_eq!(request.parse_target(), Some(("virtual", "auto")));
        request.target = "auto".to_string();
        assert_eq!(request.parse_target(), None);
        request.target = "candidate:".to_string();
        assert_eq!(request.parse_target(), None);
    }

    #[test]
    fn build_request_maps_messages_and_system() {
        let request = PlaygroundExecuteRequest {
            target: "virtual:auto".to_string(),
            messages: Some(vec![
                PlaygroundMessage {
                    role: "user".to_string(),
                    content: "hi".to_string(),
                },
                PlaygroundMessage {
                    role: "assistant".to_string(),
                    content: "hello".to_string(),
                },
            ]),
            system: Some("be brief".to_string()),
            max_tokens: Some(64),
            temperature: Some(0.2),
        };
        let (canonical, _) = build_request(&request);
        assert_eq!(canonical.system.len(), 1);
        assert_eq!(canonical.messages.len(), 2);
        assert_eq!(canonical.sampling.temperature, Some(0.2));
        assert_eq!(canonical.output.max_tokens, Some(64));
        assert!(canonical.output.stream, "playground 内部走流式以获得 TTFT");
    }
}
