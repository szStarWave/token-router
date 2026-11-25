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
    catalog_with_judge(mock_base_url, judge_mode, "local", "")
}

fn catalog_with_judge(mock_base_url: &str, judge_mode: &str, kind: &str, model: &str) -> Catalog {
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
price = {{ currency = "USD", input_micros_per_mtok = 1000000, output_micros_per_mtok = 2000000 }}

[[candidates]]
id = "strong"
tier = "cloud"
strength = 80
provider = "openai"
base_url = "{mock_base_url}"
model = "physical-strong"
window = 128000
input = ["text"]
price = {{ currency = "USD", input_micros_per_mtok = 3000000, output_micros_per_mtok = 15000000 }}

[[virtual_models]]
provider = "router"
id = "auto"
name = "Auto"
profile = "default"
candidate_pool = ["weak", "strong"]

[judge]
mode = "{judge_mode}"

[judge.service]
kind = "{kind}"
endpoint = "{mock_base_url}"
model = "{model}"
input_micros_per_mtok = 1000000
output_micros_per_mtok = 2000000
"#
    );
    ConfigFile::parse(&toml)
        .expect("parse")
        .into_catalog()
        .expect("catalog")
}

fn chat_request(text: &str) -> CanonicalRequest {
    CanonicalRequest {
        messages: vec![CanonicalMessage {
            role: CanonicalRole::User,
            blocks: vec![ContentBlock::Text {
                text: text.to_string(),
                cache_control: None,
            }],
            ..CanonicalMessage::default()
        }],
        ..CanonicalRequest::default()
    }
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
        disable_thinking: true,
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

#[tokio::test]
async fn llm_judge_cost_lands_in_routing_cost() {
    let judge_body = serde_json::json!({
        "choices": [{
            "index": 0,
            "message": {
                "role": "assistant",
                "content": "{\"value\": true, \"confidence\": 0.9, \"reason\": \"tool failed twice\"}"
            },
            "finish_reason": "stop"
        }],
        "usage": {"prompt_tokens": 1000, "completion_tokens": 500, "total_tokens": 1500}
    });
    let mock = MockUpstream::start(vec![
        Reply::Json {
            status: 200,
            body: judge_body,
        },
        Reply::ok("final answer"),
    ])
    .await;

    let db = Db::open_in_memory().expect("db");
    let pipeline = Pipeline::from_catalog(
        db,
        catalog_with_judge(&mock.base_url(), "active", "llm", "judge-model"),
    );

    let result = pipeline
        .handle(failing_request(false), RequestContext::default())
        .await
        .expect("handle");

    // 1000 × 1e6/1e6 + 500 × 2e6/1e6 = 2000 micros
    let routing = result.ledger.routing_cost.as_ref().expect("routing cost");
    assert_eq!(routing.micros, 2000);
    assert_eq!(routing.currency, "USD");

    let usage = result
        .ledger
        .judge
        .as_ref()
        .and_then(|trace| trace.usage)
        .expect("judge usage");
    assert_eq!(usage.prompt_tokens, 1000);
    assert_eq!(usage.completion_tokens, 500);
    assert_eq!(usage.cost_micros, 2000);

    let answer_cost = result.ledger.answer_cost.as_ref().expect("answer cost");
    assert_eq!(
        result.ledger.cost_equiv.as_ref().expect("equiv").micros,
        answer_cost.micros + 2000
    );

    // 审计表与账本同源：judge_calls 的成本合计必须等于 routing_cost。
    assert_eq!(
        store::judge::total_call_cost_micros(pipeline.db()).expect("call cost"),
        2000
    );

    mock.shutdown();
}

#[tokio::test]
async fn competence_guard_protects_same_task_and_suppresses_on_task_change() {
    let judge_body = serde_json::json!({
        "choices": [{
            "index": 0,
            "message": {
                "role": "assistant",
                "content": "{\"value\": 0.2, \"confidence\": 0.9, \"reason\": \"previous answer was wrong\"}"
            },
            "finish_reason": "stop"
        }],
        "usage": {"prompt_tokens": 100, "completion_tokens": 20, "total_tokens": 120}
    });
    let mock = MockUpstream::start(vec![
        Reply::ok("first answer"),
        Reply::Json {
            status: 200,
            body: judge_body,
        },
        Reply::ok("protected answer"),
        Reply::ok("new task answer"),
    ])
    .await;

    let mut catalog = catalog_with_judge(&mock.base_url(), "active", "llm", "judge-model");
    catalog
        .virtual_models
        .get_mut("auto")
        .expect("vm")
        .strategies = vec![
        "baseline".to_string(),
        "failover".to_string(),
        "competence_guard".to_string(),
    ];
    let db = Db::open_in_memory().expect("db");
    let pipeline = Pipeline::from_catalog(db, catalog);

    let context = RequestContext {
        server_session_id: Some("sess-competence".to_string()),
        ..RequestContext::default()
    };
    // 第一轮：建立 previous（弱档成功）。
    let first = pipeline
        .handle(chat_request("task A"), context.clone())
        .await
        .expect("first");
    assert_eq!(first.headers.served_model, "physical-weak");

    // 同一任务续接：判官给 0.2 → 保护性升级到强档，且 sticky 不得拉回弱档。
    let continuation = RequestContext {
        server_session_id: Some("sess-competence".to_string()),
        request_reason: Some(tr_contracts::RequestReason::Continuation),
        ..RequestContext::default()
    };
    let protected = pipeline
        .handle(chat_request("task A"), continuation)
        .await
        .expect("protected");
    assert_eq!(protected.headers.served_model, "physical-strong");
    assert!(
        protected
            .ledger
            .reason_codes
            .iter()
            .any(|reason| reason.code == tr_contracts::ReasonCodeKind::CompetenceProtected)
    );

    // 新任务：任务指纹变化 → 抑制升级；用 Direct 排除 ContextHold 的软亲和干扰。
    let new_task = pipeline
        .handle(
            chat_request("task B"),
            RequestContext {
                server_session_id: Some("sess-competence".to_string()),
                request_reason: Some(tr_contracts::RequestReason::Direct),
                ..RequestContext::default()
            },
        )
        .await
        .expect("new task");
    assert_eq!(new_task.headers.served_model, "physical-weak");
    assert!(
        new_task
            .ledger
            .reason_codes
            .iter()
            .any(|reason| reason.code == tr_contracts::ReasonCodeKind::CompetenceTaskChanged)
    );

    assert_eq!(mock.remaining(), 0);
    mock.shutdown();
}
