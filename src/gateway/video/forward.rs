//! Same-host video passthrough. The caller supplies the upstream method, path, and body.

use std::time::Duration;

use axum::http::{HeaderMap, HeaderName, HeaderValue, Method, StatusCode};
use bytes::Bytes;
use reqwest::Client;

use crate::gateway::config::ResolvedVideoUpstream;
use crate::gateway::error::{AppError, AppResult};

const FORWARD_TIMEOUT: Duration = Duration::from_secs(180);
pub const MAX_FORWARD_BODY: usize = 64 * 1024 * 1024;

const FORWARDED_HEADER_NAMES: &[&str] = &["content-type", "accept", "x-dashscope-async"];

#[derive(Debug, Clone)]
pub struct VideoForwardRequest {
    pub method: Method,
    pub path: String,
    pub headers: HeaderMap,
    pub body: Bytes,
}

#[derive(Debug, Clone)]
pub struct VideoForwardResponse {
    pub status: StatusCode,
    pub content_type: Option<String>,
    pub body: Bytes,
}

pub fn forward_client() -> Client {
    Client::builder()
        .timeout(FORWARD_TIMEOUT)
        .build()
        .unwrap_or_else(|_| Client::new())
}

pub fn parse_forward_method(raw: &str) -> AppResult<Method> {
    match raw.trim().to_ascii_uppercase().as_str() {
        "GET" => Ok(Method::GET),
        "POST" => Ok(Method::POST),
        "PUT" => Ok(Method::PUT),
        "PATCH" => Ok(Method::PATCH),
        "DELETE" => Ok(Method::DELETE),
        "HEAD" => Ok(Method::HEAD),
        other => Err(AppError::BadRequest(format!(
            "unsupported video forward method `{other}`"
        ))),
    }
}

/// Absolute path on the upstream host. Query is allowed. `..`, schemes, and other hosts are not.
pub fn resolve_forward_url(base_url: &str, path_and_query: &str) -> AppResult<reqwest::Url> {
    let path = path_and_query.trim();
    if path.is_empty() || !path.starts_with('/') || path.starts_with("//") {
        return Err(AppError::BadRequest(
            "video forward path must be an absolute path on the upstream host".into(),
        ));
    }
    if path.contains('\\') || path.contains("://") || path.contains('#') {
        return Err(AppError::BadRequest(
            "video forward path must not include a scheme, host, or fragment".into(),
        ));
    }
    let (path_only, _) = path.split_once('?').unwrap_or((path, ""));
    let decoded = percent_decode_lossy(path_only);
    if decoded.split('/').any(|seg| seg == "..") {
        return Err(AppError::BadRequest(
            "video forward path must not contain `..`".into(),
        ));
    }

    let base = reqwest::Url::parse(base_url.trim()).map_err(|_| {
        AppError::Unavailable(format!("video upstream base_url is invalid: {base_url}"))
    })?;
    let host = base.host_str().ok_or_else(|| {
        AppError::Unavailable("video upstream base_url has no host".into())
    })?;
    let origin = match base.port() {
        Some(port) => format!("{}://{host}:{port}", base.scheme()),
        None => format!("{}://{host}", base.scheme()),
    };
    let url = reqwest::Url::parse(&format!("{origin}{path}")).map_err(|e| {
        AppError::BadRequest(format!("video forward url: {e}"))
    })?;
    if url.scheme() != base.scheme()
        || url.host_str() != base.host_str()
        || url.port_or_known_default() != base.port_or_known_default()
    {
        return Err(AppError::BadRequest(
            "video forward host does not match the configured upstream".into(),
        ));
    }
    Ok(url)
}

pub fn forwarded_headers(incoming: &HeaderMap) -> HeaderMap {
    let mut out = HeaderMap::new();
    for name in FORWARDED_HEADER_NAMES {
        if let Some(value) = incoming.get(*name) {
            if let Ok(header_name) = HeaderName::try_from(*name) {
                out.insert(header_name, value.clone());
            }
        }
    }
    out
}

pub async fn forward_video(
    http: &Client,
    target: &ResolvedVideoUpstream,
    req: &VideoForwardRequest,
) -> AppResult<VideoForwardResponse> {
    let base = target
        .base_url
        .as_deref()
        .filter(|s| !s.trim().is_empty())
        .ok_or_else(|| AppError::Unavailable("video upstream base_url missing".into()))?;
    let url = resolve_forward_url(base, &req.path)?;
    let mut builder = http.request(req.method.clone(), url);
    for (name, value) in forwarded_headers(&req.headers).iter() {
        builder = builder.header(name, value);
    }
    if let Some(key) = target.api_key.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        builder = builder.bearer_auth(key);
    }
    if !req.body.is_empty() && req.method != Method::GET && req.method != Method::HEAD {
        builder = builder.body(req.body.clone());
    }

    let resp = builder
        .send()
        .await
        .map_err(|e| AppError::Upstream(format!("video forward: {e}")))?;
    let status = StatusCode::from_u16(resp.status().as_u16())
        .unwrap_or(StatusCode::BAD_GATEWAY);
    let content_type = resp
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .map(str::to_string);
    let body = resp
        .bytes()
        .await
        .map_err(|e| AppError::Upstream(format!("video forward body: {e}")))?;
    Ok(VideoForwardResponse {
        status,
        content_type,
        body,
    })
}

fn percent_decode_lossy(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Ok(hex) = std::str::from_utf8(&bytes[i + 1..i + 3]) {
                if let Ok(value) = u8::from_str_radix(hex, 16) {
                    out.push(value);
                    i += 3;
                    continue;
                }
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{Router, extract::Request, http::header, routing::any};
    use tokio::net::TcpListener;

    #[test]
    fn same_host_path_ignores_base_prefix() {
        let url = resolve_forward_url(
            "https://server.flowyaipc.cn/claw/v1",
            "/claw/video/generations/tasks",
        )
        .unwrap();
        assert_eq!(
            url.as_str(),
            "https://server.flowyaipc.cn/claw/video/generations/tasks"
        );

        let edge = resolve_forward_url(
            "http://127.0.0.1:8080/v1",
            "/v1/videos/generations?limit=1",
        )
        .unwrap();
        assert_eq!(
            edge.as_str(),
            "http://127.0.0.1:8080/v1/videos/generations?limit=1"
        );
    }

    #[test]
    fn rejects_dotdot_scheme_and_other_host() {
        assert!(resolve_forward_url("https://api.example/v1", "/a/../b").is_err());
        assert!(resolve_forward_url("https://api.example/v1", "/%2e%2e/secret").is_err());
        assert!(resolve_forward_url("https://api.example/v1", "https://evil.test/x").is_err());
        assert!(resolve_forward_url("https://api.example/v1", "//evil.test/x").is_err());
        assert!(resolve_forward_url("https://api.example/v1", "videos").is_err());
    }

    #[tokio::test]
    async fn forwards_body_status_and_replaces_authorization() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let app = Router::new().route(
            "/v2/video_generation",
            any(|request: Request| async move {
                let auth = request
                    .headers()
                    .get(header::AUTHORIZATION)
                    .and_then(|v| v.to_str().ok())
                    .unwrap_or("")
                    .to_string();
                let dash = request
                    .headers()
                    .get("x-dashscope-async")
                    .and_then(|v| v.to_str().ok())
                    .unwrap_or("")
                    .to_string();
                let body = axum::body::to_bytes(request.into_body(), 1024 * 1024)
                    .await
                    .unwrap();
                let payload = format!(
                    r#"{{"auth":"{auth}","dash":"{dash}","body":{}}}"#,
                    String::from_utf8(body.to_vec()).unwrap_or_default()
                );
                (
                    StatusCode::ACCEPTED,
                    [(header::CONTENT_TYPE, "application/json")],
                    payload,
                )
            }),
        );
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });

        let http = forward_client();
        let target = ResolvedVideoUpstream {
            provider: "minimax".into(),
            base_url: Some(format!("http://{addr}/v1")),
            api_key: Some("sk-upstream".into()),
            model: None,
            upstream_model: None,
            workflow_file: None,
            workflow_file_i2v: None,
        };
        let mut headers = HeaderMap::new();
        headers.insert(
            header::AUTHORIZATION,
            HeaderValue::from_static("Bearer client-key"),
        );
        headers.insert(
            header::CONTENT_TYPE,
            HeaderValue::from_static("application/json"),
        );
        headers.insert(
            HeaderName::from_static("x-dashscope-async"),
            HeaderValue::from_static("enable"),
        );
        let resp = forward_video(
            &http,
            &target,
            &VideoForwardRequest {
                method: Method::POST,
                path: "/v2/video_generation".into(),
                headers,
                body: Bytes::from_static(br#"{"model":"MiniMax-H3"}"#),
            },
        )
        .await
        .unwrap();

        assert_eq!(resp.status, StatusCode::ACCEPTED);
        assert_eq!(resp.content_type.as_deref(), Some("application/json"));
        let text = String::from_utf8(resp.body.to_vec()).unwrap();
        assert!(text.contains("Bearer sk-upstream"), "{text}");
        assert!(!text.contains("client-key"), "{text}");
        assert!(text.contains(r#""dash":"enable""#), "{text}");
        assert!(text.contains("MiniMax-H3"), "{text}");
    }

    #[tokio::test]
    async fn passes_upstream_error_status_through() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let app = Router::new().route(
            "/fail",
            any(|| async {
                (
                    StatusCode::UNPROCESSABLE_ENTITY,
                    [(header::CONTENT_TYPE, "application/json")],
                    r#"{"msg":"nope"}"#,
                )
            }),
        );
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });

        let resp = forward_video(
            &forward_client(),
            &ResolvedVideoUpstream {
                provider: "openai".into(),
                base_url: Some(format!("http://{addr}")),
                api_key: None,
                model: None,
                upstream_model: None,
                workflow_file: None,
                workflow_file_i2v: None,
            },
            &VideoForwardRequest {
                method: Method::GET,
                path: "/fail".into(),
                headers: HeaderMap::new(),
                body: Bytes::new(),
            },
        )
        .await
        .unwrap();
        assert_eq!(resp.status, StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(resp.body.as_ref(), br#"{"msg":"nope"}"#);
    }
}
