use std::pin::Pin;
use std::sync::Arc;
use std::time::Instant;

use async_stream::stream;
use bytes::Bytes;
use futures::StreamExt;

use crate::gateway::agent_usage::AgentCloudUsageStore;
use crate::gateway::stats::metrics::{
    inspect_sse_bytes, FinalResponseMetrics, StreamChunkAccumulator, UpstreamCallMetrics,
};
use crate::gateway::stats::{AuthKeyContext, GatewayStats};

pub type SseStream = Pin<Box<dyn futures::Stream<Item = Result<Bytes, std::io::Error>> + Send>>;

pub struct StreamRecordContext {
    pub stats: Arc<GatewayStats>,
    pub tier: &'static str,
    pub prompt_fallback: u32,
    pub cloud_input_saved: u32,
    pub record_cloud_saved: bool,
    pub edge_guard: Option<crate::gateway::edge_load::EdgeInferenceGuard>,
    pub agent_usage: Option<Arc<AgentCloudUsageStore>>,
    pub agent_id: Option<String>,
    pub auth_key: Option<AuthKeyContext>,
}

pub fn instrument_stream(inner: SseStream, ctx: StreamRecordContext) -> SseStream {
    Box::pin(stream! {
        let start = Instant::now();
        let mut acc = StreamChunkAccumulator::default();
        let mut inner = inner;
        while let Some(item) = inner.next().await {
            if let Ok(bytes) = &item {
                let elapsed_ms = start.elapsed().as_millis() as u64;
                inspect_sse_bytes(bytes, &mut acc, elapsed_ms);
            }
            yield item;
        }

        let latency_ms = start.elapsed().as_millis() as u64;
        let prompt = acc.resolve_prompt(ctx.prompt_fallback);
        let completion = acc.resolve_completion();
        let cached = acc.cached_tokens();

        ctx.stats.record_upstream_metrics(
            &UpstreamCallMetrics {
                tier: ctx.tier,
                prompt_tokens: prompt,
                completion_tokens: completion,
                cached_tokens: cached,
                latency_ms,
                ttft_ms: acc.first_token_ms,
                last_token_ms: acc.last_token_ms,
                stream: true,
            },
            ctx.auth_key.as_ref(),
        );
        ctx.stats
            .record_completion_tokens(completion, ctx.auth_key.as_ref());
        ctx.stats.record_final_response(
            &FinalResponseMetrics {
                served_tier: ctx.tier,
                cloud_input_saved: if ctx.record_cloud_saved {
                    ctx.cloud_input_saved
                } else {
                    0
                },
                completion_tokens: completion,
            },
            ctx.auth_key.as_ref(),
        );

        if ctx.tier == "cloud" {
            if let (Some(usage), Some(ref agent_id)) = (ctx.agent_usage.as_ref(), ctx.agent_id.as_ref()) {
                usage.record_tokens(agent_id, (prompt + completion) as u64);
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gateway::api::openai::{ChatCompletionRequest, Message, Role};
    use crate::gateway::stats::GatewayStats;
    use futures::stream::{self, StreamExt};
    use serde_json::json;

    fn stub_sse_stream(req: &ChatCompletionRequest, tier: &str) -> SseStream {
        let content = if tier == "edge" {
            "[token-router] edge stub — configure [upstream.edge] in ~/.token-router/config.toml"
        } else {
            "[token-router] cloud stub — configure [upstream.cloud] in ~/.token-router/config.toml"
        };

        let id = format!("token-router-stub-{}", uuid::Uuid::new_v4());
        let created = now_epoch();
        let model = req.model.clone();

        let first = json!({
            "id": id,
            "object": "chat.completion.chunk",
            "created": created,
            "model": model,
            "choices": [{
                "index": 0,
                "delta": { "role": "assistant", "content": content },
                "finish_reason": null
            }]
        });
        let last = json!({
            "id": id,
            "object": "chat.completion.chunk",
            "created": created,
            "model": model,
            "choices": [{
                "index": 0,
                "delta": {},
                "finish_reason": "stop"
            }],
            "usage": {
                "prompt_tokens": 10,
                "completion_tokens": 5,
                "prompt_tokens_details": { "cached_tokens": 3 }
            }
        });

        let events = vec![
            format!("data: {first}\n\n"),
            format!("data: {last}\n\n"),
            "data: [DONE]\n\n".to_string(),
        ];

        Box::pin(stream::iter(events).map(|line| Ok(Bytes::from(line))))
    }

    fn now_epoch() -> u64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0)
    }

    #[tokio::test]
    async fn stub_sse_emits_openai_chunks() {
        let req = ChatCompletionRequest {
            model: "flowy-auto".to_string(),
            messages: vec![Message {
                role: Role::User,
                content: Some("hi".to_string()),
                content_parts: None,
                tool_calls: None,
                tool_call_id: None,
            }],
            tools: vec![],
            stream: true,
            tool_choice: None,
            max_tokens: None,
            ..Default::default()
        };
        let mut stream = stub_sse_stream(&req, "edge");
        let first = stream.next().await.unwrap().unwrap();
        let text = String::from_utf8(first.to_vec()).unwrap();
        assert!(text.starts_with("data: "));
        assert!(text.contains("chat.completion.chunk"));
    }

    #[tokio::test]
    async fn instrument_stream_records_metrics() {
        let req = ChatCompletionRequest {
            model: "flowy-auto".to_string(),
            messages: vec![Message {
                role: Role::User,
                content: Some("hi".to_string()),
                content_parts: None,
                tool_calls: None,
                tool_call_id: None,
            }],
            tools: vec![],
            stream: true,
            tool_choice: None,
            max_tokens: None,
            ..Default::default()
        };
        let stats = GatewayStats::new_in_memory();
        let inner = stub_sse_stream(&req, "edge");
        let mut stream = instrument_stream(
            inner,
            StreamRecordContext {
                stats: stats.clone(),
                tier: "edge",
                prompt_fallback: 100,
                cloud_input_saved: 100,
                record_cloud_saved: true,
                edge_guard: None,
                agent_usage: None,
                agent_id: None,
                auth_key: None,
            },
        );
        while stream.next().await.is_some() {}

        let snap = stats.snapshot(
            crate::gateway::stats::StatsScope::Session,
            1,
            None,
            None,
            None,
            None,
            &[],
        );
        assert_eq!(snap.token_breakdown.edge.input, 10);
        assert_eq!(snap.token_breakdown.edge.output, 5);
        assert_eq!(snap.cache.cached_tokens, 3);
        assert_eq!(snap.served.edge, 1);
    }

    #[tokio::test]
    async fn instrument_stream_counts_tool_call_chunks_without_usage() {
        let req = ChatCompletionRequest {
            model: "flowy-auto".to_string(),
            messages: vec![Message {
                role: Role::User,
                content: Some("hi".to_string()),
                content_parts: None,
                tool_calls: None,
                tool_call_id: None,
            }],
            tools: vec![],
            stream: true,
            tool_choice: None,
            max_tokens: None,
            ..Default::default()
        };
        let id = "stub-tool";
        let events = vec![
            format!(
                "data: {}\n\n",
                json!({
                    "choices": [{
                        "delta": {
                            "tool_calls": [{
                                "index": 0,
                                "id": "call_1",
                                "function": { "name": "read", "arguments": "" }
                            }]
                        }
                    }]
                })
            ),
            format!(
                "data: {}\n\n",
                json!({
                    "choices": [{
                        "delta": {
                            "tool_calls": [{
                                "index": 0,
                                "function": { "arguments": "{}" }
                            }]
                        }
                    }]
                })
            ),
            "data: [DONE]\n\n".to_string(),
        ];
        let inner: SseStream = Box::pin(stream::iter(events).map(|line| Ok(Bytes::from(line))));
        let stats = GatewayStats::new_in_memory();
        let mut stream = instrument_stream(
            inner,
            StreamRecordContext {
                stats: stats.clone(),
                tier: "edge",
                prompt_fallback: 10,
                cloud_input_saved: 0,
                record_cloud_saved: false,
                edge_guard: None,
                agent_usage: None,
                agent_id: None,
                auth_key: None,
            },
        );
        while stream.next().await.is_some() {}

        let snap = stats.snapshot(
            crate::gateway::stats::StatsScope::Session,
            1,
            None,
            None,
            None,
            None,
            &[],
        );
        assert_eq!(snap.token_breakdown.edge.output, 2);
    }
}
