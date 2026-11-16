//! `Escalation` 棘轮（§6.4、P1）。
//!
//! 状态机：`Unlatched → Confirming → Latched → ReleasePending → Unlatched`。
//! 关键纪律：
//! - `Latched` 只在 `ReleaseCheck` trigger 调判官；其他轮次直达强档；
//! - **新证据**由调用点指纹决定（step_kind + streak + loop + turn），不信任模型自报；
//! - 判官失败/超时 = 无判定：`Confirming` 保持原状，`ReleasePending` 保持 latch；
//! - `strong_max_calls` 强制回落，`weak_cooldown_calls` 避免释放后立刻再升。

use serde::{Deserialize, Serialize};
use tr_contracts::{
    Candidate, Category, Choices, EvidenceRef, JudgeRequest, PlanContribution, PlanDraft, Question,
    ReasonCode, ReasonCodeKind, RequestView, StateUpdate, StateUpdateOrigin, StrategyId,
    StrategyState, Verdict,
};

use crate::strategy::{Strategy, StrategyConfig, Trigger};

const CHECK_REQUEST_ID: &str = "escalation.check";
const RELEASE_REQUEST_ID: &str = "escalation.release";

/// 棘轮策略。
#[derive(Debug, Default, Clone, Copy)]
pub struct Escalation;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
enum Phase {
    #[default]
    Unlatched,
    Confirming,
    Latched,
    ReleasePending,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
struct EscalationState {
    phase: Phase,
    streak: u32,
    category: Option<Category>,
    latch_category: Option<Category>,
    strong_calls: u32,
    seen_evidence: Vec<String>,
    /// 释放后的冷却轮数。
    cooldown_calls: u32,
}

impl EscalationState {
    fn parse(state: &StrategyState) -> Self {
        serde_json::from_value(state.blob.clone()).unwrap_or_default()
    }

    fn to_update(&self, _view: &RequestView) -> StateUpdate {
        StateUpdate {
            strategy_id: "escalation".to_string(),
            task_id: String::new(),
            origin: StateUpdateOrigin::Draft,
            patch: serde_json::to_value(self).unwrap_or(serde_json::Value::Null),
        }
    }

    fn remember(&mut self, fingerprint: &str) {
        if !self.seen_evidence.iter().any(|seen| seen == fingerprint) {
            self.seen_evidence.push(fingerprint.to_string());
            // 有界：只保留最近 16 条，避免状态无限增长。
            if self.seen_evidence.len() > 16 {
                self.seen_evidence.remove(0);
            }
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct Params {
    confirmations: u32,
    strong_min_calls: u32,
    strong_max_calls: u32,
    weak_cooldown_calls: u32,
    min_confidence: f32,
}

impl Default for Params {
    fn default() -> Self {
        let strong_min_calls = 3;
        Self {
            confirmations: 2,
            strong_min_calls,
            strong_max_calls: strong_min_calls * 4,
            weak_cooldown_calls: 1,
            min_confidence: 0.6,
        }
    }
}

impl Params {
    fn from_config(cfg: &StrategyConfig) -> Self {
        let defaults = Self::default();
        let Ok(object) = serde_json::from_value::<serde_json::Map<String, serde_json::Value>>(
            cfg.params.clone(),
        ) else {
            return defaults;
        };
        let read = |key: &str| object.get(key).and_then(|value| value.as_u64());
        let strong_min_calls = read("strong_min_calls")
            .map(|value| value as u32)
            .unwrap_or(defaults.strong_min_calls);
        Self {
            confirmations: read("confirmations")
                .map(|value| value as u32)
                .unwrap_or(defaults.confirmations),
            strong_min_calls,
            strong_max_calls: read("strong_max_calls")
                .map(|value| value as u32)
                .unwrap_or(strong_min_calls * 4),
            weak_cooldown_calls: read("weak_cooldown_calls")
                .map(|value| value as u32)
                .unwrap_or(defaults.weak_cooldown_calls),
            min_confidence: object
                .get("min_confidence")
                .and_then(|value| value.as_f64())
                .map(|value| value as f32)
                .unwrap_or(defaults.min_confidence),
        }
    }
}

impl Strategy for Escalation {
    fn id(&self) -> StrategyId {
        "escalation".to_string()
    }

    fn priority(&self) -> u8 {
        10
    }

    fn triggers(&self, cfg: &StrategyConfig) -> Vec<Trigger> {
        if cfg.triggers.is_empty() {
            vec![
                Trigger::HardSignal,
                Trigger::Failure,
                Trigger::ReleaseCheck,
                Trigger::UserTurn,
            ]
        } else {
            cfg.triggers.clone()
        }
    }

    fn draft(&self, view: &RequestView, state: &StrategyState, cfg: &StrategyConfig) -> PlanDraft {
        let params = Params::from_config(cfg);
        let mut current = EscalationState::parse(state);
        let mut draft = PlanDraft::default();

        // 冷却：释放后若干轮不判官、不升级。
        if current.phase == Phase::Unlatched && current.cooldown_calls > 0 {
            current.cooldown_calls -= 1;
            draft
                .contributions
                .push(note(ReasonCodeKind::EscalationCooldown));
            draft.state_updates.push(current.to_update(view));
            return draft;
        }

        match current.phase {
            Phase::Latched | Phase::ReleasePending => {
                current.strong_calls = current.strong_calls.saturating_add(1);

                if current.strong_calls >= params.strong_max_calls {
                    // 硬上限：强制回落 + 冷却。
                    current.phase = Phase::Unlatched;
                    current.streak = 0;
                    current.category = None;
                    current.latch_category = None;
                    current.strong_calls = 0;
                    current.cooldown_calls = params.weak_cooldown_calls;
                    draft
                        .contributions
                        .push(note(ReasonCodeKind::EscalationReleased));
                    draft.state_updates.push(current.to_update(view));
                    return draft;
                }

                if current.strong_calls >= params.strong_min_calls
                    && current.phase == Phase::Latched
                {
                    current.phase = Phase::ReleasePending;
                    draft.judge_requests.push(release_request(&params));
                    draft.state_updates.push(current.to_update(view));
                    pin_strongest(view, &mut draft);
                    return draft;
                }

                if current.phase == Phase::ReleasePending {
                    // 上一轮 ReleaseCheck 没有返回结论（超时/off）：保持 latch。
                    draft.judge_requests.push(release_request(&params));
                    draft.state_updates.push(current.to_update(view));
                    pin_strongest(view, &mut draft);
                    return draft;
                }

                pin_strongest(view, &mut draft);
                draft
                    .contributions
                    .push(note(ReasonCodeKind::EscalationLatched));
                draft.state_updates.push(current.to_update(view));
                draft
            }
            Phase::Unlatched | Phase::Confirming => {
                let Some(category) = failure_category(view) else {
                    return draft;
                };
                draft
                    .judge_requests
                    .push(check_request(view, category, &params));
                draft.state_updates.push(current.to_update(view));
                draft
            }
        }
    }

    fn refine(
        &self,
        view: &RequestView,
        state: &StrategyState,
        verdicts: &[Verdict],
        cfg: &StrategyConfig,
    ) -> PlanDraft {
        let params = Params::from_config(cfg);
        let mut current = EscalationState::parse(state);
        let mut draft = PlanDraft::default();

        match current.phase {
            Phase::ReleasePending => {
                match find_verdict(verdicts, RELEASE_REQUEST_ID) {
                    Some(Verdict::Decline { .. }) => {
                        current.phase = Phase::Unlatched;
                        current.streak = 0;
                        current.category = None;
                        current.latch_category = None;
                        current.strong_calls = 0;
                        current.cooldown_calls = params.weak_cooldown_calls;
                        draft
                            .contributions
                            .push(note(ReasonCodeKind::EscalationReleased));
                    }
                    // Escalate / Uncertain / 无判定：保守保持 latch。
                    _ => {
                        current.phase = Phase::Latched;
                        pin_strongest(view, &mut draft);
                        draft
                            .contributions
                            .push(note(ReasonCodeKind::EscalationLatched));
                    }
                }
                draft.state_updates.push(current.to_update(view));
                draft
            }
            Phase::Latched => {
                pin_strongest(view, &mut draft);
                draft.state_updates.push(current.to_update(view));
                draft
            }
            Phase::Unlatched | Phase::Confirming => {
                let Some(verdict) = find_verdict(verdicts, CHECK_REQUEST_ID) else {
                    return draft;
                };

                match verdict {
                    Verdict::Escalate {
                        category, evidence, ..
                    } => {
                        if current.category.as_ref() != Some(category) {
                            current.category = Some(category.clone());
                            current.streak = 0;
                        }
                        let is_new = !current
                            .seen_evidence
                            .iter()
                            .any(|seen| seen == &evidence.fingerprint);
                        if !is_new {
                            // 同一证据重复上报不算确认。
                            draft.state_updates.push(current.to_update(view));
                            return draft;
                        }
                        current.remember(&evidence.fingerprint);
                        current.streak += 1;

                        if current.streak >= params.confirmations {
                            current.phase = Phase::Latched;
                            current.latch_category = Some(category.clone());
                            current.strong_calls = 0;
                            current.cooldown_calls = 0;
                            pin_strongest(view, &mut draft);
                            draft
                                .contributions
                                .push(note(ReasonCodeKind::EscalationLatched));
                        } else {
                            current.phase = Phase::Confirming;
                            draft
                                .contributions
                                .push(note(ReasonCodeKind::EscalationConfirming));
                        }
                    }
                    Verdict::Decline { .. } | Verdict::Uncertain { .. } => {
                        current = EscalationState {
                            cooldown_calls: current.cooldown_calls,
                            ..EscalationState::default()
                        };
                    }
                }

                draft.state_updates.push(current.to_update(view));
                draft
            }
        }
    }
}

fn note(kind: ReasonCodeKind) -> PlanContribution {
    PlanContribution::Note {
        reason: ReasonCode::new(kind),
    }
}

fn check_request(view: &RequestView, category: Category, params: &Params) -> JudgeRequest {
    JudgeRequest {
        id: CHECK_REQUEST_ID.to_string(),
        question: Question {
            prompt: format!(
                "Current failure category is {category:?}. Should routing escalate to a stronger model \
for the next attempt? Answer true only if the current model is unlikely to recover."
            ),
            choices: Choices::Boolean,
            min_confidence: params.min_confidence,
        },
        required: false,
        priority: 10,
        category: category.clone(),
        evidence: evidence_for(view, &category),
    }
}

fn release_request(params: &Params) -> JudgeRequest {
    JudgeRequest {
        id: RELEASE_REQUEST_ID.to_string(),
        question: Question {
            prompt:
                "The strong model is currently serving. Has the original failure category stopped \
occurring, so routing can release back to the cheaper model?"
                    .to_string(),
            choices: Choices::Boolean,
            min_confidence: params.min_confidence,
        },
        required: false,
        priority: 5,
        category: Category::QualityUncertain,
        evidence: EvidenceRef {
            fingerprint: "release-check".to_string(),
            summary: "phase-aware release check".to_string(),
        },
    }
}

fn failure_category(view: &RequestView) -> Option<Category> {
    let core = &view.signals.core;
    if core.ctx_fill_ratio >= 0.9 {
        return Some(Category::ContextOverflow);
    }
    if core.consecutive_tool_error_streak > 0 || core.repeated_tool_loop.is_some() {
        return Some(Category::ToolFailure);
    }
    if core.assistant_failed_recent {
        return Some(Category::QualityUncertain);
    }
    None
}

fn evidence_for(view: &RequestView, category: &Category) -> EvidenceRef {
    let core = &view.signals.core;
    let turn = view.canonical.messages.len();
    EvidenceRef {
        fingerprint: format!(
            "{category:?}|streak={}|loop={}|turn={turn}",
            core.consecutive_tool_error_streak,
            core.repeated_tool_loop.is_some(),
        ),
        summary: format!(
            "step={:?} streak={} loop={} ctx={:.2}",
            core.step_kind,
            core.consecutive_tool_error_streak,
            core.repeated_tool_loop.is_some(),
            core.ctx_fill_ratio
        ),
    }
}

fn pin_strongest(view: &RequestView, draft: &mut PlanDraft) {
    if let Some(candidate) = strongest(view) {
        draft.contributions.push(PlanContribution::Pin {
            candidate: candidate.id.clone(),
            reason: ReasonCode::new(ReasonCodeKind::EscalationLatched),
        });
    } else {
        draft
            .contributions
            .push(note(ReasonCodeKind::LatchTargetUnavailable));
    }
}

fn strongest<'a>(view: &RequestView<'a>) -> Option<&'a Candidate> {
    view.candidates
        .iter()
        .filter(|candidate| candidate.enabled)
        .max_by_key(|candidate| (candidate.strength, candidate.id.clone()))
}

fn find_verdict<'a>(verdicts: &'a [Verdict], _request_id: &str) -> Option<&'a Verdict> {
    // P1：每个策略最多一个问题，agent 按请求顺序回填，这里取第一条即可。
    verdicts.first()
}

/// 供 `agent` 判断是否需要释放检查（诊断/测试用）。
pub fn is_release_phase(state: &StrategyState) -> bool {
    EscalationState::parse(state).phase == Phase::ReleasePending
}

#[cfg(test)]
mod tests {
    use super::*;
    use tr_contracts::{
        Candidate, CandidateHealth, CanonicalMessage, CanonicalRequest, CanonicalRole,
        CapabilityProfile, ContentBlock, DeclineReason, InputModality, LedgerSummary, PlanMode,
        ProfileConfig, ProviderKind, RequestReason, Selection, SessionFacts, Signals, UpstreamRef,
        VirtualModel,
    };

    fn candidate(id: &str, strength: u8) -> Candidate {
        Candidate {
            id: id.to_string(),
            tier: if strength > 50 { "cloud" } else { "local" }.to_string(),
            strength,
            upstream: UpstreamRef {
                provider: ProviderKind::OpenAiCompatible,
                base_url: "http://x".to_string(),
                model: format!("{id}-physical"),
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

    fn view(signals: Signals) -> RequestView<'static> {
        // 测试里用泄漏的静态数据，避免写成巨型结构体；仅测试使用。
        let canonical: &'static CanonicalRequest = Box::leak(Box::new(CanonicalRequest {
            messages: vec![CanonicalMessage {
                role: CanonicalRole::User,
                blocks: vec![ContentBlock::Text {
                    text: "hi".to_string(),
                    cache_control: None,
                }],
                ..CanonicalMessage::default()
            }],
            ..CanonicalRequest::default()
        }));
        let signals: &'static Signals = Box::leak(Box::new(signals));
        let selection: &'static Selection = Box::leak(Box::new(Selection {
            virtual_model: "auto".to_string(),
            thinking_level: "off".to_string(),
        }));
        let vm: &'static VirtualModel = Box::leak(Box::new(VirtualModel {
            id: "auto".to_string(),
            profile: "default".to_string(),
            candidate_pool: vec!["weak".to_string(), "strong".to_string()],
            ..VirtualModel::default()
        }));
        let candidates: &'static [Candidate] =
            Box::leak(vec![candidate("weak", 10), candidate("strong", 80)].into_boxed_slice());
        let health: &'static CandidateHealth = Box::leak(Box::new(CandidateHealth::default()));
        let session: &'static SessionFacts = Box::leak(Box::new(SessionFacts::default()));
        let ledger: &'static LedgerSummary = Box::leak(Box::new(LedgerSummary::default()));
        let profile: &'static ProfileConfig = Box::leak(Box::new(ProfileConfig {
            id: "default".to_string(),
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

    fn failing_signals(streak: u32) -> Signals {
        let mut signals = Signals::default();
        signals.core.consecutive_tool_error_streak = streak;
        signals
    }

    fn escalate_verdict(fingerprint: &str, streak: u32) -> Verdict {
        Verdict::Escalate {
            category: Category::ToolFailure,
            target_strength: None,
            evidence: EvidenceRef {
                fingerprint: fingerprint.to_string(),
                summary: format!("streak={streak}"),
            },
            confidence: 0.9,
        }
    }

    fn cfg() -> StrategyConfig {
        StrategyConfig::enabled("escalation")
    }

    #[test]
    fn two_new_evidences_latch() {
        let mut state = StrategyState::default();
        let view1 = view(failing_signals(1));

        let draft = Escalation.draft(&view1, &state, &cfg());
        assert_eq!(draft.judge_requests.len(), 1);

        let refine = Escalation.refine(&view1, &state, &[escalate_verdict("fp1", 1)], &cfg());
        apply(&mut state, &refine.state_updates);

        let view2 = view(failing_signals(2));
        let refine2 = Escalation.refine(&view2, &state, &[escalate_verdict("fp2", 2)], &cfg());
        apply(&mut state, &refine2.state_updates);

        let parsed = EscalationState::parse(&state);
        assert_eq!(parsed.phase, Phase::Latched);
        assert!(refine2.contributions.iter().any(|contribution| matches!(
            contribution,
            PlanContribution::Pin { candidate, .. } if candidate == "strong"
        )));
    }

    #[test]
    fn repeated_same_evidence_does_not_confirm() {
        let mut state = StrategyState::default();
        let view = view(failing_signals(1));

        let refine = Escalation.refine(&view, &state, &[escalate_verdict("same", 1)], &cfg());
        apply(&mut state, &refine.state_updates);
        // 第二轮判官仍报同一指纹：streak 不增加。
        let refine = Escalation.refine(&view, &state, &[escalate_verdict("same", 1)], &cfg());
        apply(&mut state, &refine.state_updates);
        assert_eq!(EscalationState::parse(&state).streak, 1);
    }

    #[test]
    fn decline_resets_streak() {
        let mut state = StrategyState::default();
        let view = view(failing_signals(1));

        let refine = Escalation.refine(&view, &state, &[escalate_verdict("fp1", 1)], &cfg());
        apply(&mut state, &refine.state_updates);

        let refine = Escalation.refine(
            &view,
            &state,
            &[Verdict::Decline {
                reason: DeclineReason::Sufficient,
            }],
            &cfg(),
        );
        apply(&mut state, &refine.state_updates);
        let parsed = EscalationState::parse(&state);
        assert_eq!(parsed.phase, Phase::Unlatched);
        assert_eq!(parsed.streak, 0);
    }

    #[test]
    fn release_after_min_calls_and_decline() {
        let mut state = StrategyState {
            blob: serde_json::json!({
                "phase": "latched",
                "streak": 2,
                "category": "tool_failure",
                "latch_category": "tool_failure",
                "strong_calls": 2,
                "seen_evidence": [],
                "cooldown_calls": 0
            }),
            ..StrategyState::default()
        };
        let view = view(Signals::default());

        // 第 3 次强档请求 → ReleasePending + 询问判官。
        let draft = Escalation.draft(&view, &state, &cfg());
        apply(&mut state, &draft.state_updates);
        assert_eq!(EscalationState::parse(&state).phase, Phase::ReleasePending);
        assert_eq!(draft.judge_requests[0].id, RELEASE_REQUEST_ID);

        // 判官 decline → 释放 + 冷却。
        let refine = Escalation.refine(
            &view,
            &state,
            &[Verdict::Decline {
                reason: DeclineReason::Sufficient,
            }],
            &cfg(),
        );
        apply(&mut state, &refine.state_updates);
        let parsed = EscalationState::parse(&state);
        assert_eq!(parsed.phase, Phase::Unlatched);
        assert_eq!(parsed.cooldown_calls, 1);
    }

    #[test]
    fn release_pending_without_verdict_stays_latched() {
        let mut state = StrategyState {
            blob: serde_json::json!({"phase": "release_pending", "strong_calls": 4}),
            ..StrategyState::default()
        };
        let view = view(Signals::default());

        let refine = Escalation.refine(&view, &state, &[], &cfg());
        apply(&mut state, &refine.state_updates);
        assert_eq!(EscalationState::parse(&state).phase, Phase::Latched);
    }

    #[test]
    fn hard_cap_forces_release() {
        let mut state = StrategyState {
            blob: serde_json::json!({
                "phase": "latched",
                "strong_calls": 11,
                "seen_evidence": [],
                "cooldown_calls": 0
            }),
            ..StrategyState::default()
        };
        let view = view(Signals::default());
        let draft = Escalation.draft(&view, &state, &cfg());
        apply(&mut state, &draft.state_updates);
        let parsed = EscalationState::parse(&state);
        assert_eq!(parsed.phase, Phase::Unlatched);
        assert_eq!(parsed.cooldown_calls, 1);
        assert!(draft.judge_requests.is_empty());
    }

    fn apply(state: &mut StrategyState, updates: &[StateUpdate]) {
        for update in updates {
            state.blob = update.patch.clone();
        }
    }
}
