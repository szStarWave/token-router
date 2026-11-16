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
    catalog_with(mock_base_url, "")
}

fn catalog_with(mock_base_url: &str, extra: &str) -> Catalog {
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

{extra}
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

fn post_json(uri: &str, body: serde_json::Value) -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri(uri)
        .header("content-type", "application/json")
        .body(Body::from(body.to_string()))
        .expect("request")
}

async fn body_text(response: axum::response::Response) -> String {
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body");
    String::from_utf8_lossy(&bytes).to_string()
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

#[tokio::test]
async fn streaming_chat_returns_sse() {
    let mock = MockUpstream::start(vec![Reply::chat_stream(&["Hi", " there"])]).await;
    let db = Db::open_in_memory().expect("db");
    let pipeline = Arc::new(Pipeline::from_catalog(db, catalog(&mock.base_url())));
    let app = router(AppState::new_shared(pipeline.clone(), AuthMode::Disabled));

    let response = app
        .oneshot(post_json(
            "/v1/chat/completions",
            serde_json::json!({
                "model": "auto",
                "stream": true,
                "messages": [{"role": "user", "content": "hi"}]
            }),
        ))
        .await
        .expect("response");

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()["content-type"], "text/event-stream");
    assert_eq!(
        response.headers()["x-token-router-served-model"],
        "physical-a"
    );

    let text = body_text(response).await;
    assert!(text.contains("data: "), "{text}");
    assert!(text.contains("Hi"), "{text}");
    assert!(text.contains(" there"), "{text}");
    assert!(text.contains("[DONE]"), "{text}");

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
async fn admin_stats_matches_ledger() {
    let mock = MockUpstream::start(vec![Reply::ok("hello")]).await;
    let db = Db::open_in_memory().expect("db");
    let pipeline = Arc::new(Pipeline::from_catalog(db, catalog(&mock.base_url())));
    let app = router(AppState::new_shared(pipeline, AuthMode::Disabled));

    let ok = app
        .clone()
        .oneshot(post_chat(None))
        .await
        .expect("response");
    assert_eq!(ok.status(), StatusCode::OK);

    let response = app
        .oneshot(
            Request::builder()
                .uri("/v2/admin/stats")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("response");
    assert_eq!(response.status(), StatusCode::OK);

    let value: serde_json::Value = serde_json::from_str(&body_text(response).await).expect("json");
    assert_eq!(value["requests"], 1);
    assert_eq!(value["successes"], 1);
    assert_eq!(value["success_rate"], 1.0);
    assert!(value["answer_cost_micros"].as_i64().unwrap_or(0) > 0);

    mock.shutdown();
}

#[tokio::test]
async fn control_api_requires_admin_token_and_redacts_secrets() {
    let mock = MockUpstream::start(vec![Reply::ok("hello")]).await;
    let db = Db::open_in_memory().expect("db");
    let pipeline = Arc::new(Pipeline::from_catalog(db, catalog(&mock.base_url())));

    // 先产生一条账本。
    let chat = router(AppState::new_shared(pipeline.clone(), AuthMode::Disabled));
    let response = chat.oneshot(post_chat(None)).await.expect("chat");
    assert_eq!(response.status(), StatusCode::OK);

    let app = router(AppState::new_shared(pipeline.clone(), AuthMode::Disabled)).merge(
        tr_control::router(tr_control::ControlState::new(
            pipeline,
            Some("admin-secret".to_string()),
        )),
    );

    let denied = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/v2/admin/config")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("response");
    assert_eq!(denied.status(), StatusCode::UNAUTHORIZED);

    let allowed = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/v2/admin/config")
                .header("x-admin-token", "admin-secret")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("response");
    assert_eq!(allowed.status(), StatusCode::OK);
    let value: serde_json::Value = serde_json::from_str(&body_text(allowed).await).expect("json");
    assert_eq!(value["default_virtual_model"], "auto");
    assert!(value["candidates"][0].get("credential_ref").is_some());
    // 配置视图里不能出现密钥值字段。
    assert!(value["candidates"][0].get("credential").is_none());

    let ledger = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/v2/admin/ledger?limit=10")
                .header("x-admin-token", "admin-secret")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("response");
    assert_eq!(ledger.status(), StatusCode::OK);
    let value: serde_json::Value = serde_json::from_str(&body_text(ledger).await).expect("json");
    assert_eq!(value["rows"].as_array().expect("rows").len(), 1);

    let approval = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v2/admin/approvals")
                .header("x-admin-token", "admin-secret")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({
                        "request_id": "req-1",
                        "tool_name": "shell_exec",
                        "risk_class": "high",
                        "decision": "deny"
                    })
                    .to_string(),
                ))
                .expect("request"),
        )
        .await
        .expect("response");
    assert_eq!(approval.status(), StatusCode::OK);

    let approvals = app
        .oneshot(
            Request::builder()
                .uri("/v2/admin/approvals?limit=10")
                .header("x-admin-token", "admin-secret")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("response");
    let value: serde_json::Value = serde_json::from_str(&body_text(approvals).await).expect("json");
    assert_eq!(value["rows"][0]["tool_name"], "shell_exec");

    mock.shutdown();
}

#[tokio::test]
async fn anthropic_endpoint_round_trips() {
    let mock = MockUpstream::start(vec![Reply::ok("hello anthropic")]).await;
    let db = Db::open_in_memory().expect("db");
    let pipeline = Arc::new(Pipeline::from_catalog(db, catalog(&mock.base_url())));
    let app = router(AppState::new_shared(pipeline, AuthMode::Disabled));

    let response = app
        .oneshot(post_json(
            "/v1/messages",
            serde_json::json!({
                "model": "auto",
                "max_tokens": 64,
                "messages": [{"role": "user", "content": [{"type": "text", "text": "hi"}]}]
            }),
        ))
        .await
        .expect("response");

    assert_eq!(response.status(), StatusCode::OK);
    let value: serde_json::Value = serde_json::from_str(&body_text(response).await).expect("json");
    assert_eq!(value["type"], "message");
    assert_eq!(value["content"][0]["text"], "hello anthropic");
    assert_eq!(value["stop_reason"], "end_turn");

    mock.shutdown();
}

#[tokio::test]
async fn responses_endpoint_round_trips() {
    let mock = MockUpstream::start(vec![Reply::ok("hello responses")]).await;
    let db = Db::open_in_memory().expect("db");
    let pipeline = Arc::new(Pipeline::from_catalog(db, catalog(&mock.base_url())));
    let app = router(AppState::new_shared(pipeline, AuthMode::Disabled));

    let response = app
        .oneshot(post_json(
            "/v1/responses",
            serde_json::json!({
                "model": "auto",
                "input": [{
                    "role": "user",
                    "content": [{"type": "input_text", "text": "hi"}]
                }]
            }),
        ))
        .await
        .expect("response");

    assert_eq!(response.status(), StatusCode::OK);
    let value: serde_json::Value = serde_json::from_str(&body_text(response).await).expect("json");
    assert_eq!(value["object"], "response");
    assert_eq!(value["output"][0]["content"][0]["text"], "hello responses");

    mock.shutdown();
}

#[tokio::test]
async fn anthropic_streaming_translates_events() {
    let mock = MockUpstream::start(vec![Reply::chat_stream(&["hello"])]).await;
    let db = Db::open_in_memory().expect("db");
    let pipeline = Arc::new(Pipeline::from_catalog(db, catalog(&mock.base_url())));
    let app = router(AppState::new_shared(pipeline, AuthMode::Disabled));

    let response = app
        .oneshot(post_json(
            "/v1/messages",
            serde_json::json!({
                "model": "auto",
                "max_tokens": 64,
                "stream": true,
                "messages": [{"role": "user", "content": [{"type": "text", "text": "hi"}]}]
            }),
        ))
        .await
        .expect("response");

    assert_eq!(response.status(), StatusCode::OK);
    let text = body_text(response).await;
    assert!(text.contains("event: message_start"), "{text}");
    assert!(text.contains("event: content_block_delta"), "{text}");
    assert!(text.contains("hello"), "{text}");
    assert!(text.contains("event: message_stop"), "{text}");

    mock.shutdown();
}

#[tokio::test]
async fn approval_required_maps_to_428() {
    let mock = MockUpstream::start(vec![Reply::ok("unused")]).await;
    let db = Db::open_in_memory().expect("db");
    let pipeline = Arc::new(Pipeline::from_catalog(
        db,
        catalog_with(
            &mock.base_url(),
            "[approvals]\nmode = \"enforce\"\nmin_risk = \"high\"\n",
        ),
    ));
    let app = router(AppState::new_shared(pipeline.clone(), AuthMode::Disabled));

    let response = app
        .oneshot(post_json(
            "/v1/chat/completions",
            serde_json::json!({
                "model": "auto",
                "messages": [
                    {"role": "user", "content": "run it"},
                    {
                        "role": "assistant",
                        "content": null,
                        "tool_calls": [{
                            "id": "call-1",
                            "type": "function",
                            "function": {"name": "shell_exec", "arguments": "{}"}
                        }]
                    }
                ]
            }),
        ))
        .await
        .expect("response");

    assert_eq!(response.status(), StatusCode::PRECONDITION_REQUIRED);
    assert_eq!(response.headers()["x-token-router-approval"], "required");
    assert_eq!(response.headers()["x-token-router-tool"], "shell_exec");

    let text = body_text(response).await;
    let value: serde_json::Value = serde_json::from_str(&text).expect("json");
    assert_eq!(value["error"]["type"], "approval_required");
    assert_eq!(value["error"]["tool"], "shell_exec");
    assert_eq!(value["error"]["risk_class"], "high");

    // 本地拒绝也入账；上游未被访问。
    assert_eq!(store::ledger::count(pipeline.db()).expect("count"), 1);
    assert_eq!(mock.remaining(), 1);

    mock.shutdown();
}

#[tokio::test]
async fn unknown_model_returns_400_with_code() {
    let mock = MockUpstream::start(vec![Reply::ok("unused")]).await;
    let db = Db::open_in_memory().expect("db");
    let pipeline = Arc::new(Pipeline::from_catalog(db, catalog(&mock.base_url())));
    let app = router(AppState::new_shared(pipeline, AuthMode::Disabled));

    let response = app
        .oneshot(post_json(
            "/v1/chat/completions",
            serde_json::json!({
                "model": "nope",
                "messages": [{"role": "user", "content": "hi"}]
            }),
        ))
        .await
        .expect("response");

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let text = body_text(response).await;
    let value: serde_json::Value = serde_json::from_str(&text).expect("json");
    assert_eq!(value["error"]["code"], "MODEL_NOT_FOUND");

    mock.shutdown();
}
