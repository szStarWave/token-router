//! 管理 API 鉴权（P4-019）：scoped token + bootstrap admin token。
//!
//! 规则（契约文档 §3）：
//! - `Authorization: Bearer`（兼容 `x-admin-token`）；
//! - scope 不足 → `403 admin_scope_error`；缺/错/撤销 → `401 admin_auth_error`；
//! - 常数时间比较；撤销即时生效（内存缓存 + 每请求查库）；
//! - 管理面 GET 需要 `admin:read`，写需要 `admin:write`，审批需要 `admin:approve`。

use std::sync::Arc;

use axum::http::HeaderMap;
use tr_agent::store::tokens::{self, TokenRecord};
use tr_contracts::AdminScope;

use crate::error::{AdminApiError, ApiErrorType};

/// 鉴权上下文：token store 所在 DB + bootstrap token 文件路径。
#[derive(Clone)]
pub struct AdminAuth {
    db: Arc<tr_agent::store::Db>,
    /// `<data_dir>/admin.token`（首次启动生成）。
    token_file_path: Option<std::path::PathBuf>,
}

impl AdminAuth {
    pub fn new(db: Arc<tr_agent::store::Db>, token_file_path: Option<std::path::PathBuf>) -> Self {
        Self {
            db,
            token_file_path,
        }
    }

    /// 首次启动：`tokens` 表为空时生成全 scope admin token 写入文件（0600 语义由平台处理）。
    /// 返回明文（供日志打印路径使用；日志只打印路径，不打印明文）。
    pub fn bootstrap_if_empty(&self) -> Result<Option<String>, AdminApiError> {
        let Some(path) = &self.token_file_path else {
            return Ok(None);
        };
        if path.is_file() {
            // 文件已存在：读取明文，确保 token store 里有对应记录（幂等）。
            let plaintext = std::fs::read_to_string(path)
                .map_err(|error| AdminApiError::internal(format!("read admin token: {error}")))?;
            let plaintext = plaintext.trim().to_string();
            if plaintext.is_empty() {
                return Ok(None);
            }
            self.ensure_record(&plaintext)?;
            return Ok(Some(plaintext));
        }
        if !tokens::is_empty(&self.db)
            .map_err(|error| AdminApiError::internal(error.to_string()))?
        {
            return Ok(None);
        }
        let plaintext = tokens::generate_token();
        let record = TokenRecord {
            id: uuid_v4(),
            name: "bootstrap-admin".to_string(),
            token_sha256: tokens::sha256_hex(&plaintext),
            prefix: plaintext.chars().take(8).collect(),
            scopes: vec![
                AdminScope::AdminRead,
                AdminScope::AdminWrite,
                AdminScope::AdminApprove,
            ],
            created_at: now_millis(),
            last_used_at: None,
            expires_at: None,
            revoked_at: None,
        };
        tokens::insert(&self.db, &record)
            .map_err(|error| AdminApiError::internal(error.to_string()))?;
        // 原子写 token 文件。
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|error| AdminApiError::internal(error.to_string()))?;
        }
        let temp = path.with_extension("tmp");
        std::fs::write(&temp, plaintext.as_bytes())
            .map_err(|error| AdminApiError::internal(error.to_string()))?;
        std::fs::rename(&temp, path).map_err(|error| AdminApiError::internal(error.to_string()))?;
        Ok(Some(plaintext))
    }

    /// 确保给定明文的记录存在（重启恢复路径）。
    fn ensure_record(&self, plaintext: &str) -> Result<(), AdminApiError> {
        if tokens::find_active_by_plaintext(&self.db, plaintext, now_millis())
            .map_err(|error| AdminApiError::internal(error.to_string()))?
            .is_some()
        {
            return Ok(());
        }
        let record = TokenRecord {
            id: uuid_v4(),
            name: "bootstrap-admin".to_string(),
            token_sha256: tokens::sha256_hex(plaintext),
            prefix: plaintext.chars().take(8).collect(),
            scopes: vec![
                AdminScope::AdminRead,
                AdminScope::AdminWrite,
                AdminScope::AdminApprove,
            ],
            created_at: now_millis(),
            last_used_at: None,
            expires_at: None,
            revoked_at: None,
        };
        tokens::insert(&self.db, &record)
            .map_err(|error| AdminApiError::internal(error.to_string()))
    }

    /// 鉴权：返回通过校验的 token 记录；失败返回对应错误。
    pub fn authenticate(
        &self,
        headers: &HeaderMap,
        required: AdminScope,
    ) -> Result<TokenRecord, AdminApiError> {
        let provided = headers
            .get(axum::http::header::AUTHORIZATION)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.strip_prefix("Bearer "))
            .or_else(|| {
                headers
                    .get("x-admin-token")
                    .and_then(|value| value.to_str().ok())
            });
        let Some(provided) = provided else {
            return Err(AdminApiError::new(
                ApiErrorType::AdminAuthError,
                "missing admin token",
            ));
        };
        let record = tokens::find_active_by_plaintext(&self.db, provided, now_millis())
            .map_err(|error| AdminApiError::internal(error.to_string()))?
            .ok_or_else(|| {
                AdminApiError::new(ApiErrorType::AdminAuthError, "invalid or revoked token")
            })?;
        if !record.scopes.contains(&required) {
            return Err(AdminApiError::new(
                ApiErrorType::AdminScopeError,
                format!("scope `{}` required", required.as_str()),
            ));
        }
        // last_used_at 每分钟最多一次（写失败不影响请求）。
        let _ = tokens::touch_last_used(&self.db, &record.token_sha256, now_millis());
        Ok(record)
    }
}

fn uuid_v4() -> String {
    // 与 tokens::insert 一致使用 uuid crate。
    uuid::Uuid::new_v4().to_string()
}

fn now_millis() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as i64)
        .unwrap_or(0)
}
