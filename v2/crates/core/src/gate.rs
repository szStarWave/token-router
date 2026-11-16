//! 切换成本闸（§9.4）。
//!
//! 只回答一个问题：**为了这个切换，付出 `expected_cache_loss` 值不值？**
//!
//! 规则（写死，不可配置绕过）：
//! - 已知损失：`gain > expected_cache_loss` 才允许；
//! - 未知损失：质量升级按 `unknown_price_policy` 放行，成本降级一律拒绝；
//! - 缺失 gain 输入按 0 计算并记 `GAIN_INPUT_UNKNOWN`（不猜难度）。
//!
//! P2 先 shadow：记录反事实决策，不拦截；达标后由配置切到 active。

use tr_contracts::{Cost, ReasonCode, ReasonCodeKind, UnknownPricePolicy};

/// 切换方向。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SwitchDirection {
    /// 目标更强（质量兜底）。
    QualityUpgrade,
    /// 目标更便宜。
    CostDowngrade,
}

/// 闸门输入。
pub struct GateInputs {
    pub direction: SwitchDirection,
    pub difficulty_gap: Option<f32>,
    pub hard_signal_score: Option<f32>,
    pub difficulty_weight_micros: i64,
    pub hard_signal_weight_micros: i64,
    pub expected_cache_loss: Option<Cost>,
    pub policy: UnknownPricePolicy,
}

/// 闸门结论。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GateDecision {
    Allow { reasons: Vec<ReasonCode> },
    Deny { reasons: Vec<ReasonCode> },
}

impl GateDecision {
    pub fn is_allowed(&self) -> bool {
        matches!(self, Self::Allow { .. })
    }

    pub fn reasons(&self) -> &[ReasonCode] {
        match self {
            Self::Allow { reasons } | Self::Deny { reasons } => reasons,
        }
    }
}

/// 判定是否允许切换。
pub fn decide(inputs: &GateInputs) -> GateDecision {
    let mut reasons = Vec::new();

    let gain_micros = gain(inputs, &mut reasons);

    let loss_micros = match &inputs.expected_cache_loss {
        Some(cost) => cost.micros,
        None => {
            return match (inputs.policy, inputs.direction) {
                (UnknownPricePolicy::QualityAllow, SwitchDirection::QualityUpgrade) => {
                    // 质量兜底不能被缺价格卡住。
                    reasons.push(ReasonCode::new(
                        ReasonCodeKind::SwitchAllowedQualityPriceUnknown,
                    ));
                    GateDecision::Allow { reasons }
                }
                _ => {
                    reasons.push(ReasonCode::new(ReasonCodeKind::SwitchDeniedPriceUnknown));
                    GateDecision::Deny { reasons }
                }
            };
        }
    };

    if gain_micros > loss_micros {
        GateDecision::Allow { reasons }
    } else {
        reasons.push(ReasonCode::with_detail(
            ReasonCodeKind::SwitchDeniedCacheCost,
            format!("gain={gain_micros} loss={loss_micros}"),
        ));
        GateDecision::Deny { reasons }
    }
}

fn gain(inputs: &GateInputs, reasons: &mut Vec<ReasonCode>) -> i64 {
    if inputs.difficulty_gap.is_none() && inputs.hard_signal_score.is_none() {
        reasons.push(ReasonCode::new(ReasonCodeKind::GainInputUnknown));
    }
    let difficulty = inputs.difficulty_gap.unwrap_or(0.0) as f64;
    let hard = inputs.hard_signal_score.unwrap_or(0.0) as f64;
    (difficulty * inputs.difficulty_weight_micros as f64
        + hard * inputs.hard_signal_weight_micros as f64) as i64
}

#[cfg(test)]
mod tests {
    use super::*;

    fn inputs(direction: SwitchDirection, loss: Option<i64>) -> GateInputs {
        GateInputs {
            direction,
            difficulty_gap: Some(0.5),
            hard_signal_score: Some(0.5),
            difficulty_weight_micros: 1000,
            hard_signal_weight_micros: 500,
            expected_cache_loss: loss.map(|micros| Cost {
                micros,
                currency: "USD".to_string(),
            }),
            policy: UnknownPricePolicy::QualityAllow,
        }
    }

    #[test]
    fn allows_when_gain_beats_loss() {
        // gain = 0.5*1000 + 0.5*500 = 750
        let decision = decide(&inputs(SwitchDirection::QualityUpgrade, Some(500)));
        assert!(decision.is_allowed());
    }

    #[test]
    fn denies_when_loss_beats_gain() {
        let decision = decide(&inputs(SwitchDirection::QualityUpgrade, Some(1_000)));
        assert!(!decision.is_allowed());
        assert!(
            decision
                .reasons()
                .iter()
                .any(|reason| { reason.code == ReasonCodeKind::SwitchDeniedCacheCost })
        );
    }

    #[test]
    fn unknown_price_allows_quality_upgrade() {
        let decision = decide(&inputs(SwitchDirection::QualityUpgrade, None));
        assert!(decision.is_allowed());
        assert!(
            decision
                .reasons()
                .iter()
                .any(|reason| { reason.code == ReasonCodeKind::SwitchAllowedQualityPriceUnknown })
        );
    }

    #[test]
    fn unknown_price_denies_cost_downgrade() {
        let decision = decide(&inputs(SwitchDirection::CostDowngrade, None));
        assert!(!decision.is_allowed());
        assert!(
            decision
                .reasons()
                .iter()
                .any(|reason| reason.code == ReasonCodeKind::SwitchDeniedPriceUnknown)
        );
    }

    #[test]
    fn deny_all_policy_rejects_unknown_upgrade() {
        let mut inputs = inputs(SwitchDirection::QualityUpgrade, None);
        inputs.policy = UnknownPricePolicy::DenyAll;
        assert!(!decide(&inputs).is_allowed());
    }

    #[test]
    fn missing_gain_inputs_are_flagged() {
        let mut inputs = inputs(SwitchDirection::QualityUpgrade, Some(10));
        inputs.difficulty_gap = None;
        inputs.hard_signal_score = None;
        let decision = decide(&inputs);
        assert!(
            decision
                .reasons()
                .iter()
                .any(|reason| reason.code == ReasonCodeKind::GainInputUnknown)
        );
        assert!(!decision.is_allowed()); // gain = 0 <= loss
    }
}
