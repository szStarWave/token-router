//! 账户余额查询回归：厂商 usage 接口的成功解析与「无接口」诚实空态。

use std::sync::Arc;

use axum::http::StatusCode;
use axum::routing::get;
use axum::{Json, Router};
use serde_json::{Value, json};
use tr_agent::config::{Catalog, ConfigFile};
use tr_agent::pipeline::Pipeline;
use tr_agent::store::{self, Db};
use tr_contracts::{AccountInfo, ProviderKindDto, UsageStatus};
use tr_control::AdminAuth;
use tr_control::provider_usage;
use tr_control::state::ControlState;

const BASE: &str = r#"
schema_version = 1

[profiles.default]
mode = "single"

[[candidates]]
id = "c1"
tier = "local"
strength = 10
provider = "openai"
base_url = "http://127.0.0.1:9/v1"
model = "m1"
window = 32768
input = ["text"]

[[virtual_models]]
provider = "router"
id = "auto"
name = "Auto"
profile = "default"
candidate_pool = ["c1"]
"#;

async fn mock_upstream() -> (String, tokio::task::JoinHandle<()>) {
    async fn deepseek() -> Json<Value> {
        Json(json!({
            "is_available": true,
            "balance_infos": [{
                "currency": "CNY",
                "total_balance": "12.50",
                "granted_balance": "2.50",
                "topped_up_balance": "10.00"
            }]
        }))
    }
    let app = Router::new().route("/user/balance", get(deepseek));
    serve(app).await
}

async fn mock_billing() -> (String, tokio::task::JoinHandle<()>) {
    async fn subscription() -> Json<Value> {
        Json(json!({"hard_limit_usd": 20.0}))
    }
    async fn usage() -> Json<Value> {
        Json(json!({"total_usage": 1234}))
    }
    let app = Router::new()
        .route("/v1/dashboard/billing/subscription", get(subscription))
        .route("/v1/dashboard/billing/usage", get(usage));
    serve(app).await
}

async fn mock_not_found() -> (String, tokio::task::JoinHandle<()>) {
    let app = Router::new().fallback(|| async { StatusCode::NOT_FOUND });
    serve(app).await
}

async fn mock_opencode() -> (String, tokio::task::JoinHandle<()>) {
    async fn usage() -> Json<Value> {
        Json(json!({
            "usage": {
                "rolling": {
                    "status": "ok",
                    "percent": 12.5,
                    "resetsAt": "2026-11-23T10:00:00Z"
                },
                "weekly": {
                    "status": "rate-limited",
                    "percent": 100,
                    "resetsAt": 1_795_420_000_000i64
                },
                "monthly": {
                    "status": "ok",
                    "percent": 40,
                    "resetsAt": 1_796_000_000_000i64
                }
            }
        }))
    }
    let app = Router::new().route("/zen/go/v1/usage", get(usage));
    serve(app).await
}

async fn serve(app: Router) -> (String, tokio::task::JoinHandle<()>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind mock");
    let addr = listener.local_addr().expect("addr");
    let handle = tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    (format!("http://{addr}/v1"), handle)
}

fn state(template_id: Option<&str>, base_url: &str) -> (ControlState, String) {
    let catalog: Catalog = ConfigFile::parse(BASE)
        .expect("parse")
        .into_catalog()
        .expect("catalog");
    let db = Db::open_in_memory().expect("db");
    store::providers::insert_provider(
        &db,
        "p1",
        "Mock Provider",
        ProviderKindDto::OpenAiCompatible,
        base_url,
        None,
        template_id,
    )
    .expect("insert provider");

    let dir = std::env::temp_dir().join(format!("tr-usage-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).expect("temp dir");
    let key_path = dir.join("acc.key");
    std::fs::write(&key_path, "sk-test").expect("key file");

    store::providers::insert_account(
        &db,
        &AccountInfo {
            id: "acc1".to_string(),
            provider_id: "p1".to_string(),
            label: "main".to_string(),
            credential_ref: format!("file:{}", key_path.display()),
            fingerprint: "abcd".to_string(),
            proxy_url: None,
            base_url_override: None,
            extra_headers: None,
            enabled: true,
            verified_at: None,
            verify_status: None,
            created_at: 0,
        },
    )
    .expect("insert account");

    let pipeline = Arc::new(Pipeline::from_catalog(db, catalog));
    let state = ControlState::new(
        pipeline,
        AdminAuth::new(Arc::new(Db::open_in_memory().expect("auth db")), None),
    )
    .with_config_path(dir.join("config.toml"));
    (state, dir.display().to_string())
}

#[tokio::test]
async fn deepseek_usage_is_queried_and_parsed() {
    let (base_url, server) = mock_upstream().await;
    let (state, dir) = state(Some("deepseek"), &base_url);

    let response = provider_usage::list_account_usage(&state, true)
        .await
        .expect("usage");
    assert_eq!(response.accounts.len(), 1);
    let account = &response.accounts[0];
    assert_eq!(
        account.status,
        UsageStatus::Ok,
        "error: {:?}",
        account.error
    );
    assert_eq!(account.provider_id, "p1");
    assert_eq!(account.balances[0].currency, "CNY");
    assert_eq!(account.balances[0].available.as_deref(), Some("12.50"));
    assert_eq!(account.balances[0].granted.as_deref(), Some("2.50"));
    assert_eq!(account.balances[0].topped_up.as_deref(), Some("10.00"));
    assert!(account.error.is_none());

    server.abort();
    let _ = std::fs::remove_dir_all(dir);
}

#[tokio::test]
async fn generic_billing_endpoint_is_used_when_vendor_has_no_adapter() {
    let (base_url, server) = mock_billing().await;
    let (state, dir) = state(None, &base_url);

    let response = provider_usage::list_account_usage(&state, true)
        .await
        .expect("usage");
    let account = &response.accounts[0];
    assert_eq!(
        account.status,
        UsageStatus::Ok,
        "error: {:?}",
        account.error
    );
    assert_eq!(account.balances[0].currency, "USD");
    assert_eq!(account.balances[0].total.as_deref(), Some("20"));
    assert_eq!(account.balances[0].used.as_deref(), Some("12.34"));
    assert_eq!(account.balances[0].available.as_deref(), Some("7.66"));

    server.abort();
    let _ = std::fs::remove_dir_all(dir);
}

#[tokio::test]
async fn opencode_go_windows_are_queried() {
    let (base_url, server) = mock_opencode().await;
    let (state, dir) = state(Some("opencode"), &base_url);

    let response = provider_usage::list_account_usage(&state, true)
        .await
        .expect("usage");
    let account = &response.accounts[0];
    assert_eq!(
        account.status,
        UsageStatus::Ok,
        "error: {:?}",
        account.error
    );
    assert_eq!(account.plan.as_deref(), Some("opencode_go"));
    assert_eq!(account.windows.len(), 3);
    assert_eq!(account.windows[0].label, "rolling");
    assert_eq!(account.windows[0].percent, 12.5);
    assert!(account.windows[0].resets_at.is_some());
    assert!(account.windows[1].limited);

    server.abort();
    let _ = std::fs::remove_dir_all(dir);
}

#[tokio::test]
async fn provider_without_usage_api_is_unsupported() {
    let (base_url, server) = mock_not_found().await;
    let (state, dir) = state(None, &base_url);

    let response = provider_usage::list_account_usage(&state, true)
        .await
        .expect("usage");
    assert_eq!(response.accounts.len(), 1);
    assert_eq!(response.accounts[0].status, UsageStatus::Unsupported);
    assert!(response.accounts[0].balances.is_empty());
    assert!(
        response.accounts[0]
            .error
            .as_deref()
            .unwrap_or_default()
            .contains("未提供"),
        "reason: {:?}",
        response.accounts[0].error
    );

    server.abort();
    let _ = std::fs::remove_dir_all(dir);
}
