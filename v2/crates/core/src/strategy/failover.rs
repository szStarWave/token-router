//! `Failover` 策略（§6.2、§6.3 第 6 步）：只读健康度，做过滤与接力。
//!
//! 它**不参与偏好排序**，也不与 `Escalation` 竞争：否则“健康度排序”和
//! “质量排序”会互相覆盖。接力顺序由 `merge` 从最终 `preference` 派生。

use std::collections::BTreeSet;

use tr_contracts::{
    PlanContribution, PlanDraft, ReasonCode, ReasonCodeKind, RequestView, StrategyId, StrategyState,
};

use crate::strategy::{Strategy, StrategyConfig, Trigger};

/// 健康度过滤策略。
#[derive(Debug, Default, Clone, Copy)]
pub struct Failover;

impl Strategy for Failover {
    fn id(&self) -> StrategyId {
        "failover".to_string()
    }

    fn priority(&self) -> u8 {
        50
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

    fn draft(
        &self,
        view: &RequestView,
        _state: &StrategyState,
        _cfg: &StrategyConfig,
    ) -> PlanDraft {
        let cooling_down: BTreeSet<_> = view
            .candidates
            .iter()
            .filter(|candidate| !view.health.can_attempt(&candidate.id, view.now_unix))
            .map(|candidate| candidate.id.clone())
            .collect();

        let mut draft = PlanDraft::default();
        if !cooling_down.is_empty() {
            draft.contributions.push(PlanContribution::Filter {
                exclude: cooling_down,
                reason: ReasonCode::new(ReasonCodeKind::CandidatesCoolingDown),
            });
        }
        draft
    }
}
