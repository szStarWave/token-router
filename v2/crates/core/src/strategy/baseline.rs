//! `Baseline` 策略（§6.2）：生成默认偏好顺序。
//!
//! 它不做“聪明”的事：只按 profile 的档位顺序 + 候选 `strength` 排序，
//! 把不合格候选过滤掉。升级/降级由 `Escalation` / `Classify` 叠在上面。

use std::collections::BTreeSet;

use tr_contracts::{
    Candidate, ContentBlock, InputModality, PlanContribution, PlanDraft, ReasonCode,
    ReasonCodeKind, RequestView, StrategyId, StrategyState,
};

use crate::signals::estimate_input_tokens;
use crate::strategy::{Strategy, StrategyConfig, tier_rank};

/// 默认排序策略。
#[derive(Debug, Default, Clone, Copy)]
pub struct Baseline;

impl Strategy for Baseline {
    fn id(&self) -> StrategyId {
        "baseline".to_string()
    }

    fn priority(&self) -> u8 {
        40
    }

    fn triggers(&self, cfg: &StrategyConfig) -> Vec<crate::strategy::Trigger> {
        if cfg.triggers.is_empty() {
            vec![
                crate::strategy::Trigger::UserTurn,
                crate::strategy::Trigger::Continuation,
                crate::strategy::Trigger::Retry,
                crate::strategy::Trigger::Direct,
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
        let estimated = estimate_input_tokens(view.canonical);
        let modalities = required_modalities(view.canonical);

        let mut input_unsupported = BTreeSet::new();
        let mut window_filtered = BTreeSet::new();
        let mut eligible: Vec<&Candidate> = Vec::new();

        for candidate in view.candidates {
            if !candidate.enabled {
                // 运维显式关闭，不需要理由码；不进 preference 即可。
                continue;
            }
            if modalities
                .iter()
                .any(|modality| !supports_modality(candidate, *modality))
            {
                input_unsupported.insert(candidate.id.clone());
                continue;
            }
            if candidate.window < estimated {
                window_filtered.insert(candidate.id.clone());
                continue;
            }
            eligible.push(candidate);
        }

        eligible.sort_by_key(|candidate| {
            (
                tier_rank(view.profile, &candidate.tier),
                candidate.strength,
                candidate.id.clone(),
            )
        });

        let mut draft = PlanDraft::default();

        if !input_unsupported.is_empty() {
            draft.contributions.push(PlanContribution::Filter {
                exclude: input_unsupported,
                reason: ReasonCode::new(ReasonCodeKind::ModelInputUnsupported),
            });
        }
        if !window_filtered.is_empty() {
            draft.contributions.push(PlanContribution::Filter {
                exclude: window_filtered,
                reason: ReasonCode::with_detail(
                    ReasonCodeKind::PlanFilterWindow,
                    format!("estimated_input_tokens={estimated}"),
                ),
            });
        }

        if eligible.is_empty() {
            draft.contributions.push(PlanContribution::Filter {
                exclude: BTreeSet::new(),
                reason: ReasonCode::new(ReasonCodeKind::NoEligibleCandidate),
            });
        } else {
            draft.contributions.push(PlanContribution::Prefer {
                order: eligible
                    .iter()
                    .map(|candidate| candidate.id.clone())
                    .collect(),
                reason: ReasonCode::with_detail(
                    ReasonCodeKind::BaselineOrder,
                    format!("estimated_input_tokens={estimated}"),
                ),
            });
        }

        draft.contributions.push(PlanContribution::Mode {
            mode: view.profile.mode,
            reason: ReasonCode::with_detail(ReasonCodeKind::BaselineOrder, "profile mode"),
        });

        draft
    }
}

/// 请求需要的输入模态。文本视为所有候选都支持；图片/音频必须显式声明。
pub(crate) fn required_modalities(request: &tr_contracts::CanonicalRequest) -> Vec<InputModality> {
    let mut modalities = vec![InputModality::Text];
    let has = |wanted: InputModality| {
        request.messages.iter().any(|message| {
            message.blocks.iter().any(|block| {
                matches!(
                    (wanted, block),
                    (InputModality::Image, ContentBlock::Image { .. })
                        | (InputModality::Audio, ContentBlock::Audio { .. })
                )
            })
        })
    };
    if has(InputModality::Image) {
        modalities.push(InputModality::Image);
    }
    if has(InputModality::Audio) {
        modalities.push(InputModality::Audio);
    }
    modalities
}

fn supports_modality(candidate: &Candidate, modality: InputModality) -> bool {
    match modality {
        InputModality::Text => true,
        other => candidate.supports_input(other),
    }
}
