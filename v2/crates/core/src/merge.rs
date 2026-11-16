//! 贡献合并与 Plan 校验（§6.3、§4.8）。
//!
//! 合并顺序固定，不依赖 `drafts` 的传入顺序：
//! Filter → 基础顺序 → Prefer 重排（按优先级从弱到强）→ Pin → Mode → 接力 → CacheLoss → State。
//!
//! 校验失败**不返回错误**：按 §4.8 的兜底规则修正 Plan 并写 reason code，
//! 让请求继续有候选可用；只有“一个候选都没有”才留下空 preference 交给 `agent` 转 503。

use std::collections::BTreeSet;

use tr_contracts::{
    Candidate, CandidateId, Cost, JudgeTrace, Plan, PlanContribution, PlanDraft, PlanMode,
    ReasonCode, ReasonCodeKind, RequestView, Slot, StateUpdate, Usage,
};

use crate::signals::estimate_input_tokens;
use crate::strategy::{clamp_thinking_level, tier_rank};

/// 合并多个策略的草稿。
pub fn merge(view: &RequestView, drafts: &[(u8, PlanDraft)]) -> Plan {
    let estimated_input = estimate_input_tokens(view.canonical);

    let mut excluded: BTreeSet<CandidateId> = BTreeSet::new();
    let mut prefers: Vec<(u8, Vec<CandidateId>)> = Vec::new();
    let mut pins: Vec<(u8, CandidateId)> = Vec::new();
    let mut modes: Vec<(u8, PlanMode)> = Vec::new();
    let mut cache_losses: Vec<Cost> = Vec::new();
    let mut reason_codes: Vec<ReasonCode> = Vec::new();
    let mut state_updates: Vec<StateUpdate> = Vec::new();

    for (priority, draft) in drafts {
        for contribution in &draft.contributions {
            match contribution {
                PlanContribution::Filter { exclude, reason } => {
                    excluded.extend(exclude.iter().cloned());
                    push_reason(&mut reason_codes, reason.clone());
                }
                PlanContribution::Prefer { order, reason } => {
                    prefers.push((*priority, order.clone()));
                    push_reason(&mut reason_codes, reason.clone());
                }
                PlanContribution::Pin { candidate, reason } => {
                    pins.push((*priority, candidate.clone()));
                    push_reason(&mut reason_codes, reason.clone());
                }
                PlanContribution::Mode { mode, reason } => {
                    modes.push((*priority, *mode));
                    push_reason(&mut reason_codes, reason.clone());
                }
                PlanContribution::CacheLoss { cost, reason } => {
                    cache_losses.push(cost.clone());
                    push_reason(&mut reason_codes, reason.clone());
                }
            }
        }
        state_updates.extend(draft.state_updates.iter().cloned());
    }

    // 基础顺序：未声明 Prefer 时按 strength 升序（弱档先试，Cascade 的语义）。
    let mut order: Vec<CandidateId> = view
        .candidates
        .iter()
        .filter(|candidate| candidate.enabled && !excluded.contains(&candidate.id))
        .map(|candidate| candidate.id.clone())
        .collect();
    order.sort_by_key(|id| {
        let candidate = view.candidate(id);
        (
            candidate.map_or(usize::MAX, |c| tier_rank(view.profile, &c.tier)),
            candidate.map_or(u8::MAX, |c| c.strength),
            id.clone(),
        )
    });

    // Prefer 重排：优先级从弱到强（数值大 → 小），强策略最后落笔。
    prefers.sort_by_key(|(priority, _)| std::cmp::Reverse(*priority));
    for (_, preferred) in &prefers {
        for id in preferred.iter().rev() {
            if excluded.contains(id) {
                continue;
            }
            order.retain(|existing| existing != id);
            order.insert(0, id.clone());
        }
    }

    // Pin：同样按优先级从弱到强，最终最强策略的 pin 在队首。
    pins.sort_by_key(|(priority, _)| std::cmp::Reverse(*priority));
    for (_, candidate) in &pins {
        if excluded.contains(candidate) {
            push_reason(
                &mut reason_codes,
                ReasonCode::with_detail(ReasonCodeKind::LatchTargetUnavailable, candidate.clone()),
            );
            continue;
        }
        if !view.candidates.iter().any(|c| &c.id == candidate) {
            continue;
        }
        order.retain(|existing| existing != candidate);
        order.insert(0, candidate.clone());
    }

    // Mode：优先级最高的胜出（数值最小）。
    modes.sort_by_key(|(priority, _)| *priority);
    let mut mode = modes.first().map_or(view.profile.mode, |(_, mode)| *mode);

    // 兜底与校验（§4.8）。
    order.retain(|id| view.candidates.iter().any(|candidate| &candidate.id == id));
    order.dedup();

    let before = order.len();
    order.retain(|id| {
        view.candidate(id)
            .is_none_or(|candidate| candidate.window >= estimated_input)
    });
    if order.len() != before {
        push_reason(
            &mut reason_codes,
            ReasonCode::with_detail(
                ReasonCodeKind::PlanFilterWindow,
                format!("estimated_input_tokens={estimated_input}"),
            ),
        );
    }

    if order.is_empty() {
        push_reason(
            &mut reason_codes,
            ReasonCode::with_detail(
                ReasonCodeKind::CoreFallback,
                "preference empty after filters; falling back to enabled candidates",
            ),
        );
        let mut fallback: Vec<&Candidate> = view
            .candidates
            .iter()
            .filter(|candidate| candidate.enabled)
            .collect();
        fallback
            .sort_by_key(|candidate| (std::cmp::Reverse(candidate.strength), candidate.id.clone()));
        order = fallback
            .iter()
            .map(|candidate| candidate.id.clone())
            .collect();
        if order.is_empty() {
            push_reason(
                &mut reason_codes,
                ReasonCode::new(ReasonCodeKind::NoEligibleCandidate),
            );
        }
    }

    if mode == PlanMode::Cascade && order.len() == 1 {
        mode = PlanMode::Single;
        push_reason(
            &mut reason_codes,
            ReasonCode::new(ReasonCodeKind::PlanModeDowngraded),
        );
    }

    let preference: Vec<Slot> = order
        .iter()
        .filter_map(|id| view.candidate(id))
        .map(|candidate| slot_for(view, candidate, estimated_input))
        .collect();

    // 接力顺序 = 最终偏好顺序（健康度已在 Failover 过滤过）；为空时退回偏好自身。
    let mut failover = preference.clone();
    failover.retain(|slot| view.health.can_attempt(&slot.candidate_id, view.now_unix));
    if failover.is_empty() {
        failover = preference.clone();
    }

    Plan {
        selection: view.selection.clone(),
        dispatch_source: tr_contracts::DispatchSource::Strategy,
        preference,
        failover,
        mode,
        expected_cache_loss: max_cost(cache_losses),
        judge: JudgeTrace::default(),
        reason_codes,
        state_updates: merge_state_updates(state_updates),
    }
}

fn slot_for(view: &RequestView, candidate: &Candidate, estimated_input: u32) -> Slot {
    Slot {
        candidate_id: candidate.id.clone(),
        tier: candidate.tier.clone(),
        strength: candidate.strength,
        window: candidate.window,
        thinking_level: clamp_thinking_level(candidate, &view.selection.thinking_level),
        est_cost: candidate.price.as_ref().map(|price| {
            price.estimate(&Usage {
                input_tokens: estimated_input,
                ..Usage::default()
            })
        }),
    }
}

/// CacheLoss 取最大值：保守估计（多次切换按最坏情况算）。
fn max_cost(costs: Vec<Cost>) -> Option<Cost> {
    let mut iter = costs.into_iter();
    let mut best = iter.next()?;
    for cost in iter {
        if cost.currency != best.currency {
            // 币种不同不混算：直接放弃估计，交由调用方按 unknown 处理。
            return None;
        }
        if cost.micros > best.micros {
            best = cost;
        }
    }
    Some(best)
}

/// 同一 `(strategy_id, task_id)` 后写覆盖；顺序按传入顺序（弱策略先、强策略后）。
fn merge_state_updates(updates: Vec<StateUpdate>) -> Vec<StateUpdate> {
    let mut merged: Vec<StateUpdate> = Vec::new();
    for update in updates {
        if let Some(existing) = merged.iter_mut().find(|existing| {
            existing.strategy_id == update.strategy_id && existing.task_id == update.task_id
        }) {
            *existing = update;
        } else {
            merged.push(update);
        }
    }
    merged
}

fn push_reason(reasons: &mut Vec<ReasonCode>, reason: ReasonCode) {
    if !reasons.contains(&reason) {
        reasons.push(reason);
    }
}
