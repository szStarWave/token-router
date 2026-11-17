//! `fast` 策略（v3 P5-D 可选）：偏好近期 TTFT 更低的候选。
//!
//! 数据来自账本 attempts 的平均 TTFT，由流水线注入
//! `Signals.extras["perf.ttft_ms.<candidate>"]`（Float）。
//! 只在虚拟模型显式列出 `fast` 时启用——默认策略集不含它，
//! 因为“快”与“强/省”是相互竞争的目标。

use tr_contracts::{
    PlanContribution, PlanDraft, ReasonCode, ReasonCodeKind, RequestView, SignalValue, StrategyId,
    StrategyState,
};

use tr_core::strategy::{Strategy, StrategyConfig, Trigger};

/// 速度偏好策略。
#[derive(Debug, Default, Clone, Copy)]
pub struct Fast;

impl Strategy for Fast {
    fn id(&self) -> StrategyId {
        "fast".to_string()
    }

    fn priority(&self) -> u8 {
        // 比 Baseline(40) 强、比 Failover(50) 弱：先按速度重排，再由健康度过滤。
        45
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
        // 可选的样本门槛：默认 1（有任何观测就排序）。
        let min_samples = cfg
            .params
            .get("min_samples")
            .and_then(|value| value.as_u64())
            .unwrap_or(1);

        let mut scored: Vec<(f64, &tr_contracts::Candidate)> = view
            .candidates
            .iter()
            .filter(|candidate| candidate.enabled)
            .filter_map(|candidate| {
                let key = format!("perf.ttft_ms.{}", candidate.id);
                let samples_key = format!("perf.samples.{}", candidate.id);
                let samples = match view.signals.extras.get(&samples_key) {
                    Some(SignalValue::Int(samples)) => *samples as u64,
                    _ => u64::MAX,
                };
                if samples < min_samples {
                    return None;
                }
                match view.signals.extras.get(&key) {
                    Some(SignalValue::Float(ttft)) => Some((f64::from(*ttft), candidate)),
                    _ => None,
                }
            })
            .collect();

        if scored.len() < 2 {
            return PlanDraft::default();
        }
        scored.sort_by(|(left_ttft, left), (right_ttft, right)| {
            left_ttft
                .partial_cmp(right_ttft)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| left.id.cmp(&right.id))
        });

        let mut draft = PlanDraft::default();
        draft.contributions.push(PlanContribution::Prefer {
            order: scored
                .into_iter()
                .map(|(_, candidate)| candidate.id.clone())
                .collect(),
            reason: ReasonCode::new(ReasonCodeKind::FastPrefer),
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

    #[test]
    fn prefers_lower_ttft() {
        let candidates = vec![candidate("slow"), candidate("fast")];
        let mut signals = Signals::default();
        signals
            .extras
            .insert("perf.ttft_ms.slow".to_string(), SignalValue::Float(900.0));
        signals
            .extras
            .insert("perf.ttft_ms.fast".to_string(), SignalValue::Float(120.0));
        let canonical = CanonicalRequest::default();
        let selection = Selection::default();
        let virtual_model = VirtualModel::default();
        let health = CandidateHealth::default();
        let session = SessionFacts::default();
        let ledger = LedgerSummary::default();
        let profile = ProfileConfig::default();
        let view = RequestView {
            canonical: &canonical,
            signals: &signals,
            selection: &selection,
            virtual_model: &virtual_model,
            reason: tr_contracts::RequestReason::User,
            previous: None,
            failed: None,
            context_hold: None,
            router_state: None,
            candidates: &candidates,
            health: &health,
            session: &session,
            ledger: &ledger,
            profile: &profile,
            now_unix: 1,
        };
        let draft = Fast.draft(
            &view,
            &StrategyState::default(),
            &StrategyConfig::enabled("fast"),
        );
        let order = match &draft.contributions[0] {
            PlanContribution::Prefer { order, .. } => order.clone(),
            other => panic!("unexpected: {other:?}"),
        };
        assert_eq!(order, vec!["fast".to_string(), "slow".to_string()]);
    }

    #[test]
    fn needs_two_observations() {
        let candidates = vec![candidate("only")];
        let mut signals = Signals::default();
        signals
            .extras
            .insert("perf.ttft_ms.only".to_string(), SignalValue::Float(100.0));
        let canonical = CanonicalRequest::default();
        let selection = Selection::default();
        let virtual_model = VirtualModel::default();
        let health = CandidateHealth::default();
        let session = SessionFacts::default();
        let ledger = LedgerSummary::default();
        let profile = ProfileConfig::default();
        let view = RequestView {
            canonical: &canonical,
            signals: &signals,
            selection: &selection,
            virtual_model: &virtual_model,
            reason: tr_contracts::RequestReason::User,
            previous: None,
            failed: None,
            context_hold: None,
            router_state: None,
            candidates: &candidates,
            health: &health,
            session: &session,
            ledger: &ledger,
            profile: &profile,
            now_unix: 1,
        };
        let draft = Fast.draft(
            &view,
            &StrategyState::default(),
            &StrategyConfig::enabled("fast"),
        );
        assert!(draft.contributions.is_empty());
    }
}
