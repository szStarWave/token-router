use axum::http::HeaderMap;

use crate::gateway::error::{AppError, AppResult};

/// Validate client API key when inbound keys are configured.
/// Returns `Ok(None)` when auth is disabled; `Ok(Some(key))` when matched.
pub fn require_gateway_api_key(headers: &HeaderMap, expected: &[String]) -> AppResult<Option<String>> {
    if expected.is_empty() {
        return Ok(None);
    }

    match extract_api_key(headers) {
        Some(provided) if expected.iter().any(|key| key == &provided) => Ok(Some(provided)),
        _ => Err(AppError::Unauthorized(
            "invalid or missing API key (use Authorization: Bearer <key>)".into(),
        )),
    }
}

pub fn extract_api_key(headers: &HeaderMap) -> Option<String> {
    if let Some(value) = headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
    {
        if let Some(token) = value.strip_prefix("Bearer ") {
            let token = token.trim();
            if !token.is_empty() {
                return Some(token.to_string());
            }
        }
    }

    headers
        .get("x-api-key")
        .and_then(|v| v.to_str().ok())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderValue;

    #[test]
    fn skips_auth_when_not_configured() {
        let headers = HeaderMap::new();
        assert!(require_gateway_api_key(&headers, &[]).unwrap().is_none());
    }

    #[test]
    fn accepts_bearer_token() {
        let mut headers = HeaderMap::new();
        headers.insert(
            "authorization",
            HeaderValue::from_static("Bearer flowy-local"),
        );
        let keys = vec!["flowy-local".to_string()];
        assert_eq!(
            require_gateway_api_key(&headers, &keys).unwrap(),
            Some("flowy-local".to_string())
        );
    }

    #[test]
    fn rejects_wrong_key() {
        let mut headers = HeaderMap::new();
        headers.insert(
            "authorization",
            HeaderValue::from_static("Bearer wrong"),
        );
        let keys = vec!["flowy-local".to_string()];
        assert!(matches!(
            require_gateway_api_key(&headers, &keys),
            Err(AppError::Unauthorized(_))
        ));
    }
}
