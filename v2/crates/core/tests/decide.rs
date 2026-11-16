//! `core::decide` 的纯函数测试（P0a 验收：可回放、确定性、Plan 校验）。

use std::collections::BTreeMap;

use tr_contracts::{
    Candidate, CandidateHealth, CanonicalMessage, CanonicalRequest, CanonicalRole,
    CapabilityProfile, ContentBlock, DecisionSnapshot, InputModality, LedgerSummary, MediaSource,
    PlanMode, PriceModel, ProfileConfig, ProviderKind, ReasonCodeKind, Selection, SessionFacts,
    Signals, UpstreamRef, VirtualModel,
};
use tr_core::decide;

fn candidate(
    id: &str,
    tier: &str,
    strength: u8,
    window: u32,
    input: Vec<InputModality>,
) -> Candidate {
    Candidate {
        id: id.to_string(),
        tier: tier.to_string(),
        strength,
        upstream: UpstreamRef {
            provider: ProviderKind::OpenAiCompatible,
            base_url: "http://127.0.0.1:11434/v1".to_string(),
            model: format!("{id}-physical"),
            credential: None,
        },
        window,
        capability: CapabilityProfile {
            input,
            thinking_levels: vec!["off".to_string(), "high".to_string()],
            ..CapabilityProfile::default()
        },
        price: Some(PriceModel {
            currency: "USD".to_string(),
            input_micros_per_mtok: 1_000_000,
            output_micros_per_mtok: 2_000_000,
            ..PriceModel::default()
        }),
        enabled: true,
    }
}

fn user_message(text: &str) -> CanonicalMessage {
    CanonicalMessage {
        role: CanonicalRole::User,
        blocks: vec![ContentBlock::Text {
            text: text.to_string(),
            cache_control: None,
        }],
        ..CanonicalMessage::default()
    }
}

fn snapshot(
    candidates: Vec<Candidate>,
    health: CandidateHealth,
    mode: PlanMode,
    text: &str,
) -> DecisionSnapshot {
    DecisionSnapshot {
        canonical: CanonicalRequest {
            messages: vec![user_message(text)],
            ..CanonicalRequest::default()
        },
        signals: Signals::default(),
        candidates: candidates.clone(),
        health,
        session: SessionFacts {
            session_key: "b".repeat(64),
            branch_id: "main".to_string(),
            ..SessionFacts::default()
        },
        strategy_states: BTreeMap::new(),
        ledger: LedgerSummary::default(),
        profile: ProfileConfig {
            id: "default".to_string(),
            mode,
            strength_order: vec![
                "local".to_string(),
                "cloud".to_string(),
                "premium".to_string(),
            ],
            ..ProfileConfig::default()
        },
        now_unix: 1_700_000_000,
        verdicts: Vec::new(),
        selection: Selection {
            virtual_model: "auto".to_string(),
            thinking_level: "high".to_string(),
        },
        virtual_model: VirtualModel {
            id: "auto".to_string(),
            profile: "default".to_string(),
            candidate_pool: candidates.iter().map(|c| c.id.clone()).collect(),
            ..VirtualModel::default()
        },
        reason: tr_contracts::RequestReason::User,
        previous: None,
        failed: None,
        context_hold: None,
        router_state: None,
    }
}

#[test]
fn baseline_orders_by_tier_then_strength() {
    let plan = decide(&snapshot(
        vec![
            candidate("claude", "premium", 80, 200_000, vec![InputModality::Text]),
            candidate("qwen", "local", 10, 32_768, vec![InputModality::Text]),
            candidate("deepseek", "cloud", 50, 128_000, vec![InputModality::Text]),
        ],
        CandidateHealth::default(),
        PlanMode::Cascade,
        "hello",
    ));

    let order: Vec<_> = plan
        .preference
        .iter()
        .map(|slot| slot.candidate_id.as_str())
        .collect();
    assert_eq!(order, vec!["qwen", "deepseek", "claude"]);
    assert_eq!(plan.mode, PlanMode::Cascade);
    assert_eq!(plan.failover.len(), 3);
}

#[test]
fn window_too_small_is_filtered_with_reason() {
    let long_text = "word ".repeat(5_000); // ≈ 6000+ tokens
    let plan = decide(&snapshot(
        vec![
            candidate("tiny", "local", 10, 1_000, vec![InputModality::Text]),
            candidate("big", "cloud", 50, 128_000, vec![InputModality::Text]),
        ],
        CandidateHealth::default(),
        PlanMode::Cascade,
        &long_text,
    ));

    assert_eq!(plan.preference.len(), 1);
    assert_eq!(plan.preference[0].candidate_id, "big");
    assert!(
        plan.reason_codes
            .iter()
            .any(|reason| reason.code == ReasonCodeKind::PlanFilterWindow)
    );
}

#[test]
fn image_input_requires_explicit_capability() {
    let mut snapshot = snapshot(
        vec![
            candidate("text-only", "local", 10, 128_000, vec![InputModality::Text]),
            candidate(
                "vision",
                "cloud",
                50,
                128_000,
                vec![InputModality::Text, InputModality::Image],
            ),
        ],
        CandidateHealth::default(),
        PlanMode::Cascade,
        "describe this",
    );
    snapshot.canonical.messages[0]
        .blocks
        .push(ContentBlock::Image {
            source: MediaSource::Url {
                url: "https://example.com/a.png".to_string(),
            },
            detail: None,
        });

    let plan = decide(&snapshot);
    assert_eq!(plan.preference.len(), 1);
    assert_eq!(plan.preference[0].candidate_id, "vision");
    assert!(
        plan.reason_codes
            .iter()
            .any(|reason| reason.code == ReasonCodeKind::ModelInputUnsupported)
    );
}

#[test]
fn open_breaker_is_excluded_and_reasoned() {
    let mut health = CandidateHealth::default();
    health.candidates.insert(
        "qwen".to_string(),
        tr_contracts::BreakerState {
            consecutive_failures: 3,
            open_until: Some(1_700_000_060),
            ..tr_contracts::BreakerState::default()
        },
    );

    let plan = decide(&snapshot(
        vec![
            candidate("qwen", "local", 10, 32_768, vec![InputModality::Text]),
            candidate("deepseek", "cloud", 50, 128_000, vec![InputModality::Text]),
        ],
        health,
        PlanMode::Cascade,
        "hello",
    ));

    assert_eq!(plan.preference[0].candidate_id, "deepseek");
    assert!(
        plan.reason_codes
            .iter()
            .any(|reason| reason.code == ReasonCodeKind::CandidatesCoolingDown)
    );
}

#[test]
fn cascade_with_single_candidate_downgrades_to_single() {
    let plan = decide(&snapshot(
        vec![candidate(
            "only",
            "cloud",
            50,
            128_000,
            vec![InputModality::Text],
        )],
        CandidateHealth::default(),
        PlanMode::Cascade,
        "hello",
    ));
    assert_eq!(plan.mode, PlanMode::Single);
    assert!(
        plan.reason_codes
            .iter()
            .any(|reason| reason.code == ReasonCodeKind::PlanModeDowngraded)
    );
}

#[test]
fn empty_pool_yields_empty_preference() {
    let plan = decide(&snapshot(
        vec![],
        CandidateHealth::default(),
        PlanMode::Cascade,
        "hello",
    ));
    assert!(plan.preference.is_empty());
    assert!(plan.failover.is_empty());
    assert!(
        plan.reason_codes
            .iter()
            .any(|reason| reason.code == ReasonCodeKind::NoEligibleCandidate)
    );
}

#[test]
fn disabled_candidate_never_enters_plan() {
    let mut disabled = candidate("qwen", "local", 10, 32_768, vec![InputModality::Text]);
    disabled.enabled = false;
    let plan = decide(&snapshot(
        vec![
            disabled,
            candidate("deepseek", "cloud", 50, 128_000, vec![InputModality::Text]),
        ],
        CandidateHealth::default(),
        PlanMode::Cascade,
        "hello",
    ));
    assert_eq!(plan.preference.len(), 1);
    assert_eq!(plan.preference[0].candidate_id, "deepseek");
}

#[test]
fn slots_carry_cost_and_clamp_thinking_level() {
    let mut local = candidate("qwen", "local", 10, 32_768, vec![InputModality::Text]);
    // 本地模型只支持 off；请求是 high，slot 必须被 clamp。
    local.capability.thinking_levels = vec!["off".to_string()];
    let plan = decide(&snapshot(
        vec![local],
        CandidateHealth::default(),
        PlanMode::Single,
        "hello",
    ));
    assert_eq!(plan.preference[0].thinking_level, "off");
    assert!(plan.preference[0].est_cost.is_some());
}

#[test]
fn decide_is_deterministic() {
    let snapshot = snapshot(
        vec![
            candidate("qwen", "local", 10, 32_768, vec![InputModality::Text]),
            candidate("deepseek", "cloud", 50, 128_000, vec![InputModality::Text]),
        ],
        CandidateHealth::default(),
        PlanMode::Cascade,
        "hello",
    );
    let first = decide(&snapshot);
    let second = decide(&snapshot);
    assert_eq!(first, second);
}
