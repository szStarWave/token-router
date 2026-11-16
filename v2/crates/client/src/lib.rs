//! `tr-client`：v2 管理 API 的 Rust HTTP 客户端（P4-045）。
//!
//! CLI / 集成测试 / 桌面壳共用同一套类型与错误映射；
//! 桌面壳用它替换自己的手写 HTTP 调用，UI 仍经 Tauri 代理。
//!
//! 边界：只依赖 `tr-contracts`（契约 DTO）与 `reqwest`；不做鉴权决策、
//! 不读配置文件——token 由调用方注入。

#![forbid(unsafe_code)]

use std::time::Duration;

use serde_json::Value;
use tr_contracts::{
    AdminScope, ApplyRequest, ApplyResponse, ConfigAuditEntry, ConfigDocument, CreateTokenRequest,
    CreatedToken, DoctorResponse, HealthResponse, LogEntry, RequestDetail, RequestSummary,
    StatsSummary, StatusResponse, TokenInfo, ValidateResponse,
};

/// 客户端错误：与契约错误信封一一对应。
#[derive(Debug, thiserror::Error)]
pub enum ClientError {
    /// 请求本身无法发出（网络/超时）。
    #[error("transport: {0}")]
    Transport(String),
    /// 服务端返回错误信封（status + type + issues）。
    #[error("{status} {kind}: {message}")]
    Api {
        status: u16,
        kind: String,
        message: String,
        issues: Vec<tr_contracts::ConfigIssue>,
    },
    /// 响应体无法按契约解析。
    #[error("decode: {0}")]
    Decode(String),
}

/// 管理客户端。
pub struct AdminClient {
    http: reqwest::Client,
    base_url: String,
    token: String,
}

/// 分页查询参数。
#[derive(Debug, Default, Clone)]
pub struct PageQuery {
    pub limit: Option<u32>,
    pub cursor: Option<String>,
}

/// 时间窗查询参数（Unix 毫秒，左闭右开）。
#[derive(Debug, Default, Clone)]
pub struct WindowQuery {
    pub from: Option<i64>,
    pub to: Option<i64>,
}

/// `GET /v2/admin/requests` 的过滤条件。
#[derive(Debug, Default, Clone)]
pub struct RequestFilter {
    pub session_key: Option<String>,
    pub model: Option<String>,
    pub outcome: Option<String>,
    pub protocol: Option<String>,
    pub reason_code: Option<String>,
    pub limit: Option<u32>,
    pub cursor: Option<String>,
}

impl AdminClient {
    /// 创建客户端；`base_url` 形如 `http://127.0.0.1:17621`。
    pub fn new(base_url: impl Into<String>, token: impl Into<String>) -> Result<Self, ClientError> {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .map_err(|error| ClientError::Transport(error.to_string()))?;
        Ok(Self {
            http,
            base_url: base_url.into().trim_end_matches('/').to_string(),
            token: token.into(),
        })
    }

    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    /// 当前 token 的 scope 是否覆盖所需作用域（客户端预检；服务端仍会校验）。
    pub fn scope_note(scope: AdminScope) -> &'static str {
        scope.as_str()
    }

    async fn get(&self, path: &str, query: &[(&str, &str)]) -> Result<Value, ClientError> {
        let mut request = self
            .http
            .get(format!("{}{path}", self.base_url))
            .bearer_auth(&self.token);
        for (key, value) in query {
            if !value.is_empty() {
                request = request.query(&[(key, value)]);
            }
        }
        self.send(request).await
    }

    async fn post(&self, path: &str, body: &Value) -> Result<Value, ClientError> {
        let request = self
            .http
            .post(format!("{}{path}", self.base_url))
            .bearer_auth(&self.token)
            .json(body);
        self.send(request).await
    }

    async fn send(&self, request: reqwest::RequestBuilder) -> Result<Value, ClientError> {
        let response = request
            .send()
            .await
            .map_err(|error| ClientError::Transport(error.to_string()))?;
        let status = response.status().as_u16();
        let body: Value = response
            .json()
            .await
            .map_err(|error| ClientError::Decode(error.to_string()))?;
        if !(200..300).contains(&status) {
            let error = &body["error"];
            return Err(ClientError::Api {
                status,
                kind: error["type"]
                    .as_str()
                    .unwrap_or("internal_error")
                    .to_string(),
                message: error["message"]
                    .as_str()
                    .unwrap_or("no message")
                    .to_string(),
                issues: error["issues"]
                    .as_array()
                    .map(|items| {
                        items
                            .iter()
                            .filter_map(|item| serde_json::from_value(item.clone()).ok())
                            .collect()
                    })
                    .unwrap_or_default(),
            });
        }
        Ok(body)
    }

    fn decode<T: serde::de::DeserializeOwned>(value: Value) -> Result<T, ClientError> {
        serde_json::from_value(value).map_err(|error| ClientError::Decode(error.to_string()))
    }

    /// `GET /v2/admin/status`
    pub async fn status(&self) -> Result<StatusResponse, ClientError> {
        Self::decode(self.get("/v2/admin/status", &[]).await?)
    }

    /// `GET /v2/admin/health`
    pub async fn health(&self) -> Result<HealthResponse, ClientError> {
        Self::decode(self.get("/v2/admin/health", &[]).await?)
    }

    /// `GET /v2/admin/doctor`
    pub async fn doctor(&self) -> Result<DoctorResponse, ClientError> {
        Self::decode(self.get("/v2/admin/doctor", &[]).await?)
    }

    /// `GET /v2/admin/config`
    pub async fn config(&self) -> Result<ConfigDocument, ClientError> {
        Self::decode(self.get("/v2/admin/config", &[]).await?)
    }

    /// `POST /v2/admin/config/validate`
    pub async fn config_validate(
        &self,
        document_toml: &str,
    ) -> Result<ValidateResponse, ClientError> {
        let body = serde_json::json!({ "document_toml": document_toml });
        Self::decode(self.post("/v2/admin/config/validate", &body).await?)
    }

    /// `POST /v2/admin/config/apply`（带 `If-Match` CAS）。
    pub async fn config_apply(
        &self,
        revision: &str,
        request: &ApplyRequest,
    ) -> Result<ApplyResponse, ClientError> {
        let value = serde_json::to_value(request)
            .map_err(|error| ClientError::Decode(error.to_string()))?;
        let response = self
            .http
            .post(format!("{}/v2/admin/config/apply", self.base_url))
            .bearer_auth(&self.token)
            .header("If-Match", revision)
            .json(&value)
            .send()
            .await
            .map_err(|error| ClientError::Transport(error.to_string()))?;
        let status = response.status().as_u16();
        let body: Value = response
            .json()
            .await
            .map_err(|error| ClientError::Decode(error.to_string()))?;
        if !(200..300).contains(&status) {
            let error = &body["error"];
            return Err(ClientError::Api {
                status,
                kind: error["type"]
                    .as_str()
                    .unwrap_or("internal_error")
                    .to_string(),
                message: error["message"]
                    .as_str()
                    .unwrap_or("no message")
                    .to_string(),
                issues: Vec::new(),
            });
        }
        Self::decode(body)
    }

    /// `GET /v2/admin/config/audit`
    pub async fn config_audit(&self) -> Result<Vec<ConfigAuditEntry>, ClientError> {
        Self::decode(self.get("/v2/admin/config/audit", &[]).await?)
    }

    /// `GET /v2/admin/requests`
    pub async fn requests(
        &self,
        filter: &RequestFilter,
    ) -> Result<Vec<RequestSummary>, ClientError> {
        let mut query: Vec<(&str, String)> = Vec::new();
        let pairs_source = [
            ("session", filter.session_key.clone()),
            ("model", filter.model.clone()),
            ("outcome", filter.outcome.clone()),
            ("protocol", filter.protocol.clone()),
            ("reason", filter.reason_code.clone()),
        ];
        for (key, value) in pairs_source
            .into_iter()
            .filter_map(|(key, value)| value.map(|value| (key, value)))
        {
            query.push((key, value));
        }
        if let Some(limit) = filter.limit {
            query.push(("limit", limit.to_string()));
        }
        if let Some(cursor) = &filter.cursor {
            query.push(("cursor", cursor.clone()));
        }
        let pairs: Vec<(&str, &str)> = query.iter().map(|(k, v)| (*k, v.as_str())).collect();
        let page: tr_contracts::Page<RequestSummary> =
            Self::decode(self.get("/v2/admin/requests", &pairs).await?)?;
        Ok(page.items)
    }

    /// `GET /v2/admin/requests/{id}`
    pub async fn request_detail(&self, id: &str) -> Result<RequestDetail, ClientError> {
        Self::decode(self.get(&format!("/v2/admin/requests/{id}"), &[]).await?)
    }

    /// `GET /v2/admin/stats/summary`
    pub async fn stats_summary(&self, window: &WindowQuery) -> Result<StatsSummary, ClientError> {
        let from = window
            .from
            .map(|value| value.to_string())
            .unwrap_or_default();
        let to = window.to.map(|value| value.to_string()).unwrap_or_default();
        Self::decode(
            self.get("/v2/admin/stats/summary", &[("from", &from), ("to", &to)])
                .await?,
        )
    }

    /// `GET /v2/admin/tokens`
    pub async fn tokens(&self) -> Result<Vec<TokenInfo>, ClientError> {
        let page: tr_contracts::Page<TokenInfo> =
            Self::decode(self.get("/v2/admin/tokens", &[]).await?)?;
        Ok(page.items)
    }

    /// `POST /v2/admin/tokens`（明文只返回一次）。
    pub async fn create_token(
        &self,
        request: &CreateTokenRequest,
    ) -> Result<CreatedToken, ClientError> {
        let value = serde_json::to_value(request)
            .map_err(|error| ClientError::Decode(error.to_string()))?;
        Self::decode(self.post("/v2/admin/tokens", &value).await?)
    }

    /// `DELETE /v2/admin/tokens/{id}`
    pub async fn revoke_token(&self, id: &str) -> Result<(), ClientError> {
        let response = self
            .http
            .delete(format!("{}/v2/admin/tokens/{id}", self.base_url))
            .bearer_auth(&self.token)
            .send()
            .await
            .map_err(|error| ClientError::Transport(error.to_string()))?;
        let status = response.status().as_u16();
        if !(200..300).contains(&status) {
            let body: Value = response
                .json()
                .await
                .map_err(|error| ClientError::Decode(error.to_string()))?;
            let error = &body["error"];
            return Err(ClientError::Api {
                status,
                kind: error["type"]
                    .as_str()
                    .unwrap_or("internal_error")
                    .to_string(),
                message: error["message"]
                    .as_str()
                    .unwrap_or("no message")
                    .to_string(),
                issues: Vec::new(),
            });
        }
        Ok(())
    }

    /// `GET /v2/admin/logs`
    pub async fn logs(
        &self,
        cursor: Option<i64>,
        limit: Option<u32>,
    ) -> Result<Vec<LogEntry>, ClientError> {
        let cursor_text = cursor.map(|value| value.to_string()).unwrap_or_default();
        let limit_text = limit.map(|value| value.to_string()).unwrap_or_default();
        let page: tr_contracts::Page<LogEntry> = Self::decode(
            self.get(
                "/v2/admin/logs",
                &[("cursor", &cursor_text), ("limit", &limit_text)],
            )
            .await?,
        )?;
        Ok(page.items)
    }

    /// `POST /v2/admin/shutdown`
    pub async fn shutdown(&self) -> Result<(), ClientError> {
        self.post("/v2/admin/shutdown", &serde_json::json!({}))
            .await
            .map(|_| ())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scope_note_reports_label() {
        assert_eq!(AdminClient::scope_note(AdminScope::AdminRead), "admin:read");
    }
}
