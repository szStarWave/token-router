//! 统一错误信封（P4-001 §1）：集中定义错误码，handler 禁止手写字符串。

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use tr_contracts::{ApiErrorBody, ApiErrorEnvelope, ConfigIssue};

/// 错误类型（契约冻结；对应 HTTP 状态见契约文档表）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApiErrorType {
    InvalidRequest,
    AdminAuthError,
    AdminScopeError,
    NotFound,
    ConfigConflict,
    WindowTooLarge,
    ConfigInvalid,
    InternalError,
    ServerUnavailable,
}

impl ApiErrorType {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::InvalidRequest => "invalid_request",
            Self::AdminAuthError => "admin_auth_error",
            Self::AdminScopeError => "admin_scope_error",
            Self::NotFound => "not_found",
            Self::ConfigConflict => "config_conflict",
            Self::WindowTooLarge => "window_too_large",
            Self::ConfigInvalid => "config_invalid",
            Self::InternalError => "internal_error",
            Self::ServerUnavailable => "server_unavailable",
        }
    }

    pub fn status(self) -> StatusCode {
        match self {
            Self::InvalidRequest => StatusCode::BAD_REQUEST,
            Self::AdminAuthError => StatusCode::UNAUTHORIZED,
            Self::AdminScopeError => StatusCode::FORBIDDEN,
            Self::NotFound => StatusCode::NOT_FOUND,
            Self::ConfigConflict => StatusCode::CONFLICT,
            Self::WindowTooLarge => StatusCode::PAYLOAD_TOO_LARGE,
            Self::ConfigInvalid => StatusCode::UNPROCESSABLE_ENTITY,
            Self::InternalError => StatusCode::INTERNAL_SERVER_ERROR,
            Self::ServerUnavailable => StatusCode::SERVICE_UNAVAILABLE,
        }
    }
}

/// 管理 API 错误。
#[derive(Debug, Clone)]
pub struct AdminApiError {
    pub kind: ApiErrorType,
    pub message: String,
    pub issues: Vec<ConfigIssue>,
}

impl AdminApiError {
    pub fn new(kind: ApiErrorType, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
            issues: Vec::new(),
        }
    }

    /// `std::error::Error` 实现（main.rs 用 `?` 直接传播）。
    pub fn message(&self) -> &str {
        &self.message
    }

    pub fn invalid_request(message: impl Into<String>) -> Self {
        Self::new(ApiErrorType::InvalidRequest, message)
    }

    pub fn not_found(message: impl Into<String>) -> Self {
        Self::new(ApiErrorType::NotFound, message)
    }

    pub fn with_issues(
        kind: ApiErrorType,
        message: impl Into<String>,
        issues: Vec<ConfigIssue>,
    ) -> Self {
        Self {
            kind,
            message: message.into(),
            issues,
        }
    }

    pub fn internal(message: impl Into<String>) -> Self {
        Self::new(ApiErrorType::InternalError, message)
    }
}

impl From<tr_agent::store::StoreError> for AdminApiError {
    fn from(error: tr_agent::store::StoreError) -> Self {
        Self::internal(error.to_string())
    }
}

impl std::fmt::Display for AdminApiError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{}: {}", self.kind.as_str(), self.message)
    }
}

impl std::error::Error for AdminApiError {}

impl IntoResponse for AdminApiError {
    fn into_response(self) -> Response {
        let body = ApiErrorEnvelope {
            error: ApiErrorBody {
                r#type: self.kind.as_str().to_string(),
                message: self.message,
                request_id: None,
                issues: if self.issues.is_empty() {
                    None
                } else {
                    Some(self.issues)
                },
            },
        };
        (self.kind.status(), axum::Json(body)).into_response()
    }
}
