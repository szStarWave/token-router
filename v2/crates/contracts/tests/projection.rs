//! 决策投影确定性测试（§4.7、P0a 验收②）。

use std::collections::BTreeMap;

use tr_contracts::{
    Candidate, CandidateHealth, CanonicalMessage, CanonicalRequest, CanonicalRole,
    CapabilityProfile, ContentBlock, DecisionSnapshot, InputModality, LedgerSummary, PlanMode,
    PriceModel, ProfileConfig, ProviderKind, RequestReason, Selection, SessionFacts, Signals,
    UpstreamRef, VirtualModel,
};

fn candidate(id: &str, tier: &str, strength: u8) -> Candidate {
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
        window: 128_000,
        capability: CapabilityProfile {
            input: vec![InputModality::Text],
            thinking_levels: vec!["off".to_string()],
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

fn snapshot() -> DecisionSnapshot {
    DecisionSnapshot {
        canonical: CanonicalRequest {
            messages: vec![CanonicalMessage {
                role: CanonicalRole::User,
                blocks: vec![ContentBlock::Text {
                    text: "explain the routing decision".to_string(),
                    cache_control: None,
                }],
                ..CanonicalMessage::default()
            }],
            ..CanonicalRequest::default()
        },
        signals: Signals::default(),
        candidates: vec![
            candidate("qwen-local", "local", 10),
            candidate("deepseek-chat", "cloud", 50),
        ],
        health: CandidateHealth::default(),
        session: SessionFacts {
            session_key: "a".repeat(64),
            branch_id: "main".to_string(),
            updated_at: 111,
            ..SessionFacts::default()
        },
        strategy_states: BTreeMap::new(),
        ledger: LedgerSummary::default(),
        profile: ProfileConfig {
            id: "default".to_string(),
            mode: PlanMode::Cascade,
            ..ProfileConfig::default()
        },
        now_unix: 1_700_000_000,
        verdicts: Vec::new(),
        selection: Selection {
            virtual_model: "auto".to_string(),
            thinking_level: "off".to_string(),
        },
        virtual_model: VirtualModel {
            provider: "router".to_string(),
            id: "auto".to_string(),
            name: "Auto".to_string(),
            thinking_levels: vec!["off".to_string()],
            profile: "default".to_string(),
            candidate_pool: vec!["qwen-local".to_string(), "deepseek-chat".to_string()],
            ..VirtualModel::default()
        },
        reason: RequestReason::User,
        previous: None,
        failed: None,
        context_hold: None,
        router_state: None,
    }
}

#[test]
fn projection_is_stable_across_calls() {
    let snapshot = snapshot();
    let first = snapshot.projection_digest().expect("digest");
    let second = snapshot.projection_digest().expect("digest");
    assert_eq!(first, second);
    assert_eq!(first.len(), 64, "sha256 hex 长度固定");
}

#[test]
fn timestamps_and_request_id_are_excluded() {
    let baseline = snapshot().projection_digest().expect("digest");

    let mut touched = snapshot();
    touched.session.updated_at = 999_999;
    touched.canonical.meta.request_id = Some("req-123".to_string());
    assert_eq!(
        touched.projection_digest().expect("digest"),
        baseline,
        "updated_at / request_id 不得影响投影"
    );
}

#[test]
fn semantic_changes_change_projection() {
    let baseline = snapshot().projection_digest().expect("digest");

    let mut changed = snapshot();
    changed.profile.mode = PlanMode::Single;
    assert_ne!(changed.projection_digest().expect("digest"), baseline);

    let mut reordered = snapshot();
    reordered.candidates.reverse();
    assert_ne!(
        reordered.projection_digest().expect("digest"),
        baseline,
        "候选顺序是决策输入，变化必须反映在指纹上"
    );
}

#[test]
fn view_exposes_sticky_inputs() {
    let snapshot = snapshot();
    let view = snapshot.view();
    assert_eq!(view.reason, RequestReason::User);
    assert!(view.previous.is_none());
    assert!(view.failed.is_none());
    assert_eq!(view.virtual_model.id, "auto");
    assert_eq!(
        view.candidate("deepseek-chat").expect("candidate").strength,
        50
    );
}
