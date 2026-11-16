//! HTTP 错误归一化（§18.4.2）。
//!
//! 这是**唯一允许读错误文本**的地方：把 OpenAI 兼容 / Anthropic 的错误体
//! 归一成 `ErrorClass` 后，下游（sticky、failover、熔断、账本）只认枚举。
//!
//! 分类只在 `upstream` 调用一次；`tr-proto` 这里保持纯函数、零 IO。

use std::collections::BTreeMap;

use serde_json::Value;
use tr_contracts::ErrorClass;

/// 归一化结果。
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ClassifiedError {
    pub class: ErrorClass,
    pub message: Option<String>,
    /// 上游 `type` 字段（OpenAI `invalid_request_error` / Anthropic `overloaded_error`）。
    pub upstream_type: Option<String>,
    /// 上游 `code` 字段（如 `context_length_exceeded`）。
    pub upstream_code: Option<String>,
    /// `retry-after` 归一为毫秒，供接力时尊重。
    pub retry_after_ms: Option<u64>,
}

/// 分类一次上游失败。
pub fn classify(status: u16, headers: &BTreeMap<String, String>, body: &Value) -> ClassifiedError {
    let error = body.get("error").unwrap_or(body);

    let upstream_type = error
        .get("type")
        .and_then(Value::as_str)
        .map(str::to_string);
    let upstream_code = error
        .get("code")
        .and_then(Value::as_str)
        .map(str::to_string);
    let message = error
        .get("message")
        .and_then(Value::as_str)
        .map(str::to_string);

    let retry_after_ms = retry_after_ms(headers);

    let haystack = [
        upstream_type.as_deref().unwrap_or_default(),
        upstream_code.as_deref().unwrap_or_default(),
    ]
    .join(" ")
    .to_ascii_lowercase();

    let class = if haystack.contains("overloaded") {
        // 429/503/529 都可能是过载；认 type/code 更可靠。
        ErrorClass::Overloaded
    } else {
        match status {
            400 => {
                if is_context_overflow(&upstream_code, &message) {
                    ErrorClass::ContextOverflow
                } else {
                    ErrorClass::InvalidRequest
                }
            }
            401 | 403 => ErrorClass::Auth,
            402 => ErrorClass::Quota,
            404 | 422 => ErrorClass::InvalidRequest,
            408 => ErrorClass::Timeout,
            409 => ErrorClass::Protocol,
            413 => ErrorClass::ContextOverflow,
            429 => ErrorClass::RateLimit,
            500 | 502 | 504 => ErrorClass::Transport,
            503 | 529 => ErrorClass::Overloaded,
            _ => ErrorClass::Unknown,
        }
    };

    ClassifiedError {
        class,
        message,
        upstream_type,
        upstream_code,
        retry_after_ms,
    }
}

fn is_context_overflow(code: &Option<String>, message: &Option<String>) -> bool {
    if code.as_deref() == Some("context_length_exceeded") {
        return true;
    }
    let Some(message) = message else {
        return false;
    };
    let message = message.to_ascii_lowercase();
    message.contains("maximum context length")
        || message.contains("prompt is too long")
        || message.contains("exceeds the context window")
        || message.contains("context window of this model")
}

fn retry_after_ms(headers: &BTreeMap<String, String>) -> Option<u64> {
    let lookup = |name: &str| {
        headers
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.trim().to_string())
    };

    if let Some(value) = lookup("retry-after-ms") {
        return value.parse::<u64>().ok();
    }
    let seconds = lookup("retry-after")?.parse::<u64>().ok()?;
    Some(seconds.saturating_mul(1000))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn headers(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
            .collect()
    }

    #[test]
    fn rate_limit_keeps_retry_after() {
        let body =
            json!({"error":{"type":"requests","code":"rate_limit_exceeded","message":"slow down"}});
        let classified = classify(429, &headers(&[("Retry-After", "12")]), &body);
        assert_eq!(classified.class, ErrorClass::RateLimit);
        assert_eq!(classified.retry_after_ms, Some(12_000));
    }

    #[test]
    fn overloaded_by_type_beats_status() {
        let body = json!({"error":{"type":"overloaded_error","message":"Overloaded"}});
        let classified = classify(529, &BTreeMap::new(), &body);
        assert_eq!(classified.class, ErrorClass::Overloaded);
    }

    #[test]
    fn context_overflow_by_code() {
        let body = json!({"error":{"type":"invalid_request_error","code":"context_length_exceeded","message":"too long"}});
        assert_eq!(
            classify(400, &BTreeMap::new(), &body).class,
            ErrorClass::ContextOverflow
        );
    }

    #[test]
    fn context_overflow_by_message_when_code_missing() {
        let body = json!({"error":{"type":"invalid_request_error","message":"Your input exceeds the context window of this model."}});
        assert_eq!(
            classify(400, &BTreeMap::new(), &body).class,
            ErrorClass::ContextOverflow
        );
    }

    #[test]
    fn server_errors_are_transport() {
        assert_eq!(
            classify(500, &BTreeMap::new(), &json!({})).class,
            ErrorClass::Transport
        );
        assert_eq!(
            classify(503, &BTreeMap::new(), &json!({})).class,
            ErrorClass::Overloaded
        );
    }

    #[test]
    fn auth_and_invalid_request_are_distinct() {
        assert_eq!(
            classify(401, &BTreeMap::new(), &json!({})).class,
            ErrorClass::Auth
        );
        assert_eq!(
            classify(404, &BTreeMap::new(), &json!({})).class,
            ErrorClass::InvalidRequest
        );
    }
}
