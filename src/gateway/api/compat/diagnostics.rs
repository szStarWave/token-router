use serde_json::{json, Value};

use crate::gateway::api::openai::{ChatCompletionRequest, Role};

/// Extract `error.message` (or `detail`, or raw text) from an upstream error body.
/// Build a concise client-facing message from an upstream HTTP error body.
pub fn format_upstream_client_error(status: u16, body: &str) -> String {
    if let Some(hint) = upstream_error_message_hint(body) {
        return format!("Upstream request failed: {hint}");
    }
    let first_line = body.lines().next().unwrap_or(body).trim();
    if first_line.is_empty() {
        format!("{status}: upstream request failed")
    } else {
        format!("{status}: {first_line}")
    }
}

pub fn upstream_error_message_hint(body: &str) -> Option<String> {
    if let Ok(v) = serde_json::from_str::<Value>(body) {
        if let Some(msg) = v
            .get("error")
            .and_then(|e| e.get("message"))
            .and_then(|m| m.as_str())
        {
            return Some(msg.to_string());
        }
        if let Some(detail) = v.get("detail").and_then(|d| d.as_str()) {
            return Some(detail.to_string());
        }
    }
    let first_line = body.lines().next().unwrap_or(body);
    let first_line = first_line.trim();
    if !first_line.is_empty() {
        return Some(first_line.to_string());
    }
    None
}

pub fn truncate_preview(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        format!("{}...", &s[..max])
    }
}

/// JSON body returned to the client for `AppError::Upstream`.
pub fn gateway_upstream_error_body(message: &str) -> String {
    serde_json::to_string(&json!({
        "error": {
            "message": message,
            "type": "token_router_error",
        }
    }))
    .unwrap_or_else(|_| message.to_string())
}

/// Log the full request/response exchange when an upstream call fails.
pub fn log_upstream_error_exchange(
    tier: &str,
    url: &str,
    stream: bool,
    status: Option<u16>,
    original_req: &ChatCompletionRequest,
    transformed_req: &ChatCompletionRequest,
    original_response: &str,
    client_error_message: &str,
) {
    let original_request = serde_json::to_string(original_req).unwrap_or_default();
    let transformed_request = serde_json::to_string(transformed_req).unwrap_or_default();
    let transformed_response = gateway_upstream_error_body(client_error_message);
    tracing::error!(
        tier,
        url = %url,
        status = status.unwrap_or(0),
        stream,
        original_request = %original_request,
        transformed_request = %transformed_request,
        original_response = %original_response,
        transformed_response = %transformed_response,
        "upstream error"
    );
}

pub fn count_assistant_tool_calls_missing_reasoning(req: &ChatCompletionRequest) -> usize {
    req.messages
        .iter()
        .filter(|m| {
            m.role == Role::Assistant
                && m.tool_calls.as_ref().is_some_and(|c| !c.is_empty())
                && m.reasoning_content
                    .as_deref()
                    .map(|s| s.trim().is_empty())
                    .unwrap_or(true)
        })
        .count()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_upstream_client_error_extracts_nested_message() {
        let body = r#"{"error":{"message":"Error from provider (Console Go): Upstream request failed","type":"invalid_request_error"}}"#;
        let msg = format_upstream_client_error(400, body);
        assert_eq!(
            msg,
            "Upstream request failed: Error from provider (Console Go): Upstream request failed"
        );
    }
}
