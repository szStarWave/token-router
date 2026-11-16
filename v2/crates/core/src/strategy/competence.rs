//! `CompetenceGuard`：上一阶段胜任度低于 floor 时，下一轮强制强档（P2-6，可选）。
//!
//! 来源（HiRoute 胜任度保护）：每个阶段有一个 0..1 的 competence score；
//! 分数低于 floor，且该问题仍适用于当前任务时，下一阶段直接使用强档。
//!
//! v2 的边界：
//! - **不是新判官通道**：它复用 `DecisionModel`，把问题定义为
//!   `Choices::ScoreBelowFloor`（低分 → Escalate），只在判官模式允许时被咨询；
//! - **任务变化时抑制**：任务指纹 = 首条 system/首条 user/工具名；指纹变化或
//!   候选组变化时清空旧分数，不升级；
//! - 合并优先级固定为 `Escalation(10) > CompetenceGuard(20) > Classify(30) > Baseline(40)`；
//! - 默认不启用：只有虚拟模型的 `strategies` 列表显式包含 `competence_guard` 时才运行。

use serde::{Deserialize, Serialize};
use tr_contracts::{
    Category, Choices, EvidenceRef, JudgeRequest, PlanContribution, PlanDraft, Question,
    ReasonCode, ReasonCodeKind, RequestView, StateUpdate, StateUpdateOrigin, StrategyId,
    StrategyState, Verdict,
};

use super::{Strategy, StrategyConfig, Trigger};

/// 问题 id；`refine` 用证据指纹（而不是 id）认领自己的 verdict。
const REQUEST_ID: &str = "competence_guard.check";
/// 证据指纹前缀：避免与 Escalation 的工具失败指纹混淆。
const EVIDENCE_PREFIX: &str = "competence:";
/// 默认 floor：HiRoute 默认 0.5。
const DEFAULT_FLOOR: f32 = 0.5;

/// 胜任度保护。
#[derive(Debug, Default, Clone, Copy)]
pub struct CompetenceGuard;

/// 持久化状态（JSON blob）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(default)]
struct CompetenceState {
    /// 上一阶段分数；`None` 表示未评分/已清空。
    score: Option<f32>,
    /// 任务指纹。
    task: Option<String>,
    /// 候选组 + profile 指纹（近似 plan 版本）。
    group: Option<String>,
}

impl CompetenceState {
    fn parse(state: &StrategyState) -> Self {
        serde_json::from_value(state.blob.clone()).unwrap_or_default()
    }

    fn to_update(&self) -> StateUpdate {
        StateUpdate {
            strategy_id: "competence_guard".to_string(),
            task_id: String::new(),
            origin: StateUpdateOrigin::Draft,
            patch: serde_json::to_value(self).unwrap_or(serde_json::Value::Null),
        }
    }

    fn has_any(&self) -> bool {
        self.score.is_some() && self.task.is_some()
    }
}

#[derive(Debug, Clone, Copy)]
struct Params {
    floor: f32,
}

impl Params {
    fn from_config(cfg: &StrategyConfig) -> Self {
        let floor = cfg
            .params
            .get("floor")
            .and_then(serde_json::Value::as_f64)
            .map(|value| value.clamp(0.0, 1.0) as f32)
            .unwrap_or(DEFAULT_FLOOR);
        Self { floor }
    }
}

impl Strategy for CompetenceGuard {
    fn id(&self) -> StrategyId {
        "competence_guard".to_string()
    }

    fn priority(&self) -> u8 {
        // Escalation(10) > CompetenceGuard(20) > Classify(30) > Baseline(40)
        20
    }

    fn triggers(&self, cfg: &StrategyConfig) -> Vec<Trigger> {
        if cfg.triggers.is_empty() {
            vec![Trigger::Continuation, Trigger::Retry, Trigger::HardSignal]
        } else {
            cfg.triggers.clone()
        }
    }

    fn draft(&self, view: &RequestView, state: &StrategyState, cfg: &StrategyConfig) -> PlanDraft {
        let mut draft = PlanDraft::default();
        if !cfg.enabled {
            return draft;
        }
        let params = Params::from_config(cfg);
        let current = CompetenceState::parse(state);
        let task = task_fingerprint(view);
        let group = group_fingerprint(view);

        // 任务/候选组变化：旧分数不再适用，清空并明确记录（§13 P2 验收⑤）。
        let task_changed = current.task.as_deref().is_some_and(|old| old != task);
        let group_changed = current.group.as_deref().is_some_and(|old| old != group);
        if task_changed || group_changed {
            draft
                .contributions
                .push(note(ReasonCodeKind::CompetenceTaskChanged));
            if current.has_any() {
                draft
                    .state_updates
                    .push(CompetenceState::default().to_update());
            }
            return draft;
        }

        if let Some(score) = current.score
            && score < params.floor
        {
            pin_strongest(
                view,
                &mut draft,
                ReasonCode::with_detail(
                    ReasonCodeKind::CompetenceProtected,
                    format!("score={score:.2} floor={:.2}", params.floor),
                ),
            );
            // 已有低分保护时不重复咨询判官，避免每轮消耗预算。
            return draft;
        }

        // 只对“有上一阶段”的请求评分；新任务首轮没有可评分的上一阶段。
        if view.previous.is_some() {
            draft
                .judge_requests
                .push(competence_request(&task, params.floor));
        }
        draft
    }

    fn refine(
        &self,
        view: &RequestView,
        _state: &StrategyState,
        verdicts: &[Verdict],
        cfg: &StrategyConfig,
    ) -> PlanDraft {
        let mut draft = PlanDraft::default();
        if !cfg.enabled {
            return draft;
        }
        let params = Params::from_config(cfg);
        let task = task_fingerprint(view);
        let group = group_fingerprint(view);
        let fingerprint = format!("{EVIDENCE_PREFIX}{task}");

        // 只认领带自己证据指纹的 Escalate；Escalation 的 verdict 不会误命中。
        let low_score = verdicts.iter().any(|verdict| {
            matches!(
                verdict,
                Verdict::Escalate { evidence, .. } if evidence.fingerprint == fingerprint
            )
        });
        if !low_score {
            return draft;
        }

        pin_strongest(
            view,
            &mut draft,
            ReasonCode::with_detail(
                ReasonCodeKind::CompetenceProtected,
                format!("judge score < floor {:.2}", params.floor),
            ),
        );
        draft.state_updates.push(
            CompetenceState {
                score: Some(0.0),
                task: Some(task),
                group: Some(group),
            }
            .to_update(),
        );
        draft
    }
}

fn competence_request(task: &str, floor: f32) -> JudgeRequest {
    JudgeRequest {
        id: REQUEST_ID.to_string(),
        question: Question {
            prompt: format!(
                "How competently was the previous step completed? \
                 Score 0.0 = failed, wrong or misleading; 1.0 = fully correct and useful. \
                 A score below {floor:.2} means the next step should use the strongest model."
            ),
            choices: Choices::ScoreBelowFloor { floor },
            min_confidence: 0.0,
        },
        required: false,
        // 低于 Escalation（10），高于 Classify；判官预算紧张时让硬信号先问。
        priority: 20,
        category: Category::QualityUncertain,
        evidence: EvidenceRef {
            fingerprint: format!("{EVIDENCE_PREFIX}{task}"),
            summary: "previous stage competence".to_string(),
        },
    }
}

/// 把最强候选置顶；没有可用候选时什么都不做。
fn pin_strongest(view: &RequestView, draft: &mut PlanDraft, reason: ReasonCode) {
    let strongest = view
        .candidates
        .iter()
        .filter(|candidate| candidate.enabled)
        .max_by(|left, right| {
            left.strength
                .cmp(&right.strength)
                .then_with(|| right.id.cmp(&left.id))
        });
    if let Some(candidate) = strongest {
        draft.contributions.push(PlanContribution::Pin {
            candidate: candidate.id.clone(),
            reason,
        });
    }
}

fn note(kind: ReasonCodeKind) -> PlanContribution {
    PlanContribution::Note {
        reason: ReasonCode::new(kind),
    }
}

/// 任务指纹：首条 system + 首条 user + 工具名；只在任务真正变化时改变。
pub(crate) fn task_fingerprint(view: &RequestView) -> String {
    let mut material = String::new();
    if let Some(block) = view.canonical.system.first() {
        material.push_str(&block.text);
    }
    material.push('\u{1f}');
    if let Some(message) = view
        .canonical
        .messages
        .iter()
        .find(|message| message.role == tr_contracts::CanonicalRole::User)
    {
        for block in &message.blocks {
            if let tr_contracts::ContentBlock::Text { text, .. } = block {
                material.push_str(text);
            }
        }
    }
    for tool in &view.canonical.tools {
        material.push('\u{1e}');
        material.push_str(&tool.name);
    }
    format!("{:016x}", fnv1a(material.as_bytes()))
}

/// 候选组 + profile + branch 指纹（近似 plan 版本）。
pub(crate) fn group_fingerprint(view: &RequestView) -> String {
    let mut ids: Vec<&str> = view
        .candidates
        .iter()
        .map(|candidate| candidate.id.as_str())
        .collect();
    ids.sort_unstable();
    let material = format!(
        "{}\u{1f}{}\u{1f}{}",
        view.profile.id,
        view.session.branch_id,
        ids.join(",")
    );
    format!("{:016x}", fnv1a(material.as_bytes()))
}

/// FNV-1a：稳定的跨进程指纹（`DefaultHasher` 跨版本不保证稳定）。
fn fnv1a(bytes: &[u8]) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325u64;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

#[cfg(test)]
mod tests {
    use super::*;
    use tr_contracts::{
        Candidate, CandidateHealth, CanonicalMessage, CanonicalRequest, CanonicalRole,
        CapabilityProfile, ContentBlock, InputModality, LedgerSummary, PlanMode, ProfileConfig,
        ProviderKind, RequestReason, Selection, SessionFacts, Signals, UpstreamRef, VirtualModel,
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

    fn view_with(text: &str) -> RequestView<'static> {
        let canonical: &'static CanonicalRequest = Box::leak(Box::new(CanonicalRequest {
            messages: vec![CanonicalMessage {
                role: CanonicalRole::User,
                blocks: vec![ContentBlock::Text {
                    text: text.to_string(),
                    cache_control: None,
                }],
                ..CanonicalMessage::default()
            }],
            ..CanonicalRequest::default()
        }));
        let signals: &'static Signals = Box::leak(Box::new(Signals::default()));
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
        let session: &'static SessionFacts = Box::leak(Box::new(SessionFacts {
            branch_id: "main".to_string(),
            ..SessionFacts::default()
        }));
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
            reason: RequestReason::Continuation,
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

    fn state_for(view: &RequestView, score: f32) -> StrategyState {
        StrategyState {
            strategy_id: "competence_guard".to_string(),
            blob: serde_json::json!({
                "score": score,
                "task": task_fingerprint(view),
                "group": group_fingerprint(view),
            }),
            ..StrategyState::default()
        }
    }

    fn pinned(draft: &PlanDraft) -> Option<&str> {
        draft.contributions.iter().find_map(|contribution| {
            if let PlanContribution::Pin { candidate, .. } = contribution {
                Some(candidate.as_str())
            } else {
                None
            }
        })
    }

    #[test]
    fn low_score_same_task_pins_strongest() {
        let view = view_with("fix the parser");
        let state = state_for(&view, 0.2);
        let draft =
            CompetenceGuard.draft(&view, &state, &StrategyConfig::enabled("competence_guard"));
        assert_eq!(pinned(&draft), Some("strong"));
        assert!(draft.reason_code(ReasonCodeKind::CompetenceProtected));
    }

    #[test]
    fn task_change_suppresses_and_clears_state() {
        let old_view = view_with("old task");
        let state = state_for(&old_view, 0.1);

        let new_view = view_with("a completely different task");
        let draft = CompetenceGuard.draft(
            &new_view,
            &state,
            &StrategyConfig::enabled("competence_guard"),
        );

        assert_eq!(pinned(&draft), None, "task change must not escalate");
        assert!(draft.reason_code(ReasonCodeKind::CompetenceTaskChanged));
        assert_eq!(draft.state_updates.len(), 1);
        assert_eq!(
            draft.state_updates[0].patch["score"],
            serde_json::Value::Null
        );
        assert!(draft.judge_requests.is_empty(), "no score for the new task");
    }

    #[test]
    fn high_score_does_nothing() {
        let view = view_with("same task");
        let state = state_for(&view, 0.9);
        let draft =
            CompetenceGuard.draft(&view, &state, &StrategyConfig::enabled("competence_guard"));
        assert_eq!(pinned(&draft), None);
        assert!(draft.judge_requests.is_empty());
    }

    #[test]
    fn no_score_asks_only_when_previous_stage_exists() {
        let view = view_with("same task");
        let draft = CompetenceGuard.draft(
            &view,
            &StrategyState::default(),
            &StrategyConfig::enabled("competence_guard"),
        );
        assert!(
            draft.judge_requests.is_empty(),
            "first turn has no previous"
        );

        let mut with_previous = view;
        with_previous.previous = Some(Box::leak(Box::new(tr_contracts::Dispatch {
            physical_model: "weak-physical".to_string(),
            provider: "OpenAiCompatible".to_string(),
            thinking_level: "off".to_string(),
        })));
        let draft = CompetenceGuard.draft(
            &with_previous,
            &StrategyState::default(),
            &StrategyConfig::enabled("competence_guard"),
        );
        assert_eq!(draft.judge_requests.len(), 1);
        assert_eq!(draft.judge_requests[0].id, REQUEST_ID);
        assert!(matches!(
            draft.judge_requests[0].question.choices,
            Choices::ScoreBelowFloor { .. }
        ));
    }

    #[test]
    fn refine_claims_its_own_verdict_and_writes_state() {
        let view = view_with("same task");
        let fingerprint = format!("{EVIDENCE_PREFIX}{}", task_fingerprint(&view));
        let verdict = Verdict::Escalate {
            category: Category::QualityUncertain,
            target_strength: None,
            evidence: EvidenceRef {
                fingerprint,
                summary: "competence".to_string(),
            },
            confidence: 0.9,
        };
        let draft = CompetenceGuard.refine(
            &view,
            &StrategyState::default(),
            &[verdict],
            &StrategyConfig::enabled("competence_guard"),
        );
        assert_eq!(pinned(&draft), Some("strong"));
        assert_eq!(draft.state_updates.len(), 1);
        assert_eq!(draft.state_updates[0].patch["score"], 0.0);
    }

    #[test]
    fn refine_ignores_foreign_verdicts() {
        let view = view_with("same task");
        let verdict = Verdict::Escalate {
            category: Category::ToolFailure,
            target_strength: None,
            evidence: EvidenceRef {
                fingerprint: "tool:shell_exec".to_string(),
                summary: "unrelated".to_string(),
            },
            confidence: 0.9,
        };
        let draft = CompetenceGuard.refine(
            &view,
            &StrategyState::default(),
            &[verdict],
            &StrategyConfig::enabled("competence_guard"),
        );
        assert_eq!(pinned(&draft), None);
        assert!(draft.state_updates.is_empty());
    }

    #[test]
    fn disabled_strategy_is_inert() {
        let view = view_with("same task");
        let state = state_for(&view, 0.0);
        let draft =
            CompetenceGuard.draft(&view, &state, &StrategyConfig::disabled("competence_guard"));
        assert!(draft.contributions.is_empty());
        assert!(draft.judge_requests.is_empty());
    }

    #[test]
    fn priority_sits_between_escalation_and_classify() {
        assert_eq!(CompetenceGuard.priority(), 20);
        assert!(CompetenceGuard.priority() > 10);
        assert!(CompetenceGuard.priority() < 30);
    }

    // 便于断言 reason code：PlanDraft 没有理由码，用贡献里的 ReasonCode。
    trait DraftReasons {
        fn reason_code(&self, kind: ReasonCodeKind) -> bool;
    }

    impl DraftReasons for PlanDraft {
        fn reason_code(&self, kind: ReasonCodeKind) -> bool {
            self.contributions.iter().any(|contribution| {
                let reason = match contribution {
                    PlanContribution::Filter { reason, .. }
                    | PlanContribution::Prefer { reason, .. }
                    | PlanContribution::Pin { reason, .. }
                    | PlanContribution::Mode { reason, .. }
                    | PlanContribution::CacheLoss { reason, .. }
                    | PlanContribution::Note { reason } => reason,
                };
                reason.code == kind
            })
        }
    }
}
