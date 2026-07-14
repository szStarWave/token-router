use futures::StreamExt;
use reqwest::Client;
use std::time::Instant;

use crate::config::DEFAULT_CLOUD_BUDGET_AGENT_ID;
use crate::gateway::agent_usage::AgentCloudUsageStore;
use crate::gateway::api::codex_catalog::is_router_auto_model;
use crate::gateway::api::openai::{ChatCompletionRequest, ChatCompletionResponse, TokenRouterMeta};
use crate::gateway::config::AppConfig;
use crate::gateway::config_manager::ConfigManager;
use crate::gateway::error::{AppError, AppResult};
use crate::gateway::multimodal::{MultimodalStore, MultimodalStrategy};
use std::sync::Arc;

use crate::gateway::edge_load::{EdgeInferenceGuard, EdgeInferenceTracker};
use crate::gateway::routing::{RouteDecision, RouteTier, WorkStrategy};
use crate::gateway::stats::metrics::{
    effective_upstream_model, normalize_upstream_model, tokens_from_response, FinalResponseMetrics,
    UpstreamCallMetrics,
};
use crate::gateway::stats::{AuthKeyContext, GatewayStats};
use crate::gateway::upstream::sse::{instrument_stream, StreamRecordContext, SseStream};
use crate::gateway::upstream::verify::cloud_verifies_edge;
use crate::gateway::api::compat::{format_upstream_client_error, log_upstream_error_exchange};
use crate::gateway::api::meta::log_upstream_served;
use crate::gateway::routing_log::RoutingLogStore;
use crate::gateway::session::SessionStore;
use crate::gateway::served_outcome::{CloudCacheSettings, StreamPostServe};

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
    agent_usage: Arc<AgentCloudUsageStore>,
    routing_logs: Arc<RoutingLogStore>,
    sessions: Arc<SessionStore>,
}

impl UpstreamClient {
    pub fn new(
        config_mgr: Arc<ConfigManager>,
        stats: Arc<GatewayStats>,
        multimodal: Arc<MultimodalStore>,
        edge_load: Arc<EdgeInferenceTracker>,
        agent_usage: Arc<AgentCloudUsageStore>,
        routing_logs: Arc<RoutingLogStore>,
        sessions: Arc<SessionStore>,
    ) -> Self {
        Self {
            http: Client::new(),
            config_mgr,
            stats,
            multimodal,
            edge_load,
            agent_usage,
            routing_logs,
            sessions,
        }
    }

    fn cfg(&self) -> AppConfig {
        self.config_mgr.get()
    }

    pub fn edge_configured(&self) -> bool {
        self.cfg().edge_base_url.is_some()
    }

    fn cloud_token_budget_limit(&self, agent_id: Option<&str>) -> Option<u64> {
        let c = self.cfg();
        let id = agent_id.unwrap_or(DEFAULT_CLOUD_BUDGET_AGENT_ID);
        c.agents.get(id).and_then(|a| a.cloud_token_budget)
    }

    fn cloud_token_budget_ok(&self, agent_id: Option<&str>, tokens_in: u32) -> bool {
        let id = agent_id.unwrap_or(DEFAULT_CLOUD_BUDGET_AGENT_ID);
        let limit = self.cloud_token_budget_limit(agent_id);
        self.agent_usage.check_budget(id, limit, tokens_in)
    }

    fn record_cloud_tokens_complete(&self, agent_id: Option<&str>, resp: &ChatCompletionResponse, prompt_fallback: u32) {
        let id = agent_id.unwrap_or(DEFAULT_CLOUD_BUDGET_AGENT_ID);
        let c = self.cfg();
        let has_budget = c
            .agents
            .get(id)
            .and_then(|a| a.cloud_token_budget)
            .unwrap_or(0)
            > 0;
        if !has_budget {
            return;
        }
        let (prompt, completion, _) = tokens_from_response(resp, prompt_fallback);
        self.agent_usage.record_tokens(id, (prompt + completion) as u64);
    }

    pub async fn complete(
        &self,
        req: &ChatCompletionRequest,
        decision: &RouteDecision,
        agent_id: Option<&str>,
        auth_key: Option<&AuthKeyContext>,
    ) -> AppResult<ChatCompletionResponse> {
        if decision.multimodal_strategy != MultimodalStrategy::None {
            return self.complete_multimodal(req, decision, agent_id, auth_key).await;
        }

        if decision.work_strategy == WorkStrategy::Verify {
            return self.complete_work_verify(req, decision, agent_id, auth_key).await;
        }

        match decision.route {
            RouteTier::Edge if decision.casual_quality_fallback => {
                self.complete_edge_with_quality_fallback(req, decision, agent_id, auth_key)
                    .await
            }
            RouteTier::Edge if !self.allow_cross_tier_fallback() => {
                self.complete_edge_only(req, decision, agent_id, auth_key).await
            }
            RouteTier::Edge => {
                self.complete_edge_with_context_fallback(req, decision, agent_id, auth_key)
                    .await
            }
            RouteTier::Cloud if !self.allow_cross_tier_fallback() => {
                self.complete_cloud_only(req, decision, agent_id, auth_key).await
            }
            RouteTier::Cloud => {
                if self.cloud_token_budget_ok(agent_id, decision.tokens_in_estimate) {
                    let t = self.target_cloud(agent_id);
                    let resp = self.call_target(req, t, decision.tokens_in_estimate, auth_key).await?;
                    Ok(self.finish_non_stream(req, resp, decision, "cloud", false, agent_id, auth_key))
                } else {
                    let edge = self.target_edge(agent_id);
                    let edge_tried = edge.base_url.is_some();
                    if edge_tried {
                        if let Ok(resp) = self
                            .call_target(req, edge, decision.tokens_in_estimate, auth_key)
                            .await
                        {
                            if cascade_gate_pass(&resp) {
                                self.stats.record_cascade_edge_ok();
                                return Ok(self.finish_non_stream(req, resp, decision, "edge", false, agent_id, auth_key));
                            }
                        }
                        self.stats.record_cascade_fallback();
                    }
                    let cloud = self.target_cloud(agent_id);
                    let resp = self
                        .call_target(req, cloud, decision.tokens_in_estimate, auth_key)
                        .await?;
                    Ok(self.finish_non_stream(req, resp, decision, "cloud", edge_tried, agent_id, auth_key))
                }
            }
            RouteTier::Cascade => {
                self.complete_edge_with_quality_fallback(req, decision, agent_id, auth_key)
                    .await
            }
        }
    }

    async fn complete_edge_with_quality_fallback(
        &self,
        req: &ChatCompletionRequest,
        decision: &RouteDecision,
        agent_id: Option<&str>,
        auth_key: Option<&AuthKeyContext>,
    ) -> AppResult<ChatCompletionResponse> {
        let edge = self.target_edge(agent_id);
        let edge_tried = edge.base_url.is_some();
        if edge_tried {
            if let Ok(resp) = self
                .call_target(req, edge, decision.tokens_in_estimate, auth_key)
                .await
            {
                if cascade_gate_pass(&resp) {
                    self.stats.record_cascade_edge_ok();
                    return Ok(self.finish_non_stream(req, resp, decision, "edge", false, agent_id, auth_key));
                }
            }
            self.stats.record_cascade_fallback();
        }
        let cloud = self.target_cloud(agent_id);
        let resp = self
            .call_target(req, cloud, decision.tokens_in_estimate, auth_key)
            .await?;
        Ok(self.finish_non_stream(req, resp, decision, "cloud", edge_tried, agent_id, auth_key))
    }

    pub async fn stream(
        &self,
        req: &ChatCompletionRequest,
        decision: &RouteDecision,
        agent_id: Option<&str>,
        auth_key: Option<&AuthKeyContext>,
    ) -> AppResult<(SseStream, bool)> {
        if decision.multimodal_strategy != MultimodalStrategy::None {
            return self.stream_multimodal(req, decision, agent_id, auth_key).await;
        }

        if decision.work_strategy == WorkStrategy::Verify {
            return self.stream_cascade(req, decision, agent_id, auth_key).await;
        }

        match decision.route {
            RouteTier::Edge if decision.casual_quality_fallback => {
                self.stream_cascade(req, decision, agent_id, auth_key).await
            }
            RouteTier::Edge if !self.allow_cross_tier_fallback() => {
                self.stream_edge_only(req, decision, agent_id, auth_key).await
            }
            RouteTier::Edge => self
                .stream_edge_with_context_fallback(req, decision, agent_id, auth_key)
                .await,
            RouteTier::Cloud if !self.allow_cross_tier_fallback() => {
                self.stream_cloud_only(req, decision, agent_id, auth_key).await
            }
            RouteTier::Cloud => {
                if self.cloud_token_budget_ok(agent_id, decision.tokens_in_estimate) {
                    self.stream_target(req, self.target_cloud(agent_id), decision, agent_id, auth_key)
                        .await
                        .map(|s| (s, false))
                } else {
                    self.stream_cascade(req, decision, agent_id, auth_key).await
                }
            }
            RouteTier::Cascade => self.stream_cascade(req, decision, agent_id, auth_key).await,
        }
    }

    async fn complete_multimodal(
        &self,
        req: &ChatCompletionRequest,
        decision: &RouteDecision,
        agent_id: Option<&str>,
        auth_key: Option<&AuthKeyContext>,
    ) -> AppResult<ChatCompletionResponse> {
        match decision.multimodal_strategy {
            MultimodalStrategy::CachedEdge | MultimodalStrategy::CachedEdgeFallback => {
                let resp = self
                    .call_target(req, self.target_edge(agent_id), decision.tokens_in_estimate, auth_key)
                    .await?;
                Ok(self.finish_non_stream(req, resp, decision, "edge", false, agent_id, auth_key))
            }
            MultimodalStrategy::CachedCloud => {
                let resp = self
                    .call_target(req, self.target_cloud(agent_id), decision.tokens_in_estimate, auth_key)
                    .await?;
                Ok(self.finish_non_stream(req, resp, decision, "cloud", true, agent_id, auth_key))
            }
            MultimodalStrategy::Probe => {
                self.complete_multimodal_probe(req, decision, agent_id, auth_key).await
            }
            MultimodalStrategy::None => unreachable!(),
        }
    }

    async fn complete_multimodal_probe(
        &self,
        req: &ChatCompletionRequest,
        decision: &RouteDecision,
        agent_id: Option<&str>,
        auth_key: Option<&AuthKeyContext>,
    ) -> AppResult<ChatCompletionResponse> {
        let model = &req.model;
        let edge = self.target_edge(agent_id);

        if edge.base_url.is_some() {
            match self
                .call_target(req, edge, decision.tokens_in_estimate, auth_key)
                .await
            {
                Ok(resp) if cascade_gate_pass(&resp) => {
                    self.multimodal.record_edge(&self.cfg(), model, true);
                    self.stats.record_cascade_edge_ok();
                    return Ok(self.finish_non_stream(req, resp, decision, "edge", false, agent_id, auth_key));
                }
                Ok(_) => self.multimodal.record_edge(&self.cfg(), model, false),
                Err(_) => self.multimodal.record_edge(&self.cfg(), model, false),
            }
        }

        self.stats.record_cascade_fallback();
        let cloud = self.target_cloud(agent_id);
        match self
            .call_target(req, cloud, decision.tokens_in_estimate, auth_key)
            .await
        {
            Ok(resp) => {
                self.multimodal.record_cloud(&self.cfg(), model, true);
                return Ok(self.finish_non_stream(req, resp, decision, "cloud", true, agent_id, auth_key));
            }
            Err(_) => self.multimodal.record_cloud(&self.cfg(), model, false),
        }

        let resp = self
            .call_target(req, self.target_edge(agent_id), decision.tokens_in_estimate, auth_key)
            .await?;
        Ok(self.finish_non_stream(req, resp, decision, "edge", true, agent_id, auth_key))
    }

    async fn complete_work_verify(
        &self,
        req: &ChatCompletionRequest,
        decision: &RouteDecision,
        agent_id: Option<&str>,
        auth_key: Option<&AuthKeyContext>,
    ) -> AppResult<ChatCompletionResponse> {
        let edge = self.target_edge(agent_id);
        let edge_tried = edge.base_url.is_some();

        if edge.base_url.is_some() {
            if let Ok(edge_resp) = self
                .call_target(req, edge, decision.tokens_in_estimate, auth_key)
                .await
            {
                if cascade_gate_pass(&edge_resp) {
                    let cloud = self.target_cloud(agent_id);
                    if let Ok(_cloud_resp) = self
                        .call_target(req, cloud, decision.tokens_in_estimate, auth_key)
                        .await
                    {
                        if cloud_verifies_edge(&edge_resp, &_cloud_resp) {
                            self.stats.record_cascade_edge_ok();
                            return Ok(self.finish_non_stream(req, edge_resp, decision, "edge", false, agent_id, auth_key));
                        }
                    }
                }
            }
        }

        if edge_tried {
            self.stats.record_cascade_fallback();
        }
        let cloud = self.target_cloud(agent_id);
        let resp = self
            .call_target(req, cloud, decision.tokens_in_estimate, auth_key)
            .await?;
        Ok(self.finish_non_stream(req, resp, decision, "cloud", edge_tried, agent_id, auth_key))
    }

    async fn stream_multimodal(
        &self,
        req: &ChatCompletionRequest,
        decision: &RouteDecision,
        agent_id: Option<&str>,
        auth_key: Option<&AuthKeyContext>,
    ) -> AppResult<(SseStream, bool)> {
        match decision.multimodal_strategy {
            MultimodalStrategy::CachedEdge | MultimodalStrategy::CachedEdgeFallback => {
                self.stream_target(req, self.target_edge(agent_id), decision, agent_id, auth_key)
                    .await
                    .map(|s| (s, false))
            }
            MultimodalStrategy::CachedCloud => {
                self.stream_target(req, self.target_cloud(agent_id), decision, agent_id, auth_key)
                    .await
                    .map(|s| (s, true))
            }
            MultimodalStrategy::Probe => self.stream_multimodal_probe(req, decision, agent_id, auth_key).await,
            MultimodalStrategy::None => unreachable!(),
        }
    }

    async fn stream_multimodal_probe(
        &self,
        req: &ChatCompletionRequest,
        decision: &RouteDecision,
        agent_id: Option<&str>,
        auth_key: Option<&AuthKeyContext>,
    ) -> AppResult<(SseStream, bool)> {
        let model = &req.model;
        let edge = self.target_edge(agent_id);
        let edge_tried = edge.base_url.is_some();

        if edge.base_url.is_some() {
            match self.stream_target(req, edge, decision, agent_id, auth_key).await {
                Ok(stream) => {
                    self.multimodal.record_edge(&self.cfg(), model, true);
                    return Ok((stream, false));
                }
                Err(_) => self.multimodal.record_edge(&self.cfg(), model, false),
            }
        }

        self.stats.record_cascade_fallback();
        let cloud = self.target_cloud(agent_id);
        if cloud.base_url.is_some() {
            match self.stream_target(req, cloud, decision, agent_id, auth_key).await {
                Ok(stream) => {
                    self.multimodal.record_cloud(&self.cfg(), model, true);
                    return Ok((stream, edge_tried));
                }
                Err(_) => self.multimodal.record_cloud(&self.cfg(), model, false),
            }
        }

        self.stream_target(req, self.target_edge(agent_id), decision, agent_id, auth_key)
            .await
            .map(|s| (s, edge_tried))
    }

    async fn stream_cascade(
        &self,
        req: &ChatCompletionRequest,
        decision: &RouteDecision,
        agent_id: Option<&str>,
        auth_key: Option<&AuthKeyContext>,
    ) -> AppResult<(SseStream, bool)> {
        let edge = self.target_edge(agent_id);
        let edge_tried = edge.base_url.is_some();
        if edge_tried {
            match self.stream_target(req, edge, decision, agent_id, auth_key).await {
                Ok(stream) => return Ok((stream, false)),
                Err(_) if self.target_cloud(agent_id).base_url.is_some() => {
                    self.stats.record_cascade_fallback();
                }
                Err(e) => return Err(e),
            }
        }

        self.stream_target(req, self.target_cloud(agent_id), decision, agent_id, auth_key)
            .await
            .map(|s| (s, edge_tried))
    }

    fn target_edge(&self, agent_id: Option<&str>) -> UpstreamTarget {
        let c = self.cfg();
        let resolved = c.resolve_upstream(agent_id, "edge");
        UpstreamTarget {
            base_url: resolved.base_url,
            api_key: resolved.api_key,
            model: resolved.model,
            tier: "edge",
        }
    }

    fn target_cloud(&self, agent_id: Option<&str>) -> UpstreamTarget {
        let c = self.cfg();
        let resolved = c.resolve_upstream(agent_id, "cloud");
        UpstreamTarget {
            base_url: resolved.base_url,
            api_key: resolved.api_key,
            model: resolved.model,
            tier: "cloud",
        }
    }

    async fn call_target(
        &self,
        req: &ChatCompletionRequest,
        target: UpstreamTarget,
        prompt_fallback: u32,
        auth_key: Option<&AuthKeyContext>,
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
            auth_key,
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
        auth_key: Option<&AuthKeyContext>,
    ) -> AppResult<ChatCompletionResponse> {
        let _edge_guard = self.edge_guard_for_tier(tier);
        self.record_upstream_call(tier);
        let start = Instant::now();
        let url = format!("{}/chat/completions", base.trim_end_matches('/'));
        let url_preview = url.clone();
        let upstream_req = apply_upstream_model(req, endpoint_model, tier, Some(base));
        let mut builder = self.http.post(url).json(&upstream_req);
        if let Some(key) = api_key {
            builder = builder.bearer_auth(key);
        }

        let resp = match builder.send().await {
            Ok(resp) => resp,
            Err(e) => {
                let msg = e.to_string();
                log_upstream_error_exchange(
                    tier,
                    &url_preview,
                    false,
                    None,
                    req,
                    &upstream_req,
                    &msg,
                    &msg,
                );
                return Err(AppError::Upstream(msg));
            }
        };

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            let client_msg = format_upstream_client_error(status.as_u16(), &body);
            log_upstream_error_exchange(
                tier,
                &url_preview,
                false,
                Some(status.as_u16()),
                req,
                &upstream_req,
                &body,
                &client_msg,
            );
            return Err(AppError::Upstream(client_msg));
        }

        let text = resp.text().await.map_err(|e| {
            let msg = e.to_string();
            log_upstream_error_exchange(
                tier,
                &url_preview,
                false,
                Some(200),
                req,
                &upstream_req,
                &msg,
                &msg,
            );
            AppError::Upstream(msg)
        })?;
        let body: ChatCompletionResponse = serde_json::from_str(&text).map_err(|e| {
            let msg = e.to_string();
            log_upstream_error_exchange(
                tier,
                &url_preview,
                false,
                Some(200),
                req,
                &upstream_req,
                &text,
                &msg,
            );
            AppError::Upstream(msg)
        })?;
        let latency_us = start.elapsed().as_micros() as u64;
        let latency_ms = latency_us / 1000;
        let (prompt, completion, cached) = tokens_from_response(&body, prompt_fallback);
        let tier_static = tier_static(tier);
        let model_name = normalize_upstream_model(upstream_req.model.as_str());
        self.stats.record_upstream_metrics(
            &UpstreamCallMetrics {
                tier: tier_static,
                model: model_name,
                prompt_tokens: prompt,
                completion_tokens: completion,
                cached_tokens: cached,
                latency_ms,
                ttft_ms: None,
                last_token_ms: None,
                latency_us,
                ttft_us: None,
                last_token_us: None,
                stream: false,
            },
            auth_key,
        );
        Ok(body)
    }

    async fn stream_target(
        &self,
        req: &ChatCompletionRequest,
        target: UpstreamTarget,
        decision: &RouteDecision,
        agent_id: Option<&str>,
        auth_key: Option<&AuthKeyContext>,
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
            agent_id,
            auth_key,
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
        agent_id: Option<&str>,
        auth_key: Option<&AuthKeyContext>,
    ) -> AppResult<SseStream> {
        let edge_guard = self.edge_guard_for_tier(tier);
        self.record_upstream_call(tier);
        let url = format!("{}/chat/completions", base.trim_end_matches('/'));
        let url_preview = url.clone();
        let upstream_req = apply_upstream_model(req, endpoint_model, tier, Some(base));
        let mut builder = self.http.post(url).json(&upstream_req);
        if let Some(key) = api_key {
            builder = builder.bearer_auth(key);
        }

        let resp = match builder.send().await {
            Ok(resp) => resp,
            Err(e) => {
                let msg = e.to_string();
                log_upstream_error_exchange(
                    tier,
                    &url_preview,
                    true,
                    None,
                    req,
                    &upstream_req,
                    &msg,
                    &msg,
                );
                return Err(AppError::Upstream(msg));
            }
        };

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            let client_msg = format_upstream_client_error(status.as_u16(), &body);
            log_upstream_error_exchange(
                tier,
                &url_preview,
                true,
                Some(status.as_u16()),
                req,
                &upstream_req,
                &body,
                &client_msg,
            );
            return Err(AppError::Upstream(client_msg));
        }

        let raw = Box::pin(
            resp.bytes_stream().map(|r| r.map_err(std::io::Error::other)),
        );
        let tier_static = tier_static(tier);
        let cfg = self.cfg();
        let budget_agent_id = agent_id.unwrap_or(DEFAULT_CLOUD_BUDGET_AGENT_ID).to_string();
        let stream_agent_id = Some(budget_agent_id.clone());
        let stream_agent_usage = {
            let has_budget = cfg
                .agents
                .get(budget_agent_id.as_str())
                .and_then(|a| a.cloud_token_budget)
                .unwrap_or(0)
                > 0;
            if tier_static == "cloud" && has_budget {
                Some(self.agent_usage.clone())
            } else {
                None
            }
        };
        let stream_agent_id = stream_agent_usage.as_ref().and(stream_agent_id);
        let fallback_flag = tier_static == "cloud"
            && matches!(decision.route, RouteTier::Edge | RouteTier::Cascade);
        let served_model = upstream_req.model.clone();
        let log_model = served_model.clone();
        let post_serve = Some(StreamPostServe {
            sessions: self.sessions.clone(),
            routing_logs: self.routing_logs.clone(),
            settings: CloudCacheSettings::from_config(&self.cfg()),
            req: req.clone(),
            decision: decision.clone(),
            assistant_failed: decision.assistant_failed_recent,
            fallback: fallback_flag,
            served_model: served_model.clone(),
        });
        let stream = instrument_stream(
            raw,
            StreamRecordContext {
                stats: self.stats.clone(),
                tier: tier_static,
                model: served_model,
                prompt_fallback: decision.tokens_in_estimate,
                cloud_input_saved: decision.cloud_input_saved_estimate,
                record_cloud_saved: tier_static == "edge",
                edge_guard,
                agent_usage: stream_agent_usage,
                agent_id: stream_agent_id,
                auth_key: auth_key.cloned(),
                post_serve,
            },
        );
        let fallback = fallback_flag;
        log_upstream_served(
            Some(self.routing_logs.as_ref()),
            decision.routing_log_id,
            decision,
            tier_static,
            fallback,
            true,
            Some(&log_model),
            agent_id,
        );
        Ok(stream)
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
        req: &ChatCompletionRequest,
        mut resp: ChatCompletionResponse,
        decision: &RouteDecision,
        served_tier: &'static str,
        fallback: bool,
        agent_id: Option<&str>,
        auth_key: Option<&AuthKeyContext>,
    ) -> ChatCompletionResponse {
        let (_, completion, _) = tokens_from_response(&resp, decision.tokens_in_estimate);
        self.stats.record_completion_tokens(completion, auth_key);
        self.stats.record_final_response(
            &FinalResponseMetrics {
                served_tier,
                cloud_input_saved: if served_tier == "edge" {
                    decision.cloud_input_saved_estimate
                } else {
                    0
                },
                completion_tokens: completion,
            },
            auth_key,
        );
        if served_tier == "cloud" {
            self.record_cloud_tokens_complete(agent_id, &resp, decision.tokens_in_estimate);
        }
        let forwarded = self.resolve_served_model(req, served_tier, agent_id);
        let served_model = effective_upstream_model(&forwarded, &resp.model);
        resp.upstream_forwarded_model = Some(forwarded);
        log_upstream_served(
            Some(self.routing_logs.as_ref()),
            decision.routing_log_id,
            decision,
            served_tier,
            fallback,
            false,
            Some(&served_model),
            agent_id,
        );
        attach_meta(resp, decision, fallback)
    }

    fn resolve_served_model(
        &self,
        req: &ChatCompletionRequest,
        served_tier: &str,
        agent_id: Option<&str>,
    ) -> String {
        let target = if served_tier == "cloud" {
            self.target_cloud(agent_id)
        } else {
            self.target_edge(agent_id)
        };
        apply_upstream_model(
            req,
            target.model.as_deref(),
            served_tier,
            target.base_url.as_deref(),
        )
        .model
    }

    fn cloud_configured(&self) -> bool {
        self.cfg().cloud_base_url.is_some()
    }

    /// Fixed `edge` / `cloud` must never cross tiers; errors propagate as-is.
    fn allow_cross_tier_fallback(&self) -> bool {
        !matches!(
            self.cfg().fixed_route,
            Some(RouteTier::Edge) | Some(RouteTier::Cloud)
        )
    }

    async fn complete_edge_only(
        &self,
        req: &ChatCompletionRequest,
        decision: &RouteDecision,
        agent_id: Option<&str>,
        auth_key: Option<&AuthKeyContext>,
    ) -> AppResult<ChatCompletionResponse> {
        let edge = self.target_edge(agent_id);
        if edge.base_url.is_none() {
            return Err(missing_upstream("edge"));
        }
        let resp = self
            .call_target(req, edge, decision.tokens_in_estimate, auth_key)
            .await?;
        Ok(self.finish_non_stream(req, resp, decision, "edge", false, agent_id, auth_key))
    }

    async fn stream_edge_only(
        &self,
        req: &ChatCompletionRequest,
        decision: &RouteDecision,
        agent_id: Option<&str>,
        auth_key: Option<&AuthKeyContext>,
    ) -> AppResult<(SseStream, bool)> {
        let edge = self.target_edge(agent_id);
        if edge.base_url.is_none() {
            return Err(missing_upstream("edge"));
        }
        self.stream_target(req, edge, decision, agent_id, auth_key)
            .await
            .map(|s| (s, false))
    }

    async fn complete_cloud_only(
        &self,
        req: &ChatCompletionRequest,
        decision: &RouteDecision,
        agent_id: Option<&str>,
        auth_key: Option<&AuthKeyContext>,
    ) -> AppResult<ChatCompletionResponse> {
        if !self.cloud_token_budget_ok(agent_id, decision.tokens_in_estimate) {
            return Err(AppError::Unavailable(
                "cloud token budget exceeded; fixed route=cloud does not fall back to edge".into(),
            ));
        }
        let cloud = self.target_cloud(agent_id);
        if cloud.base_url.is_none() {
            return Err(missing_upstream("cloud"));
        }
        let resp = self
            .call_target(req, cloud, decision.tokens_in_estimate, auth_key)
            .await?;
        Ok(self.finish_non_stream(req, resp, decision, "cloud", false, agent_id, auth_key))
    }

    async fn stream_cloud_only(
        &self,
        req: &ChatCompletionRequest,
        decision: &RouteDecision,
        agent_id: Option<&str>,
        auth_key: Option<&AuthKeyContext>,
    ) -> AppResult<(SseStream, bool)> {
        if !self.cloud_token_budget_ok(agent_id, decision.tokens_in_estimate) {
            return Err(AppError::Unavailable(
                "cloud token budget exceeded; fixed route=cloud does not fall back to edge".into(),
            ));
        }
        let cloud = self.target_cloud(agent_id);
        if cloud.base_url.is_none() {
            return Err(missing_upstream("cloud"));
        }
        self.stream_target(req, cloud, decision, agent_id, auth_key)
            .await
            .map(|s| (s, false))
    }

    async fn complete_edge_with_context_fallback(
        &self,
        req: &ChatCompletionRequest,
        decision: &RouteDecision,
        agent_id: Option<&str>,
        auth_key: Option<&AuthKeyContext>,
    ) -> AppResult<ChatCompletionResponse> {
        let edge = self.target_edge(agent_id);
        let edge_tried = edge.base_url.is_some();
        if edge_tried {
            match self
                .call_target(req, edge, decision.tokens_in_estimate, auth_key)
                .await
            {
                Ok(resp) => {
                    return Ok(self.finish_non_stream(req, resp, decision, "edge", false, agent_id, auth_key));
                }
                Err(err) if is_context_overflow_upstream_error(&err)
                    && self.allow_cross_tier_fallback()
                    && self.cloud_configured() => {
                    self.stats.record_cascade_fallback();
                }
                Err(err) => return Err(err),
            }
        } else {
            return Err(missing_upstream("edge"));
        }

        let cloud = self.target_cloud(agent_id);
        let resp = self
            .call_target(req, cloud, decision.tokens_in_estimate, auth_key)
            .await?;
        Ok(self.finish_non_stream(req, resp, decision, "cloud", true, agent_id, auth_key))
    }

    async fn stream_edge_with_context_fallback(
        &self,
        req: &ChatCompletionRequest,
        decision: &RouteDecision,
        agent_id: Option<&str>,
        auth_key: Option<&AuthKeyContext>,
    ) -> AppResult<(SseStream, bool)> {
        let edge = self.target_edge(agent_id);
        let edge_tried = edge.base_url.is_some();
        if edge_tried {
            match self
                .stream_target(req, edge, decision, agent_id, auth_key)
                .await
            {
                Ok(stream) => return Ok((stream, false)),
                Err(err) if is_context_overflow_upstream_error(&err)
                    && self.allow_cross_tier_fallback()
                    && self.cloud_configured() => {
                    self.stats.record_cascade_fallback();
                }
                Err(err) => return Err(err),
            }
        } else {
            return Err(missing_upstream("edge"));
        }

        self.stream_target(req, self.target_cloud(agent_id), decision, agent_id, auth_key)
            .await
            .map(|s| (s, true))
    }

    fn record_upstream_call(&self, tier: &str) {
        match tier {
            "edge" => self.stats.record_upstream_edge(),
            "cloud" => self.stats.record_upstream_cloud(),
            _ => {}
        }
    }
}

fn apply_upstream_model(
    req: &ChatCompletionRequest,
    endpoint_model: Option<&str>,
    tier: &str,
    upstream_base: Option<&str>,
) -> ChatCompletionRequest {
    let mut upstream_req = if tier == "edge" {
        req.for_edge_upstream(upstream_base)
    } else {
        req.for_upstream(upstream_base)
    };
    if let Some(model) = endpoint_model {
        let m = model.trim();
        if !m.is_empty() && !is_router_auto_model(m) {
            upstream_req.model = m.to_string();
        }
    }
    upstream_req
}

fn missing_upstream(tier: &str) -> AppError {
    AppError::Unavailable(format!(
        "upstream.{tier} not configured ??set [upstream.{tier}] in config.toml"
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

    resp.token_router_meta = Some(TokenRouterMeta {
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

fn is_context_overflow_upstream_error(err: &AppError) -> bool {
    let AppError::Upstream(msg) = err else {
        return false;
    };
    let lower = msg.to_ascii_lowercase();
    [
        "context length",
        "context window",
        "context size",
        "maximum context",
        "max context",
        "context overflow",
        "too many tokens",
        "token limit",
        "exceeds the context",
        "exceeds maximum",
        "exceeds the maximum",
        "input too long",
        "prompt is too long",
        "reduce the length",
        "n_ctx",
        "num_ctx",
        "requested token",
    ]
    .iter()
    .any(|needle| lower.contains(needle))
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
    !text.is_empty()
        && !crate::gateway::routing::response_has_uncertainty(text)
        && text.len() > 8
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
mod context_overflow_error_tests {
    use super::*;

    #[test]
    fn detects_common_context_overflow_messages() {
        assert!(is_context_overflow_upstream_error(&AppError::Upstream(
            "400 Bad Request: context length exceeded".into()
        )));
        assert!(is_context_overflow_upstream_error(&AppError::Upstream(
            "maximum context length".into()
        )));
        assert!(!is_context_overflow_upstream_error(&AppError::Upstream(
            "rate limit exceeded".into()
        )));
    }
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
            token_router_meta: None,
            upstream_forwarded_model: None,
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
            reasoning_content: None,
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
            reasoning_content: None,
        });
        assert!(!cascade_gate_pass(&resp));
    }
}
