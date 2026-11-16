//! `tr-cli replay` 契约测试（P0a-6 / G1 验收②）。

#[test]
fn replay_produces_plan_from_snapshot() {
    let snapshot = serde_json::json!({
        "canonical": {
            "messages": [{
                "role": "user",
                "blocks": [{"type": "text", "text": "hi"}]
            }]
        },
        "candidates": [{
            "id": "a",
            "tier": "local",
            "strength": 10,
            "upstream": {
                "provider": "open_ai_compatible",
                "base_url": "http://127.0.0.1:1/v1",
                "model": "physical-a"
            },
            "window": 32768,
            "enabled": true
        }],
        "profile": {
            "id": "default",
            "mode": "cascade",
            "strength_order": ["local"]
        },
        "selection": {"virtual_model": "auto", "thinking_level": "off"},
        "virtual_model": {
            "id": "auto",
            "profile": "default",
            "candidate_pool": ["a"]
        },
        "now_unix": 1700000000
    })
    .to_string();

    let plan = tr_cli::replay_str(&snapshot).expect("replay");
    assert_eq!(plan.preference.len(), 1);
    assert_eq!(plan.preference[0].candidate_id, "a");
    // 单候选 cascade 必须降级为 single（§4.8）。
    assert_eq!(plan.mode, tr_contracts::PlanMode::Single);
}

#[test]
fn replay_is_deterministic() {
    let snapshot: tr_contracts::DecisionSnapshot = serde_json::from_value(serde_json::json!({
        "now_unix": 1700000000,
        "profile": {"id": "default", "strength_order": []},
    }))
    .expect("snapshot");

    let first = tr_cli::replay_digest(&snapshot).expect("digest");
    let second = tr_cli::replay_digest(&snapshot).expect("digest");
    assert_eq!(first, second);
    assert_eq!(first.len(), 64);
}
