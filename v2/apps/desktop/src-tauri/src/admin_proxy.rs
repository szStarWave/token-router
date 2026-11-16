//! Admin 代理（技术方案 §6.3）：token 只在 Rust 侧，不进 WebView。
//!
//! 规则：
//! - `admin_request` 只接受 `/v2/admin` 前缀的相对路径；拒绝绝对 URL 与 `..`；
//! - 从 Rust 内存读 token，注入 `Authorization: Bearer`；
//! - 统一错误映射 `{status, error:{type,message,issues}}`；
//! - 响应体上限 8 MiB。

use serde::{Deserialize, Serialize};

/// admin 请求入参（WebView 侧不携带 token）。
#[derive(Debug, Deserialize)]
pub struct AdminRequest {
    pub method: String,
    /// 必须以 `/v2/admin` 开头。
    pub path: String,
    #[serde(default)]
    pub body: Option<serde_json::Value>,
}

/// admin 响应（成功 2xx 返回 200 + 原始 JSON；错误返回 status + 信封）。
#[derive(Debug, Serialize)]
pub struct AdminResponse {
    pub status: u16,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub body: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<serde_json::Value>,
}

/// 路径白名单校验。
pub fn validate_path(path: &str) -> Result<(), String> {
    if !path.starts_with("/v2/admin") {
        return Err(format!("path must start with /v2/admin, got `{path}`"));
    }
    if path.contains("..") {
        return Err("path traversal rejected".to_string());
    }
    if path.starts_with("http://") || path.starts_with("https://") {
        return Err("absolute URL rejected".to_string());
    }
    Ok(())
}

/// 方法白名单。
pub fn validate_method(method: &str) -> Result<(), String> {
    match method.to_ascii_uppercase().as_str() {
        "GET" | "POST" | "PATCH" | "DELETE" => Ok(()),
        other => Err(format!("method `{other}` not allowed")),
    }
}

pub const MAX_RESPONSE_BYTES: usize = 8 * 1024 * 1024;

/// 转发请求：`base` 是服务端 base URL，`token` 来自 Rust 侧内存。
pub async fn forward(
    base: &str,
    token: &str,
    request: &AdminRequest,
) -> AdminResponse {
    if let Err(message) = validate_path(&request.path) {
        return error_response(400, "invalid_request", &message);
    }
    if let Err(message) = validate_method(&request.method) {
        return error_response(400, "invalid_request", &message);
    }

    let client = match reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
    {
        Ok(client) => client,
        Err(error) => return error_response(500, "internal_error", &error.to_string()),
    };

    let url = format!("{}{}", base.trim_end_matches('/'), request.path);
    let method = reqwest::Method::from_bytes(request.method.as_bytes())
        .unwrap_or(reqwest::Method::GET);
    let mut builder = client.request(method, &url).bearer_auth(token);
    if let Some(body) = &request.body {
        builder = builder.json(body);
    }

    let response = match builder.send().await {
        Ok(response) => response,
        Err(error) => {
            return error_response(502, "server_unavailable", &format!("transport: {error}"));
        }
    };
    let status = response.status().as_u16();

    // 响应体大小限制。
    let bytes = match response.bytes().await {
        Ok(bytes) => bytes,
        Err(error) => return error_response(502, "server_unavailable", &error.to_string()),
    };
    if bytes.len() > MAX_RESPONSE_BYTES {
        return error_response(413, "invalid_request", "response too large");
    }

    let body: serde_json::Value = serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null);
    if (200..300).contains(&status) {
        AdminResponse {
            status,
            body: Some(body),
            error: None,
        }
    } else {
        AdminResponse {
            status,
            body: None,
            error: Some(body),
        }
    }
}

fn error_response(status: u16, kind: &str, message: &str) -> AdminResponse {
    AdminResponse {
        status,
        body: None,
        error: Some(serde_json::json!({
            "error": {
                "type": kind,
                "message": message,
            }
        })),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn path_whitelist() {
        assert!(validate_path("/v2/admin/status").is_ok());
        assert!(validate_path("/v1/chat/completions").is_err());
        assert!(validate_path("http://evil.example/v2/admin").is_err());
        assert!(validate_path("/v2/admin/../secret").is_err());
    }

    #[test]
    fn method_whitelist() {
        for method in ["GET", "POST", "PATCH", "DELETE"] {
            assert!(validate_method(method).is_ok());
        }
        assert!(validate_method("PUT").is_err());
        assert!(validate_method("TRACE").is_err());
    }
}
