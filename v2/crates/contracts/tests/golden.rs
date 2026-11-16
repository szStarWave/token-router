//! 契约 JSON 形状 golden（§4.7 规则 5、§11.1）。
//!
//! 这些断言故意写成完整对象：它们是**契约的书面形式**，改动必须同步更新
//! 设计文档与账本/响应头，不允许“顺手加个字段”。

use tr_contracts::{Plan, Selection};

#[test]
fn plan_json_shape_is_stable() {
    let plan = Plan {
        selection: Selection {
            virtual_model: "auto".to_string(),
            thinking_level: "high".to_string(),
        },
        ..Plan::default()
    };

    let value = serde_json::to_value(&plan).expect("serialize plan");
    let expected = serde_json::json!({
        "selection": { "virtual_model": "auto", "thinking_level": "high" },
        "dispatch_source": "strategy",
        "preference": [],
        "failover": [],
        "mode": "single",
        "expected_cache_loss": null,
        "judge": {
            "service_id": null,
            "service_revision": null,
            "protocol_digest": null,
            "context_digest": null,
            "local_decision_digest": "",
            "effective_source": "heuristic",
            "fallback_used": false,
            "fallback_reason": null,
            "duration_ms": 0,
            "usage": null
        },
        "reason_codes": [],
        "state_updates": [],
        "contributions": []
    });

    assert_eq!(value, expected);
}

#[test]
fn plan_deserializes_without_new_fields() {
    // 旧账本/旧回放文件没有后续字段时必须仍可读（§18.4.1 序列化约定）。
    let plan: Plan = serde_json::from_str(r#"{"selection":{"virtual_model":"auto"}}"#)
        .expect("deserialize old plan");
    assert_eq!(plan.selection.virtual_model, "auto");
    assert_eq!(plan.dispatch_source, tr_contracts::DispatchSource::Strategy);
    assert!(plan.preference.is_empty());
}
