//! 成本闸模式（§9.4，P2）。
//!
//! - `off`：不计算；
//! - `shadow`（默认）：计算并记录反事实结论，**不拦截**；
//! - `active`：拒绝切换时把上一成功候选排回队首。

/// 成本闸模式。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CostGateMode {
    Off,
    #[default]
    Shadow,
    Active,
}

impl CostGateMode {
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "off" => Some(Self::Off),
            "shadow" => Some(Self::Shadow),
            "active" => Some(Self::Active),
            _ => None,
        }
    }
}

use tr_contracts::{
    Candidate, Cost, ProfileConfig, ReasonCode, ReasonCodeKind, UnknownPricePolicy,
};
use tr_core::cache_cost::{CacheInputs, expected_cache_loss};
use tr_core::gate::{GateDecision, GateInputs, SwitchDirection, decide};

/// 切换评估输入。
pub struct SwitchContext<'a> {
    pub previous: &'a Candidate,
    pub next: &'a Candidate,
    pub previous_cached_tokens: u64,
    pub prompt_tokens: u64,
    /// 同 provider + 同 model + 前缀匹配时才可复用缓存。
    pub same_prefix_reusable: bool,
    pub difficulty_gap: Option<f32>,
    pub hard_signal_score: Option<f32>,
    pub profile: &'a ProfileConfig,
    pub policy: UnknownPricePolicy,
}

/// 评估结论。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SwitchVerdict {
    /// 同强度/同候选：闸门不参与。
    Skip,
    Allow {
        expected_cache_loss: Option<Cost>,
        reasons: Vec<ReasonCode>,
    },
    Deny {
        expected_cache_loss: Option<Cost>,
        reasons: Vec<ReasonCode>,
    },
}

/// 纯函数：评估一次“从 previous 切到 next”的成本。
pub fn evaluate(context: &SwitchContext<'_>) -> SwitchVerdict {
    if context.previous.id == context.next.id {
        return SwitchVerdict::Skip;
    }
    let direction = match context.next.strength.cmp(&context.previous.strength) {
        std::cmp::Ordering::Greater => SwitchDirection::QualityUpgrade,
        std::cmp::Ordering::Less => SwitchDirection::CostDowngrade,
        std::cmp::Ordering::Equal => return SwitchVerdict::Skip,
    };

    let mut reasons = Vec::new();

    let expected_cache_loss = match (context.previous.price.as_ref(), context.next.price.as_ref()) {
        (Some(old_price), Some(new_price)) => {
            let loss = expected_cache_loss(&CacheInputs {
                previous_cached_tokens: context.previous_cached_tokens,
                prompt_tokens: context.prompt_tokens,
                old_price,
                new_price,
                same_prefix_reusable: context.same_prefix_reusable,
            });
            match loss {
                Some(loss) => {
                    reasons.extend(loss.assumptions);
                    Some(loss.cost)
                }
                None => None,
            }
        }
        // 缺价格：按 unknown 策略（由 gate::decide 处理 None）。
        _ => {
            reasons.push(ReasonCode::new(ReasonCodeKind::PriceUnknown));
            None
        }
    };

    let decision = decide(&GateInputs {
        direction,
        difficulty_gap: context.difficulty_gap,
        hard_signal_score: context.hard_signal_score,
        difficulty_weight_micros: context.profile.difficulty_weight_micros,
        hard_signal_weight_micros: context.profile.hard_signal_weight_micros,
        expected_cache_loss: expected_cache_loss.clone(),
        policy: context.policy,
    });

    match decision {
        GateDecision::Allow {
            reasons: gate_reasons,
        } => {
            reasons.extend(gate_reasons);
            SwitchVerdict::Allow {
                expected_cache_loss,
                reasons,
            }
        }
        GateDecision::Deny {
            reasons: gate_reasons,
        } => {
            reasons.extend(gate_reasons);
            SwitchVerdict::Deny {
                expected_cache_loss,
                reasons,
            }
        }
    }
}

/// 仅供测试：构造最小候选。
#[cfg(test)]
fn candidate(id: &str, strength: u8, price: Option<tr_contracts::PriceModel>) -> Candidate {
    Candidate {
        id: id.to_string(),
        tier: "cloud".to_string(),
        strength,
        upstream: tr_contracts::UpstreamRef {
            provider: tr_contracts::ProviderKind::OpenAiCompatible,
            base_url: "http://x".to_string(),
            model: format!("{id}-physical"),
            credential: None,
        },
        window: 128_000,
        capability: Default::default(),
        price,
        enabled: true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tr_contracts::PriceModel;

    fn profile() -> ProfileConfig {
        ProfileConfig {
            difficulty_weight_micros: 1000,
            hard_signal_weight_micros: 500,
            ..ProfileConfig::default()
        }
    }

    fn price(input: u64) -> PriceModel {
        PriceModel {
            currency: "USD".to_string(),
            input_micros_per_mtok: input,
            ..PriceModel::default()
        }
    }

    #[test]
    fn skip_same_candidate() {
        let weak = candidate("a", 10, Some(price(1_000_000)));
        let verdict = evaluate(&SwitchContext {
            previous: &weak,
            next: &weak,
            previous_cached_tokens: 0,
            prompt_tokens: 1_000,
            same_prefix_reusable: false,
            difficulty_gap: None,
            hard_signal_score: None,
            profile: &profile(),
            policy: UnknownPricePolicy::QualityAllow,
        });
        assert_eq!(verdict, SwitchVerdict::Skip);
    }

    #[test]
    fn cost_downgrade_without_price_is_denied() {
        let strong = candidate("strong", 80, None);
        let weak = candidate("weak", 10, None);
        let verdict = evaluate(&SwitchContext {
            previous: &strong,
            next: &weak,
            previous_cached_tokens: 0,
            prompt_tokens: 1_000,
            same_prefix_reusable: false,
            difficulty_gap: Some(0.0),
            hard_signal_score: Some(0.0),
            profile: &profile(),
            policy: UnknownPricePolicy::QualityAllow,
        });
        assert!(matches!(verdict, SwitchVerdict::Deny { .. }));
    }

    #[test]
    fn quality_upgrade_without_price_is_allowed() {
        let weak = candidate("weak", 10, None);
        let strong = candidate("strong", 80, None);
        let verdict = evaluate(&SwitchContext {
            previous: &weak,
            next: &strong,
            previous_cached_tokens: 0,
            prompt_tokens: 1_000,
            same_prefix_reusable: false,
            difficulty_gap: Some(1.0),
            hard_signal_score: Some(1.0),
            profile: &profile(),
            policy: UnknownPricePolicy::QualityAllow,
        });
        assert!(matches!(verdict, SwitchVerdict::Allow { .. }));
    }

    #[test]
    fn known_loss_blocks_weak_gain() {
        let weak = candidate("weak", 10, Some(price(100_000)));
        let strong = candidate("strong", 80, Some(price(1_000_000)));
        let verdict = evaluate(&SwitchContext {
            previous: &weak,
            next: &strong,
            previous_cached_tokens: 0,
            prompt_tokens: 1_000_000,
            same_prefix_reusable: false,
            difficulty_gap: Some(0.1),
            hard_signal_score: Some(0.1),
            profile: &profile(),
            policy: UnknownPricePolicy::QualityAllow,
        });
        match verdict {
            SwitchVerdict::Deny {
                expected_cache_loss,
                reasons,
            } => {
                assert!(expected_cache_loss.is_some());
                assert!(
                    reasons
                        .iter()
                        .any(|reason| reason.code == ReasonCodeKind::SwitchDeniedCacheCost)
                );
            }
            other => panic!("expected deny, got {other:?}"),
        }
    }
}
