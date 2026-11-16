//! `Classify` 策略（§6.2、§6.3 第 3 步，P1-7）。
//!
//! 事前分类：在候选集内根据难度/硬信号选择目标强度。P1 只允许**升档到最强**；
//! 降档与阈值校准留给 P2（成本闸与评测）。
//!
//! `mode`：
//! - `off`（默认）：不参与；
//! - `shadow`：只写 `Note` 理由，不改变顺序（用于观察建议命中率）；
//! - `active`：把最强候选提到队首。

use serde_json::Value;
use tr_contracts::{
    PlanContribution, PlanDraft, ReasonCode, ReasonCodeKind, RequestView, StrategyId, StrategyState,
};

use crate::strategy::{Strategy, StrategyConfig, Trigger};

/// 事前分类策略。
#[derive(Debug, Default, Clone, Copy)]
pub struct Classify;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    Off,
    Shadow,
    Active,
}

struct Params {
    mode: Mode,
    hard_signal_threshold: f32,
}

impl Default for Params {
    fn default() -> Self {
        Self {
            mode: Mode::Off,
            hard_signal_threshold: 0.6,
        }
    }
}

impl Params {
    fn from_config(cfg: &StrategyConfig) -> Self {
        let defaults = Self::default();
        let Ok(object) =
            serde_json::from_value::<serde_json::Map<String, Value>>(cfg.params.clone())
        else {
            return defaults;
        };
        Self {
            mode: match object.get("mode").and_then(Value::as_str) {
                Some("active") => Mode::Active,
                Some("shadow") => Mode::Shadow,
                _ => Mode::Off,
            },
            hard_signal_threshold: object
                .get("hard_signal_threshold")
                .and_then(Value::as_f64)
                .map(|value| value as f32)
                .unwrap_or(defaults.hard_signal_threshold),
        }
    }
}

impl Strategy for Classify {
    fn id(&self) -> StrategyId {
        "classify".to_string()
    }

    fn priority(&self) -> u8 {
        30
    }

    fn triggers(&self, cfg: &StrategyConfig) -> Vec<Trigger> {
        if cfg.triggers.is_empty() {
            vec![Trigger::UserTurn, Trigger::NewTask, Trigger::Ambiguous]
        } else {
            cfg.triggers.clone()
        }
    }

    fn draft(&self, view: &RequestView, _state: &StrategyState, cfg: &StrategyConfig) -> PlanDraft {
        let params = Params::from_config(cfg);
        if params.mode == Mode::Off {
            return PlanDraft::default();
        }

        let Some(score) = view.signals.core.hard_signal_score else {
            return PlanDraft::default();
        };
        if score < params.hard_signal_threshold {
            return PlanDraft::default();
        }

        let Some(strongest) = view
            .candidates
            .iter()
            .filter(|candidate| candidate.enabled)
            .max_by_key(|candidate| (candidate.strength, candidate.id.clone()))
        else {
            return PlanDraft::default();
        };

        let reason = ReasonCode::with_detail(
            ReasonCodeKind::ClassifyPrefer,
            format!(
                "hard_signal_score={score:.2} >= {:.2}",
                params.hard_signal_threshold
            ),
        );

        let mut draft = PlanDraft::default();
        match params.mode {
            Mode::Shadow => draft.contributions.push(PlanContribution::Note {
                reason: ReasonCode::with_detail(
                    ReasonCodeKind::ClassifyPrefer,
                    format!("shadow: would prefer `{}`", strongest.id),
                ),
            }),
            Mode::Active => draft.contributions.push(PlanContribution::Prefer {
                order: vec![strongest.id.clone()],
                reason,
            }),
            Mode::Off => {}
        }
        draft
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tr_contracts::{
        Candidate, CandidateHealth, CanonicalRequest, CapabilityProfile, InputModality,
        LedgerSummary, PlanMode, ProfileConfig, RequestReason, Selection, SessionFacts, Signals,
        UpstreamRef, VirtualModel,
    };

    fn candidate(id: &str, strength: u8) -> Candidate {
        Candidate {
            id: id.to_string(),
            tier: "cloud".to_string(),
            strength,
            upstream: UpstreamRef {
                provider: tr_contracts::ProviderKind::OpenAiCompatible,
                base_url: "http://x".to_string(),
                model: id.to_string(),
                credential: None,
            },
            window: 128_000,
            capability: CapabilityProfile {
                input: vec![InputModality::Text],
                ..CapabilityProfile::default()
            },
            price: None,
            enabled: true,
        }
    }

    fn view(score: Option<f32>) -> RequestView<'static> {
        let canonical: &'static CanonicalRequest = Box::leak(Box::new(CanonicalRequest::default()));
        let mut signals = Signals::default();
        signals.core.hard_signal_score = score;
        let signals: &'static Signals = Box::leak(Box::new(signals));
        let selection: &'static Selection = Box::leak(Box::new(Selection::default()));
        let vm: &'static VirtualModel = Box::leak(Box::new(VirtualModel {
            id: "auto".to_string(),
            profile: "default".to_string(),
            ..VirtualModel::default()
        }));
        let candidates: &'static [Candidate] =
            Box::leak(vec![candidate("weak", 10), candidate("strong", 80)].into_boxed_slice());
        let health: &'static CandidateHealth = Box::leak(Box::new(CandidateHealth::default()));
        let session: &'static SessionFacts = Box::leak(Box::new(SessionFacts::default()));
        let ledger: &'static LedgerSummary = Box::leak(Box::new(LedgerSummary::default()));
        let profile: &'static ProfileConfig = Box::leak(Box::new(ProfileConfig {
            mode: PlanMode::Cascade,
            ..ProfileConfig::default()
        }));

        RequestView {
            canonical,
            signals,
            selection,
            virtual_model: vm,
            reason: RequestReason::User,
            previous: None,
            failed: None,
            context_hold: None,
            router_state: None,
            candidates,
            health,
            session,
            ledger,
            profile,
            now_unix: 0,
        }
    }

    fn cfg(mode: &str) -> StrategyConfig {
        let mut cfg = StrategyConfig::enabled("classify");
        cfg.params = serde_json::json!({"mode": mode, "hard_signal_threshold": 0.6});
        cfg
    }

    fn state() -> StrategyState {
        StrategyState::default()
    }

    #[test]
    fn off_mode_is_noop() {
        let draft = Classify.draft(&view(Some(0.9)), &state(), &cfg("off"));
        assert!(draft.contributions.is_empty());
    }

    #[test]
    fn below_threshold_is_noop() {
        let draft = Classify.draft(&view(Some(0.2)), &state(), &cfg("active"));
        assert!(draft.contributions.is_empty());
    }

    #[test]
    fn active_prefers_strongest() {
        let draft = Classify.draft(&view(Some(0.9)), &state(), &cfg("active"));
        match &draft.contributions[0] {
            PlanContribution::Prefer { order, .. } => {
                assert_eq!(order, &vec!["strong".to_string()])
            }
            other => panic!("expected Prefer, got {other:?}"),
        }
    }

    #[test]
    fn shadow_only_notes() {
        let draft = Classify.draft(&view(Some(0.9)), &state(), &cfg("shadow"));
        assert!(matches!(
            draft.contributions[0],
            PlanContribution::Note { .. }
        ));
    }

    #[test]
    fn missing_score_is_noop() {
        let draft = Classify.draft(&view(None), &state(), &cfg("active"));
        assert!(draft.contributions.is_empty());
    }
}
