//! `quota-reset` 策略（v3 P5-B）：配额感知的接力排序。
//!
//! 走既有 `Strategy` trait：只读 `Signals.extras` 里的配额信号，产出：
//! - **Filter**：本窗口已耗尽的候选排除（理由 `QUOTA_DEPLETED`）；
//! - **Prefer**（可选，`prefer_soonest_reset=true`）：剩余额度 > 0 的候选按
//!   重置时间升序（先到期先使用，magpie 的 smart 语义）。
//!
//! 边界：配额 ≠ 健康。熔断仍由 `Failover` 管；这里不做冷却、不改账本。

use std::collections::BTreeSet;

use tr_contracts::{
    PlanContribution, PlanDraft, ReasonCode, ReasonCodeKind, RequestView, SignalValue, StrategyId,
    StrategyState,
};

use tr_core::strategy::{Strategy, StrategyConfig, Trigger};

/// 配额感知策略。
#[derive(Debug, Default, Clone, Copy)]
pub struct QuotaReset;

impl Strategy for QuotaReset {
    fn id(&self) -> StrategyId {
        "quota-reset".to_string()
    }

    fn priority(&self) -> u8 {
        // Escalation(10) 之后、CompetenceGuard(20) 之前：配额是硬约束，先于质量微调。
        15
    }

    fn triggers(&self, cfg: &StrategyConfig) -> Vec<Trigger> {
        if cfg.triggers.is_empty() {
            vec![
                Trigger::UserTurn,
                Trigger::Continuation,
                Trigger::Retry,
                Trigger::Direct,
            ]
        } else {
            cfg.triggers.clone()
        }
    }

    fn draft(&self, view: &RequestView, _state: &StrategyState, cfg: &StrategyConfig) -> PlanDraft {
        let mut exhausted = BTreeSet::new();
        for candidate in view.candidates {
            if !candidate.enabled {
                continue;
            }
            let key = format!("quota.exhausted.{}", candidate.id);
            if matches!(view.signals.extras.get(&key), Some(SignalValue::Bool(true))) {
                exhausted.insert(candidate.id.clone());
            }
        }

        let mut draft = PlanDraft::default();
        if !exhausted.is_empty() {
            draft.contributions.push(PlanContribution::Filter {
                exclude: exhausted.clone(),
                reason: ReasonCode::new(ReasonCodeKind::QuotaDepleted),
            });
        }

        // 可选的 reset-aware 排序：只在其一已显式启用时生效。
        let prefer_soonest_reset = cfg
            .params
            .get("prefer_soonest_reset")
            .and_then(|value| value.as_bool())
            .unwrap_or(false);
        if !prefer_soonest_reset {
            return draft;
        }

        let mut ordered: Vec<(i64, &tr_contracts::Candidate)> = view
            .candidates
            .iter()
            .filter(|candidate| candidate.enabled && !exhausted.contains(&candidate.id))
            .filter_map(|candidate| {
                let key = format!("quota.reset_at.{}", candidate.id);
                match view.signals.extras.get(&key) {
                    Some(SignalValue::Int(reset_at)) => Some((*reset_at, candidate)),
                    _ => None,
                }
            })
            .collect();
        ordered.sort_by(|(left_reset, left), (right_reset, right)| {
            left_reset
                .cmp(right_reset)
                .then_with(|| left.id.cmp(&right.id))
        });
        if ordered.is_empty() {
            return draft;
        }
        draft.contributions.push(PlanContribution::Prefer {
            order: ordered
                .into_iter()
                .map(|(_, candidate)| candidate.id.clone())
                .collect(),
            reason: ReasonCode::new(ReasonCodeKind::QuotaResetPreferred),
        });
        draft
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tr_contracts::{
        Candidate, CandidateHealth, CanonicalRequest, LedgerSummary, ProfileConfig, Selection,
        SessionFacts, Signals, VirtualModel,
    };

    fn candidate(id: &str) -> Candidate {
        Candidate {
            id: id.to_string(),
            tier: "cloud".to_string(),
            strength: 50,
            enabled: true,
            window: 128_000,
            ..Candidate::default()
        }
    }

    fn params(prefer: bool) -> StrategyConfig {
        let mut config = StrategyConfig::enabled("quota-reset");
        config.params = serde_json::json!({ "prefer_soonest_reset": prefer });
        config
    }

    #[allow(clippy::too_many_arguments)]
    fn view<'a>(
        candidates: &'a [Candidate],
        signals: &'a Signals,
        canonical: &'a CanonicalRequest,
        selection: &'a Selection,
        virtual_model: &'a VirtualModel,
        health: &'a CandidateHealth,
        session: &'a SessionFacts,
        ledger: &'a LedgerSummary,
        profile: &'a ProfileConfig,
    ) -> RequestView<'a> {
        RequestView {
            canonical,
            signals,
            selection,
            virtual_model,
            reason: tr_contracts::RequestReason::User,
            previous: None,
            failed: None,
            context_hold: None,
            router_state: None,
            candidates,
            health,
            session,
            ledger,
            profile,
            now_unix: 1_700_000_000,
        }
    }

    #[test]
    fn filters_exhausted_candidates() {
        let candidates = vec![candidate("a"), candidate("b")];
        let mut signals = Signals::default();
        signals
            .extras
            .insert("quota.exhausted.a".to_string(), SignalValue::Bool(true));
        let canonical = CanonicalRequest::default();
        let selection = Selection::default();
        let virtual_model = VirtualModel::default();
        let health = CandidateHealth::default();
        let session = SessionFacts::default();
        let ledger = LedgerSummary::default();
        let profile = ProfileConfig::default();
        let draft = QuotaReset.draft(
            &view(
                &candidates,
                &signals,
                &canonical,
                &selection,
                &virtual_model,
                &health,
                &session,
                &ledger,
                &profile,
            ),
            &StrategyState::default(),
            &params(false),
        );
        assert_eq!(draft.contributions.len(), 1, "只产出 exhaustion 过滤");
        match &draft.contributions[0] {
            PlanContribution::Filter { exclude, reason } => {
                assert!(exclude.contains("a"));
                assert_eq!(reason.code, ReasonCodeKind::QuotaDepleted);
            }
            other => panic!("unexpected contribution: {other:?}"),
        }
    }

    #[test]
    fn no_signal_no_contribution() {
        let candidates = vec![candidate("a")];
        let signals = Signals::default();
        let canonical = CanonicalRequest::default();
        let selection = Selection::default();
        let virtual_model = VirtualModel::default();
        let health = CandidateHealth::default();
        let session = SessionFacts::default();
        let ledger = LedgerSummary::default();
        let profile = ProfileConfig::default();
        let draft = QuotaReset.draft(
            &view(
                &candidates,
                &signals,
                &canonical,
                &selection,
                &virtual_model,
                &health,
                &session,
                &ledger,
                &profile,
            ),
            &StrategyState::default(),
            &params(true),
        );
        assert!(draft.contributions.is_empty());
    }
}
