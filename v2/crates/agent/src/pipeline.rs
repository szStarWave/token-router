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
    AttemptRecord, BreakerState, CacheAnchor, Candidate, CandidateHealth, CandidateId,
    CanonicalRequest, CanonicalResponse, CanonicalStreamEvent, Cost, DecisionSnapshot, Dispatch,
    DispatchSource, ErrorClass, FactsCompleteness, IngressProtocol, LedgerRecord, OutcomeKind,
    Plan, PlanMode, ReasonCode, ReasonCodeKind, RequestReason, Selection, SessionFacts,
    SessionKeySource, Slot, SlotRef, Usage, VirtualModel,
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
    /// 分支 id（fork/`/tree`）；默认 `main`。状态与账本都按它隔离。
    pub branch_id: Option<String>,
    /// 授权身份（§8.5）；P0–P3 只填 `authority_id`。
    pub authority: Option<tr_contracts::RequestAuthority>,
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
    #[error("approval required for tool `{tool}` ({risk_class})")]
    ApprovalRequired {
        tool: String,
        risk_class: String,
        request_id: Uuid,
    },
    #[error("approval denied for tool `{tool}` ({risk_class})")]
    ApprovalDenied {
        tool: String,
        risk_class: String,
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
    judge: std::sync::Arc<crate::judge::JudgeRuntime>,
    cost_gate: crate::gate::CostGateMode,
    /// shadow 记录异步写入队列（P1-6：不阻塞主路径）。
    shadow_tx: std::sync::mpsc::Sender<store::judge::ShadowRecord>,
    /// 会话键退化告警去重：每个键只 warn 一次（§8.1）。
    warned_sessions: std::sync::Mutex<std::collections::BTreeSet<String>>,
}

impl Pipeline {
    pub fn new(db: Db, catalog: Catalog, upstream: HttpUpstream) -> Self {
        // 判官构建失败（TLS/配置）只可能是进程级问题：降级为 off 并记录，请求仍可服务。
        let judge = match crate::judge::JudgeRuntime::new(catalog.judge.clone()) {
            Ok(runtime) => runtime,
            Err(error) => {
                eprintln!("judge disabled: {error}");
                crate::judge::JudgeRuntime::new(crate::judge::JudgeSettings::default())
                    .expect("off judge runtime")
            }
        };
        let cost_gate = catalog.cost_gate;
        let db = std::sync::Arc::new(db);
        let shadow_tx = spawn_shadow_writer(std::sync::Arc::clone(&db));
        Self {
            db,
            catalog: std::sync::Arc::new(catalog),
            upstream,
            judge: std::sync::Arc::new(judge),
            cost_gate,
            shadow_tx,
            warned_sessions: std::sync::Mutex::new(std::collections::BTreeSet::new()),
        }
    }

    pub fn from_catalog(db: Db, catalog: Catalog) -> Self {
        Self::new(db, catalog, HttpUpstream::default())
    }

    /// 判官是否 enabled（诊断/测试）。
    pub fn judge_mode(&self) -> crate::judge::JudgeMode {
        self.judge.mode()
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

        let branch_id = context
            .branch_id
            .clone()
            .filter(|branch| !branch.is_empty())
            .unwrap_or_else(|| "main".to_string());
        let task_id = String::new();

        let session_input = session::build_input(&request, context.server_session_id.clone());
        let derived = session::derive(&session_input);

        // 退化键只 warn 一次，避免每个请求刷屏（§8.1 禁止静默失效）。
        if derived.collision_suspect {
            let mut warned = self
                .warned_sessions
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            if warned.insert(derived.key.clone()) {
                eprintln!(
                    "session key fallback for {}: content too sparse, per-turn state is degraded",
                    derived.key
                );
            }
        }

        let mut facts =
            store::sessions::load(&self.db, &derived.key, &branch_id)?.unwrap_or_else(|| {
                SessionFacts {
                    session_key: derived.key.clone(),
                    branch_id: branch_id.clone(),
                    ..SessionFacts::default()
                }
            });
        // 授权身份随请求更新；账本与 SessionFacts 同源（§8.5）。
        facts.authority = context.authority.clone().unwrap_or_default();

        // 会话恢复：未指定虚拟模型时优先沿用上次选择；存储的选择已不存在 → 回退默认并记 reason。
        let stored_selection = facts.selected_virtual_model.clone();
        let mut virtual_model_missing = false;
        let virtual_model_id = match request.meta.virtual_model.clone() {
            Some(id) => id,
            None => match stored_selection {
                Some(id) if self.catalog.virtual_models.contains_key(&id) => id,
                Some(_) => {
                    virtual_model_missing = true;
                    self.catalog.default_virtual_model.clone()
                }
                None => self.catalog.default_virtual_model.clone(),
            },
        };
        let virtual_model = self
            .catalog
            .virtual_model(&virtual_model_id)
            .cloned()
            .ok_or_else(|| PipelineError::ModelNotFound(virtual_model_id.clone()))?;
        // 信任档位别名（P3-14）：只解析到配置里已有的 profile，虚拟模型本身不变。
        let (profile_ref, trust_tier_applied) = self
            .catalog
            .profile_for_trust(&virtual_model, request.meta.trust_tier.as_deref())
            .ok_or_else(|| {
                PipelineError::ModelNotFound(format!("profile:{}", virtual_model.profile))
            })?;
        let profile = profile_ref.clone();
        let thinking_level = request
            .meta
            .thinking_level
            .clone()
            .filter(|level| !level.is_empty())
            .unwrap_or_else(|| "off".to_string());
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
            strategy_states: strategy_states.clone(),
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
            failed: if request_reason == RequestReason::Retry {
                facts.last_failed.clone()
            } else {
                None
            },
            context_hold: facts.context_holds.get(&virtual_model.id).cloned(),
            router_state: router_state.clone(),
        };

        // 两阶段：draft 产出 Plan 与判官请求；active 模式下 refine 覆盖 Plan。
        let group_digest = group_digest(&virtual_model, &profile);
        let plan_revision = plan_revision(&group_digest);
        let strategies = strategy_set(&virtual_model);
        let (mut plan, requests) = tr_core::decide_and_collect(&snapshot, &strategies);
        // 未知档位不报错（客户端可能先于配置升级），回退虚拟模型 profile 并留下审计理由。
        if let Some(tier) = request.meta.trust_tier.as_deref()
            && !trust_tier_applied
        {
            plan.reason_codes.push(ReasonCode::with_detail(
                ReasonCodeKind::TrustTierUnknown,
                tier,
            ));
        }
        let plan_uuid = Uuid::new_v4().simple().to_string();
        let plan_id = format!("plan-{}", &plan_uuid[..12]);

        let mut shadow: Option<ShadowInfo> = None;
        let mut routing_cost: Option<Cost> = None;
        if !requests.is_empty() && self.judge.mode() != crate::judge::JudgeMode::Off {
            let outcome = self
                .judge
                .consult(&derived.key, &plan_id, &snapshot, &requests)
                .await;

            routing_cost = outcome.routing_cost.clone();

            if let Some(kind) = crate::judge::JudgeRuntime::unavailable_reason(&outcome) {
                plan.reason_codes.push(ReasonCode::new(kind));
            }

            if self.judge.is_active() && !outcome.verdicts.is_empty() {
                plan = tr_core::decide_with_verdicts(&snapshot, &strategies, &outcome.verdicts);
            }

            if let Some(trace) = outcome.traces.first().cloned() {
                plan.judge = trace.clone();
                if self.judge.is_shadow() {
                    let kernel = plan.preference.first();
                    let max_strength = candidates
                        .iter()
                        .map(|candidate| candidate.strength)
                        .max()
                        .unwrap_or(0);
                    shadow = Some(ShadowInfo {
                        kernel_choice: kernel
                            .map(|slot| slot.candidate_id.clone())
                            .unwrap_or_default(),
                        kernel_is_strongest: kernel
                            .is_some_and(|slot| slot.strength >= max_strength && max_strength > 0),
                        verdict: outcome.verdicts.first().cloned(),
                        value: outcome
                            .first_value
                            .as_ref()
                            .and_then(|value| serde_json::to_value(value).ok()),
                        question_id: requests.first().map(|request| request.id.clone()),
                        effective_source: trace.effective_source.as_str().to_string(),
                        latency_ms: outcome.latency_ms,
                        snapshot_digest: snapshot.projection_digest().unwrap_or_default(),
                    });
                }
            }

            for call in &outcome.calls {
                if let Err(error) = store::judge::insert_call(&self.db, call) {
                    eprintln!("judge_calls insert failed: {error}");
                }
            }
        }

        // 会话恢复时虚拟模型已不存在：已回退默认模型，这里补审计理由。
        if virtual_model_missing {
            plan.reason_codes
                .push(ReasonCode::new(ReasonCodeKind::VirtualModelMissing));
        }

        // Sticky / ContextHold：硬复用（Retry/Continuation）与软亲和（User）。
        //
        // 例外：Escalation latch 与 CompetenceGuard 低分保护都是权威状态。
        // latch 生效时跳过 sticky，否则 agent 工具续接（Continuation）会把强档拉回弱档；
        // 胜任度保护同理，否则上一成功候选的 StickyDispatch 会覆盖保护。
        let latched = plan
            .reason_codes
            .iter()
            .any(|reason| reason.code == ReasonCodeKind::EscalationLatched);
        let protected = plan
            .reason_codes
            .iter()
            .any(|reason| reason.code == ReasonCodeKind::CompetenceProtected);
        if !latched && !protected {
            let resolution = crate::dispatch::resolve(&crate::dispatch::DispatchContext {
                reason: request_reason,
                previous: snapshot.previous.as_ref(),
                failed: snapshot.failed.as_ref(),
                hold: snapshot.context_hold.as_ref(),
                candidates: &candidates,
                health: &snapshot.health,
                now_unix,
                prompt_tokens: tr_core::signals::estimate_input_tokens(&request),
                required_input: required_input(&request),
                virtual_model_id: &virtual_model.id,
                profile_id: &profile.id,
                ingress_protocol: request.meta.ingress_protocol.as_str(),
                identity_digest: &derived.key,
                group_digest: &group_digest,
                plan_revision,
                direct_opts_into_sticky: false,
            });
            apply_dispatch(&mut plan, resolution);
        }

        // 成本闸：P2 默认 shadow（只记录）；active 时拒绝无收益的切换。
        self.apply_cost_gate(&mut plan, &facts, &profile, &snapshot);

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
                    authority: facts.authority.clone(),
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
                    judge: Some(plan.judge.clone()),
                    routing_cost: routing_cost.clone(),
                    contributions: plan.contributions.clone(),
                },
                Vec::new(),
                None,
                OutcomeKind::UpstreamError,
                Some(503),
                0,
            );
            store::commit(&self.db, None, &[], &ledger, None)?;
            return Err(PipelineError::NoEligibleCandidate {
                plan_id,
                reason_codes: plan.reason_codes,
            });
        }

        // 审批实时拦截（P3-12/13）：只在本地判定，不访问上游、不进入判官 deadline。
        let approval = crate::approval::evaluate(
            &self.db,
            &self.catalog.approvals,
            &derived.key,
            &request,
            now_unix,
        )?;
        let pending = match &approval {
            crate::approval::ApprovalOutcome::Clear => None,
            crate::approval::ApprovalOutcome::Required(tool) => {
                plan.reason_codes.push(ReasonCode::with_detail(
                    ReasonCodeKind::ApprovalRequired,
                    format!("{}:{}", tool.tool_name, tool.risk.as_str()),
                ));
                Some((tool.clone(), 428u16, false))
            }
            crate::approval::ApprovalOutcome::Denied(tool) => {
                plan.reason_codes.push(ReasonCode::with_detail(
                    ReasonCodeKind::ApprovalDenied,
                    format!("{}:{}", tool.tool_name, tool.risk.as_str()),
                ));
                Some((tool.clone(), 403u16, true))
            }
        };

        // observe 只记录；enforce 且 fail_closed 才拒绝。审批与路由不共享预算池。
        if let Some((tool, status, denied)) = pending
            && self.catalog.approvals.mode == crate::approval::ApprovalMode::Enforce
            && self.catalog.approvals.fail_closed
        {
            let ledger = build_ledger(
                &LedgerInput {
                    plan_id: plan_id.clone(),
                    session_key: derived.key.clone(),
                    branch_id: branch_id.clone(),
                    task_id: task_id.clone(),
                    session_key_source: derived.source,
                    collision_suspect: derived.collision_suspect,
                    ingress_protocol: request.meta.ingress_protocol,
                    authority: facts.authority.clone(),
                    virtual_model_id: virtual_model.id.clone(),
                    thinking_level: thinking_level.clone(),
                    request_reason,
                    plan_mode: plan.mode,
                    preference: plan.preference_ids().into_iter().cloned().collect(),
                    failover: plan
                        .failover
                        .iter()
                        .map(|slot| slot.candidate_id.clone())
                        .collect(),
                    dispatch_source: plan.dispatch_source,
                    reason_codes: plan.reason_codes.clone(),
                    profile_revision: profile.revision.clone(),
                    config_digest: self.catalog.config_digest.clone(),
                    now_unix,
                    judge: Some(plan.judge.clone()),
                    routing_cost: routing_cost.clone(),
                    contributions: plan.contributions.clone(),
                },
                Vec::new(),
                None,
                OutcomeKind::Rejected,
                Some(status),
                0,
            );
            let request_id = ledger.request_id;
            if !denied {
                // 记录“已拦截、等待人工”；pending 不是结论，不会让后续请求放行。
                let _ = store::approvals::insert(
                    &self.db,
                    &store::approvals::ApprovalRecord {
                        request_id: request_id.to_string(),
                        session_key: Some(derived.key.clone()),
                        tool_name: tool.tool_name.clone(),
                        risk_class: tool.risk.as_str().to_string(),
                        decision: "pending".to_string(),
                        operator: None,
                        wait_ms: None,
                        created_at: now_unix,
                    },
                );
            }
            store::commit(&self.db, None, &[], &ledger, None)?;
            return Err(if denied {
                PipelineError::ApprovalDenied {
                    tool: tool.tool_name,
                    risk_class: tool.risk.as_str().to_string(),
                    request_id,
                }
            } else {
                PipelineError::ApprovalRequired {
                    tool: tool.tool_name,
                    risk_class: tool.risk.as_str().to_string(),
                    request_id,
                }
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
            strategy_states,
            shadow,
            group_digest,
            plan_revision,
            routing_cost,
            router_state,
        })
    }

    /// 成本闸：在决策与执行之间调整 Plan（§9.4）。
    ///
    /// - `off`：不计算；
    /// - `shadow`：只写结论与 `expected_cache_loss`，不改变顺序；
    /// - `active`：拒绝时把上一成功候选排回队首（若仍在计划里）。
    fn apply_cost_gate(
        &self,
        plan: &mut Plan,
        facts: &SessionFacts,
        profile: &tr_contracts::ProfileConfig,
        snapshot: &DecisionSnapshot,
    ) {
        if self.cost_gate == crate::gate::CostGateMode::Off {
            return;
        }
        let Some(previous_ref) = facts.last_served_slot.as_ref() else {
            return;
        };
        let Some(next_slot) = plan.preference.first().cloned() else {
            return;
        };
        if previous_ref.candidate_id == next_slot.candidate_id {
            return;
        }
        let (Some(previous), Some(next)) = (
            self.catalog.candidate(&previous_ref.candidate_id),
            self.catalog.candidate(&next_slot.candidate_id),
        ) else {
            return;
        };

        let anchor = facts.cache_anchor.as_ref();
        let same_prefix = anchor.is_some_and(|anchor| {
            anchor.physical_model == next.upstream.model
                && anchor.provider == format!("{:?}", next.upstream.provider)
        });

        let verdict = crate::gate::evaluate(&crate::gate::SwitchContext {
            previous,
            next,
            previous_cached_tokens: anchor.map_or(0, |anchor| anchor.cached_tokens),
            prompt_tokens: u64::from(facts.last_prompt_tokens.unwrap_or(0)),
            same_prefix_reusable: same_prefix,
            difficulty_gap: snapshot.signals.core.difficulty_gap,
            hard_signal_score: snapshot.signals.core.hard_signal_score,
            profile,
            policy: profile.unknown_price_policy,
        });

        match verdict {
            crate::gate::SwitchVerdict::Skip => {}
            crate::gate::SwitchVerdict::Allow {
                expected_cache_loss,
                reasons,
            } => {
                plan.expected_cache_loss = expected_cache_loss;
                plan.reason_codes.extend(reasons);
            }
            crate::gate::SwitchVerdict::Deny {
                expected_cache_loss,
                reasons,
            } => {
                plan.expected_cache_loss = expected_cache_loss;
                plan.reason_codes.extend(reasons);

                if self.cost_gate == crate::gate::CostGateMode::Active {
                    if let Some(position) = plan
                        .preference
                        .iter()
                        .position(|slot| slot.candidate_id == previous_ref.candidate_id)
                    {
                        let slot = plan.preference.remove(position);
                        plan.preference.insert(0, slot);
                    }
                    if let Some(position) = plan
                        .failover
                        .iter()
                        .position(|slot| slot.candidate_id == previous_ref.candidate_id)
                    {
                        let slot = plan.failover.remove(position);
                        plan.failover.insert(0, slot);
                    }
                }
            }
        }
    }

    /// 非流式处理。
    pub async fn handle(
        &self,
        request: CanonicalRequest,
        context: RequestContext,
    ) -> Result<PipelineResponse, PipelineError> {
        let started = Instant::now();
        let prepared = self.prepare(request, context).await?;
        let span = decision_span(&prepared);
        tracing::info!(parent: &span, "plan");

        let mut attempts: Vec<AttemptRecord> = Vec::new();
        let mut breakers: BTreeMap<CandidateId, BreakerState> = BTreeMap::new();
        let mut last_failure: Option<AttemptFailure> = None;
        let mut success: Option<(usize, Candidate, Slot, CanonicalResponse)> = None;
        let health = store::health::load(&self.db)?;

        for (index, slot) in prepared.plan.preference.iter().enumerate() {
            let Some(candidate) = self.catalog.candidate(&slot.candidate_id).cloned() else {
                continue;
            };
            if !probe_allowed(&self.db, &health, &candidate.id, prepared.now_unix)? {
                continue;
            }

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
            let failed_facts = build_failed_facts(
                &self.catalog,
                &prepared,
                Some(failure.class),
                failure.status,
                failure.message.as_deref(),
                &attempts,
            );
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
            store::commit(&self.db, failed_facts.as_ref(), &breakers, &ledger, None)?;
            persist_state_updates(&self.db, &prepared);
            tracing::info!(
                parent: &span,
                outcome = "upstream_error",
                error_class = %failure.class.as_str(),
                attempts = ledger.attempts.len(),
            );
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
        let route_state = prepared.next_route_state(&candidate, &slot, ledger.switches);
        let breakers: Vec<_> = breakers.into_iter().collect();
        let session_outcome = store::commit(
            &self.db,
            Some(&next_facts),
            &breakers,
            &ledger,
            route_state.as_ref(),
        )?;
        if matches!(session_outcome, Some(SaveOutcome::Conflict)) {
            store::ledger::replace_reason_codes(
                &self.db,
                &ledger.request_id.to_string(),
                &with_cas_conflict(&ledger.reason_codes),
            )?;
        }

        let headers = prepared.headers(&candidate, &slot, index, &ledger.reason_codes);

        persist_protocol_state(&self.db, &prepared, &response);
        persist_state_updates(&self.db, &prepared);
        enqueue_shadow(&self.shadow_tx, &prepared, ledger.request_id);
        tracing::info!(
            parent: &span,
            outcome = "success",
            served = %candidate.id,
            physical = %candidate.upstream.model,
            switches = ledger.switches,
            input_tokens = ledger.input_tokens,
            output_tokens = ledger.output_tokens,
        );

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
        let span = decision_span(&prepared);
        tracing::info!(parent: &span, "plan");
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
            if !probe_allowed(&self.db, &health, &candidate.id, prepared.now_unix)? {
                continue;
            }

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
                    let switches = attempts.len().saturating_sub(1);
                    tracing::info!(
                        parent: &span,
                        outcome = "stream_committed",
                        served = %candidate.id,
                        physical = %candidate.upstream.model,
                        switches,
                        ttft_ms,
                    );

                    let db = self.db.clone();
                    let task_catalog = self.catalog.clone();
                    let task_prepared = prepared;
                    let task_candidate = candidate;
                    let task_slot = slot.clone();
                    let task_attempts = attempts;
                    let task_breakers = breakers;
                    let task_shadow_tx = self.shadow_tx.clone();

                    tokio::spawn(async move {
                        forward_stream(
                            db,
                            task_catalog,
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
                            task_shadow_tx,
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
        let failed_facts = build_failed_facts(
            &self.catalog,
            &prepared,
            Some(failure.class),
            failure.status,
            failure.message.as_deref(),
            &attempts,
        );
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
        store::commit(&self.db, failed_facts.as_ref(), &breakers, &ledger, None)?;
        persist_state_updates(&self.db, &prepared);
        tracing::info!(
            parent: &span,
            outcome = "upstream_error",
            error_class = %failure.class.as_str(),
            attempts = ledger.attempts.len(),
        );
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
    strategy_states:
        std::collections::BTreeMap<tr_contracts::StrategyId, tr_contracts::StrategyState>,
    shadow: Option<ShadowInfo>,
    group_digest: String,
    plan_revision: u64,
    routing_cost: Option<Cost>,
    /// 决策时读到的 route state（P3-9）：成功后据此写回新版本。
    router_state: Option<tr_contracts::VirtualModelState>,
}

/// shadow 模式记录：判官建议 vs 内核结论。
#[derive(Debug, Clone)]
struct ShadowInfo {
    kernel_choice: String,
    kernel_is_strongest: bool,
    verdict: Option<tr_contracts::Verdict>,
    value: Option<serde_json::Value>,
    question_id: Option<String>,
    effective_source: String,
    latency_ms: u32,
    snapshot_digest: String,
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
            authority: self.facts.authority.clone(),
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
            judge: Some(self.plan.judge.clone()),
            routing_cost: self.routing_cost.clone(),
            contributions: self.plan.contributions.clone(),
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
        // 成功后清空失败派发：Retry 不应复用已经失败的候选。
        facts.last_failed = None;
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
        facts.context_holds.insert(
            self.virtual_model.id.clone(),
            tr_contracts::ContextHold {
                candidate_id: candidate.id.clone(),
                virtual_model_id: self.virtual_model.id.clone(),
                group_digest: self.group_digest.clone(),
                plan_revision: self.plan_revision,
                profile_id: self.profile.id.clone(),
                ingress_protocol: self.canonical.meta.ingress_protocol.as_str().to_string(),
                identity_digest: self.session_key.clone(),
                updated_at: self.now_unix,
                expires_at: self.now_unix + 3600,
            },
        );
        facts.updated_at = self.now_unix;
        facts
    }

    /// 成功派发后的 route state（P3-9）：写审计历史，不回头影响本轮 Plan。
    fn next_route_state(
        &self,
        candidate: &Candidate,
        slot: &Slot,
        switches: u32,
    ) -> Option<tr_contracts::VirtualModelState> {
        crate::route_state::next_state(
            self.router_state.as_ref(),
            &self.session_key,
            &self.branch_id,
            &self.virtual_model.id,
            crate::route_state::RouteStep {
                candidate_id: candidate.id.clone(),
                physical_model: candidate.upstream.model.clone(),
                provider: format!("{:?}", candidate.upstream.provider),
                thinking_level: slot.thinking_level.clone(),
                request_reason: self.request_reason,
                dispatch_source: self.plan.dispatch_source,
                switches,
                at: self.now_unix,
            },
        )
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

/// `decision` span（P2-11）：一次请求的所有策略贡献、理由码与最终派发。
///
/// 不进入用户错误路径：span 只是观测，任何字段缺失都不影响决策。
fn decision_span(prepared: &Prepared) -> tracing::Span {
    let contributions: Vec<String> = prepared
        .plan
        .contributions
        .iter()
        .map(|record| format!("{}:{}", record.strategy_id, record.reason().code.as_str()))
        .collect();
    let reason_codes: Vec<&str> = prepared
        .plan
        .reason_codes
        .iter()
        .map(|reason| reason.code.as_str())
        .collect();
    let preference: Vec<&str> = prepared
        .plan
        .preference
        .iter()
        .map(|slot| slot.candidate_id.as_str())
        .collect();
    tracing::info_span!(
        "decision",
        plan_id = %prepared.plan_id,
        session_key = %prepared.session_key,
        virtual_model = %prepared.virtual_model.id,
        request_reason = %prepared.request_reason.as_str(),
        mode = %prepared.plan.mode.as_str(),
        preference = ?preference,
        reason_codes = ?reason_codes,
        contributions = ?contributions,
    )
}

#[allow(clippy::too_many_arguments)]
async fn forward_stream(
    db: std::sync::Arc<Db>,
    catalog: std::sync::Arc<Catalog>,
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
    shadow_tx: std::sync::mpsc::Sender<store::judge::ShadowRecord>,
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
    let failed_facts = if outcome == OutcomeKind::UpstreamError {
        build_failed_facts(
            &catalog,
            &prepared,
            attempts.last().and_then(|attempt| attempt.error_class),
            attempts.last().and_then(|attempt| attempt.http_status),
            error_message.as_deref(),
            &attempts,
        )
    } else {
        None
    };
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
        failed_facts
    };
    let route_state = facts
        .as_ref()
        .and_then(|_| prepared.next_route_state(&candidate, &slot, ledger.switches));
    let breakers: Vec<_> = breakers.into_iter().collect();
    if let Err(error) = store::commit(
        &db,
        facts.as_ref(),
        &breakers,
        &ledger,
        route_state.as_ref(),
    ) {
        eprintln!(
            "stream ledger commit failed plan_id={}: {error}",
            ledger.plan_id
        );
    }
    persist_state_updates(&db, &prepared);
    enqueue_shadow(&shadow_tx, &prepared, ledger.request_id);

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

/// 把 `Plan.state_updates` 合并进策略状态（RFC 7386 风格，顶层层级）。
///
/// 失败路径也要写：latch 的“确认次数”必须跨失败请求保留，否则永远升不上去。
fn persist_state_updates(db: &Db, prepared: &Prepared) {
    for update in &prepared.plan.state_updates {
        let mut state = prepared
            .strategy_states
            .get(&update.strategy_id)
            .cloned()
            .unwrap_or_else(|| tr_contracts::StrategyState {
                branch_id: prepared.branch_id.clone(),
                task_id: prepared.task_id.clone(),
                virtual_model_id: prepared.virtual_model.id.clone(),
                strategy_id: update.strategy_id.clone(),
                ..tr_contracts::StrategyState::default()
            });
        state.blob = merge_patch(state.blob, update.patch.clone());
        state.updated_at = prepared.now_unix;
        if let Err(error) = store::strategy::save_strategy_state(db, &prepared.session_key, &state)
        {
            eprintln!("strategy state save failed: {error}");
        }
    }
}

fn merge_patch(target: serde_json::Value, patch: serde_json::Value) -> serde_json::Value {
    match (target, patch) {
        (serde_json::Value::Object(mut target), serde_json::Value::Object(patch)) => {
            for (key, value) in patch {
                match value {
                    serde_json::Value::Null => {
                        target.remove(&key);
                    }
                    other => {
                        let existing = target.remove(&key).unwrap_or(serde_json::Value::Null);
                        target.insert(key, merge_patch(existing, other));
                    }
                }
            }
            serde_json::Value::Object(target)
        }
        (_, patch) => patch,
    }
}

/// 把 shadow 记录投入后台写队列（P1-6）：请求路径只做内存入队，不等 SQLite。
fn enqueue_shadow(
    tx: &std::sync::mpsc::Sender<store::judge::ShadowRecord>,
    prepared: &Prepared,
    request_id: Uuid,
) {
    let Some(record) = shadow_record(prepared, request_id) else {
        return;
    };
    if tx.send(record).is_err() {
        eprintln!("shadow writer stopped; dropping shadow record");
    }
}

fn shadow_record(prepared: &Prepared, request_id: Uuid) -> Option<store::judge::ShadowRecord> {
    let shadow = prepared.shadow.as_ref()?;
    Some(store::judge::ShadowRecord {
        request_id,
        selection_model: prepared.virtual_model.id.clone(),
        kernel_choice: shadow.kernel_choice.clone(),
        judge_question_id: shadow.question_id.clone(),
        judge_value: shadow.value.clone(),
        agreement: crate::judge::JudgeRuntime::agreement(
            shadow.verdict.as_ref(),
            shadow.kernel_is_strongest,
        ),
        effective_source: Some(shadow.effective_source.clone()),
        judge_latency_ms: Some(shadow.latency_ms),
        judge_cost_micros: prepared.routing_cost.as_ref().map(|cost| cost.micros),
        snapshot_digest: shadow.snapshot_digest.clone(),
        created_at: prepared.now_unix,
    })
}

/// 启动 shadow 写入线程：按批 drain 队列，失败只记日志（影子不进入用户错误路径）。
fn spawn_shadow_writer(
    db: std::sync::Arc<Db>,
) -> std::sync::mpsc::Sender<store::judge::ShadowRecord> {
    let (tx, rx) = std::sync::mpsc::channel();
    let spawned = std::thread::Builder::new()
        .name("tr-shadow-writer".to_string())
        .spawn(move || {
            while let Ok(first) = rx.recv() {
                let mut batch = vec![first];
                while let Ok(next) = rx.try_recv() {
                    batch.push(next);
                    if batch.len() >= 64 {
                        break;
                    }
                }
                for record in batch {
                    if let Err(error) = store::judge::insert_shadow(&db, &record) {
                        eprintln!("shadow record insert failed: {error}");
                    }
                }
            }
        });
    if let Err(error) = spawned {
        eprintln!("shadow writer thread failed to start: {error}");
    }
    tx
}

struct LedgerInput {
    plan_id: String,
    session_key: String,
    branch_id: String,
    task_id: String,
    session_key_source: SessionKeySource,
    collision_suspect: bool,
    ingress_protocol: IngressProtocol,
    authority: tr_contracts::RequestAuthority,
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
    judge: Option<tr_contracts::JudgeTrace>,
    routing_cost: Option<Cost>,
    contributions: Vec<tr_contracts::PlanContributionRecord>,
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
    let routing_cost = input.routing_cost.clone();
    // 等价成本必须同时包含用户流量与判官成本；币种不一致时宁可给 None 也不混算。
    let cost_equiv = match (&answer_cost, &routing_cost) {
        (Some(answer), Some(routing)) => answer.checked_add(routing),
        (Some(answer), None) => Some(answer.clone()),
        (None, Some(routing)) => Some(routing.clone()),
        (None, None) => None,
    };

    LedgerRecord {
        request_id: Uuid::new_v4(),
        plan_id: input.plan_id.clone(),
        session_key: input.session_key.clone(),
        branch_id: input.branch_id.clone(),
        task_id: input.task_id.clone(),
        session_key_source: input.session_key_source,
        collision_suspect: input.collision_suspect,
        ingress_protocol: input.ingress_protocol,
        authority: input.authority.clone(),
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
        judge: input.judge.clone(),
        contributions: input.contributions.clone(),
        answer_cost,
        routing_cost,
        cost_equiv,
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

/// 失败请求也要更新会话事实（P3-6）：`Retry` 需要知道上一次失败的候选。
fn build_failed_facts(
    catalog: &Catalog,
    prepared: &Prepared,
    class: Option<ErrorClass>,
    status: Option<u16>,
    message: Option<&str>,
    attempts: &[AttemptRecord],
) -> Option<SessionFacts> {
    let attempt = attempts.last()?;
    let candidate = catalog.candidate(&attempt.candidate_id);
    let mut facts = prepared.facts.clone();
    facts.last_failed = Some(tr_contracts::FailedDispatch {
        dispatch: Dispatch {
            physical_model: candidate
                .map(|candidate| candidate.upstream.model.clone())
                .unwrap_or_default(),
            provider: candidate
                .map(|candidate| format!("{:?}", candidate.upstream.provider))
                .unwrap_or_default(),
            thinking_level: prepared.thinking_level.clone(),
        },
        assistant_message: None,
        error_class: class,
        http_status: status,
        error: message.map(str::to_string),
        stop_reason: None,
    });
    facts.updated_at = prepared.now_unix;
    Some(facts)
}

/// 是否允许本次请求尝试该候选（P0a-10 half-open 单探针）。
fn probe_allowed(
    db: &Db,
    health: &CandidateHealth,
    candidate_id: &str,
    now_unix: u64,
) -> Result<bool, StoreError> {
    let Some(state) = health.state_of(candidate_id) else {
        return Ok(true);
    };
    match state.open_until {
        Some(until) if until > now_unix => Ok(false),
        Some(_) => store::health::acquire_half_open_probe(db, candidate_id, now_unix),
        None => Ok(true),
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

/// 候选组指纹：池 + profile 版本。ContextHold 只认这个，不认价格变化。
fn group_digest(virtual_model: &VirtualModel, profile: &tr_contracts::ProfileConfig) -> String {
    use sha2::{Digest, Sha256};
    let material = format!(
        "{}|{}|{}",
        virtual_model.id,
        profile.revision,
        virtual_model.candidate_pool.join(",")
    );
    let digest = Sha256::digest(material.as_bytes());
    let mut out = String::with_capacity(32);
    for byte in &digest[..8] {
        use std::fmt::Write as _;
        let _ = write!(out, "{byte:02x}");
    }
    out
}

fn plan_revision(group_digest: &str) -> u64 {
    u64::from_str_radix(group_digest, 16).unwrap_or(0)
}

/// 请求需要的输入模态（文本视为所有候选都支持）。
fn required_input(request: &CanonicalRequest) -> Vec<tr_contracts::InputModality> {
    let mut modalities = vec![tr_contracts::InputModality::Text];
    let has_image = request.messages.iter().any(|message| {
        message
            .blocks
            .iter()
            .any(|block| matches!(block, tr_contracts::ContentBlock::Image { .. }))
    });
    let has_audio = request.messages.iter().any(|message| {
        message
            .blocks
            .iter()
            .any(|block| matches!(block, tr_contracts::ContentBlock::Audio { .. }))
    });
    if has_image {
        modalities.push(tr_contracts::InputModality::Image);
    }
    if has_audio {
        modalities.push(tr_contracts::InputModality::Audio);
    }
    modalities
}

/// 虚拟模型可用的策略集：`virtual_models[].strategies` 非空时只启用列出的策略。
fn strategy_set(virtual_model: &VirtualModel) -> Vec<Box<dyn tr_core::strategy::Strategy>> {
    use tr_core::strategy::{Baseline, Classify, CompetenceGuard, Escalation, Failover, Strategy};

    if virtual_model.strategies.is_empty() {
        return tr_core::default_strategies();
    }

    let mut strategies: Vec<Box<dyn Strategy>> = Vec::new();
    for id in &virtual_model.strategies {
        match id.as_str() {
            "baseline" => strategies.push(Box::new(Baseline)),
            "failover" => strategies.push(Box::new(Failover)),
            "escalation" => strategies.push(Box::new(Escalation)),
            "classify" => strategies.push(Box::new(Classify)),
            "competence_guard" => strategies.push(Box::new(CompetenceGuard)),
            // 未知策略由配置校验负责报错；运行时静默忽略并保持可用。
            _ => {}
        }
    }
    if strategies.is_empty() {
        tr_core::default_strategies()
    } else {
        strategies
    }
}

/// 把 Sticky/ContextHold 的结论落到 Plan 顺序上。
fn apply_dispatch(plan: &mut Plan, resolution: crate::dispatch::Resolution) {
    if let Some(candidate) = resolution.candidate {
        if let Some(position) = plan
            .preference
            .iter()
            .position(|slot| slot.candidate_id == candidate)
        {
            let slot = plan.preference.remove(position);
            plan.preference.insert(0, slot);
            if let Some(position) = plan
                .failover
                .iter()
                .position(|slot| slot.candidate_id == candidate)
            {
                let slot = plan.failover.remove(position);
                plan.failover.insert(0, slot);
            }
            plan.dispatch_source = resolution.source;
        } else {
            plan.reason_codes.push(ReasonCode::with_detail(
                ReasonCodeKind::StickyTargetUnavailable,
                candidate,
            ));
        }
    }
    plan.reason_codes.extend(resolution.reasons);
}
