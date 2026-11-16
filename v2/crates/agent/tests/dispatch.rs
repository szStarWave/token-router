//! P3 sticky / ContextHold 的 pipeline 集成测试。

use mock_upstream::{MockUpstream, Reply};
use tr_agent::config::{Catalog, ConfigFile};
use tr_agent::pipeline::{Pipeline, RequestContext};
use tr_agent::store::Db;
use tr_contracts::{
    CanonicalMessage, CanonicalRequest, CanonicalRole, ContentBlock, RequestReason,
};

fn catalog(mock_base_url: &str) -> Catalog {
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
"#
    );
    ConfigFile::parse(&toml)
        .expect("parse")
        .into_catalog()
        .expect("catalog")
}

fn request(texts: &[&str]) -> CanonicalRequest {
    CanonicalRequest {
        messages: texts
            .iter()
            .map(|text| CanonicalMessage {
                role: CanonicalRole::User,
                blocks: vec![ContentBlock::Text {
                    text: (*text).to_string(),
                    cache_control: None,
                }],
                ..CanonicalMessage::default()
            })
            .collect(),
        ..CanonicalRequest::default()
    }
}

#[tokio::test]
async fn continuation_reports_sticky_previous() {
    let mock = MockUpstream::start(vec![Reply::ok("one"), Reply::ok("two")]).await;
    let db = Db::open_in_memory().expect("db");
    let pipeline = Pipeline::from_catalog(db, catalog(&mock.base_url()));

    let first = pipeline
        .handle(request(&["hello"]), RequestContext::default())
        .await
        .expect("first");
    assert_eq!(first.headers.served_model, "physical-weak");
    assert_eq!(first.headers.dispatch_source, "strategy");

    let second = pipeline
        .handle(
            request(&["hello"]),
            RequestContext {
                request_reason: Some(RequestReason::Continuation),
                ..RequestContext::default()
            },
        )
        .await
        .expect("second");
    assert_eq!(second.headers.dispatch_source, "sticky_previous");
    assert_eq!(second.headers.served_model, "physical-weak");
    assert!(
        second.headers.request_reason == "continuation",
        "{}",
        second.headers.request_reason
    );

    mock.shutdown();
}

#[tokio::test]
async fn user_turn_reports_context_hold() {
    let mock = MockUpstream::start(vec![Reply::ok("one"), Reply::ok("two")]).await;
    let db = Db::open_in_memory().expect("db");
    let pipeline = Pipeline::from_catalog(db, catalog(&mock.base_url()));

    let first = pipeline
        .handle(request(&["hello"]), RequestContext::default())
        .await
        .expect("first");
    assert_eq!(first.headers.dispatch_source, "strategy");

    // 同一会话（首条 user 相同），第二轮 User → ContextHold 软亲和。
    let second = pipeline
        .handle(request(&["hello", "again"]), RequestContext::default())
        .await
        .expect("second");
    assert_eq!(second.headers.dispatch_source, "context_hold");
    assert!(second.headers.reason_codes.contains("CONTEXT_HOLD_APPLIED"));

    mock.shutdown();
}

#[tokio::test]
async fn direct_reports_strategy() {
    let mock = MockUpstream::start(vec![Reply::ok("one")]).await;
    let db = Db::open_in_memory().expect("db");
    let pipeline = Pipeline::from_catalog(db, catalog(&mock.base_url()));

    // Direct 不读写 state：即使没有 previous，也走 Plan。
    let result = pipeline
        .handle(
            request(&["hello"]),
            RequestContext {
                request_reason: Some(RequestReason::Direct),
                ..RequestContext::default()
            },
        )
        .await
        .expect("direct");
    assert_eq!(result.headers.dispatch_source, "strategy");

    mock.shutdown();
}
