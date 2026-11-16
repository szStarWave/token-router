//! typed admin API（P3-15）：配置只读视图、账本查询、shadow 报告、审批记录。
//!
//! 安全边界：
//! - 不返回任何密钥：凭据只显示引用类型（`env:`/`file:`），不显示值；
//! - 写端点（审批）在配置了 `admin_token` 时必须带 `x-admin-token`；
//! - 不提供“在线改配置”：配置变更走文件 + 重启，避免半加载状态。

#![forbid(unsafe_code)]

use std::sync::Arc;

use axum::extract::{Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::{Value, json};

use tr_agent::pipeline::Pipeline;

/// 控制面状态。
#[derive(Clone)]
pub struct ControlState {
    pub pipeline: Arc<Pipeline>,
    pub admin_token: Option<String>,
}

impl ControlState {
    pub fn new(pipeline: Arc<Pipeline>, admin_token: Option<String>) -> Self {
        Self {
            pipeline,
            admin_token,
        }
    }
}

/// 控制面路由（挂在 `/v2/admin` 下）。
pub fn router(state: ControlState) -> Router {
    Router::new()
        .route("/v2/admin/config", get(config))
        .route("/v2/admin/ledger", get(ledger))
        .route("/v2/admin/shadow", get(shadow))
        .route("/v2/admin/trust-tiers", get(trust_tiers))
        .route("/v2/admin/approvals", get(approvals).post(record_approval))
        .with_state(state)
}

#[derive(Debug, Deserialize)]
struct LimitQuery {
    #[serde(default = "default_limit")]
    limit: u32,
}

fn default_limit() -> u32 {
    50
}

async fn config(State(state): State<ControlState>, headers: HeaderMap) -> Response {
    if let Some(response) = require_admin(&state, &headers) {
        return response;
    }
    let catalog = state.pipeline.catalog();
    let candidates: Vec<Value> = catalog
        .candidates
        .values()
        .map(|candidate| {
            json!({
                "id": candidate.id,
                "tier": candidate.tier,
                "strength": candidate.strength,
                "provider": format!("{:?}", candidate.upstream.provider),
                "model": candidate.upstream.model,
                "window": candidate.window,
                "enabled": candidate.enabled,
                "credential_ref": candidate
                    .upstream
                    .credential
                    .as_ref()
                    .map(|reference| reference.kind().to_string()),
            })
        })
        .collect();
    let virtual_models: Vec<Value> = catalog
        .virtual_models
        .values()
        .map(|model| {
            json!({
                "id": model.id,
                "provider": model.provider,
                "profile": model.profile,
                "candidate_pool": model.candidate_pool,
                "strategies": model.strategies,
                "thinking_levels": model.thinking_levels,
            })
        })
        .collect();

    Json(json!({
        "config_digest": catalog.config_digest,
        "default_virtual_model": catalog.default_virtual_model,
        "server": {
            "listen": catalog.server.listen,
            "auth_enabled": catalog.server.auth_enabled,
            "auth_mode": catalog.server.auth_mode,
        },
        "candidates": candidates,
        "virtual_models": virtual_models,
        "judge_mode": format!("{:?}", catalog.judge.mode),
        "cost_gate": format!("{:?}", catalog.cost_gate),
        "deprecations": [],
    }))
    .into_response()
}

async fn ledger(
    State(state): State<ControlState>,
    headers: HeaderMap,
    Query(query): Query<LimitQuery>,
) -> Response {
    if let Some(response) = require_admin(&state, &headers) {
        return response;
    }
    match tr_agent::store::ledger::recent(state.pipeline.db(), query.limit) {
        Ok(rows) => Json(json!({"rows": rows})).into_response(),
        Err(error) => {
            eprintln!("ledger query failed: {error}");
            error_response(StatusCode::INTERNAL_SERVER_ERROR, "ledger query failed")
        }
    }
}

/// 信任档位映射（P3-14）：只展示别名 → profile，不暴露内核策略。
async fn trust_tiers(State(state): State<ControlState>, headers: HeaderMap) -> Response {
    if let Some(response) = require_admin(&state, &headers) {
        return response;
    }
    let catalog = state.pipeline.catalog();
    let tiers: Vec<Value> = catalog
        .trust_tiers
        .iter()
        .map(|(alias, profile_id)| {
            let profile = catalog.profiles.get(profile_id);
            json!({
                "alias": alias,
                "profile": profile_id,
                "revision": profile.map(|profile| profile.revision.clone()),
                "mode": profile.map(|profile| profile.mode.as_str()),
            })
        })
        .collect();
    Json(json!({"tiers": tiers})).into_response()
}

async fn shadow(State(state): State<ControlState>, headers: HeaderMap) -> Response {
    if let Some(response) = require_admin(&state, &headers) {
        return response;
    }
    match tr_agent::store::judge::shadow_summary(state.pipeline.db()) {
        Ok(summary) => Json(json!({
            "total": summary.total,
            "agreed": summary.agreed,
            "disagreed": summary.disagreed,
            "unknown": summary.unknown,
            "agreement_rate": summary.agreement_rate(),
            "avg_latency_ms": summary.avg_latency_ms,
        }))
        .into_response(),
        Err(error) => {
            eprintln!("shadow query failed: {error}");
            error_response(StatusCode::INTERNAL_SERVER_ERROR, "shadow query failed")
        }
    }
}

#[derive(Debug, Deserialize)]
struct ApprovalBody {
    request_id: String,
    #[serde(default)]
    session_key: Option<String>,
    tool_name: String,
    risk_class: String,
    decision: String,
    #[serde(default)]
    operator: Option<String>,
}

async fn record_approval(
    State(state): State<ControlState>,
    headers: HeaderMap,
    Json(body): Json<ApprovalBody>,
) -> Response {
    if let Some(response) = require_admin(&state, &headers) {
        return response;
    }
    let record = tr_agent::store::approvals::ApprovalRecord {
        request_id: body.request_id,
        session_key: body.session_key,
        tool_name: body.tool_name,
        risk_class: body.risk_class,
        decision: body.decision,
        operator: body.operator,
        wait_ms: None,
        created_at: now_secs(),
    };
    match tr_agent::store::approvals::insert(state.pipeline.db(), &record) {
        Ok(()) => Json(json!({"ok": true})).into_response(),
        Err(error) => {
            eprintln!("approval insert failed: {error}");
            error_response(StatusCode::INTERNAL_SERVER_ERROR, "approval insert failed")
        }
    }
}

async fn approvals(
    State(state): State<ControlState>,
    headers: HeaderMap,
    Query(query): Query<LimitQuery>,
) -> Response {
    if let Some(response) = require_admin(&state, &headers) {
        return response;
    }
    match tr_agent::store::approvals::list(state.pipeline.db(), query.limit) {
        Ok(rows) => Json(json!({"rows": rows})).into_response(),
        Err(error) => {
            eprintln!("approvals query failed: {error}");
            error_response(StatusCode::INTERNAL_SERVER_ERROR, "approvals query failed")
        }
    }
}

fn require_admin(state: &ControlState, headers: &HeaderMap) -> Option<Response> {
    let Some(expected) = &state.admin_token else {
        return None;
    };
    if expected.is_empty() {
        return None;
    }
    let provided = headers
        .get("x-admin-token")
        .and_then(|value| value.to_str().ok());
    if provided.is_some_and(|provided| constant_time_eq(provided.as_bytes(), expected.as_bytes())) {
        None
    } else {
        Some(error_response(
            StatusCode::UNAUTHORIZED,
            "invalid or missing admin token",
        ))
    }
}

fn constant_time_eq(left: &[u8], right: &[u8]) -> bool {
    if left.len() != right.len() {
        return false;
    }
    let mut diff = 0u8;
    for (a, b) in left.iter().zip(right.iter()) {
        diff |= a ^ b;
    }
    diff == 0
}

fn error_response(status: StatusCode, message: &str) -> Response {
    (
        status,
        Json(json!({"error": {"type": "admin_error", "message": message}})),
    )
        .into_response()
}

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
}
