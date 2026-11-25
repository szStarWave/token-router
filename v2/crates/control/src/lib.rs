//! 管理 API（P4 M1/M2）：`/v2/admin/*`。
//!
//! 边界（技术方案 §3.1）：
//! - UI 不读文件/DB：日志、账本、配置一律走 HTTP；
//! - 服务端只写启动时加载的那一个配置文件；
//! - 不返回任何密钥值：凭据只显示引用；
//! - 不做在线改内存：配置 apply 走文档级 CAS + 原子写 + 重启。
//!
//! 错误码集中定义在 [`crate::error`]；鉴权在 [`crate::auth`]。

#![forbid(unsafe_code)]

pub mod auth;
pub mod config_service;
pub mod error;
pub mod events;
pub mod logs;
pub mod modelsdev;
pub mod playground;
pub mod provider_service;
pub mod provider_templates;
pub mod provider_usage;
pub mod state;

use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{delete, get, patch, post};
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::json;
use tr_contracts::{
    AccountCreateRequest, AccountUpdateRequest, AdminScope, CatalogRefreshRequest,
    CatalogSelectRequest, CompileRequest, ConfigAuditEntry, ConfigReloadResponse,
    CreateTokenRequest, DoctorCheck, DoctorResponse, ImportScanRequest, Page,
    ProviderUpsertRequest, TokenInfo,
};

pub use crate::auth::AdminAuth;
use crate::error::{AdminApiError, ApiErrorType};
pub use crate::logs::LogTail;
pub use crate::state::{ControlState, LogRing};

/// 管理面路由（挂在 `/v2/admin` 下）。
pub fn router(state: ControlState) -> Router {
    Router::new()
        .route("/v2/admin/status", get(status))
        .route("/v2/admin/health", get(health))
        .route("/v2/admin/doctor", get(doctor))
        .route("/v2/admin/config", get(config))
        .route("/v2/admin/config/validate", post(config_validate))
        .route("/v2/admin/config/apply", post(config_apply))
        .route("/v2/admin/config/reload", post(config_reload))
        .route("/v2/admin/config/rollback", post(config_rollback))
        .route("/v2/admin/config/audit", get(config_audit))
        .route("/v2/admin/requests", get(requests))
        .route("/v2/admin/requests/{id}", get(request_detail))
        .route(
            "/v2/admin/requests/{id}/transcript",
            get(request_transcript),
        )
        .route("/v2/admin/judge/calls", get(judge_calls_list))
        .route("/v2/admin/sessions", get(sessions_list))
        .route("/v2/admin/stats/summary", get(stats_summary))
        .route("/v2/admin/stats", get(stats_summary_alias))
        .route("/v2/admin/stats/series", get(stats_series))
        .route("/v2/admin/stats/models", get(stats_models))
        .route("/v2/admin/shadow", get(shadow))
        .route("/v2/admin/tokens", get(tokens_list).post(tokens_create))
        .route(
            "/v2/admin/tokens/{id}",
            delete(tokens_revoke).patch(tokens_update_limits),
        )
        .route("/v2/admin/tokens/{id}/rotate", post(tokens_rotate))
        .route("/v2/admin/tokens/{id}/purge", post(tokens_purge))
        .route("/v2/admin/logs", get(logs_endpoint))
        .route("/v2/admin/events", get(events_endpoint))
        .route("/v2/admin/shutdown", post(shutdown))
        .route("/v2/admin/provider-templates", get(provider_templates_list))
        .route(
            "/v2/admin/providers",
            get(providers_list).post(providers_create),
        )
        .route(
            "/v2/admin/providers/{id}",
            get(providers_get)
                .patch(providers_update)
                .delete(providers_delete),
        )
        .route(
            "/v2/admin/providers/{id}/accounts",
            get(accounts_list).post(accounts_create),
        )
        .route(
            "/v2/admin/accounts/{id}",
            patch(account_update).delete(account_delete),
        )
        .route(
            "/v2/admin/providers/{id}/accounts/verify",
            post(accounts_verify),
        )
        .route("/v2/admin/providers/{id}/catalog", get(catalog_get))
        .route(
            "/v2/admin/providers/{id}/catalog/refresh",
            post(catalog_refresh),
        )
        .route(
            "/v2/admin/providers/{id}/catalog/enrich",
            post(catalog_enrich),
        )
        .route(
            "/v2/admin/providers/{id}/catalog/select",
            post(catalog_select),
        )
        .route(
            "/v2/admin/providers/{id}/catalog/{model_id}",
            delete(catalog_delete),
        )
        .route(
            "/v2/admin/providers/{id}/catalog/auto-include",
            post(catalog_auto_include),
        )
        .route("/v2/admin/providers/compile", post(providers_compile))
        .route("/v2/admin/providers/import", post(providers_import))
        .route("/v2/admin/quotas", get(quotas_list))
        .route("/v2/admin/provider-usage", get(provider_usage_list))
        .route("/v2/admin/usage", get(usage_endpoint))
        .route("/v2/admin/playground/execute", post(playground_execute))
        .route("/v2/admin/usage/heatmap", get(usage_heatmap_endpoint))
        .route("/v2/admin/quotas/{id}", delete(quotas_reset))
        .with_state(state)
}

// ---------------------------------------------------------------------------
// status / health / doctor
// ---------------------------------------------------------------------------

async fn status(
    State(state): State<ControlState>,
    headers: HeaderMap,
) -> Result<Response, AdminApiError> {
    state.auth.authenticate(&headers, AdminScope::AdminRead)?;
    let catalog = state.pipeline.catalog();
    let health = tr_agent::store::health::load(state.pipeline.db())?;
    let mut counts = tr_contracts::BreakerCounts::default();
    for state_value in health.candidates.values() {
        if state_value.is_open(now_millis() as u64) {
            counts.open += 1;
        } else if state_value.half_open_since.is_some() {
            counts.half_open += 1;
        } else {
            counts.closed += 1;
        }
    }
    let requests_total = tr_agent::store::ledger::count(state.pipeline.db())?;
    let response = tr_contracts::StatusResponse {
        api_version: tr_contracts::API_VERSION,
        version: env!("CARGO_PKG_VERSION").to_string(),
        started_at: state.started_at,
        uptime_ms: now_millis() - state.started_at,
        listen: catalog.server.listen.clone(),
        schema_version: tr_agent::config::SCHEMA_VERSION,
        db_schema_version: state.pipeline.db().schema_version()?,
        config_digest: catalog.config_digest.clone(),
        default_virtual_model: catalog.default_virtual_model.clone(),
        modes: tr_contracts::ModeSummary {
            judge: mode_label(&catalog.judge.mode).to_string(),
            cost_gate: mode_label_cost(&catalog.cost_gate).to_string(),
        },
        requests_total,
        breakers: counts,
    };
    Ok(Json(response).into_response())
}

async fn health(
    State(state): State<ControlState>,
    headers: HeaderMap,
) -> Result<Response, AdminApiError> {
    state.auth.authenticate(&headers, AdminScope::AdminRead)?;
    let catalog = state.pipeline.catalog();
    let loaded = tr_agent::store::health::load(state.pipeline.db())?;
    let now = now_millis() as u64;
    let mut items = Vec::new();
    let mut counts = tr_contracts::BreakerCounts::default();
    // 配置里的候选为准（DB 中可能残留已删除候选）。
    for candidate_id in catalog.candidates.keys() {
        let breaker = loaded.state_of(candidate_id).cloned().unwrap_or_default();
        let (status_name, is_open, is_half_open) = if breaker.is_open(now) {
            counts.open += 1;
            ("open", true, false)
        } else if breaker.half_open_since.is_some() {
            counts.half_open += 1;
            ("half_open", false, true)
        } else {
            counts.closed += 1;
            ("closed", false, false)
        };
        items.push(tr_contracts::CandidateHealthItem {
            candidate_id: candidate_id.clone(),
            status: status_name.to_string(),
            consecutive_failures: breaker.consecutive_failures,
            opened_count: breaker.opened_count,
            open_until: if is_open {
                breaker.open_until.map(millis_of)
            } else {
                None
            },
            half_open_since: if is_half_open {
                breaker.half_open_since.map(millis_of)
            } else {
                None
            },
            last_error_class: breaker
                .last_error_class
                .map(|class| class.as_str().to_string()),
            last_failure_at: breaker.last_failure_at.map(millis_of),
            updated_at: millis_of(breaker.updated_at) as i64,
        });
    }
    Ok(Json(tr_contracts::HealthResponse { items, counts }).into_response())
}

async fn doctor(
    State(state): State<ControlState>,
    headers: HeaderMap,
) -> Result<Response, AdminApiError> {
    state.auth.authenticate(&headers, AdminScope::AdminRead)?;
    let catalog = state.pipeline.catalog();
    let mut checks = Vec::new();

    // 配置可解析 + digest。
    checks.push(DoctorCheck {
        name: "config".to_string(),
        status: "ok".to_string(),
        detail: Some(format!(
            "schema v{}, digest {}",
            tr_agent::config::SCHEMA_VERSION,
            &catalog.config_digest[..16.min(catalog.config_digest.len())]
        )),
    });

    // DB schema 与账本可写性。
    let db_version = state.pipeline.db().schema_version()?;
    checks.push(DoctorCheck {
        name: "db_schema".to_string(),
        status: "ok".to_string(),
        detail: Some(format!("migration v{db_version}")),
    });

    // 判官自检（仅配置启用时）。
    if mode_label(&catalog.judge.mode) != "off" {
        checks.push(DoctorCheck {
            name: "judge".to_string(),
            status: "ok".to_string(),
            detail: Some(format!("mode {}", mode_label(&catalog.judge.mode))),
        });
    }

    // 磁盘空间（数据目录）。
    if let Some(dir) = &state.data_dir {
        match std::fs::metadata(dir) {
            Ok(_) => checks.push(DoctorCheck {
                name: "disk".to_string(),
                status: "ok".to_string(),
                detail: Some(dir.display().to_string()),
            }),
            Err(error) => checks.push(DoctorCheck {
                name: "disk".to_string(),
                status: "fail".to_string(),
                detail: Some(error.to_string()),
            }),
        }
    }

    // 日志文件存在性。
    checks.push(DoctorCheck {
        name: "logs".to_string(),
        status: if state.logs.is_some() { "ok" } else { "warn" }.to_string(),
        detail: Some(if state.logs.is_some() {
            "ring buffer active".to_string()
        } else {
            "log capture not initialized".to_string()
        }),
    });

    // token 是否存在。
    let has_token = !tr_agent::store::tokens::is_empty(state.pipeline.db())?;
    checks.push(DoctorCheck {
        name: "tokens".to_string(),
        status: if has_token { "ok" } else { "warn" }.to_string(),
        detail: Some(if has_token {
            "admin token present".to_string()
        } else {
            "no active admin token".to_string()
        }),
    });

    Ok(Json(DoctorResponse { checks }).into_response())
}

// ---------------------------------------------------------------------------
// requests
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct RequestsQuery {
    from: Option<i64>,
    to: Option<i64>,
    session: Option<String>,
    model: Option<String>,
    outcome: Option<String>,
    protocol: Option<String>,
    reason: Option<String>,
    limit: Option<u32>,
    cursor: Option<String>,
}

async fn requests(
    State(state): State<ControlState>,
    headers: HeaderMap,
    Query(query): Query<RequestsQuery>,
) -> Result<Response, AdminApiError> {
    state.auth.authenticate(&headers, AdminScope::AdminRead)?;
    let cursor = match &query.cursor {
        Some(text) => Some(
            tr_agent::store::admin_query::decode_cursor(text)
                .ok_or_else(|| AdminApiError::invalid_request("invalid cursor"))?,
        ),
        None => None,
    };
    let db_query = tr_agent::store::admin_query::LedgerQuery {
        from: query.from,
        to: query.to,
        session_key: query.session,
        model: query.model,
        outcome: query.outcome,
        protocol: query.protocol,
        reason_code: query.reason,
        limit: query.limit.unwrap_or(50),
        cursor,
    };
    let (rows, next_cursor) =
        tr_agent::store::admin_query::list_requests_page(state.pipeline.db(), &db_query)?;
    Ok(Json(Page {
        items: rows,
        next_cursor,
    })
    .into_response())
}

/// 游标需要 ledger 的 `id` 列；Summary 不含它，用 (created_at, request_id) 的确定性
async fn request_detail(
    State(state): State<ControlState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Response, AdminApiError> {
    state.auth.authenticate(&headers, AdminScope::AdminRead)?;
    let detail = tr_agent::store::admin_query::request_detail(state.pipeline.db(), &id)?;
    match detail {
        Some(mut detail) => {
            // 决策模型调用随请求返回：审计表只存结构化字段，不含 prompt 正文。
            detail.judge_calls = tr_agent::store::judge::list_calls(
                state.pipeline.db(),
                &tr_agent::store::judge::JudgeCallFilter {
                    request_id: Some(id.clone()),
                    limit: 50,
                    ..Default::default()
                },
            )?;
            Ok(Json(detail).into_response())
        }
        None => Err(AdminApiError::not_found(format!("request {id} not found"))),
    }
}

/// `GET /v2/admin/requests/{id}/transcript`：这一轮的消息与回复。
///
/// 没有记录时返回 `recorded: false`（关闭记录或早于该能力的请求），不当作错误。
async fn request_transcript(
    State(state): State<ControlState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Response, AdminApiError> {
    state.auth.authenticate(&headers, AdminScope::AdminRead)?;
    let transcript = tr_agent::store::transcript::get(state.pipeline.db(), &id)?
        .unwrap_or_else(|| tr_agent::store::transcript::empty(&id));
    Ok(Json(transcript).into_response())
}

#[derive(Debug, Deserialize)]
struct JudgeCallsQuery {
    request_id: Option<String>,
    session: Option<String>,
    from: Option<i64>,
    to: Option<i64>,
    limit: Option<u32>,
}

/// `GET /v2/admin/judge-calls`：判官（决策模型）调用审计。
async fn judge_calls_list(
    State(state): State<ControlState>,
    headers: HeaderMap,
    Query(query): Query<JudgeCallsQuery>,
) -> Result<Response, AdminApiError> {
    state.auth.authenticate(&headers, AdminScope::AdminRead)?;
    let items = tr_agent::store::judge::list_calls(
        state.pipeline.db(),
        &tr_agent::store::judge::JudgeCallFilter {
            request_id: query.request_id,
            session_key: query.session,
            from_ms: query.from,
            to_ms: query.to,
            limit: query.limit.unwrap_or(100),
        },
    )?;
    Ok(Json(Page {
        items,
        next_cursor: None,
    })
    .into_response())
}

#[derive(Debug, Deserialize)]
struct SessionsQuery {
    from: Option<i64>,
    to: Option<i64>,
    limit: Option<u32>,
}

/// `GET /v2/admin/sessions`：会话级完整调用汇总（服务端聚合）。
async fn sessions_list(
    State(state): State<ControlState>,
    headers: HeaderMap,
    Query(query): Query<SessionsQuery>,
) -> Result<Response, AdminApiError> {
    state.auth.authenticate(&headers, AdminScope::AdminRead)?;
    let (from, to) = resolve_window(query.from, query.to)?;
    let items = tr_agent::store::admin_query::sessions(
        state.pipeline.db(),
        from,
        to,
        query.limit.unwrap_or(50),
    )?;
    Ok(Json(Page {
        items,
        next_cursor: None,
    })
    .into_response())
}

// ---------------------------------------------------------------------------
// stats
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct WindowParams {
    from: Option<i64>,
    to: Option<i64>,
}

async fn stats_summary(
    State(state): State<ControlState>,
    headers: HeaderMap,
    Query(params): Query<WindowParams>,
) -> Result<Response, AdminApiError> {
    state.auth.authenticate(&headers, AdminScope::AdminRead)?;
    let (from, to) = resolve_window(params.from, params.to)?;
    let summary = tr_agent::store::admin_query::stats_summary(state.pipeline.db(), from, to)?;
    Ok(Json(summary).into_response())
}

/// 保留 `GET /v2/admin/stats` 作为 summary 的兼容别名。
async fn stats_summary_alias(
    state: State<ControlState>,
    headers: HeaderMap,
    params: Query<WindowParams>,
) -> Result<Response, AdminApiError> {
    stats_summary(state, headers, params).await
}

#[derive(Debug, Deserialize)]
struct SeriesParams {
    from: Option<i64>,
    to: Option<i64>,
    #[serde(default = "default_bucket")]
    bucket: String,
    #[serde(default = "default_group_by")]
    group_by: String,
    #[serde(default = "default_metric")]
    metric: String,
}

fn default_bucket() -> String {
    "hour".to_string()
}

fn default_group_by() -> String {
    "dispatch_model".to_string()
}

fn default_metric() -> String {
    "requests".to_string()
}

async fn stats_series(
    State(state): State<ControlState>,
    headers: HeaderMap,
    Query(params): Query<SeriesParams>,
) -> Result<Response, AdminApiError> {
    state.auth.authenticate(&headers, AdminScope::AdminRead)?;
    let (from, to) = resolve_window(params.from, params.to)?;
    let series = tr_agent::store::admin_query::stats_series(
        state.pipeline.db(),
        from,
        to,
        &params.bucket,
        &params.group_by,
        &params.metric,
    )
    .map_err(|error| match error {
        tr_agent::store::StoreError::Invalid(message) => AdminApiError::invalid_request(message),
        other => AdminApiError::from(other),
    })?;
    Ok(Json(series).into_response())
}

async fn stats_models(
    State(state): State<ControlState>,
    headers: HeaderMap,
    Query(params): Query<WindowParams>,
) -> Result<Response, AdminApiError> {
    state.auth.authenticate(&headers, AdminScope::AdminRead)?;
    let (from, to) = resolve_window(params.from, params.to)?;
    let rows = tr_agent::store::admin_query::stats_by_model(state.pipeline.db(), from, to)?;
    Ok(Json(json!({ "items": rows })).into_response())
}

/// 时间窗解析：缺省最近 24h；最大 90 天（`413 window_too_large`）。
fn resolve_window(from: Option<i64>, to: Option<i64>) -> Result<(i64, i64), AdminApiError> {
    let to = to.unwrap_or_else(now_millis);
    let from = from.unwrap_or(to - 24 * 3600 * 1000);
    if to <= from {
        return Err(AdminApiError::invalid_request("to must be after from"));
    }
    if to - from > tr_agent::store::admin_query::MAX_WINDOW_MS {
        return Err(AdminApiError::new(
            ApiErrorType::WindowTooLarge,
            "window exceeds 90 days; narrow from/to or add filters",
        ));
    }
    Ok((from, to))
}

// ---------------------------------------------------------------------------
// shadow（保留 v2 既有能力，补 scope）
// ---------------------------------------------------------------------------

async fn shadow(
    State(state): State<ControlState>,
    headers: HeaderMap,
) -> Result<Response, AdminApiError> {
    state.auth.authenticate(&headers, AdminScope::AdminRead)?;
    let summary = tr_agent::store::judge::shadow_summary(state.pipeline.db())?;
    Ok(Json(json!({
        "total": summary.total,
        "agreed": summary.agreed,
        "disagreed": summary.disagreed,
        "unknown": summary.unknown,
        "agreement_rate": summary.agreement_rate(),
        "avg_latency_ms": summary.avg_latency_ms,
        "p95_latency_ms": summary.p95_latency_ms,
        "total_cost_micros": summary.total_cost_micros,
    }))
    .into_response())
}

// ---------------------------------------------------------------------------
// tokens
// ---------------------------------------------------------------------------

async fn tokens_list(
    State(state): State<ControlState>,
    headers: HeaderMap,
) -> Result<Response, AdminApiError> {
    state.auth.authenticate(&headers, AdminScope::AdminRead)?;
    let items: Vec<TokenInfo> = tr_agent::store::tokens::list(state.pipeline.db())?;
    Ok(Json(Page {
        items,
        next_cursor: None,
    })
    .into_response())
}

async fn tokens_create(
    State(state): State<ControlState>,
    headers: HeaderMap,
    Json(body): Json<CreateTokenRequest>,
) -> Result<Response, AdminApiError> {
    let record = state.auth.authenticate(&headers, AdminScope::AdminWrite)?;
    if body.name.is_empty() {
        return Err(AdminApiError::invalid_request("name must not be empty"));
    }
    if body.scopes.is_empty() {
        return Err(AdminApiError::invalid_request(
            "at least one scope required",
        ));
    }
    let mut token_record: tr_agent::store::tokens::TokenRecord =
        std::convert::TryFrom::try_from(&body).map_err(AdminApiError::from)?;
    let plaintext = tr_agent::store::tokens::generate_token();
    token_record.token_sha256 = tr_agent::store::tokens::sha256_hex(&plaintext);
    token_record.prefix = plaintext.chars().take(8).collect();
    token_record.created_at = now_millis();
    tr_agent::store::tokens::insert(state.pipeline.db(), &token_record)?;
    let _ = record;
    let info = TokenInfo {
        id: token_record.id.clone(),
        name: token_record.name.clone(),
        prefix: token_record.prefix.clone(),
        scopes: token_record.scopes.clone(),
        created_at: token_record.created_at,
        last_used_at: None,
        expires_at: token_record.expires_at,
        revoked_at: None,
        limits: None,
    };
    Ok(Json(tr_contracts::CreatedToken {
        info,
        token: plaintext,
    })
    .into_response())
}

async fn tokens_revoke(
    State(state): State<ControlState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Response, AdminApiError> {
    state.auth.authenticate(&headers, AdminScope::AdminWrite)?;
    let revoked = tr_agent::store::tokens::revoke(state.pipeline.db(), &id, now_millis())?;
    if !revoked {
        return Err(AdminApiError::not_found(format!(
            "token {id} not found or already revoked"
        )));
    }
    Ok(Json(json!({"ok": true})).into_response())
}

/// 硬删除已撤销的 token（purge）：清掉撤销后的残留记录。
///
/// 撤销是软删除（保留审计）；这是第二步的清理，只删已撤销的记录。
/// bootstrap token 的明文另有文件副本，删记录会让文件与库不一致——因此拒绝。
async fn tokens_purge(
    State(state): State<ControlState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Response, AdminApiError> {
    state.auth.authenticate(&headers, AdminScope::AdminWrite)?;
    let existing = tr_agent::store::tokens::find_by_id(state.pipeline.db(), &id)?
        .ok_or_else(|| AdminApiError::not_found(format!("token {id} not found")))?;
    // bootstrap 先判：它的明文另有文件副本，删记录会让文件与库不一致。
    if state.auth.is_bootstrap_token(&existing) {
        return Err(AdminApiError::invalid_request(
            "bootstrap token cannot be purged; its plaintext lives in admin.token",
        ));
    }
    if existing.revoked_at.is_none() {
        return Err(AdminApiError::invalid_request(
            "token is still active; revoke it first",
        ));
    }
    let purged = tr_agent::store::tokens::purge(state.pipeline.db(), &id)?;
    if !purged {
        return Err(AdminApiError::not_found(format!("token {id} not found")));
    }
    Ok(Json(json!({"ok": true})).into_response())
}

/// 重新签发 token 明文（rotate）：同 id / 名称 / 作用域，换新的明文与前缀。
///
/// 旧明文立即失效；明文只在响应里出现这一次。bootstrap token 的明文写在
/// `<data_dir>/admin.token`，重新签发会让该文件与库里的哈希不一致——因此拒绝。
async fn tokens_rotate(
    State(state): State<ControlState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Response, AdminApiError> {
    state.auth.authenticate(&headers, AdminScope::AdminWrite)?;
    let existing = tr_agent::store::tokens::find_by_id(state.pipeline.db(), &id)?
        .ok_or_else(|| AdminApiError::not_found(format!("token {id} not found")))?;
    if existing.revoked_at.is_some() {
        return Err(AdminApiError::invalid_request("token is revoked"));
    }
    if state.auth.is_bootstrap_token(&existing) {
        return Err(AdminApiError::invalid_request(
            "bootstrap token cannot be rotated; its plaintext lives in admin.token",
        ));
    }
    let plaintext = tr_agent::store::tokens::generate_token();
    let rotated = tr_agent::store::tokens::rotate(state.pipeline.db(), &id, &plaintext)?;
    if !rotated {
        return Err(AdminApiError::not_found(format!(
            "token {id} not found or already revoked"
        )));
    }
    let updated = tr_agent::store::tokens::find_by_id(state.pipeline.db(), &id)?
        .ok_or_else(|| AdminApiError::internal("token vanished after rotate"))?;
    let info = TokenInfo {
        id: updated.id.clone(),
        name: updated.name.clone(),
        prefix: updated.prefix.clone(),
        scopes: updated.scopes.clone(),
        created_at: updated.created_at,
        last_used_at: updated.last_used_at,
        expires_at: updated.expires_at,
        revoked_at: updated.revoked_at,
        limits: updated.limits.clone(),
    };
    Ok(Json(tr_contracts::CreatedToken {
        info,
        token: plaintext,
    })
    .into_response())
}

// ---------------------------------------------------------------------------
// logs
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct LogsQuery {
    cursor: Option<i64>,
    limit: Option<u32>,
    #[serde(default)]
    level: Option<String>,
    #[serde(default)]
    target: Option<String>,
}

async fn logs_endpoint(
    State(state): State<ControlState>,
    headers: HeaderMap,
    Query(query): Query<LogsQuery>,
) -> Result<Response, AdminApiError> {
    state.auth.authenticate(&headers, AdminScope::AdminRead)?;
    let response = state.log_tail(
        query.cursor.unwrap_or(0),
        query.limit.unwrap_or(200),
        |entry| {
            level_matches(&entry.level, query.level.as_deref())
                && target_matches(&entry.target, query.target.as_deref())
        },
    )?;
    Ok(Json(response).into_response())
}

fn level_matches(actual: &str, filter: Option<&str>) -> bool {
    match filter {
        Some(level) => actual.eq_ignore_ascii_case(level),
        None => true,
    }
}

fn target_matches(actual: &str, filter: Option<&str>) -> bool {
    match filter {
        Some(target) => actual.starts_with(target),
        None => true,
    }
}

// ---------------------------------------------------------------------------
// shutdown
// ---------------------------------------------------------------------------

async fn shutdown(
    State(state): State<ControlState>,
    headers: HeaderMap,
) -> Result<Response, AdminApiError> {
    state.auth.authenticate(&headers, AdminScope::AdminWrite)?;
    state.request_shutdown();
    Ok(Json(json!({"ok": true, "message": "shutting down"})).into_response())
}

// ---------------------------------------------------------------------------
// events（P4-081：SSE）
// ---------------------------------------------------------------------------

/// `GET /v2/admin/events`：SSE 流；`Last-Event-ID` 续传；心跳 15s。
///
/// 认证在首个响应字节前完成（SSE 一旦开始就无法改状态码）。
async fn events_endpoint(
    State(state): State<ControlState>,
    headers: HeaderMap,
) -> Result<Response, AdminApiError> {
    state.auth.authenticate(&headers, AdminScope::AdminRead)?;
    let last_event_id = headers
        .get("Last-Event-ID")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<i64>().ok())
        .unwrap_or(0);

    let (events, overflow) = state.events.after(last_event_id);
    let mut stream_body = String::new();
    if overflow {
        // ring 已滚动：客户端应重新全量拉取。
        stream_body
            .push_str("event: overflow\ndata: {\"message\":\"events behind ring; refetch\"}\n\n");
    }
    for event in &events {
        stream_body.push_str(&event.to_sse());
    }

    let bus = state.events.clone();
    let body_stream = async_stream::stream! {
        // 先发续传页。
        yield Ok::<_, std::convert::Infallible>(axum::body::Bytes::from(stream_body.clone()));
        let mut cursor = events.last().map(|event| event.id).unwrap_or(last_event_id);
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
            let (new_events, overflow) = bus.after(cursor);
            if overflow {
                yield Ok(axum::body::Bytes::from_static(
                    b"event: overflow\ndata: {\"message\":\"events behind ring; refetch\"}\n\n",
                ));
                cursor = bus.latest_id();
                continue;
            }
            for event in &new_events {
                cursor = event.id;
                yield Ok(axum::body::Bytes::from(event.to_sse()));
            }
            // 心跳（15s = 15 次轮询；用计数器避免每秒 ping）。
            static PING_COUNTER: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
            let ticks = PING_COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            if ticks % 15 == 14 {
                yield Ok(axum::body::Bytes::from_static(
                    crate::events::ping_frame().as_bytes(),
                ));
            }
        }
    };

    Ok(Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "text/event-stream")
        .header(header::CACHE_CONTROL, "no-store")
        .body(axum::body::Body::from_stream(body_stream))
        .expect("build SSE response"))
}

// ---------------------------------------------------------------------------
// config（M2，实现在 config_service）
// ---------------------------------------------------------------------------

async fn config(
    State(state): State<ControlState>,
    headers: HeaderMap,
) -> Result<Response, AdminApiError> {
    state.auth.authenticate(&headers, AdminScope::AdminRead)?;
    let document = config_service::config_document(&state.pipeline.catalog(), &state)?;
    Ok(Json(document).into_response())
}

#[derive(Debug, Deserialize)]
struct ValidateBody {
    document_toml: String,
}

async fn config_validate(
    State(state): State<ControlState>,
    headers: HeaderMap,
    Json(body): Json<ValidateBody>,
) -> Result<Response, AdminApiError> {
    state.auth.authenticate(&headers, AdminScope::AdminRead)?;
    let response =
        config_service::validate(&state.pipeline.catalog(), &state, &body.document_toml)?;
    // validate 也写审计（契约 §4.7）。normalized_toml 已脱敏后才回传/审计。
    let _ = tr_agent::store::config_audit::insert(
        state.pipeline.db(),
        &tr_agent::store::config_audit::AuditRecord {
            at: now_millis(),
            actor: "admin",
            action: "validate",
            from_revision: None,
            to_revision: response.revision.as_deref(),
            note: None,
            diff: &[],
        },
    );
    Ok(Json(response).into_response())
}

async fn config_apply(
    State(state): State<ControlState>,
    headers: HeaderMap,
    Json(body): Json<ValidateBody>,
) -> Result<Response, AdminApiError> {
    let record = state.auth.authenticate(&headers, AdminScope::AdminWrite)?;
    // If-Match 必需：缺失直接 400，不允许无锁盲写。
    let expected_revision = headers
        .get("If-Match")
        .and_then(|value| value.to_str().ok())
        .map(str::to_string)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            AdminApiError::invalid_request(
                "If-Match header with the current config revision is required",
            )
        })?;
    // 写盘 + 热替换运行时目录在同一把写锁内完成（不再依赖重启）。
    let (response, reload) =
        config_service::apply_and_reload(&state, &body.document_toml, &expected_revision)?;
    let restart_required = match reload {
        Ok(outcome) => outcome.restart_required,
        Err(error) => {
            eprintln!("config applied but runtime reload failed: {error:?}");
            true
        }
    };
    tr_agent::store::config_audit::insert(
        state.pipeline.db(),
        &tr_agent::store::config_audit::AuditRecord {
            at: now_millis(),
            actor: "token",
            action: "apply",
            from_revision: response.previous_revision.as_deref(),
            to_revision: Some(&response.revision),
            note: Some(&record.id),
            diff: &response.diff,
        },
    )?;
    // SSE：config.applied（§4.9）。
    state.events.publish(
        crate::events::EventKind::ConfigApplied,
        json!({
            "revision": response.revision,
            "restart_required": restart_required,
        }),
    );
    Ok(Json(tr_contracts::ApplyResponse {
        revision: response.revision,
        restart_required,
        backup_path: response.backup_path,
    })
    .into_response())
}

async fn config_rollback(
    State(state): State<ControlState>,
    headers: HeaderMap,
) -> Result<Response, AdminApiError> {
    let record = state.auth.authenticate(&headers, AdminScope::AdminWrite)?;
    let (response, reload) = config_service::rollback_and_reload(&state)?;
    let restart_required = match reload {
        Ok(outcome) => outcome.restart_required,
        Err(error) => {
            eprintln!("config rolled back but runtime reload failed: {error:?}");
            true
        }
    };
    tr_agent::store::config_audit::insert(
        state.pipeline.db(),
        &tr_agent::store::config_audit::AuditRecord {
            at: now_millis(),
            actor: "token",
            action: "rollback",
            from_revision: None,
            to_revision: Some(&response.revision),
            note: Some(&record.id),
            diff: &response.diff,
        },
    )?;
    Ok(Json(tr_contracts::ApplyResponse {
        revision: response.revision,
        restart_required,
        backup_path: response.backup_path,
    })
    .into_response())
}

/// `POST /v2/admin/config/reload`：重读磁盘配置并热替换运行时（不写盘）。
///
/// 用于外部编辑配置后的手动生效；非法配置返回 422 + issues，运行时保持原样。
async fn config_reload(
    State(state): State<ControlState>,
    headers: HeaderMap,
) -> Result<Response, AdminApiError> {
    let record = state.auth.authenticate(&headers, AdminScope::AdminWrite)?;
    let outcome = config_service::reload_runtime(&state)?;
    tr_agent::store::config_audit::insert(
        state.pipeline.db(),
        &tr_agent::store::config_audit::AuditRecord {
            at: now_millis(),
            actor: "token",
            action: "reload",
            from_revision: Some(&outcome.previous_revision),
            to_revision: Some(&outcome.revision),
            note: Some(&record.id),
            diff: &[],
        },
    )?;
    state.events.publish(
        crate::events::EventKind::ConfigApplied,
        json!({
            "revision": outcome.revision,
            "restart_required": outcome.restart_required,
        }),
    );
    Ok(Json(ConfigReloadResponse {
        revision: outcome.revision,
        reloaded: true,
        restart_required: outcome.restart_required,
        issues: Vec::new(),
    })
    .into_response())
}

async fn config_audit(
    State(state): State<ControlState>,
    headers: HeaderMap,
    Query(query): Query<LimitQuery>,
) -> Result<Response, AdminApiError> {
    state.auth.authenticate(&headers, AdminScope::AdminRead)?;
    let items: Vec<ConfigAuditEntry> =
        tr_agent::store::config_audit::list(state.pipeline.db(), query.limit)?;
    Ok(Json(json!({ "items": items })).into_response())
}

// ---------------------------------------------------------------------------
// 共享
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct LimitQuery {
    #[serde(default = "default_limit")]
    limit: u32,
}

fn default_limit() -> u32 {
    50
}

fn millis_of(secs: u64) -> u64 {
    secs.saturating_mul(1000)
}

fn mode_label(mode: &tr_agent::judge::JudgeMode) -> &'static str {
    match mode {
        tr_agent::judge::JudgeMode::Off => "off",
        tr_agent::judge::JudgeMode::Shadow => "shadow",
        tr_agent::judge::JudgeMode::Active => "active",
    }
}

fn mode_label_cost(mode: &tr_agent::gate::CostGateMode) -> &'static str {
    match mode {
        tr_agent::gate::CostGateMode::Off => "off",
        tr_agent::gate::CostGateMode::Shadow => "shadow",
        tr_agent::gate::CostGateMode::Active => "active",
    }
}

fn now_millis() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as i64)
        .unwrap_or(0)
}

// ── Provider / Account / 目录 / 编译（v3 P5-A）─────────────────────────────

async fn provider_templates_list(
    State(state): State<ControlState>,
    headers: HeaderMap,
) -> Result<Response, AdminApiError> {
    state.auth.authenticate(&headers, AdminScope::AdminRead)?;
    let mut items = provider_service::list_templates();
    // models.dev 补充长尾供应商（内置模板优先；同 id / 同 host 不重复）。
    // 拉取失败或超时只返回内置模板，添加供应商入口不会被网络阻塞太久。
    if let Ok(Ok(extra)) = tokio::time::timeout(
        std::time::Duration::from_secs(12),
        crate::modelsdev::provider_templates(&state),
    )
    .await
    {
        let known_ids: std::collections::BTreeSet<String> =
            items.iter().map(|item| item.id.clone()).collect();
        let known_hosts: std::collections::BTreeSet<String> = items
            .iter()
            .filter_map(|item| host_of(&item.base_url))
            .collect();
        for template in extra {
            if known_ids.contains(&template.id) {
                continue;
            }
            if host_of(&template.base_url)
                .map(|host| known_hosts.contains(&host))
                .unwrap_or(false)
            {
                continue;
            }
            items.push(template);
        }
    }
    Ok(Json(items).into_response())
}

/// base_url 的 host（去 scheme / 端口 / 路径），用于模板去重。
fn host_of(url: &str) -> Option<String> {
    let rest = url.split_once("://").map(|(_, rest)| rest).unwrap_or(url);
    let host = rest.split('/').next().unwrap_or_default();
    let host = host.rsplit('@').next().unwrap_or(host);
    let host = host.split(':').next().unwrap_or(host);
    (!host.is_empty()).then(|| host.to_ascii_lowercase())
}

async fn providers_list(
    State(state): State<ControlState>,
    headers: HeaderMap,
) -> Result<Response, AdminApiError> {
    state.auth.authenticate(&headers, AdminScope::AdminRead)?;
    Ok(Json(provider_service::list_providers(&state)?).into_response())
}

async fn providers_create(
    State(state): State<ControlState>,
    headers: HeaderMap,
    Json(body): Json<ProviderUpsertRequest>,
) -> Result<Response, AdminApiError> {
    state.auth.authenticate(&headers, AdminScope::AdminWrite)?;
    if body.display_name.as_deref().unwrap_or("").is_empty() && body.template_id.is_none() {
        return Err(AdminApiError::invalid_request(
            "display_name or template_id required",
        ));
    }
    Ok(Json(provider_service::create_provider(&state, body)?).into_response())
}

async fn providers_get(
    State(state): State<ControlState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Response, AdminApiError> {
    state.auth.authenticate(&headers, AdminScope::AdminRead)?;
    Ok(Json(provider_service::find_provider(&state, &id)?).into_response())
}

async fn providers_update(
    State(state): State<ControlState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(body): Json<ProviderUpsertRequest>,
) -> Result<Response, AdminApiError> {
    state.auth.authenticate(&headers, AdminScope::AdminWrite)?;
    Ok(Json(provider_service::update_provider(&state, &id, body)?).into_response())
}

async fn providers_delete(
    State(state): State<ControlState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Response, AdminApiError> {
    state.auth.authenticate(&headers, AdminScope::AdminWrite)?;
    provider_service::delete_provider(&state, &id)?;
    Ok(StatusCode::NO_CONTENT.into_response())
}

async fn accounts_list(
    State(state): State<ControlState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Response, AdminApiError> {
    state.auth.authenticate(&headers, AdminScope::AdminRead)?;
    Ok(Json(provider_service::list_accounts(&state, &id)?).into_response())
}

async fn accounts_create(
    State(state): State<ControlState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(body): Json<AccountCreateRequest>,
) -> Result<Response, AdminApiError> {
    state.auth.authenticate(&headers, AdminScope::AdminWrite)?;
    Ok(Json(provider_service::create_account(&state, &id, body)?).into_response())
}

/// verify 端点请求体。
#[derive(Deserialize)]
struct AccountVerifyBody {
    account_id: String,
}

async fn accounts_verify(
    State(state): State<ControlState>,
    headers: HeaderMap,
    Path(_id): Path<String>,
    Json(body): Json<AccountVerifyBody>,
) -> Result<Response, AdminApiError> {
    state.auth.authenticate(&headers, AdminScope::AdminWrite)?;
    Ok(Json(provider_service::verify_account(&state, &body.account_id).await?).into_response())
}

async fn catalog_get(
    State(state): State<ControlState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Response, AdminApiError> {
    state.auth.authenticate(&headers, AdminScope::AdminRead)?;
    Ok(Json(provider_service::catalog(&state, &id)?).into_response())
}

async fn catalog_refresh(
    State(state): State<ControlState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(body): Json<CatalogRefreshRequest>,
) -> Result<Response, AdminApiError> {
    state.auth.authenticate(&headers, AdminScope::AdminWrite)?;
    Ok(Json(provider_service::refresh_catalog(&state, &id, body).await?).into_response())
}

/// `POST /v2/admin/providers/{id}/catalog/enrich`：用 models.dev 补全模型元数据。
///
/// 只填空字段（窗口 / 输出上限 / 价格 / 推理档位），不覆盖用户值；成功后重编译候选。
async fn catalog_enrich(
    State(state): State<ControlState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Response, AdminApiError> {
    state.auth.authenticate(&headers, AdminScope::AdminWrite)?;
    let report = crate::modelsdev::enrich_provider(&state, &id).await?;
    if let Err(error) = provider_service::compile_candidates(
        &state,
        CompileRequest {
            expected_revision: None,
        },
    ) {
        eprintln!("catalog enrich applied but compile failed: {error:?}");
    }
    Ok(Json(json!({
        "provider_id": id,
        "source_provider": report.source_provider,
        "matched": report.matched,
        "filled": report.filled,
    }))
    .into_response())
}

async fn catalog_select(
    State(state): State<ControlState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(body): Json<CatalogSelectRequest>,
) -> Result<Response, AdminApiError> {
    state.auth.authenticate(&headers, AdminScope::AdminWrite)?;
    Ok(Json(provider_service::select_model(&state, &id, body)?).into_response())
}

async fn catalog_delete(
    State(state): State<ControlState>,
    headers: HeaderMap,
    Path((id, model_id)): Path<(String, String)>,
) -> Result<Response, AdminApiError> {
    state.auth.authenticate(&headers, AdminScope::AdminWrite)?;
    Ok(Json(provider_service::delete_catalog_model(
        &state, &id, &model_id,
    )?)
    .into_response())
}

async fn providers_compile(
    State(state): State<ControlState>,
    headers: HeaderMap,
    Json(body): Json<CompileRequest>,
) -> Result<Response, AdminApiError> {
    let record = state.auth.authenticate(&headers, AdminScope::AdminWrite)?;
    let response = provider_service::compile_candidates(&state, body)?;
    tr_agent::store::config_audit::insert(
        state.pipeline.db(),
        &tr_agent::store::config_audit::AuditRecord {
            at: now_millis(),
            actor: "token",
            action: "compile",
            from_revision: Some(&response.previous_revision),
            to_revision: Some(&response.revision),
            note: Some(&record.id),
            diff: &[],
        },
    )?;
    state.events.publish(
        crate::events::EventKind::ConfigApplied,
        serde_json::json!({
            "revision": response.revision,
            "restart_required": true,
            "source": "providers_compile",
        }),
    );
    Ok(Json(response).into_response())
}

async fn account_update(
    State(state): State<ControlState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(body): Json<AccountUpdateRequest>,
) -> Result<Response, AdminApiError> {
    state.auth.authenticate(&headers, AdminScope::AdminWrite)?;
    Ok(Json(provider_service::update_account(&state, &id, body)?).into_response())
}

async fn account_delete(
    State(state): State<ControlState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Response, AdminApiError> {
    state.auth.authenticate(&headers, AdminScope::AdminWrite)?;
    provider_service::delete_account(&state, &id)?;
    Ok(StatusCode::NO_CONTENT.into_response())
}

/// `POST /v2/admin/providers/import`：从本机 agent 配置反向导入草稿（只读扫描）。
async fn providers_import(
    State(state): State<ControlState>,
    headers: HeaderMap,
    Json(body): Json<ImportScanRequest>,
) -> Result<Response, AdminApiError> {
    state.auth.authenticate(&headers, AdminScope::AdminRead)?;
    Ok(Json(provider_service::import_scan(&state, body)?).into_response())
}

/// `GET /v2/admin/quotas`：各候选的配额窗口视图（v3 P5-B）。
async fn quotas_list(
    State(state): State<ControlState>,
    headers: HeaderMap,
) -> Result<Response, AdminApiError> {
    state.auth.authenticate(&headers, AdminScope::AdminRead)?;
    let items = tr_agent::store::quota::list(state.pipeline.db())?;
    Ok(Json(serde_json::json!({ "items": items })).into_response())
}

/// `GET /v2/admin/provider-usage`：向厂商 usage 接口查询账户余额/订阅。
///
/// `?refresh=true` 跳过 60s 缓存；无公开接口的厂商返回 `unsupported`。
async fn provider_usage_list(
    State(state): State<ControlState>,
    headers: HeaderMap,
    Query(params): Query<ProviderUsageParams>,
) -> Result<Response, AdminApiError> {
    state.auth.authenticate(&headers, AdminScope::AdminRead)?;
    let usage = provider_usage::list_account_usage(&state, params.refresh).await?;
    Ok(Json(usage).into_response())
}

#[derive(Debug, Default, Deserialize)]
struct ProviderUsageParams {
    #[serde(default)]
    refresh: bool,
}

/// `DELETE /v2/admin/quotas/{id}`：手动清除某候选的配额窗口（上游恢复后立即生效）。
async fn quotas_reset(
    State(state): State<ControlState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Response, AdminApiError> {
    state.auth.authenticate(&headers, AdminScope::AdminWrite)?;
    let reset = tr_agent::store::quota::reset(state.pipeline.db(), &id)?;
    Ok(Json(serde_json::json!({ "ok": reset })).into_response())
}

/// `PATCH /v2/admin/tokens/{id}`：设置/清除网关 key 限额（v3 P5-B）。
async fn tokens_update_limits(
    State(state): State<ControlState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(body): Json<tr_contracts::UpdateTokenLimitsRequest>,
) -> Result<Response, AdminApiError> {
    state.auth.authenticate(&headers, AdminScope::AdminWrite)?;
    if tr_agent::store::tokens::find_by_id(state.pipeline.db(), &id)?.is_none() {
        return Err(AdminApiError::not_found(format!("token {id} not found")));
    }
    let limits = body.limits.as_ref();
    tr_agent::store::tokens::update_limits(state.pipeline.db(), &id, limits)?;
    let items = tr_agent::store::tokens::list(state.pipeline.db())?;
    let info = items
        .into_iter()
        .find(|token| token.id == id)
        .ok_or_else(|| AdminApiError::internal("token vanished after update"))?;
    Ok(Json(info).into_response())
}

/// `GET /v2/admin/usage?by=agent|model|candidate|account|session&from=&to=`。
#[derive(Deserialize)]
struct UsageParams {
    #[serde(default)]
    by: Option<String>,
    #[serde(default)]
    from: Option<i64>,
    #[serde(default)]
    to: Option<i64>,
}

async fn usage_endpoint(
    State(state): State<ControlState>,
    headers: HeaderMap,
    Query(params): Query<UsageParams>,
) -> Result<Response, AdminApiError> {
    state.auth.authenticate(&headers, AdminScope::AdminRead)?;
    let now = now_millis();
    let to = params.to.unwrap_or(now);
    let from = params.from.unwrap_or(to - 7 * 24 * 3_600_000);
    let dimension = params
        .by
        .as_deref()
        .map(tr_contracts::UsageDimension::parse)
        .unwrap_or(tr_contracts::UsageDimension::Agent);
    let response = tr_agent::store::usage::usage_by(state.pipeline.db(), dimension, from, to)?;
    Ok(Json(response).into_response())
}

async fn usage_heatmap_endpoint(
    State(state): State<ControlState>,
    headers: HeaderMap,
    Query(params): Query<UsageParams>,
) -> Result<Response, AdminApiError> {
    state.auth.authenticate(&headers, AdminScope::AdminRead)?;
    let now = now_millis();
    let to = params.to.unwrap_or(now);
    let from = params.from.unwrap_or(to - 7 * 24 * 3_600_000);
    let response = tr_agent::store::usage::usage_heatmap(state.pipeline.db(), from, to)?;
    Ok(Json(response).into_response())
}

/// ：管理面测试请求（v3 P5-D）。
async fn playground_execute(
    State(state): State<ControlState>,
    headers: HeaderMap,
    Json(body): Json<tr_contracts::PlaygroundExecuteRequest>,
) -> Result<Response, AdminApiError> {
    state.auth.authenticate(&headers, AdminScope::AdminWrite)?;
    let response = playground::execute(&state, body).await?;
    Ok(Json(response).into_response())
}

/// auto-include 通配符请求体。
#[derive(Deserialize)]
struct AutoIncludeBody {
    pattern: String,
}

/// \`POST /v2/admin/providers/{id}/catalog/auto-include\`：设置目录自动纳入通配符。
async fn catalog_auto_include(
    State(state): State<ControlState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(body): Json<AutoIncludeBody>,
) -> Result<Response, AdminApiError> {
    state.auth.authenticate(&headers, AdminScope::AdminWrite)?;
    Ok(Json(provider_service::set_auto_include(
        &state,
        &id,
        &body.pattern,
    )?)
    .into_response())
}
