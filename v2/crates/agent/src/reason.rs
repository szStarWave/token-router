//! 请求原因推断（§4.9.1 的 `RequestReason`）。
//!
//! 规则保守：能从协议形状确定的才推断，其余默认 `User`。客户端可以用
//! `x-token-router-request-reason` 显式覆盖（`server` 负责解析）。

use tr_contracts::{CanonicalRequest, CanonicalRole, RequestReason};

/// 根据请求形状推断原因。
pub fn infer(request: &CanonicalRequest) -> RequestReason {
    if request.meta.previous_response_id.is_some() {
        return RequestReason::Continuation;
    }
    match request.messages.last().map(|message| message.role) {
        Some(CanonicalRole::Tool) => RequestReason::Continuation,
        _ => RequestReason::User,
    }
}

/// 解析显式覆盖头。
pub fn parse_header(value: &str) -> Option<RequestReason> {
    match value.trim().to_ascii_lowercase().as_str() {
        "user" => Some(RequestReason::User),
        "continuation" => Some(RequestReason::Continuation),
        "retry" => Some(RequestReason::Retry),
        "direct" => Some(RequestReason::Direct),
        _ => None,
    }
}

/// 稳定字符串（响应头/日志）。
pub fn as_str(reason: RequestReason) -> &'static str {
    reason.as_str()
}

#[cfg(test)]
mod tests {
    use super::*;
    use tr_contracts::{CanonicalMessage, ContentBlock};

    #[test]
    fn tool_tail_is_continuation() {
        let request = CanonicalRequest {
            messages: vec![CanonicalMessage {
                role: CanonicalRole::Tool,
                blocks: vec![ContentBlock::Text {
                    text: "result".to_string(),
                    cache_control: None,
                }],
                ..CanonicalMessage::default()
            }],
            ..CanonicalRequest::default()
        };
        assert_eq!(infer(&request), RequestReason::Continuation);
    }

    #[test]
    fn user_text_defaults_to_user() {
        let request = CanonicalRequest {
            messages: vec![CanonicalMessage {
                role: CanonicalRole::User,
                blocks: vec![ContentBlock::Text {
                    text: "hi".to_string(),
                    cache_control: None,
                }],
                ..CanonicalMessage::default()
            }],
            ..CanonicalRequest::default()
        };
        assert_eq!(infer(&request), RequestReason::User);
    }

    #[test]
    fn header_parsing() {
        assert_eq!(parse_header("RETRY"), Some(RequestReason::Retry));
        assert_eq!(parse_header("nonsense"), None);
    }
}
