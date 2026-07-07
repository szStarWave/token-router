use serde_json::Value;

use crate::gateway::api::openai::{ChatCompletionRequest, Role};

/// Extract `error.message` (or `detail`, or raw text) from an upstream error body.
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
