//! 请求编排（§5）：加载 → 信号 → 决策 → 执行 → 事务提交。
//!
//! 纯决策在 `tr-core`；本模块只做 IO 与顺序编排。
//!
//! P0b 起支持两条执行路径，共享同一份"决策准备"：
//! - [`Pipeline::handle`]：非流式；
//! - [`Pipeline::handle_stream`]：流式，**第一个事件发出即语义提交**，之后不再换候选。
//!
//! 失败也必须写账本（§8.4）：`upstream_error` + 逐 attempt 的 `ErrorClass`。

use std::collections::BTreeMap;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use tokio::sync::mpsc;
use tr_contracts::{
    AttemptRecord, BreakerState, CacheAnchor, Candidate, CandidateId, CanonicalRequest,
    CanonicalResponse, CanonicalStreamEvent, Cost, DecisionSnapshot, Dispatch, DispatchSource,
    ErrorClass, FactsCompleteness, IngressProtocol, LedgerRecord, OutcomeKind, Plan, PlanMode,
    ReasonCode, ReasonCodeKind, RequestReason, Selection, SessionFacts, SessionKeySource, Slot,
    SlotRef, Usage, VirtualModel,
};
use tr_upstream::{AttemptFailure, HttpUpstream, StreamItem};
use uuid::Uuid;

use crate::config::Catalog;
use crate::reason;
use crate::session;
use crate::store::{self, Db, StoreError, sessions::SaveOutcome};

/// 请求上下文：HTTP 层能提供、但 canonical 里没有的信息。
#[derive(Debug, Clone, Default)]
pub struct RequestContext {
    pub server_session_id: Option<String>,
    pub request_reason: Option<RequestReason>,
}

/// 决策解释头（§11.1）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecisionHeaders {
    pub selected_model: String,
    pub served_model: String,
    pub thinking_level: String,
    pub request_reason: &'static str,
    pub dispatch_source: &'static str,
    pub plan_id: String,
    pub served_slot: String,
    pub mode: &'static str,
    pub reason_codes: String,
    pub judge: &'static str,
}

/// 一次成功请求的结果（非流式）。
#[derive(Debug, Clone)]
pub struct PipelineResponse {
    pub response: CanonicalResponse,
    pub headers: DecisionHeaders,
    pub ledger: LedgerRecord,
}

/// 流式输出块。
#[derive(Debug, Clone, PartialEq)]
pub enum StreamChunk {
    Event(CanonicalStreamEvent),
    Error { message: String },
    Done,
}

/// 流式响应：headers 在首事件到达后返回，`events` 继续产出。
pub struct StreamingResponse {
    pub headers: DecisionHeaders,
    pub events: mpsc::Receiver<StreamChunk>,
}

#[derive(Debug, thiserror::Error)]
pub enum PipelineError {
    #[error("model not found: {0}")]
    ModelNotFound(String),
    #[error("no eligible candidate")]
    NoEligibleCandidate {
        plan_id: String,
        reason_codes: Vec<ReasonCode>,
    },
    #[error("upstream failure: {class:?} (status {status:?})")]
    Upstream {
        class: ErrorClass,
        status: Option<u16>,
        message: Option<String>,
        request_id: Uuid,
    },
    #[error("store: {0}")]
    Store(#[from] StoreError),
}

/// 编排器。内部字段用 `Arc` 以便流式任务在请求结束后继续收尾。
pub struct Pipeline {
    db: std::sync::Arc<Db>,
    catalog: std::sync::Arc<Catalog>,
    upstream: HttpUpstream,
}

impl Pipeline {
    pub fn new(db: Db, catalog: Catalog, upstream: HttpUpstream) -> Self {
        Self {
            db: std::sync::Arc::new(db),
            catalog: std::sync::Arc::new(catalog),
            upstream,
        }
    }

    pub fn from_catalog(db: Db, catalog: Catalog) -> Self {
        Self::new(db, catalog, HttpUpstream::default())
    }

    pub fn catalog(&self) -> &Catalog {
        &self.catalog
    }

    pub fn db(&self) -> &Db {
        &self.db
    }

    /// 决策准备：解析 Selection、加载状态、抽信号、跑 `core::decide`。
    async fn prepare(
        &self,
        request: CanonicalRequest,
        context: RequestContext,
    ) -> Result<Prepared, PipelineError> {
        let now_unix = now_secs();

        let virtual_model_id = request
            .meta
            .virtual_model
            .clone()
            .unwrap_or_else(|| self.catalog.default_virtual_model.clone());
        let virtual_model = self
            .catalog
            .virtual_model(&virtual_model_id)
            .cloned()
            .ok_or_else(|| PipelineError::ModelNotFound(virtual_model_id.clone()))?;
        let profile = self
            .catalog
            .profile_for(&virtual_model)
            .cloned()
            .ok_or_else(|| {
                PipelineError::ModelNotFound(format!("profile:{}", virtual_model.profile))
            })?;
        let thinking_level = request
            .meta
            .thinking_level
            .clone()
            .filter(|level| !level.is_empty())
            .unwrap_or_else(|| "off".to_string());

        let branch_id = "main".to_string();
        let task_id = String::new();

        let session_input = session::build_input(&request, context.server_session_id.clone());
        let derived = session::derive(&session_input);

        let facts =
            store::sessions::load(&self.db, &derived.key, &branch_id)?.unwrap_or_else(|| {
                SessionFacts {
                    session_key: derived.key.clone(),
                    branch_id: branch_id.clone(),
                    ..SessionFacts::default()
                }
            });
        let strategy_states = store::strategy::load_strategy_states(
            &self.db,
            &derived.key,
            &branch_id,
            &task_id,
            &virtual_model.id,
        )?;
        let router_state = store::strategy::load_virtual_model_state(
            &self.db,
            &derived.key,
            &branch_id,
            &virtual_model.id,
        )?;
        let health = store::health::load(&self.db)?;

        let candidates = self.catalog.pool(&virtual_model);
        let window_hint = virtual_model.context_window.or_else(|| {
            candidates
                .iter()
                .filter(|candidate| candidate.enabled)
                .map(|candidate| candidate.window)
                .min()
        });
        let signals = tr_core::build_signals(&request, Some(&facts), window_hint);
        let request_reason = context
            .request_reason
            .unwrap_or_else(|| reason::infer(&request));

        // `previous_response_id` 缺失/过期只降级、不报错（§10.4）。
        let protocol_state_missing = match &request.meta.previous_response_id {
            Some(previous) => {
                store::protocol::load(&self.db, &format!("resp:{previous}"), now_unix)?.is_none()
            }
            None => false,
        };

        let snapshot = DecisionSnapshot {
            canonical: request.clone(),
            signals,
            candidates: candidates.clone(),
            health,
            session: facts.clone(),
            strategy_states,
            ledger: Default::default(),
            profile: profile.clone(),
            now_unix,
            verdicts: Vec::new(),
            selection: Selection {
                virtual_model: virtual_model.id.clone(),
                thinking_level: thinking_level.clone(),
            },
            virtual_model: virtual_model.clone(),
            reason: request_reason,
            previous: facts.last_successful_dispatch.clone(),
            failed: None,
            context_hold: facts.context_holds.get(&virtual_model.id).cloned(),
            router_state,
        };

        let plan = tr_core::decide(&snapshot);
        let plan_uuid = Uuid::new_v4().simple().to_string();
        let plan_id = format!("plan-{}", &plan_uuid[..12]);

        if plan.preference.is_empty() {
            let ledger = build_ledger(
                &LedgerInput {
                    plan_id: plan_id.clone(),
                    session_key: derived.key.clone(),
                    branch_id: branch_id.clone(),
                    task_id: task_id.clone(),
                    session_key_source: derived.source,
                    collision_suspect: derived.collision_suspect,
                    ingress_protocol: request.meta.ingress_protocol,
                    virtual_model_id: virtual_model.id.clone(),
                    thinking_level: thinking_level.clone(),
                    request_reason,
                    plan_mode: plan.mode,
                    preference: plan.preference_ids().into_iter().cloned().collect(),
                    failover: plan
                        .failover
                        .iter()
                        .map(|s| s.candidate_id.clone())
                        .collect(),
                    dispatch_source: plan.dispatch_source,
                    reason_codes: plan.reason_codes.clone(),
                    profile_revision: profile.revision.clone(),
                    config_digest: self.catalog.config_digest.clone(),
                    now_unix,
                },
                Vec::new(),
                None,
                OutcomeKind::UpstreamError,
                Some(503),
                0,
            );
            store::commit(&self.db, None, &[], &ledger)?;
            return Err(PipelineError::NoEligibleCandidate {
                plan_id,
                reason_codes: plan.reason_codes,
            });
        }

        Ok(Prepared {
            canonical: request,
            virtual_model,
            profile,
            thinking_level,
            session_key: derived.key,
            branch_id,
            task_id,
            session_key_source: derived.source,
            collision_suspect: derived.collision_suspect,
            prefix_hash: derived.prefix_hash,
            request_reason,
            plan,
            plan_id,
            facts,
            now_unix,
            protocol_state_missing,
        })
    }

    /// 非流式处理。
    pub async fn handle(
        &self,
        request: CanonicalRequest,
        context: RequestContext,
    ) -> Result<PipelineResponse, PipelineError> {
        let started = Instant::now();
        let prepared = self.prepare(request, context).await?;

        let mut attempts: Vec<AttemptRecord> = Vec::new();
        let mut breakers: BTreeMap<CandidateId, BreakerState> = BTreeMap::new();
        let mut last_failure: Option<AttemptFailure> = None;
        let mut success: Option<(usize, Candidate, Slot, CanonicalResponse)> = None;
        let health = store::health::load(&self.db)?;

        for (index, slot) in prepared.plan.preference.iter().enumerate() {
            let Some(candidate) = self.catalog.candidate(&slot.candidate_id).cloned() else {
                continue;
            };

            let outcome = self.upstream.execute(&candidate, &prepared.canonical).await;
            attempts.push(AttemptRecord {
                candidate_id: candidate.id.clone(),
                error_class: outcome.failure.as_ref().map(|failure| failure.class),
                http_status: outcome.http_status,
                committed: false,
                duration_ms: outcome.duration_ms,
                ttft_ms: outcome.ttft_ms,
                usage: outcome.response.as_ref().map(|response| response.usage),
            });

            if let Some(response) = outcome.response {
                breakers.insert(
                    candidate.id.clone(),
                    reset_breaker(health.state_of(&candidate.id), prepared.now_unix),
                );
                success = Some((index, candidate, slot.clone(), response));
                break;
            }

            let failure = outcome.failure.expect("attempt 必须有 response 或 failure");
            breakers.insert(
                candidate.id.clone(),
                next_breaker(health.state_of(&candidate.id), &failure, prepared.now_unix),
            );
            let retryable = failure.class.retryable_elsewhere();
            last_failure = Some(failure);
            if !retryable {
                break;
            }
        }

        let Some((index, candidate, slot, response)) = success else {
            let failure = last_failure.unwrap_or(no_attempt_failure());
            let ledger = build_ledger(
                &prepared.ledger_input(DispatchSource::Failover),
                attempts,
                None,
                OutcomeKind::UpstreamError,
                failure.status.or(Some(502)),
                started.elapsed().as_millis() as u64,
            );
            let breakers: Vec<_> = breakers.into_iter().collect();
            let request_id = ledger.request_id;
            store::commit(&self.db, None, &breakers, &ledger)?;
            return Err(PipelineError::Upstream {
                class: failure.class,
                status: failure.status,
                message: failure.message,
                request_id,
            });
        };

        let usage = response.usage;
        let ledger = build_ledger(
            &prepared.ledger_input(prepared.plan.dispatch_source),
            attempts,
            Some(Served {
                slot: slot.clone(),
                candidate: candidate.clone(),
                usage,
                ttft_ms: None,
            }),
            OutcomeKind::Success,
            Some(200),
            started.elapsed().as_millis() as u64,
        );

        let next_facts = prepared.updated_facts(&candidate, &slot, usage);
        let breakers: Vec<_> = breakers.into_iter().collect();
        let session_outcome = store::commit(&self.db, Some(&next_facts), &breakers, &ledger)?;
        if matches!(session_outcome, Some(SaveOutcome::Conflict)) {
            store::ledger::replace_reason_codes(
                &self.db,
                &ledger.request_id.to_string(),
                &with_cas_conflict(&ledger.reason_codes),
            )?;
        }

        let headers = prepared.headers(&candidate, &slot, index, &ledger.reason_codes);

        persist_protocol_state(&self.db, &prepared, &response);

        Ok(PipelineResponse {
            response,
            headers,
            ledger,
        })
    }

    /// 流式处理。
    ///
    /// 返回时已完成"首事件到达"：此时才向客户端提交 headers；之后不再切换候选。
    pub async fn handle_stream(
        &self,
        request: CanonicalRequest,
        context: RequestContext,
    ) -> Result<StreamingResponse, PipelineError> {
        let started = Instant::now();
        let prepared = self.prepare(request, context).await?;
        let health = store::health::load(&self.db)?;

        let mut attempts: Vec<AttemptRecord> = Vec::new();
        let mut breakers: BTreeMap<CandidateId, BreakerState> = BTreeMap::new();
        let mut last_failure: Option<AttemptFailure> = None;

        // 克隆偏好顺序：成功分支会把 `prepared` 移交给后台收尾任务。
        let preference = prepared.plan.preference.clone();
        for (index, slot) in preference.iter().enumerate() {
            let Some(candidate) = self.catalog.candidate(&slot.candidate_id).cloned() else {
                continue;
            };

            let attempt = match self
                .upstream
                .execute_stream(&candidate, &prepared.canonical)
                .await
            {
                Ok(attempt) => attempt,
                Err(failure) => {
                    attempts.push(AttemptRecord {
                        candidate_id: candidate.id.clone(),
                        error_class: Some(failure.class),
                        http_status: failure.status,
                        committed: false,
                        duration_ms: started.elapsed().as_millis() as u64,
                        ttft_ms: None,
                        usage: None,
                    });
                    breakers.insert(
                        candidate.id.clone(),
                        next_breaker(health.state_of(&candidate.id), &failure, prepared.now_unix),
                    );
                    let retryable = failure.class.retryable_elsewhere();
                    last_failure = Some(failure);
                    if retryable {
                        continue;
                    }
                    break;
                }
            };

            let mut events = attempt.events;
            match events.recv().await {
                Some(StreamItem::Event(first)) => {
                    let ttft_ms = started.elapsed().as_millis() as u64;
                    breakers.insert(
                        candidate.id.clone(),
                        reset_breaker(health.state_of(&candidate.id), prepared.now_unix),
                    );
                    attempts.push(AttemptRecord {
                        candidate_id: candidate.id.clone(),
                        error_class: None,
                        http_status: Some(200),
                        committed: true,
                        duration_ms: 0,
                        ttft_ms: Some(ttft_ms),
                        usage: None,
                    });

                    let headers = prepared.headers(
                        &candidate,
                        slot,
                        index,
                        prepared.plan.reason_codes.as_slice(),
                    );
                    let (tx, out_rx) = mpsc::channel(64);

                    let db = self.db.clone();
                    let task_prepared = prepared;
                    let task_candidate = candidate;
                    let task_slot = slot.clone();
                    let task_attempts = attempts;
                    let task_breakers = breakers;

                    tokio::spawn(async move {
                        forward_stream(
                            db,
                            task_prepared,
                            task_candidate,
                            task_slot,
                            task_attempts,
                            task_breakers,
                            events,
                            first,
                            tx,
                            ttft_ms,
                            started,
                        )
                        .await;
                    });

                    return Ok(StreamingResponse {
                        headers,
                        events: out_rx,
                    });
                }
                Some(StreamItem::Failed(failure)) => {
                    attempts.push(AttemptRecord {
                        candidate_id: candidate.id.clone(),
                        error_class: Some(failure.class),
                        http_status: failure.status,
                        committed: false,
                        duration_ms: started.elapsed().as_millis() as u64,
                        ttft_ms: None,
                        usage: None,
                    });
                    breakers.insert(
                        candidate.id.clone(),
                        next_breaker(health.state_of(&candidate.id), &failure, prepared.now_unix),
                    );
                    let retryable = failure.class.retryable_elsewhere();
                    last_failure = Some(failure);
                    if retryable {
                        continue;
                    }
                    break;
                }
                Some(StreamItem::Ended) | None => {
                    let failure = AttemptFailure {
                        class: ErrorClass::Protocol,
                        status: Some(200),
                        message: Some("upstream ended before first event".to_string()),
                        retry_after_ms: None,
                        cause: tr_upstream::http::AttemptCause::MalformedResponse,
                    };
                    attempts.push(AttemptRecord {
                        candidate_id: candidate.id.clone(),
                        error_class: Some(failure.class),
                        http_status: failure.status,
                        committed: false,
                        duration_ms: started.elapsed().as_millis() as u64,
                        ttft_ms: None,
                        usage: None,
                    });
                    breakers.insert(
                        candidate.id.clone(),
                        next_breaker(health.state_of(&candidate.id), &failure, prepared.now_unix),
                    );
                    last_failure = Some(failure);
                }
            }
        }

        let failure = last_failure.unwrap_or(no_attempt_failure());
        let ledger = build_ledger(
            &prepared.ledger_input(DispatchSource::Failover),
            attempts,
            None,
            OutcomeKind::UpstreamError,
            failure.status.or(Some(502)),
            started.elapsed().as_millis() as u64,
        );
        let breakers: Vec<_> = breakers.into_iter().collect();
        let request_id = ledger.request_id;
        store::commit(&self.db, None, &breakers, &ledger)?;
        Err(PipelineError::Upstream {
            class: failure.class,
            status: failure.status,
            message: failure.message,
            request_id,
        })
    }
}

/// 决策准备结果。
struct Prepared {
    canonical: CanonicalRequest,
    virtual_model: VirtualModel,
    profile: tr_contracts::ProfileConfig,
    thinking_level: String,
    session_key: String,
    branch_id: String,
    task_id: String,
    session_key_source: SessionKeySource,
    collision_suspect: bool,
    prefix_hash: String,
    request_reason: RequestReason,
    plan: Plan,
    plan_id: String,
    facts: SessionFacts,
    now_unix: u64,
    protocol_state_missing: bool,
}

impl Prepared {
    fn ledger_input(&self, dispatch_source: DispatchSource) -> LedgerInput {
        LedgerInput {
            plan_id: self.plan_id.clone(),
            session_key: self.session_key.clone(),
            branch_id: self.branch_id.clone(),
            task_id: self.task_id.clone(),
            session_key_source: self.session_key_source,
            collision_suspect: self.collision_suspect,
            ingress_protocol: self.canonical.meta.ingress_protocol,
            virtual_model_id: self.virtual_model.id.clone(),
            thinking_level: self.thinking_level.clone(),
            request_reason: self.request_reason,
            plan_mode: self.plan.mode,
            preference: self.plan.preference_ids().into_iter().cloned().collect(),
            failover: self
                .plan
                .failover
                .iter()
                .map(|slot| slot.candidate_id.clone())
                .collect(),
            dispatch_source,
            reason_codes: {
                let mut codes = self.plan.reason_codes.clone();
                if self.protocol_state_missing {
                    codes.push(ReasonCode::new(ReasonCodeKind::ProtocolStateMissing));
                }
                codes
            },
            profile_revision: self.profile.revision.clone(),
            config_digest: self.profile.config_digest.clone(),
            now_unix: self.now_unix,
        }
    }

    fn updated_facts(&self, candidate: &Candidate, slot: &Slot, usage: Usage) -> SessionFacts {
        let mut facts = self.facts.clone();
        facts.selected_virtual_model = Some(self.virtual_model.id.clone());
        facts.selected_thinking_level = Some(self.thinking_level.clone());
        facts.last_successful_dispatch = Some(Dispatch {
            physical_model: candidate.upstream.model.clone(),
            provider: format!("{:?}", candidate.upstream.provider),
            thinking_level: slot.thinking_level.clone(),
        });
        facts.last_served_slot = Some(SlotRef {
            candidate_id: candidate.id.clone(),
            preference_index: self
                .plan
                .preference
                .iter()
                .position(|preferred| preferred.candidate_id == candidate.id)
                .unwrap_or(0) as u32,
        });
        facts.last_prompt_tokens = Some(usage.input_tokens);
        facts.consecutive_failures = 0;
        facts.cache_anchor = Some(CacheAnchor {
            provider: format!("{:?}", candidate.upstream.provider),
            physical_model: candidate.upstream.model.clone(),
            prefix_hash: self.prefix_hash.clone(),
            cached_tokens: u64::from(usage.cached_tokens.unwrap_or(0)),
            prompt_tokens: u64::from(usage.input_tokens),
            updated_at: self.now_unix,
        });
        facts.updated_at = self.now_unix;
        facts
    }

    fn headers(
        &self,
        candidate: &Candidate,
        slot: &Slot,
        index: usize,
        reason_codes: &[ReasonCode],
    ) -> DecisionHeaders {
        DecisionHeaders {
            selected_model: self.virtual_model.id.clone(),
            served_model: candidate.upstream.model.clone(),
            thinking_level: slot.thinking_level.clone(),
            request_reason: reason::as_str(self.request_reason),
            dispatch_source: self.plan.dispatch_source.as_str(),
            plan_id: self.plan_id.clone(),
            served_slot: format!("{}#{}", candidate.id, index),
            mode: self.plan.mode.as_str(),
            reason_codes: reason_codes
                .iter()
                .map(|reason| reason.code.as_str())
                .collect::<Vec<_>>()
                .join(","),
            judge: "off",
        }
    }
}

#[allow(clippy::too_many_arguments)]
async fn forward_stream(
    db: std::sync::Arc<Db>,
    prepared: Prepared,
    candidate: Candidate,
    slot: Slot,
    mut attempts: Vec<AttemptRecord>,
    mut breakers: BTreeMap<CandidateId, BreakerState>,
    mut events: mpsc::Receiver<StreamItem>,
    first: CanonicalStreamEvent,
    tx: mpsc::Sender<StreamChunk>,
    ttft_ms: u64,
    started: Instant,
) {
    let mut usage: Option<Usage> = None;
    let mut outcome = OutcomeKind::Success;
    let mut error_message: Option<String> = None;
    let mut text = String::new();
    let mut response_id = String::new();
    let mut response_model = String::new();
    let mut finish_reason = tr_contracts::FinishReason::Stop;

    if tx.send(StreamChunk::Event(first)).await.is_err() {
        outcome = OutcomeKind::Cancelled;
    } else {
        while let Some(item) = events.recv().await {
            match item {
                StreamItem::Event(event) => {
                    match &event {
                        CanonicalStreamEvent::Started { id, model } => {
                            response_id = id.clone();
                            response_model = model.clone();
                        }
                        CanonicalStreamEvent::ContentDelta { text: delta } => {
                            text.push_str(delta);
                        }
                        CanonicalStreamEvent::Finished { reason } => {
                            finish_reason = reason.clone();
                        }
                        CanonicalStreamEvent::Usage {
                            input,
                            output,
                            cached,
                        } => {
                            usage = Some(Usage {
                                input_tokens: *input,
                                output_tokens: *output,
                                cached_tokens: *cached,
                                cache_write_tokens: None,
                            });
                        }
                        _ => {}
                    }
                    if tx.send(StreamChunk::Event(event)).await.is_err() {
                        outcome = OutcomeKind::Cancelled;
                        break;
                    }
                }
                StreamItem::Failed(failure) => {
                    breakers.insert(
                        candidate.id.clone(),
                        next_breaker(breakers.get(&candidate.id), &failure, prepared.now_unix),
                    );
                    error_message = failure.message.clone();
                    let _ = tx
                        .send(StreamChunk::Error {
                            message: failure
                                .message
                                .unwrap_or_else(|| "upstream stream failed".to_string()),
                        })
                        .await;
                    outcome = OutcomeKind::UpstreamError;
                    break;
                }
                StreamItem::Ended => break,
            }
        }
    }

    if outcome == OutcomeKind::Success {
        let _ = tx.send(StreamChunk::Done).await;
    }

    if let Some(served_usage) = usage {
        if let Some(attempt) = attempts.last_mut() {
            attempt.usage = Some(served_usage);
            attempt.duration_ms = started.elapsed().as_millis() as u64;
        }
    }

    let duration_ms = started.elapsed().as_millis() as u64;
    let ledger = build_ledger(
        &prepared.ledger_input(prepared.plan.dispatch_source),
        attempts,
        Some(Served {
            slot: slot.clone(),
            candidate: candidate.clone(),
            usage: usage.unwrap_or_default(),
            ttft_ms: Some(ttft_ms),
        }),
        outcome,
        if outcome == OutcomeKind::Success {
            Some(200)
        } else if outcome == OutcomeKind::Cancelled {
            None
        } else {
            Some(502)
        },
        duration_ms,
    );

    let facts = if outcome == OutcomeKind::Success {
        Some(prepared.updated_facts(&candidate, &slot, usage.unwrap_or_default()))
    } else {
        None
    };
    let breakers: Vec<_> = breakers.into_iter().collect();
    if let Err(error) = store::commit(&db, facts.as_ref(), &breakers, &ledger) {
        eprintln!(
            "stream ledger commit failed plan_id={}: {error}",
            ledger.plan_id
        );
    }

    if outcome != OutcomeKind::Success
        && let Some(message) = error_message
    {
        eprintln!(
            "stream upstream failure plan_id={} candidate={}: {message}",
            ledger.plan_id, candidate.id
        );
    }

    if outcome == OutcomeKind::Success && !response_id.is_empty() {
        let response = CanonicalResponse {
            id: response_id,
            model: response_model,
            blocks: if text.is_empty() {
                Vec::new()
            } else {
                vec![tr_contracts::ContentBlock::Text {
                    text,
                    cache_control: None,
                }]
            },
            finish_reason,
            usage: usage.unwrap_or_default(),
        };
        persist_protocol_state(&db, &prepared, &response);
    }
}

fn persist_protocol_state(db: &Db, prepared: &Prepared, response: &CanonicalResponse) {
    if prepared.canonical.meta.ingress_protocol != IngressProtocol::Responses
        || response.id.is_empty()
    {
        return;
    }
    let Ok(payload) = serde_json::to_value(response) else {
        return;
    };
    // 7 天 TTL：足够覆盖 Codex 的多轮工具循环，又不会让表无限增长。
    let _ = store::protocol::save(
        db,
        &format!("resp:{}", response.id),
        &prepared.session_key,
        &prepared.branch_id,
        "previous_response",
        &payload,
        7 * 24 * 3600,
        prepared.now_unix,
    );
}

struct LedgerInput {
    plan_id: String,
    session_key: String,
    branch_id: String,
    task_id: String,
    session_key_source: SessionKeySource,
    collision_suspect: bool,
    ingress_protocol: IngressProtocol,
    virtual_model_id: String,
    thinking_level: String,
    request_reason: RequestReason,
    plan_mode: PlanMode,
    preference: Vec<CandidateId>,
    failover: Vec<CandidateId>,
    dispatch_source: DispatchSource,
    reason_codes: Vec<ReasonCode>,
    profile_revision: String,
    config_digest: String,
    now_unix: u64,
}

struct Served {
    slot: Slot,
    candidate: Candidate,
    usage: Usage,
    ttft_ms: Option<u64>,
}

#[allow(clippy::too_many_arguments)]
fn build_ledger(
    input: &LedgerInput,
    attempts: Vec<AttemptRecord>,
    served: Option<Served>,
    outcome: OutcomeKind,
    http_status: Option<u16>,
    duration_ms: u64,
) -> LedgerRecord {
    let (served_slot, served_physical_model, served_provider, served_thinking_level) = match &served
    {
        Some(served) => (
            Some(SlotRef {
                candidate_id: served.slot.candidate_id.clone(),
                preference_index: input
                    .preference
                    .iter()
                    .position(|id| id == &served.slot.candidate_id)
                    .unwrap_or(0) as u32,
            }),
            Some(served.candidate.upstream.model.clone()),
            Some(format!("{:?}", served.candidate.upstream.provider)),
            Some(served.slot.thinking_level.clone()),
        ),
        None => (None, None, None, None),
    };

    let answer_cost: Option<Cost> = served.as_ref().and_then(|served| {
        served
            .candidate
            .price
            .as_ref()
            .map(|price| price.estimate(&served.usage))
    });

    LedgerRecord {
        request_id: Uuid::new_v4(),
        plan_id: input.plan_id.clone(),
        session_key: input.session_key.clone(),
        branch_id: input.branch_id.clone(),
        task_id: input.task_id.clone(),
        session_key_source: input.session_key_source,
        collision_suspect: input.collision_suspect,
        ingress_protocol: input.ingress_protocol,
        selected_virtual_model: Some(input.virtual_model_id.clone()),
        selected_thinking_level: Some(input.thinking_level.clone()),
        request_reason: input.request_reason,
        plan_mode: input.plan_mode,
        preference: input.preference.clone(),
        failover: input.failover.clone(),
        served_slot,
        served_physical_model,
        served_provider,
        served_thinking_level,
        dispatch_source: input.dispatch_source,
        switches: attempts.len().saturating_sub(1) as u32,
        input_tokens: served.as_ref().map(|served| served.usage.input_tokens),
        output_tokens: served.as_ref().map(|served| served.usage.output_tokens),
        cache_read_tokens: served
            .as_ref()
            .and_then(|served| served.usage.cached_tokens)
            .map(u64::from),
        cache_write_tokens: served
            .as_ref()
            .and_then(|served| served.usage.cache_write_tokens)
            .map(u64::from),
        cache_hit_tokens: served
            .as_ref()
            .and_then(|served| served.usage.cached_tokens)
            .map(u64::from),
        attempts,
        judge: None,
        answer_cost: answer_cost.clone(),
        routing_cost: None,
        cost_equiv: answer_cost,
        facts_completeness: FactsCompleteness::Complete,
        reason_codes: input.reason_codes.clone(),
        outcome,
        http_status,
        profile_revision: Some(input.profile_revision.clone()),
        config_digest: Some(input.config_digest.clone()),
        ttft_ms: served.as_ref().and_then(|served| served.ttft_ms),
        duration_ms,
        created_at: input.now_unix,
    }
}

fn reset_breaker(previous: Option<&BreakerState>, now_unix: u64) -> BreakerState {
    let mut state = previous.cloned().unwrap_or_default();
    state.consecutive_failures = 0;
    state.open_until = None;
    state.half_open_since = None;
    state.updated_at = now_unix;
    state
}

/// 按 §18.4.2 的行为矩阵更新熔断状态。
fn next_breaker(
    previous: Option<&BreakerState>,
    failure: &AttemptFailure,
    now_unix: u64,
) -> BreakerState {
    let mut state = previous.cloned().unwrap_or_default();
    state.updated_at = now_unix;

    if !failure.class.counts_toward_breaker() {
        return state;
    }

    let increment = if failure.class == ErrorClass::Overloaded {
        2
    } else {
        1
    };
    state.consecutive_failures = state.consecutive_failures.saturating_add(increment);
    state.last_error_class = Some(failure.class);
    state.last_failure_at = Some(now_unix);

    if state.consecutive_failures >= 3 && !state.is_open(now_unix) {
        let cooldown_secs = failure
            .retry_after_ms
            .map(|ms| (ms / 1000).max(1))
            .unwrap_or(match failure.class {
                ErrorClass::Overloaded => 30,
                ErrorClass::RateLimit => 60,
                _ => 15,
            });
        state.open_until = Some(now_unix + cooldown_secs);
        state.opened_count = state.opened_count.saturating_add(1);
    }

    state
}

fn no_attempt_failure() -> AttemptFailure {
    AttemptFailure {
        class: ErrorClass::Unknown,
        status: None,
        message: Some("no attempt executed".to_string()),
        retry_after_ms: None,
        cause: tr_upstream::http::AttemptCause::Transport,
    }
}

fn with_cas_conflict(reasons: &[ReasonCode]) -> Vec<ReasonCode> {
    reasons
        .iter()
        .cloned()
        .chain(std::iter::once(ReasonCode::new(
            ReasonCodeKind::StateCasConflict,
        )))
        .collect()
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
}
