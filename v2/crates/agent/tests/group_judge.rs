//! M1 组级判断 e2e：规则/能力选择、fail-open、隐私与全局判官抑制。
//!
//! 用 `mock-upstream` 的顺序回复模拟 LLM 判官与真实上游：组判断先发一次
//! `/v1/chat/completions`，随后主请求再发一次。

use mock_upstream::{MockUpstream, Reply};
use tr_agent::config::{Catalog, ConfigFile};
use tr_agent::pipeline::{Pipeline, RequestContext};
use tr_agent::store::{self, Db};
use tr_contracts::{
    CanonicalMessage, CanonicalRequest, CanonicalRole, ContentBlock, PlanContribution,
    ReasonCodeKind,
};

const RULES: &str = r#"
[[virtual_models.rules]]
id = "coding"
intent = "code implementation and debugging"
prefer = "strong"
"#;

fn catalog(
    mock_base_url: &str,
    group_mode: &str,
    budget_extra: &str,
    rules: &str,
    global_judge: &str,
) -> Catalog {
    catalog_with_service(
        mock_base_url,
        group_mode,
        budget_extra,
        rules,
        global_judge,
        "llm",
        mock_base_url,
        "group-judge",
    )
}

#[allow(clippy::too_many_arguments)]
fn catalog_with_service(
    mock_base_url: &str,
    group_mode: &str,
    budget_extra: &str,
    rules: &str,
    global_judge: &str,
    service_kind: &str,
    service_endpoint: &str,
    service_model: &str,
) -> Catalog {
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

{global_judge}

[[virtual_models]]
provider = "router"
id = "auto"
name = "Auto"
profile = "default"
candidate_pool = ["weak", "strong"]

[virtual_models.judge]
mode = "{group_mode}"
{budget_extra}

[virtual_models.judge.service]
kind = "{service_kind}"
endpoint = "{service_endpoint}"
model = "{service_model}"
timeout_ms = 2000
input_micros_per_mtok = 1000000
output_micros_per_mtok = 2000000

{rules}
"#
    );
    ConfigFile::parse(&toml)
        .expect("parse")
        .into_catalog()
        .expect("catalog")
}

fn judge_body(value: &str) -> serde_json::Value {
    let content =
        serde_json::json!({"value": value, "confidence": 0.9, "reason": "matched"}).to_string();
    serde_json::json!({
        "choices": [{
            "index": 0,
            "message": {"role": "assistant", "content": content},
            "finish_reason": "stop"
        }],
        "usage": {"prompt_tokens": 100, "completion_tokens": 20, "total_tokens": 120}
    })
}

fn judge_reply(value: &str) -> Reply {
    Reply::Json {
        status: 200,
        body: judge_body(value),
    }
}

/// SystemOne（JEV）choice 响应。
fn system_one_reply(choice: &str) -> Reply {
    let mut probabilities = serde_json::Map::new();
    probabilities.insert(choice.to_string(), serde_json::json!(0.9));
    probabilities.insert("none".to_string(), serde_json::json!(0.1));
    Reply::Json {
        status: 200,
        body: serde_json::json!({
            "answers": {
                "route": {
                    "type": "choice",
                    "choice": choice,
                    "probabilities": probabilities,
                    "confidence": 0.9
                }
            },
            "usage": {"input_tokens": 10, "output_tokens": 2}
        }),
    }
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

/// 触发内核全局判官请求（工具连续失败）的请求。
fn failing_request() -> CanonicalRequest {
    CanonicalRequest {
        messages: vec![
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
        ],
        ..CanonicalRequest::default()
    }
}

fn has_reason(result: &tr_agent::pipeline::PipelineResponse, code: ReasonCodeKind) -> bool {
    result
        .ledger
        .reason_codes
        .iter()
        .any(|reason| reason.code == code)
}

#[tokio::test]
async fn rules_mode_promotes_rule_preference() {
    let mock =
        MockUpstream::start(vec![judge_reply("coding"), Reply::ok("served by strong")]).await;
    let db = Db::open_in_memory().expect("db");
    let pipeline = Pipeline::from_catalog(db, catalog(&mock.base_url(), "active", "", RULES, ""));

    let result = pipeline
        .handle(
            chat_request("implement a parser"),
            RequestContext::default(),
        )
        .await
        .expect("handle");

    // 基线顺序是 weak 在前；规则命中后置首 strong。
    assert_eq!(result.headers.served_model, "physical-strong");
    assert!(has_reason(&result, ReasonCodeKind::GroupJudgeSelected));

    let record = result
        .ledger
        .contributions
        .iter()
        .find(|record| record.strategy_id == "group_judge")
        .expect("group_judge contribution");
    assert_eq!(record.priority, 5);
    match &record.contribution {
        PlanContribution::Pin { candidate, reason } => {
            assert_eq!(candidate.as_str(), "strong");
            assert_eq!(reason.code, ReasonCodeKind::GroupJudgeSelected);
            assert_eq!(reason.detail.as_deref(), Some("rule:coding -> strong"));
        }
        other => panic!("expected pin contribution, got {other:?}"),
    }

    assert_eq!(store::judge::count_calls(pipeline.db()).expect("calls"), 1);
    // LLM 判官也记录详情：实际发出的 messages（system 规则 + TASK/QUESTION）与答案。
    let calls = store::judge::list_calls(
        pipeline.db(),
        &store::judge::JudgeCallFilter {
            limit: 10,
            ..Default::default()
        },
    )
    .expect("call list");
    let call = calls.first().expect("judge call");
    assert_eq!(call.effective_source, "external_llm");
    let messages = call.request.as_ref().expect("request detail")["messages"]
        .as_array()
        .expect("messages");
    assert_eq!(messages[0]["role"], "system");
    assert!(
        messages[0]["content"]
            .as_str()
            .unwrap()
            .contains("MANDATORY RULES")
    );
    assert!(
        messages[1]["content"]
            .as_str()
            .unwrap()
            .contains("TASK:\nimplement a parser")
    );
    assert_eq!(
        call.response.as_ref().expect("response detail")["value"],
        "coding"
    );
    // 100 prompt × 1e6/1e6 + 20 completion × 2e6/1e6 = 140 micros。
    let routing = result.ledger.routing_cost.as_ref().expect("routing cost");
    assert_eq!(routing.micros, 140);
    assert_eq!(routing.currency, "USD");
    assert_eq!(mock.remaining(), 0);
    mock.shutdown();
}

#[tokio::test]
async fn capability_mode_promotes_selected_candidate() {
    let mock =
        MockUpstream::start(vec![judge_reply("strong"), Reply::ok("served by strong")]).await;
    let db = Db::open_in_memory().expect("db");
    let pipeline = Pipeline::from_catalog(db, catalog(&mock.base_url(), "active", "", "", ""));

    let result = pipeline
        .handle(chat_request("write a compiler"), RequestContext::default())
        .await
        .expect("handle");

    assert_eq!(result.headers.served_model, "physical-strong");
    assert!(has_reason(&result, ReasonCodeKind::GroupJudgeSelected));
    let record = result
        .ledger
        .contributions
        .iter()
        .find(|record| record.strategy_id == "group_judge")
        .expect("group_judge contribution");
    match &record.contribution {
        PlanContribution::Pin { candidate, reason } => {
            assert_eq!(candidate.as_str(), "strong");
            assert_eq!(reason.detail.as_deref(), Some("capability -> strong"));
        }
        other => panic!("expected pin contribution, got {other:?}"),
    }
    assert_eq!(store::judge::count_calls(pipeline.db()).expect("calls"), 1);
    assert_eq!(mock.remaining(), 0);
    mock.shutdown();
}

#[tokio::test]
async fn judge_5xx_fails_open_without_reordering() {
    let mock = MockUpstream::start(vec![
        Reply::error(503, "server_error", "judge down"),
        Reply::ok("served by weak"),
    ])
    .await;
    let db = Db::open_in_memory().expect("db");
    let pipeline = Pipeline::from_catalog(db, catalog(&mock.base_url(), "active", "", RULES, ""));

    let result = pipeline
        .handle(
            chat_request("implement a parser"),
            RequestContext::default(),
        )
        .await
        .expect("HTTP must still succeed");

    assert_eq!(result.headers.served_model, "physical-weak");
    assert!(has_reason(&result, ReasonCodeKind::GroupJudgeUnavailable));
    assert!(
        !result
            .ledger
            .contributions
            .iter()
            .any(|record| record.strategy_id == "group_judge")
    );
    assert_eq!(store::judge::count_calls(pipeline.db()).expect("calls"), 1);
    mock.shutdown();
}

#[tokio::test]
async fn invalid_answer_fails_open_without_reordering() {
    let mock =
        MockUpstream::start(vec![judge_reply("not-a-rule"), Reply::ok("served by weak")]).await;
    let db = Db::open_in_memory().expect("db");
    let pipeline = Pipeline::from_catalog(db, catalog(&mock.base_url(), "active", "", RULES, ""));

    let result = pipeline
        .handle(
            chat_request("implement a parser"),
            RequestContext::default(),
        )
        .await
        .expect("HTTP must still succeed");

    assert_eq!(result.headers.served_model, "physical-weak");
    assert!(has_reason(&result, ReasonCodeKind::GroupJudgeUnavailable));
    assert_eq!(store::judge::count_calls(pipeline.db()).expect("calls"), 1);
    mock.shutdown();
}

#[tokio::test]
async fn timeout_fails_open_without_reordering() {
    let mock = MockUpstream::start(vec![
        Reply::Delay {
            millis: 400,
            body: judge_body("coding"),
        },
        Reply::ok("served by weak"),
    ])
    .await;
    let db = Db::open_in_memory().expect("db");
    let pipeline = Pipeline::from_catalog(
        db,
        catalog(&mock.base_url(), "active", "deadline_ms = 50", RULES, ""),
    );

    let result = pipeline
        .handle(
            chat_request("implement a parser"),
            RequestContext::default(),
        )
        .await
        .expect("HTTP must still succeed");

    assert_eq!(result.headers.served_model, "physical-weak");
    assert!(has_reason(&result, ReasonCodeKind::GroupJudgeUnavailable));
    assert_eq!(store::judge::count_calls(pipeline.db()).expect("calls"), 1);
    mock.shutdown();
}

#[tokio::test]
async fn none_answer_records_no_match() {
    let mock = MockUpstream::start(vec![judge_reply("none"), Reply::ok("served by weak")]).await;
    let db = Db::open_in_memory().expect("db");
    let pipeline = Pipeline::from_catalog(db, catalog(&mock.base_url(), "active", "", RULES, ""));

    let result = pipeline
        .handle(chat_request("hello"), RequestContext::default())
        .await
        .expect("handle");

    assert_eq!(result.headers.served_model, "physical-weak");
    assert!(has_reason(&result, ReasonCodeKind::GroupJudgeNoMatch));
    assert_eq!(store::judge::count_calls(pipeline.db()).expect("calls"), 1);
    mock.shutdown();
}

#[tokio::test]
async fn group_off_never_calls_judge() {
    let mock = MockUpstream::start(vec![Reply::ok("served by weak")]).await;
    let db = Db::open_in_memory().expect("db");
    let pipeline = Pipeline::from_catalog(db, catalog(&mock.base_url(), "off", "", "", ""));

    let result = pipeline
        .handle(chat_request("hello"), RequestContext::default())
        .await
        .expect("handle");

    assert_eq!(result.headers.served_model, "physical-weak");
    assert_eq!(store::judge::count_calls(pipeline.db()).expect("calls"), 0);
    assert_eq!(mock.remaining(), 0);
    mock.shutdown();
}

#[tokio::test]
async fn active_group_suppresses_global_judge() {
    let global_judge = r#"
[judge]
mode = "active"

[judge.service]
kind = "llm"
endpoint = "http://127.0.0.1:1/v1"
model = "global-judge"
timeout_ms = 2000
"#;
    let mock =
        MockUpstream::start(vec![judge_reply("strong"), Reply::ok("served by strong")]).await;
    let db = Db::open_in_memory().expect("db");
    let pipeline = Pipeline::from_catalog(
        db,
        catalog(&mock.base_url(), "active", "", "", global_judge),
    );

    let result = pipeline
        .handle(failing_request(), RequestContext::default())
        .await
        .expect("handle");

    assert_eq!(result.headers.served_model, "physical-strong");
    // 只有组判断写 judge_calls；全局判官阶段被跳过。
    assert_eq!(store::judge::count_calls(pipeline.db()).expect("calls"), 1);
    assert_eq!(mock.remaining(), 0);
    mock.shutdown();
}

#[tokio::test]
async fn jev_rules_mode_selects_rule_and_records_cost() {
    let mock = MockUpstream::start(vec![
        system_one_reply("coding"),
        Reply::ok("served by strong"),
    ])
    .await;
    let db = Db::open_in_memory().expect("db");
    let endpoint = format!("{}/systemone", mock.base_url());
    let pipeline = Pipeline::from_catalog(
        db,
        catalog_with_service(
            &mock.base_url(),
            "active",
            "",
            RULES,
            "",
            "jev",
            &endpoint,
            "jev-1.13",
        ),
    );

    let result = pipeline
        .handle(
            chat_request("implement a parser"),
            RequestContext::default(),
        )
        .await
        .expect("handle");

    assert_eq!(result.headers.served_model, "physical-strong");
    assert!(has_reason(&result, ReasonCodeKind::GroupJudgeSelected));
    let record = result
        .ledger
        .contributions
        .iter()
        .find(|record| record.strategy_id == "group_judge")
        .expect("group_judge contribution");
    match &record.contribution {
        PlanContribution::Pin { candidate, reason } => {
            assert_eq!(candidate.as_str(), "strong");
            assert_eq!(reason.detail.as_deref(), Some("rule:coding -> strong"));
        }
        other => panic!("expected pin contribution, got {other:?}"),
    }

    // 10 input × 1e6/1e6 + 2 output × 2e6/1e6 = 14 micros。
    let routing = result.ledger.routing_cost.as_ref().expect("routing cost");
    assert_eq!(routing.micros, 14);
    assert_eq!(routing.currency, "USD");
    assert_eq!(store::judge::count_calls(pipeline.db()).expect("calls"), 1);
    assert_eq!(
        store::judge::total_call_cost_micros(pipeline.db()).expect("call cost"),
        14
    );

    // 请求体是 SystemOne shape，不是 chat completions。
    let requests = mock.requests();
    assert_eq!(requests.len(), 2, "judge + upstream");
    let judge_request = &requests[0];
    assert_eq!(judge_request["model"], "jev-1.13");
    assert_eq!(judge_request["state"]["task"], "implement a parser");
    assert_eq!(judge_request["questions"]["route"]["type"], "choice");
    let criteria = &judge_request["questions"]["route"]["criteria"];
    assert_eq!(criteria["coding"], "code implementation and debugging");
    assert!(criteria.get("none").is_some());
    assert!(judge_request.get("messages").is_none());
    assert_eq!(mock.remaining(), 0);
    mock.shutdown();
}

#[tokio::test]
async fn jev_unknown_choice_fails_open() {
    let mock = MockUpstream::start(vec![
        system_one_reply("not-a-rule"),
        Reply::ok("served by weak"),
    ])
    .await;
    let db = Db::open_in_memory().expect("db");
    let endpoint = format!("{}/systemone", mock.base_url());
    let pipeline = Pipeline::from_catalog(
        db,
        catalog_with_service(
            &mock.base_url(),
            "active",
            "",
            RULES,
            "",
            "jev",
            &endpoint,
            "jev-1.13",
        ),
    );

    let result = pipeline
        .handle(
            chat_request("implement a parser"),
            RequestContext::default(),
        )
        .await
        .expect("HTTP must still succeed");

    assert_eq!(result.headers.served_model, "physical-weak");
    assert!(has_reason(&result, ReasonCodeKind::GroupJudgeUnavailable));
    assert!(
        !result
            .ledger
            .contributions
            .iter()
            .any(|record| record.strategy_id == "group_judge")
    );
    assert_eq!(store::judge::count_calls(pipeline.db()).expect("calls"), 1);
    mock.shutdown();
}

#[tokio::test]
async fn jev_5xx_fails_open() {
    let mock = MockUpstream::start(vec![
        Reply::error(503, "server_error", "jev down"),
        Reply::ok("served by weak"),
    ])
    .await;
    let db = Db::open_in_memory().expect("db");
    let endpoint = format!("{}/systemone", mock.base_url());
    let pipeline = Pipeline::from_catalog(
        db,
        catalog_with_service(
            &mock.base_url(),
            "active",
            "",
            RULES,
            "",
            "jev",
            &endpoint,
            "jev-1.13",
        ),
    );

    let result = pipeline
        .handle(
            chat_request("implement a parser"),
            RequestContext::default(),
        )
        .await
        .expect("HTTP must still succeed");

    assert_eq!(result.headers.served_model, "physical-weak");
    assert!(has_reason(&result, ReasonCodeKind::GroupJudgeUnavailable));
    assert_eq!(store::judge::count_calls(pipeline.db()).expect("calls"), 1);
    mock.shutdown();
}

#[tokio::test]
async fn judge_prompt_contains_only_last_user_message() {
    let mock =
        MockUpstream::start(vec![judge_reply("coding"), Reply::ok("served by strong")]).await;
    let db = Db::open_in_memory().expect("db");
    let pipeline = Pipeline::from_catalog(db, catalog(&mock.base_url(), "active", "", RULES, ""));

    let request = CanonicalRequest {
        messages: vec![
            CanonicalMessage {
                role: CanonicalRole::System,
                blocks: vec![ContentBlock::Text {
                    text: "SECRET SYSTEM PROMPT".to_string(),
                    cache_control: None,
                }],
                ..CanonicalMessage::default()
            },
            CanonicalMessage {
                role: CanonicalRole::User,
                blocks: vec![ContentBlock::Text {
                    text: "first user message".to_string(),
                    cache_control: None,
                }],
                ..CanonicalMessage::default()
            },
            CanonicalMessage {
                role: CanonicalRole::User,
                blocks: vec![ContentBlock::Text {
                    text: "last user message".to_string(),
                    cache_control: None,
                }],
                ..CanonicalMessage::default()
            },
        ],
        ..CanonicalRequest::default()
    };

    pipeline
        .handle(request, RequestContext::default())
        .await
        .expect("handle");

    let requests = mock.requests();
    assert_eq!(requests.len(), 2, "judge + upstream");
    let judge_request = requests[0].to_string();
    assert!(
        judge_request.contains("last user message"),
        "{judge_request}"
    );
    assert!(
        !judge_request.contains("first user message"),
        "judge 只应收到最后一条 user 消息: {judge_request}"
    );
    assert!(
        !judge_request.contains("SECRET SYSTEM PROMPT"),
        "judge 不得收到用户 system 内容: {judge_request}"
    );
    // 对照组：真实上游仍然收到 system 提示词（证明不是请求构造错误）。
    let upstream_request = requests[1].to_string();
    assert!(
        upstream_request.contains("SECRET SYSTEM PROMPT"),
        "{upstream_request}"
    );

    mock.shutdown();
}
