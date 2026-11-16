//! 请求编排（§5）：加载 → 信号 → 决策 → 执行 → 事务提交。
//!
//! 纯决策在 `tr-core`；本模块只做 IO 与顺序编排。P0a 特性：
//! - 非流式、OpenAI 兼容单协议；
//! - 无判官、无状态学习（`Plan.state_updates` 只落 `strategy_states`，暂不消费）；
//! - sticky / ContextHold 读取但不改变初始 dispatch（P3 启用）。
//!
//! 失败也必须写账本（§8.4）：`upstream_error` + `attempts` 记录每个候选的 `ErrorClass`。

use std::collections::BTreeMap;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use tr_contracts::{
    AttemptRecord, BreakerState, CacheAnchor, Candidate, CandidateId, CanonicalRequest,
    CanonicalResponse, Cost, DecisionSnapshot, Dispatch, DispatchSource, ErrorClass,
    FactsCompleteness, IngressProtocol, LedgerRecord, OutcomeKind, PlanMode, ReasonCode,
    ReasonCodeKind, RequestReason, Selection, SessionFacts, Slot, SlotRef, Usage,
};
use tr_upstream::{AttemptFailure, HttpUpstream};
use uuid::Uuid;

use crate::config::Catalog;
use crate::reason;
use crate::session;
use crate::store::{self, Db, StoreError, sessions::SaveOutcome};

/// 请求上下文：HTTP 层能提供、但 canonical 里没有的信息。
#[derive(Debug, Clone, Default)]
pub struct RequestContext {
    /// 服务端下发过的会话 id（客户端带回时优先）。
    pub server_session_id: Option<String>,
    /// 客户端显式声明的 `RequestReason`。
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

/// 一次成功请求的结果。
#[derive(Debug, Clone)]
pub struct PipelineResponse {
    pub response: CanonicalResponse,
    pub headers: DecisionHeaders,
    pub ledger: LedgerRecord,
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

/// 编排器。
pub struct Pipeline {
    db: Db,
    catalog: Catalog,
    upstream: HttpUpstream,
}

impl Pipeline {
    pub fn new(db: Db, catalog: Catalog, upstream: HttpUpstream) -> Self {
        Self {
            db,
            catalog,
            upstream,
        }
    }

    /// 用默认 HTTP 客户端构造（CLI/服务端入口使用）。
    pub fn from_catalog(db: Db, catalog: Catalog) -> Self {
        Self::new(db, catalog, HttpUpstream::default())
    }

    pub fn catalog(&self) -> &Catalog {
        &self.catalog
    }

    pub fn db(&self) -> &Db {
        &self.db
    }

    /// 处理一次非流式请求。
    pub async fn handle(
        &self,
        request: CanonicalRequest,
        context: RequestContext,
    ) -> Result<PipelineResponse, PipelineError> {
        let started = Instant::now();
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

        let snapshot = DecisionSnapshot {
            canonical: request.clone(),
            signals,
            candidates: candidates.clone(),
            health: health.clone(),
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
                started.elapsed().as_millis() as u64,
            );
            store::commit(&self.db, None, &[], &ledger)?;
            return Err(PipelineError::NoEligibleCandidate {
                plan_id,
                reason_codes: plan.reason_codes,
            });
        }

        let mut attempts: Vec<AttemptRecord> = Vec::new();
        let mut breakers: BTreeMap<CandidateId, BreakerState> = BTreeMap::new();
        let mut last_failure: Option<AttemptFailure> = None;
        let mut success: Option<(usize, Candidate, Slot, CanonicalResponse)> = None;

        for (index, slot) in plan.preference.iter().enumerate() {
            let Some(candidate) = self.catalog.candidate(&slot.candidate_id).cloned() else {
                continue;
            };

            let outcome = self.upstream.execute(&candidate, &request).await;
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
                    reset_breaker(health.state_of(&candidate.id), now_unix),
                );
                success = Some((index, candidate, slot.clone(), response));
                break;
            }

            let failure = outcome.failure.expect("attempt 必须有 response 或 failure");
            breakers.insert(
                candidate.id.clone(),
                next_breaker(health.state_of(&candidate.id), &failure, now_unix),
            );
            let retryable = failure.class.retryable_elsewhere();
            last_failure = Some(failure);
            if !retryable {
                break;
            }
        }

        let Some((index, candidate, slot, response)) = success else {
            let failure = last_failure.unwrap_or(AttemptFailure {
                class: ErrorClass::Unknown,
                status: None,
                message: Some("no attempt executed".to_string()),
                retry_after_ms: None,
                cause: tr_upstream::http::AttemptCause::Transport,
            });
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
                    dispatch_source: DispatchSource::Failover,
                    reason_codes: plan.reason_codes.clone(),
                    profile_revision: profile.revision.clone(),
                    config_digest: self.catalog.config_digest.clone(),
                    now_unix,
                },
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
        let ttft_ms = None;

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
            attempts,
            Some(Served {
                slot: slot.clone(),
                candidate: candidate.clone(),
                usage,
                ttft_ms,
            }),
            OutcomeKind::Success,
            Some(200),
            started.elapsed().as_millis() as u64,
        );

        let mut next_facts = facts.clone();
        next_facts.selected_virtual_model = Some(virtual_model.id.clone());
        next_facts.selected_thinking_level = Some(thinking_level.clone());
        next_facts.last_successful_dispatch = Some(Dispatch {
            physical_model: candidate.upstream.model.clone(),
            provider: format!("{:?}", candidate.upstream.provider),
            thinking_level: slot.thinking_level.clone(),
        });
        next_facts.last_served_slot = Some(SlotRef {
            candidate_id: candidate.id.clone(),
            preference_index: index as u32,
        });
        next_facts.last_prompt_tokens = Some(usage.input_tokens);
        next_facts.consecutive_failures = 0;
        next_facts.cache_anchor = Some(CacheAnchor {
            provider: format!("{:?}", candidate.upstream.provider),
            physical_model: candidate.upstream.model.clone(),
            prefix_hash: derived.prefix_hash.clone(),
            cached_tokens: u64::from(usage.cached_tokens.unwrap_or(0)),
            prompt_tokens: u64::from(usage.input_tokens),
            updated_at: now_unix,
        });
        next_facts.updated_at = now_unix;

        let breakers: Vec<_> = breakers.into_iter().collect();
        let session_outcome = store::commit(&self.db, Some(&next_facts), &breakers, &ledger)?;
        if matches!(session_outcome, Some(SaveOutcome::Conflict)) {
            store::ledger::replace_reason_codes(
                &self.db,
                &ledger.request_id.to_string(),
                &ledger
                    .reason_codes
                    .iter()
                    .cloned()
                    .chain(std::iter::once(ReasonCode::new(
                        ReasonCodeKind::StateCasConflict,
                    )))
                    .collect::<Vec<_>>(),
            )?;
        }

        let headers = DecisionHeaders {
            selected_model: virtual_model.id.clone(),
            served_model: candidate.upstream.model.clone(),
            thinking_level: slot.thinking_level.clone(),
            request_reason: reason::as_str(request_reason),
            dispatch_source: plan.dispatch_source.as_str(),
            plan_id,
            served_slot: format!("{}#{}", candidate.id, index),
            mode: plan.mode.as_str(),
            reason_codes: ledger
                .reason_codes
                .iter()
                .map(|reason| reason.code.as_str())
                .collect::<Vec<_>>()
                .join(","),
            judge: "off",
        };

        Ok(PipelineResponse {
            response,
            headers,
            ledger,
        })
    }
}

struct LedgerInput {
    plan_id: String,
    session_key: String,
    branch_id: String,
    task_id: String,
    session_key_source: tr_contracts::SessionKeySource,
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
///
/// `InvalidRequest` / `Auth` / `Quota` / `ContextOverflow` 不计数（不是候选故障）。
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

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
}
