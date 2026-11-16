//! P1 判官/棘轮 e2e：active 模式升级、shadow 不改变决策、审计落表。

use mock_upstream::{MockUpstream, Reply};
use tr_agent::config::{Catalog, ConfigFile};
use tr_agent::pipeline::{Pipeline, RequestContext};
use tr_agent::store::{self, Db};
use tr_contracts::{
    CanonicalMessage, CanonicalRequest, CanonicalRole, ContentBlock, DecisionSnapshot,
    StrategyState,
};

fn catalog(mock_base_url: &str, judge_mode: &str) -> Catalog {
    let toml = format!(
        r#"
schema_version = 1

[profiles.default]
mode = "cascade"
strength_order = ["local", "cloud"]

[[candidates]]
id = "weak"
tier = "local"
strength = 10
provider = "openai"
base_url = "{mock_base_url}"
model = "physical-weak"
window = 32768
input = ["text"]

[[candidates]]
id = "strong"
tier = "cloud"
strength = 80
provider = "openai"
base_url = "{mock_base_url}"
model = "physical-strong"
window = 128000
input = ["text"]

[[virtual_models]]
provider = "router"
id = "auto"
name = "Auto"
profile = "default"
candidate_pool = ["weak", "strong"]

[judge]
mode = "{judge_mode}"

[judge.service]
kind = "local"
"#
    );
    ConfigFile::parse(&toml)
        .expect("parse")
        .into_catalog()
        .expect("catalog")
}

fn failing_request(extra_turn: bool) -> CanonicalRequest {
    let mut messages = vec![
        CanonicalMessage {
            role: CanonicalRole::User,
            blocks: vec![ContentBlock::Text {
                text: "read the file".to_string(),
                cache_control: None,
            }],
            ..CanonicalMessage::default()
        },
        CanonicalMessage {
            role: CanonicalRole::Assistant,
            blocks: vec![ContentBlock::ToolUse {
                id: "call_1".to_string(),
                name: "read_file".to_string(),
                arguments: serde_json::json!({"path": "a.txt"}),
            }],
            ..CanonicalMessage::default()
        },
        CanonicalMessage {
            role: CanonicalRole::Tool,
            blocks: vec![ContentBlock::ToolResult {
                tool_use_id: "call_1".to_string(),
                content: vec![ContentBlock::Text {
                    text: "permission denied".to_string(),
                    cache_control: None,
                }],
                is_error: true,
            }],
            ..CanonicalMessage::default()
        },
        CanonicalMessage {
            role: CanonicalRole::Tool,
            blocks: vec![ContentBlock::ToolResult {
                tool_use_id: "call_1".to_string(),
                content: vec![ContentBlock::Text {
                    text: "still denied".to_string(),
                    cache_control: None,
                }],
                is_error: true,
            }],
            ..CanonicalMessage::default()
        },
    ];
    if extra_turn {
        // 再追加一次同类失败：turn 变化 → 证据指纹变化（新证据），同时保留错误尾部信号。
        messages.push(CanonicalMessage {
            role: CanonicalRole::Tool,
            blocks: vec![ContentBlock::ToolResult {
                tool_use_id: "call_1".to_string(),
                content: vec![ContentBlock::Text {
                    text: "denied again".to_string(),
                    cache_control: None,
                }],
                is_error: true,
            }],
            ..CanonicalMessage::default()
        });
    }
    CanonicalRequest {
        messages,
        ..CanonicalRequest::default()
    }
}

async fn wait_for<F>(mut predicate: F)
where
    F: FnMut() -> bool,
{
    for _ in 0..100 {
        if predicate() {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
}

#[tokio::test]
async fn active_judge_latches_after_second_evidence() {
    let mock =
        MockUpstream::start(vec![Reply::ok("weak answer"), Reply::ok("strong answer")]).await;
    let db = Db::open_in_memory().expect("db");
    let pipeline = Pipeline::from_catalog(db, catalog(&mock.base_url(), "active"));

    let first = pipeline
        .handle(failing_request(false), RequestContext::default())
        .await
        .expect("first");
    assert_eq!(first.headers.served_model, "physical-weak");
    assert_eq!(store::judge::count_calls(pipeline.db()).expect("calls"), 1);

    let second = pipeline
        .handle(failing_request(true), RequestContext::default())
        .await
        .expect("second");
    assert_eq!(second.headers.served_model, "physical-strong");
    assert!(
        second.headers.reason_codes.contains("ESCALATION_LATCHED"),
        "{:?}",
        second.headers.reason_codes
    );

    // 状态已持久化：直接读 strategy_states。
    let states = store::strategy::load_strategy_states(
        pipeline.db(),
        &second.ledger.session_key,
        "main",
        "",
        "auto",
    )
    .expect("states");
    let escalation: &StrategyState = states.get("escalation").expect("escalation state");
    assert_eq!(escalation.blob["phase"], "latched");

    mock.shutdown();
}

#[tokio::test]
async fn shadow_mode_records_but_does_not_change_dispatch() {
    let mock = MockUpstream::start(vec![
        Reply::ok("weak answer"),
        Reply::ok("weak answer again"),
    ])
    .await;
    let db = Db::open_in_memory().expect("db");
    let pipeline = Pipeline::from_catalog(db, catalog(&mock.base_url(), "shadow"));

    let first = pipeline
        .handle(failing_request(false), RequestContext::default())
        .await
        .expect("first");
    assert_eq!(first.headers.served_model, "physical-weak");

    let second = pipeline
        .handle(failing_request(true), RequestContext::default())
        .await
        .expect("second");
    // shadow 不改变内核结论：仍然按 baseline 走弱档。
    assert_eq!(second.headers.served_model, "physical-weak");

    wait_for(|| store::judge::count_shadow(pipeline.db()).expect("shadow") == 2).await;
    assert_eq!(
        store::judge::count_shadow(pipeline.db()).expect("shadow"),
        2
    );

    mock.shutdown();
}

#[test]
fn judge_off_never_calls() {
    // 同步占位：off 模式的运行时行为由 prepare 的 mode 判断覆盖，
    // 这里只固定配置解析不会把 off 解析成 shadow。
    let catalog = catalog("http://127.0.0.1:1/v1", "off");
    assert_eq!(catalog.judge.mode, tr_agent::judge::JudgeMode::Off);
}

#[test]
fn snapshot_digest_is_stable_for_shadow() {
    let snapshot = DecisionSnapshot::default();
    let first = snapshot.projection_digest().expect("digest");
    let second = snapshot.projection_digest().expect("digest");
    assert_eq!(first, second);
}

#[test]
fn shadow_summary_counts_agreement() {
    let db = Db::open_in_memory().expect("db");
    for (agreement, latency) in [(Some(true), 12u32), (None, 40u32)] {
        store::judge::insert_shadow(
            &db,
            &store::judge::ShadowRecord {
                request_id: uuid::Uuid::new_v4(),
                selection_model: "auto".to_string(),
                kernel_choice: "weak".to_string(),
                judge_question_id: Some("q".to_string()),
                judge_value: None,
                agreement,
                effective_source: Some("heuristic".to_string()),
                judge_latency_ms: Some(latency),
                judge_cost_micros: None,
                snapshot_digest: "digest".to_string(),
                created_at: 1_700_000_000,
            },
        )
        .expect("insert shadow");
    }

    let summary = store::judge::shadow_summary(&db).expect("summary");
    assert_eq!(summary.total, 2);
    assert_eq!(summary.agreed, 1);
    assert_eq!(summary.unknown, 1);
    assert_eq!(summary.agreement_rate(), Some(1.0));
    assert!(summary.avg_latency_ms.unwrap() > 0.0);
}

#[tokio::test]
async fn llm_judge_adapter_parses_structured_answer() {
    let mock = MockUpstream::start(vec![Reply::Json {
        status: 200,
        body: serde_json::json!({
            "choices": [{
                "index": 0,
                "message": {
                    "role": "assistant",
                    "content": "{\"value\": true, \"confidence\": 0.9, \"reason\": \"tool failed twice\"}"
                },
                "finish_reason": "stop"
            }],
            "usage": {"prompt_tokens": 20, "completion_tokens": 10, "total_tokens": 30}
        }),
    }])
    .await;

    let model = tr_decide::llm::LlmJudgeModel::new(tr_decide::llm::LlmJudgeConfig {
        endpoint: mock.base_url(),
        model: "mock-judge".to_string(),
        credential: None,
        timeout: std::time::Duration::from_secs(2),
        structured_output: tr_decide::llm::StructuredOutput::JsonMode,
        temperature: 0.0,
        max_output_tokens: 64,
        service_id: "test-judge".to_string(),
    })
    .expect("model");

    let input = tr_contracts::JudgeInput {
        question: tr_contracts::Question {
            prompt: "escalate?".to_string(),
            choices: tr_contracts::Choices::Boolean,
            min_confidence: 0.6,
        },
        category: tr_contracts::Category::ToolFailure,
        evidence: tr_contracts::EvidenceRef::default(),
        context: Default::default(),
    };

    let answer = tr_decide::consult(&model, &input, std::time::Duration::from_secs(2))
        .await
        .expect("answer");
    assert_eq!(answer.value, tr_contracts::JudgeValue::Bool(true));
    assert_eq!(answer.confidence, Some(0.9));

    mock.shutdown();
}
