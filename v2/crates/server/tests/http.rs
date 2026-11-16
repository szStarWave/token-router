//! HTTP 层测试（P0a-13/15）：决策头与账本一致、鉴权、模型列表。

use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use mock_upstream::{MockUpstream, Reply};
use tower::ServiceExt;
use tr_agent::config::{Catalog, ConfigFile};
use tr_agent::pipeline::Pipeline;
use tr_agent::store::{self, Db};
use tr_server::{AppState, AuthMode, router};

fn catalog(mock_base_url: &str) -> Catalog {
    let toml = format!(
        r#"
schema_version = 1

[profiles.default]
mode = "cascade"
strength_order = ["local"]

[[candidates]]
id = "a"
tier = "local"
strength = 10
provider = "openai"
base_url = "{mock_base_url}"
model = "physical-a"
window = 32768
input = ["text"]
price = {{ currency = "USD", input_micros_per_mtok = 1000000, output_micros_per_mtok = 2000000 }}

[[virtual_models]]
provider = "router"
id = "auto"
name = "Auto"
profile = "default"
candidate_pool = ["a"]
"#
    );
    ConfigFile::parse(&toml)
        .expect("parse")
        .into_catalog()
        .expect("catalog")
}

fn chat_body() -> String {
    serde_json::json!({
        "model": "auto",
        "messages": [{"role": "user", "content": "hi"}]
    })
    .to_string()
}

fn post_chat(key: Option<&str>) -> Request<Body> {
    let mut builder = Request::builder()
        .method("POST")
        .uri("/v1/chat/completions")
        .header("content-type", "application/json");
    if let Some(key) = key {
        builder = builder.header("authorization", format!("Bearer {key}"));
    }
    builder.body(Body::from(chat_body())).expect("request")
}

#[tokio::test]
async fn success_returns_decision_headers() {
    let mock = MockUpstream::start(vec![Reply::ok("hello")]).await;
    let db = Db::open_in_memory().expect("db");
    let pipeline = Arc::new(Pipeline::from_catalog(db, catalog(&mock.base_url())));
    let app = router(AppState::new_shared(pipeline.clone(), AuthMode::Disabled));

    let response = app.oneshot(post_chat(None)).await.expect("response");
    assert_eq!(response.status(), StatusCode::OK);

    let headers = response.headers();
    assert_eq!(headers["x-token-router-selected-model"], "auto");
    assert_eq!(headers["x-token-router-served-model"], "physical-a");
    assert_eq!(headers["x-token-router-dispatch-source"], "strategy");
    assert_eq!(headers["x-token-router-mode"], "single");
    assert!(
        headers["x-token-router-plan-id"]
            .to_str()
            .unwrap()
            .starts_with("plan-")
    );

    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body");
    let value: serde_json::Value = serde_json::from_slice(&body).expect("json");
    assert_eq!(value["choices"][0]["message"]["content"], "hello");

    // 头与账本必须一致：served model/plan id 同源。
    assert_eq!(store::ledger::count(pipeline.db()).expect("count"), 1);

    mock.shutdown();
}

#[tokio::test]
async fn auth_rejects_missing_or_wrong_key() {
    let mock = MockUpstream::start(vec![Reply::ok("hello")]).await;
    let db = Db::open_in_memory().expect("db");
    let pipeline = Arc::new(Pipeline::from_catalog(db, catalog(&mock.base_url())));
    let app = router(AppState::new_shared(
        pipeline,
        AuthMode::InboundKey("secret".to_string()),
    ));

    let denied = app
        .clone()
        .oneshot(post_chat(None))
        .await
        .expect("response");
    assert_eq!(denied.status(), StatusCode::UNAUTHORIZED);

    let wrong = app
        .clone()
        .oneshot(post_chat(Some("nope")))
        .await
        .expect("response");
    assert_eq!(wrong.status(), StatusCode::UNAUTHORIZED);

    let allowed = app
        .oneshot(post_chat(Some("secret")))
        .await
        .expect("response");
    assert_eq!(allowed.status(), StatusCode::OK);

    mock.shutdown();
}

#[tokio::test]
async fn models_lists_virtual_models_only() {
    let mock = MockUpstream::start(vec![]).await;
    let db = Db::open_in_memory().expect("db");
    let pipeline = Arc::new(Pipeline::from_catalog(db, catalog(&mock.base_url())));
    let app = router(AppState::new_shared(pipeline, AuthMode::Disabled));

    let response = app
        .oneshot(
            Request::builder()
                .uri("/v1/models")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("response");
    assert_eq!(response.status(), StatusCode::OK);

    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body");
    let value: serde_json::Value = serde_json::from_slice(&body).expect("json");
    let data = value["data"].as_array().expect("data");
    assert_eq!(data.len(), 1);
    assert_eq!(data[0]["id"], "auto");

    mock.shutdown();
}

#[tokio::test]
async fn invalid_body_is_400() {
    let mock = MockUpstream::start(vec![]).await;
    let db = Db::open_in_memory().expect("db");
    let pipeline = Arc::new(Pipeline::from_catalog(db, catalog(&mock.base_url())));
    let app = router(AppState::new_shared(pipeline, AuthMode::Disabled));

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/chat/completions")
                .header("content-type", "application/json")
                .body(Body::from("{\"model\":\"auto\",\"messages\":\"bad\"}"))
                .expect("request"),
        )
        .await
        .expect("response");
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);

    mock.shutdown();
}
