use futures::StreamExt;
use reqwest::Client;
use std::time::Instant;

use crate::gateway::api::openai::{ChatCompletionRequest, ChatCompletionResponse, FlowyMeta};
use crate::gateway::config::AppConfig;
use crate::gateway::config_manager::ConfigManager;
use crate::gateway::error::{AppError, AppResult};
use crate::gateway::multimodal::{MultimodalStore, MultimodalStrategy};
use std::sync::Arc;

use crate::gateway::edge_load::{EdgeInferenceGuard, EdgeInferenceTracker};
use crate::gateway::routing::{RouteDecision, RouteTier, WorkStrategy};
use crate::gateway::stats::metrics::{
    tokens_from_response, FinalResponseMetrics, UpstreamCallMetrics,
};
use crate::gateway::stats::GatewayStats;
use crate::gateway::upstream::sse::{instrument_stream, StreamRecordContext, SseStream};
use crate::gateway::upstream::verify::cloud_verifies_edge;

struct UpstreamTarget {
    base_url: Option<String>,
    api_key: Option<String>,
    model: Option<String>,
    tier: &'static str,
}

#[derive(Clone)]
pub struct UpstreamClient {
    http: Client,
    config_mgr: Arc<ConfigManager>,
    stats: Arc<GatewayStats>,
    multimodal: Arc<MultimodalStore>,
    edge_load: Arc<EdgeInferenceTracker>,
}

impl UpstreamClient {
    pub fn new(
        config_mgr: Arc<ConfigManager>,
        stats: Arc<GatewayStats>,
        multimodal: Arc<MultimodalStore>,
        edge_load: Arc<EdgeInferenceTracker>,
    ) -> Self {
        Self {
            http: Client::new(),
            config_mgr,
            stats,
            multimodal,
            edge_load,
        }
    }

    fn cfg(&self) -> AppConfig {
        self.config_mgr.get()
    }

    pub fn edge_configured(&self) -> bool {
        self.cfg().edge_base_url.is_some()
    }

    pub async fn complete(
        &self,
        req: &ChatCompletionRequest,
        decision: &RouteDecision,
    ) -> AppResult<ChatCompletionResponse> {
        if decision.multimodal_strategy != MultimodalStrategy::None {
            return self.complete_multimodal(req, decision).await;
        }

        if decision.work_strategy == WorkStrategy::CachedEdge {
            let resp = self
                .call_target(req, self.target_edge(), decision.tokens_in_estimate)
                .await?;
            return Ok(self.finish_non_stream(resp, decision, "edge", false));
        }

        if decision.work_strategy == WorkStrategy::Verify {
            return self.complete_work_verify(req, decision).await;
        }

        match decision.route {
            RouteTier::Edge => {
                let resp = self
                    .call_target(req, self.target_edge(), decision.tokens_in_estimate)
                    .await?;
                Ok(self.finish_non_stream(resp, decision, "edge", false))
            }
            RouteTier::Cloud => {
                let t = self.target_cloud();
                let resp = self.call_target(req, t, decision.tokens_in_estimate).await?;
                Ok(self.finish_non_stream(resp, decision, "cloud", false))
            }
            RouteTier::Cascade => {
                let edge = self.target_edge();
                let edge_tried = edge.base_url.is_some();
                if edge_tried {
                    if let Ok(resp) = self
                        .call_target(req, edge, decision.tokens_in_estimate)
                        .await
                    {
                        if cascade_gate_pass(&resp) {
                            self.stats.record_cascade_edge_ok();
                            return Ok(self.finish_non_stream(resp, decision, "edge", false));
                        }
                    }
                    self.stats.record_cascade_fallback();
                }
                let cloud = self.target_cloud();
                let resp = self
                    .call_target(req, cloud, decision.tokens_in_estimate)
                    .await?;
                Ok(self.finish_non_stream(resp, decision, "cloud", edge_tried))
            }
        }
    }

    pub async fn stream(
        &self,
        req: &ChatCompletionRequest,
        decision: &RouteDecision,
    ) -> AppResult<(SseStream, bool)> {
        if decision.multimodal_strategy != MultimodalStrategy::None {
            return self.stream_multimodal(req, decision).await;
        }

        if decision.work_strategy == WorkStrategy::CachedEdge {
            let edge = self.target_edge();
            return self
                .stream_target(req, edge, decision)
                .await
                .map(|s| (s, false));
        }

        if decision.work_strategy == WorkStrategy::Verify {
            // Streaming cannot run cloud verification; try edge then fall back on HTTP error.
            return self.stream_cascade(req, decision).await;
        }

        match decision.route {
            RouteTier::Edge => self
                .stream_target(req, self.target_edge(), decision)
                .await
                .map(|s| (s, false)),
            RouteTier::Cloud => {
                self.stream_target(req, self.target_cloud(), decision)
                    .await
                    .map(|s| (s, false))
            }
            RouteTier::Cascade => self.stream_cascade(req, decision).await,
        }
    }

    async fn complete_multimodal(
        &self,
        req: &ChatCompletionRequest,
        decision: &RouteDecision,
    ) -> AppResult<ChatCompletionResponse> {
        match decision.multimodal_strategy {
            MultimodalStrategy::CachedEdge | MultimodalStrategy::CachedEdgeFallback => {
                let resp = self
                    .call_target(req, self.target_edge(), decision.tokens_in_estimate)
                    .await?;
                Ok(self.finish_non_stream(resp, decision, "edge", false))
            }
            MultimodalStrategy::CachedCloud => {
                let resp = self
                    .call_target(req, self.target_cloud(), decision.tokens_in_estimate)
                    .await?;
                Ok(self.finish_non_stream(resp, decision, "cloud", true))
            }
            MultimodalStrategy::Probe => {
                self.complete_multimodal_probe(req, decision).await
            }
            MultimodalStrategy::None => unreachable!(),
        }
    }

    async fn complete_multimodal_probe(
        &self,
        req: &ChatCompletionRequest,
        decision: &RouteDecision,
    ) -> AppResult<ChatCompletionResponse> {
        let model = &req.model;
        let edge = self.target_edge();

        if edge.base_url.is_some() {
            match self
                .call_target(req, edge, decision.tokens_in_estimate)
                .await
            {
                Ok(resp) if cascade_gate_pass(&resp) => {
                    self.multimodal.record_edge(&self.cfg(), model, true);
                    self.stats.record_cascade_edge_ok();
                    return Ok(self.finish_non_stream(resp, decision, "edge", false));
                }
                Ok(_) => self.multimodal.record_edge(&self.cfg(), model, false),
                Err(_) => self.multimodal.record_edge(&self.cfg(), model, false),
            }
        }

        self.stats.record_cascade_fallback();
        let cloud = self.target_cloud();
        match self
            .call_target(req, cloud, decision.tokens_in_estimate)
            .await
        {
            Ok(resp) => {
                self.multimodal.record_cloud(&self.cfg(), model, true);
                return Ok(self.finish_non_stream(resp, decision, "cloud", true));
            }
            Err(_) => self.multimodal.record_cloud(&self.cfg(), model, false),
        }

        let resp = self
            .call_target(req, self.target_edge(), decision.tokens_in_estimate)
            .await?;
        Ok(self.finish_non_stream(resp, decision, "edge", true))
    }

    async fn complete_work_verify(
        &self,
        req: &ChatCompletionRequest,
        decision: &RouteDecision,
    ) -> AppResult<ChatCompletionResponse> {
        let edge = self.target_edge();
        let edge_tried = edge.base_url.is_some();

        if edge.base_url.is_some() {
            if let Ok(edge_resp) = self
                .call_target(req, edge, decision.tokens_in_estimate)
                .await
            {
                if cascade_gate_pass(&edge_resp) {
                    let cloud = self.target_cloud();
                    if let Ok(_cloud_resp) = self
                        .call_target(req, cloud, decision.tokens_in_estimate)
                        .await
                    {
                        if cloud_verifies_edge(&edge_resp, &_cloud_resp) {
                            self.stats.record_cascade_edge_ok();
                            return Ok(self.finish_non_stream(edge_resp, decision, "edge", false));
                        }
                    }
                }
            }
        }

        if edge_tried {
            self.stats.record_cascade_fallback();
        }
        let cloud = self.target_cloud();
        let resp = self
            .call_target(req, cloud, decision.tokens_in_estimate)
            .await?;
        Ok(self.finish_non_stream(resp, decision, "cloud", edge_tried))
    }

    async fn stream_multimodal(
        &self,
        req: &ChatCompletionRequest,
        decision: &RouteDecision,
    ) -> AppResult<(SseStream, bool)> {
        match decision.multimodal_strategy {
            MultimodalStrategy::CachedEdge | MultimodalStrategy::CachedEdgeFallback => {
                self.stream_target(req, self.target_edge(), decision)
                    .await
                    .map(|s| (s, false))
            }
            MultimodalStrategy::CachedCloud => {
                self.stream_target(req, self.target_cloud(), decision)
                    .await
                    .map(|s| (s, true))
            }
            MultimodalStrategy::Probe => self.stream_multimodal_probe(req, decision).await,
            MultimodalStrategy::None => unreachable!(),
        }
    }

    async fn stream_multimodal_probe(
        &self,
        req: &ChatCompletionRequest,
        decision: &RouteDecision,
    ) -> AppResult<(SseStream, bool)> {
        let model = &req.model;
        let edge = self.target_edge();
        let edge_tried = edge.base_url.is_some();

        if edge.base_url.is_some() {
            match self.stream_target(req, edge, decision).await {
                Ok(stream) => {
                    self.multimodal.record_edge(&self.cfg(), model, true);
                    return Ok((stream, false));
                }
                Err(_) => self.multimodal.record_edge(&self.cfg(), model, false),
            }
        }

        self.stats.record_cascade_fallback();
        let cloud = self.target_cloud();
        if cloud.base_url.is_some() {
            match self.stream_target(req, cloud, decision).await {
                Ok(stream) => {
                    self.multimodal.record_cloud(&self.cfg(), model, true);
                    return Ok((stream, edge_tried));
                }
                Err(_) => self.multimodal.record_cloud(&self.cfg(), model, false),
            }
        }

        self.stream_target(req, self.target_edge(), decision)
            .await
            .map(|s| (s, edge_tried))
    }

    async fn stream_cascade(
        &self,
        req: &ChatCompletionRequest,
        decision: &RouteDecision,
    ) -> AppResult<(SseStream, bool)> {
        let edge = self.target_edge();
        let edge_tried = edge.base_url.is_some();
        if edge_tried {
            match self.stream_target(req, edge, decision).await {
                Ok(stream) => return Ok((stream, false)),
                Err(_) if self.cfg().cloud_base_url.is_some() => {
                    self.stats.record_cascade_fallback();
                }
                Err(e) => return Err(e),
            }
        }

        self.stream_target(req, self.target_cloud(), decision)
            .await
            .map(|s| (s, edge_tried))
    }

    fn target_edge(&self) -> UpstreamTarget {
        let c = self.cfg();
        UpstreamTarget {
            base_url: c.edge_base_url.clone(),
            api_key: c.edge_api_key.clone(),
            model: c.edge_model.clone(),
            tier: "edge",
        }
    }

    fn target_cloud(&self) -> UpstreamTarget {
        let c = self.cfg();
        UpstreamTarget {
            base_url: c.cloud_base_url.clone(),
            api_key: c.cloud_api_key.clone(),
            model: c.cloud_model.clone(),
            tier: "cloud",
        }
    }

    async fn call_target(
        &self,
        req: &ChatCompletionRequest,
        target: UpstreamTarget,
        prompt_fallback: u32,
    ) -> AppResult<ChatCompletionResponse> {
        let Some(url) = target.base_url.as_deref() else {
            return Err(missing_upstream(target.tier));
        };
        self.call_url(
            req,
            url,
            target.api_key.as_deref(),
            target.model.as_deref(),
            target.tier,
            prompt_fallback,
        )
        .await
    }

    async fn call_url(
        &self,
        req: &ChatCompletionRequest,
        base: &str,
        api_key: Option<&str>,
        endpoint_model: Option<&str>,
        tier: &str,
        prompt_fallback: u32,
    ) -> AppResult<ChatCompletionResponse> {
        let _edge_guard = self.edge_guard_for_tier(tier);
        self.record_upstream_call(tier);
        let start = Instant::now();
        let url = format!("{}/chat/completions", base.trim_end_matches('/'));
        let upstream_req = apply_upstream_model(req, endpoint_model);
        let mut builder = self.http.post(url).json(&upstream_req);
        if let Some(key) = api_key {
            builder = builder.bearer_auth(key);
        }

        let resp = builder
            .send()
            .await
            .map_err(|e| AppError::Upstream(e.to_string()))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(AppError::Upstream(format!("{status}: {body}")));
        }

        let body = resp
            .json::<ChatCompletionResponse>()
            .await
            .map_err(|e| AppError::Upstream(e.to_string()))?;
        let latency_ms = start.elapsed().as_millis() as u64;
        let (prompt, completion, cached) = tokens_from_response(&body, prompt_fallback);
        let tier_static = tier_static(tier);
        self.stats.record_upstream_metrics(&UpstreamCallMetrics {
            tier: tier_static,
            prompt_tokens: prompt,
            completion_tokens: completion,
            cached_tokens: cached,
            latency_ms,
            ttft_ms: None,
            stream: false,
        });
        Ok(body)
    }

    async fn stream_target(
        &self,
        req: &ChatCompletionRequest,
        target: UpstreamTarget,
        decision: &RouteDecision,
    ) -> AppResult<SseStream> {
        let url = target
            .base_url
            .as_deref()
            .ok_or_else(|| missing_upstream(target.tier))?;
        self.stream_url(
            req,
            url,
            target.api_key.as_deref(),
            target.model.as_deref(),
            target.tier,
            decision,
        )
        .await
    }

    async fn stream_url(
        &self,
        req: &ChatCompletionRequest,
        base: &str,
        api_key: Option<&str>,
        endpoint_model: Option<&str>,
        tier: &str,
        decision: &RouteDecision,
    ) -> AppResult<SseStream> {
        let edge_guard = self.edge_guard_for_tier(tier);
        self.record_upstream_call(tier);
        let url = format!("{}/chat/completions", base.trim_end_matches('/'));
        let upstream_req = apply_upstream_model(req, endpoint_model);
        let mut builder = self.http.post(url).json(&upstream_req);
        if let Some(key) = api_key {
            builder = builder.bearer_auth(key);
        }

        let resp = builder
            .send()
            .await
            .map_err(|e| AppError::Upstream(e.to_string()))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(AppError::Upstream(format!("{status}: {body}")));
        }

        let raw = Box::pin(
            resp.bytes_stream().map(|r| r.map_err(std::io::Error::other)),
        );
        let tier_static = tier_static(tier);
        Ok(instrument_stream(
            raw,
            StreamRecordContext {
                stats: self.stats.clone(),
                tier: tier_static,
                prompt_fallback: decision.tokens_in_estimate,
                cloud_input_saved: decision.cloud_input_saved_estimate,
                record_cloud_saved: tier_static == "edge",
                edge_guard,
            },
        ))
    }

    fn edge_guard_for_tier(&self, tier: &str) -> Option<EdgeInferenceGuard> {
        if tier == "edge" {
            Some(self.edge_load.begin())
        } else {
            None
        }
    }

    fn finish_non_stream(
        &self,
        resp: ChatCompletionResponse,
        decision: &RouteDecision,
        served_tier: &'static str,
        fallback: bool,
    ) -> ChatCompletionResponse {
        let (_, completion, _) = tokens_from_response(&resp, decision.tokens_in_estimate);
        self.stats.record_completion_tokens(completion);
        self.stats.record_final_response(&FinalResponseMetrics {
            served_tier,
            cloud_input_saved: if served_tier == "edge" {
                decision.cloud_input_saved_estimate
            } else {
                0
            },
            completion_tokens: completion,
        });
        attach_meta(resp, decision, fallback)
    }

    fn record_upstream_call(&self, tier: &str) {
        match tier {
            "edge" => self.stats.record_upstream_edge(),
            "cloud" => self.stats.record_upstream_cloud(),
            _ => {}
        }
    }
}

fn apply_upstream_model(req: &ChatCompletionRequest, endpoint_model: Option<&str>) -> ChatCompletionRequest {
    let mut upstream_req = req.for_upstream();
    if let Some(model) = endpoint_model {
        let m = model.trim();
        if !m.is_empty() && !m.eq_ignore_ascii_case("auto") {
            upstream_req.model = m.to_string();
        }
    }
    upstream_req
}

fn missing_upstream(tier: &str) -> AppError {
    AppError::Unavailable(format!(
        "upstream.{tier} not configured — set [upstream.{tier}] in config.toml"
    ))
}

fn tier_static(tier: &str) -> &'static str {
    if tier == "cloud" { "cloud" } else { "edge" }
}

fn attach_meta(
    mut resp: ChatCompletionResponse,
    decision: &RouteDecision,
    fallback: bool,
) -> ChatCompletionResponse {
    let (tokens_in, tokens_out, _) =
        tokens_from_response(&resp, decision.tokens_in_estimate);
    let input_ratio = if tokens_in + tokens_out > 0 {
        tokens_in as f32 / (tokens_in + tokens_out) as f32
    } else {
        0.0
    };

    resp.flowy_meta = Some(FlowyMeta {
        route: format!("{:?}", decision.route).to_ascii_lowercase(),
        fallback,
        difficulty_score: decision.difficulty,
        step_kind: format!("{:?}", decision.step_kind).to_ascii_lowercase(),
        reason_codes: decision.reason_codes.clone(),
        tokens_in,
        tokens_out,
        input_ratio,
        cloud_input_saved: decision.cloud_input_saved_estimate,
        profile: format!("{:?}", decision.profile).to_ascii_lowercase(),
    });
    resp
}

fn cascade_gate_pass(resp: &ChatCompletionResponse) -> bool {
    let Some(choice) = resp.choices.first() else {
        return false;
    };
    if cascade_text_pass(choice) {
        return true;
    }
    cascade_tool_calls_pass(choice)
}

fn cascade_text_pass(choice: &crate::gateway::api::openai::Choice) -> bool {
    let Some(text) = choice.message.content.as_ref() else {
        return false;
    };
    !text.is_empty() && !text.contains("不确定") && text.len() > 8
}

fn cascade_tool_calls_pass(choice: &crate::gateway::api::openai::Choice) -> bool {
    choice.message.tool_calls.as_ref().is_some_and(|calls| {
        !calls.is_empty()
            && calls.iter().all(|c| {
                !c.function.name.trim().is_empty() && !c.function.arguments.trim().is_empty()
            })
    })
}

#[cfg(test)]
mod cascade_gate_tests {
    use super::*;
    use crate::gateway::api::openai::{
        ChatCompletionResponse, Choice, FunctionCallPayload, Message, Role, ToolCall,
    };

    fn response_with_choice(message: Message) -> ChatCompletionResponse {
        ChatCompletionResponse {
            id: "test".into(),
            object: "chat.completion".into(),
            created: 0,
            model: "test".into(),
            choices: vec![Choice {
                index: 0,
                message,
                finish_reason: "stop".into(),
            }],
            usage: None,
            flowy_meta: None,
        }
    }

    #[test]
    fn cascade_gate_passes_valid_tool_calls_without_text() {
        let resp = response_with_choice(Message {
            role: Role::Assistant,
            content: None,
            content_parts: None,
            tool_calls: Some(vec![ToolCall {
                id: "call_1".into(),
                call_type: "function".into(),
                function: FunctionCallPayload {
                    name: "exec".into(),
                    arguments: r#"{"cmd":"ls"}"#.into(),
                },
            }]),
            tool_call_id: None,
        });
        assert!(cascade_gate_pass(&resp));
    }

    #[test]
    fn cascade_gate_rejects_empty_response() {
        let resp = response_with_choice(Message {
            role: Role::Assistant,
            content: None,
            content_parts: None,
            tool_calls: None,
            tool_call_id: None,
        });
        assert!(!cascade_gate_pass(&resp));
    }
}
