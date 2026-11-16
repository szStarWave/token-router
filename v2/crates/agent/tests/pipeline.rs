//! P0a 垂直切片 e2e：真实 HTTP → mock 上游 → 决策 → 账本 → 会话（§13 P0a）。
//!
//! 覆盖：
//! - 单协议非流式成功；
//! - 错误分类驱动的接力（503 → 下一个候选）；
//! - 失败也写账本；
//! - 熔断计数与成功重置。

use mock_upstream::{MockUpstream, Reply};
use tr_agent::config::{Catalog, ConfigFile};
use tr_agent::pipeline::{Pipeline, PipelineError, RequestContext, StreamChunk};
use tr_agent::store::{self, Db};
use tr_contracts::{
    CanonicalMessage, CanonicalRequest, CanonicalRole, CanonicalStreamEvent, ContentBlock,
    ErrorClass, OutcomeKind,
};

fn catalog(mock_base_url: &str, candidate_ids: &[&str]) -> Catalog {
    let candidates = candidate_ids
        .iter()
        .enumerate()
        .map(|(index, id)| {
            format!(
                r#"
[[candidates]]
id = "{id}"
tier = "{}"
strength = {}
provider = "openai"
base_url = "{mock_base_url}"
model = "physical-{id}"
window = 32768
input = ["text"]
price = {{ currency = "USD", input_micros_per_mtok = 1000000, output_micros_per_mtok = 2000000 }}
"#,
                if index == 0 { "local" } else { "cloud" },
                10 + index as u8 * 10
            )
        })
        .collect::<String>();

    let pool = candidate_ids
        .iter()
        .map(|id| format!("\"{id}\""))
        .collect::<Vec<_>>()
        .join(", ");

    let toml = format!(
        r#"
schema_version = 1

[profiles.default]
mode = "cascade"
strength_order = ["local", "cloud"]

{candidates}

[[virtual_models]]
provider = "router"
id = "auto"
name = "Auto"
profile = "default"
candidate_pool = [{pool}]
"#
    );

    ConfigFile::parse(&toml)
        .expect("parse config")
        .into_catalog()
        .expect("build catalog")
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

#[tokio::test]
async fn success_writes_ledger_and_session() {
    let mock = MockUpstream::start(vec![Reply::ok("hello from mock")]).await;
    let db = Db::open_in_memory().expect("db");
    let pipeline = Pipeline::from_catalog(db, catalog(&mock.base_url(), &["a"]));

    let result = pipeline
        .handle(chat_request("hi"), RequestContext::default())
        .await
        .expect("handle");

    let text = result
        .response
        .blocks
        .iter()
        .filter_map(|block| match block {
            ContentBlock::Text { text, .. } => Some(text.as_str()),
            _ => None,
        })
        .collect::<String>();
    assert_eq!(text, "hello from mock");
    assert_eq!(result.headers.served_model, "physical-a");
    assert_eq!(result.headers.dispatch_source, "strategy");
    assert_eq!(result.ledger.outcome, OutcomeKind::Success);
    assert_eq!(result.ledger.input_tokens, Some(12));
    assert!(result.ledger.answer_cost.as_ref().expect("cost").micros > 0);
    assert_eq!(result.ledger.switches, 0);

    let db = pipeline.db();
    assert_eq!(store::ledger::count(db).expect("count"), 1);

    let facts = store::sessions::load(db, &result.ledger.session_key, "main")
        .expect("load")
        .expect("session exists");
    let dispatch = facts.last_successful_dispatch.expect("dispatch");
    assert_eq!(dispatch.physical_model, "physical-a");
    assert_eq!(facts.last_prompt_tokens, Some(12));
    assert_eq!(facts.version, 1);

    assert_eq!(mock.remaining(), 0);
    mock.shutdown();
}

#[tokio::test]
async fn overloaded_fails_over_and_counts_breaker() {
    let mock = MockUpstream::start(vec![
        Reply::error(503, "overloaded_error", "server busy"),
        Reply::ok("second candidate"),
    ])
    .await;
    let db = Db::open_in_memory().expect("db");
    let pipeline = Pipeline::from_catalog(db, catalog(&mock.base_url(), &["a", "b"]));

    let result = pipeline
        .handle(chat_request("hi"), RequestContext::default())
        .await
        .expect("handle");

    assert_eq!(result.headers.served_model, "physical-b");
    assert_eq!(result.ledger.switches, 1);
    assert_eq!(result.ledger.attempts.len(), 2);
    assert_eq!(
        result.ledger.attempts[0].error_class,
        Some(ErrorClass::Overloaded)
    );

    let health = store::health::load(pipeline.db()).expect("health");
    let breaker = health.state_of("a").expect("breaker a");
    // Overloaded 权重 2：一次失败记 2 次，尚未达到 3 次阈值。
    assert_eq!(breaker.consecutive_failures, 2);
    assert_eq!(breaker.last_error_class, Some(ErrorClass::Overloaded));
    assert!(!breaker.is_open(0));

    mock.shutdown();
}

#[tokio::test]
async fn upstream_failure_still_writes_ledger() {
    let mock = MockUpstream::start(vec![Reply::error(
        400,
        "invalid_request_error",
        "messages: invalid",
    )])
    .await;
    let db = Db::open_in_memory().expect("db");
    let pipeline = Pipeline::from_catalog(db, catalog(&mock.base_url(), &["a"]));

    let error = pipeline
        .handle(chat_request("hi"), RequestContext::default())
        .await
        .expect_err("must fail");
    match error {
        PipelineError::Upstream { class, status, .. } => {
            assert_eq!(class, ErrorClass::InvalidRequest);
            assert_eq!(status, Some(400));
        }
        other => panic!("unexpected error: {other}"),
    }

    assert_eq!(store::ledger::count(pipeline.db()).expect("count"), 1);

    // InvalidRequest 不是候选故障：熔断不计数。
    let health = store::health::load(pipeline.db()).expect("health");
    let breaker = health.state_of("a").expect("breaker a");
    assert_eq!(breaker.consecutive_failures, 0);

    mock.shutdown();
}

#[tokio::test]
async fn unknown_virtual_model_is_rejected_before_upstream() {
    let mock = MockUpstream::start(vec![Reply::ok("unused")]).await;
    let db = Db::open_in_memory().expect("db");
    let pipeline = Pipeline::from_catalog(db, catalog(&mock.base_url(), &["a"]));

    let mut request = chat_request("hi");
    request.meta.virtual_model = Some("nope".to_string());

    let error = pipeline
        .handle(request, RequestContext::default())
        .await
        .expect_err("must fail");
    assert!(matches!(error, PipelineError::ModelNotFound(model) if model == "nope"));

    assert_eq!(store::ledger::count(pipeline.db()).expect("count"), 0);
    assert_eq!(mock.remaining(), 1, "不应访问上游");
    mock.shutdown();
}

fn streaming_request(text: &str) -> CanonicalRequest {
    let mut request = chat_request(text);
    request.output.stream = true;
    request
}

#[tokio::test]
async fn stream_forwards_events_and_writes_ledger() {
    let mock = MockUpstream::start(vec![Reply::chat_stream(&["Hello", " world"])]).await;
    let db = Db::open_in_memory().expect("db");
    let pipeline = Pipeline::from_catalog(db, catalog(&mock.base_url(), &["a"]));

    let mut streaming = pipeline
        .handle_stream(streaming_request("hi"), RequestContext::default())
        .await
        .expect("stream");

    assert_eq!(streaming.headers.served_model, "physical-a");

    let mut events = Vec::new();
    while let Some(chunk) = streaming.events.recv().await {
        match chunk {
            StreamChunk::Event(event) => events.push(event),
            StreamChunk::Done => break,
            StreamChunk::Error { message } => panic!("stream error: {message}"),
        }
    }

    let text: String = events
        .iter()
        .filter_map(|event| match event {
            CanonicalStreamEvent::ContentDelta { text } => Some(text.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(text, "Hello world");

    // 账本在流结束后由后台任务提交；给一点时间。
    for _ in 0..100 {
        if store::ledger::count(pipeline.db()).expect("count") == 1 {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    assert_eq!(store::ledger::count(pipeline.db()).expect("count"), 1);

    mock.shutdown();
}

#[tokio::test]
async fn stream_fails_over_before_first_event() {
    let mock = MockUpstream::start(vec![
        Reply::error(503, "overloaded_error", "busy"),
        Reply::chat_stream(&["served by b"]),
    ])
    .await;
    let db = Db::open_in_memory().expect("db");
    let pipeline = Pipeline::from_catalog(db, catalog(&mock.base_url(), &["a", "b"]));

    let mut streaming = pipeline
        .handle_stream(streaming_request("hi"), RequestContext::default())
        .await
        .expect("stream");
    assert_eq!(streaming.headers.served_model, "physical-b");

    let mut text = String::new();
    while let Some(chunk) = streaming.events.recv().await {
        match chunk {
            StreamChunk::Event(CanonicalStreamEvent::ContentDelta { text: delta }) => {
                text.push_str(&delta);
            }
            StreamChunk::Done => break,
            _ => {}
        }
    }
    assert_eq!(text, "served by b");

    mock.shutdown();
}

#[tokio::test]
async fn missing_previous_response_id_degrades_but_succeeds() {
    let mock = MockUpstream::start(vec![Reply::ok("ok")]).await;
    let db = Db::open_in_memory().expect("db");
    let pipeline = Pipeline::from_catalog(db, catalog(&mock.base_url(), &["a"]));

    let mut request = chat_request("hi");
    request.meta.ingress_protocol = tr_contracts::IngressProtocol::Responses;
    request.meta.previous_response_id = Some("resp_missing".to_string());

    let result = pipeline
        .handle(request, RequestContext::default())
        .await
        .expect("must degrade, not fail");

    assert!(
        result
            .ledger
            .reason_codes
            .iter()
            .any(|reason| reason.code == tr_contracts::ReasonCodeKind::ProtocolStateMissing)
    );

    mock.shutdown();
}

#[tokio::test]
async fn responses_success_stores_protocol_state() {
    let mock = MockUpstream::start(vec![Reply::ok("stored")]).await;
    let db = Db::open_in_memory().expect("db");
    let pipeline = Pipeline::from_catalog(db, catalog(&mock.base_url(), &["a"]));

    let mut request = chat_request("hi");
    request.meta.ingress_protocol = tr_contracts::IngressProtocol::Responses;

    let result = pipeline
        .handle(request, RequestContext::default())
        .await
        .expect("handle");
    let key = format!("resp:{}", result.response.id);
    let payload = store::protocol::load(pipeline.db(), &key, now_secs())
        .expect("load")
        .expect("protocol state saved");
    assert_eq!(payload["id"], result.response.id);

    mock.shutdown();
}

#[tokio::test]
async fn new_virtual_model_only_needs_config() {
    let mock = MockUpstream::start(vec![Reply::ok("premium answer")]).await;
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
base_url = "{base}"
model = "physical-weak"
window = 32768
input = ["text"]

[[candidates]]
id = "strong"
tier = "cloud"
strength = 80
provider = "openai"
base_url = "{base}"
model = "physical-strong"
window = 128000
input = ["text"]

[[virtual_models]]
provider = "router"
id = "auto"
name = "Auto"
profile = "default"
candidate_pool = ["weak", "strong"]

# 新增虚拟模型只改配置：单候选池 + 固定 profile。
[[virtual_models]]
provider = "router"
id = "premium"
name = "Premium"
profile = "default"
candidate_pool = ["strong"]
"#,
        base = mock.base_url()
    );
    let catalog = ConfigFile::parse(&toml)
        .expect("parse")
        .into_catalog()
        .expect("catalog");
    let db = Db::open_in_memory().expect("db");
    let pipeline = Pipeline::from_catalog(db, catalog);

    let mut request = chat_request("hi");
    request.meta.virtual_model = Some("premium".to_string());
    let result = pipeline
        .handle(request, RequestContext::default())
        .await
        .expect("handle");

    assert_eq!(result.headers.selected_model, "premium");
    assert_eq!(result.headers.served_model, "physical-strong");

    mock.shutdown();
}

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
}
