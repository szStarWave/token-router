//! 上游错误归一化（§18.4.2）。
//!
//! 分类只在 `upstream` 发生一次，随后写入 `FailedDispatch.error_class`、账本 attempt
//! 与熔断器。下游（sticky、failover、SSE、账本）**不得**再匹配错误文本。

use serde::{Deserialize, Serialize};

/// 上游错误分类。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorClass {
    /// 连接失败 / 重置 / DNS / TLS。
    Transport,
    /// 读写超时或整体 deadline。
    Timeout,
    /// 429 且带 `retry-after` / `x-ratelimit-*`。
    RateLimit,
    /// 429 `overloaded_error` / 503。
    Overloaded,
    /// 401 / 403：候选没问题，凭据有问题。
    Auth,
    /// 配额用尽（402 / quota 错误体）。
    Quota,
    /// 输入超出上下文窗口（413、`context_length_exceeded`）。
    ContextOverflow,
    /// 请求本身非法（400 / 404 model_not_found / 422）。
    InvalidRequest,
    /// 内容策略拒绝。
    ContentPolicy,
    /// 上游违反协议：非法 JSON、坏帧、事件顺序错。
    Protocol,
    /// 客户端主动断开。
    Cancelled,
    #[default]
    Unknown,
}

impl ErrorClass {
    /// 与 `ReasonCodeKind` / 账本 / 日志一致的稳定字符串。
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Transport => "transport",
            Self::Timeout => "timeout",
            Self::RateLimit => "rate_limit",
            Self::Overloaded => "overloaded",
            Self::Auth => "auth",
            Self::Quota => "quota",
            Self::ContextOverflow => "context_overflow",
            Self::InvalidRequest => "invalid_request",
            Self::ContentPolicy => "content_policy",
            Self::Protocol => "protocol",
            Self::Cancelled => "cancelled",
            Self::Unknown => "unknown",
        }
    }

    /// 是否触发 `STICKY_RETRY_SWITCH`（§4.9.1 的 `is_overload_or_overflow`）。
    ///
    /// 只有这两类允许 Retry 跳过 `StickyFailed`：复用必然失败或必然溢出。
    pub const fn forces_sticky_switch(self) -> bool {
        matches!(self, Self::Overloaded | Self::ContextOverflow)
    }

    /// 是否计入候选熔断（`InvalidRequest` / `Auth` / `Quota` 是请求或凭据问题）。
    pub const fn counts_toward_breaker(self) -> bool {
        matches!(
            self,
            Self::Transport | Self::Timeout | Self::Overloaded | Self::Protocol | Self::Unknown
        )
    }

    /// 是否允许在未提交字节时换候选。
    pub const fn retryable_elsewhere(self) -> bool {
        matches!(
            self,
            Self::Transport
                | Self::Timeout
                | Self::RateLimit
                | Self::Overloaded
                | Self::Auth
                | Self::Quota
                | Self::Protocol
                | Self::Unknown
        )
    }

    /// 熔断计数权重（以 0.5 为单位，避免浮点）：正常 2，限流 1，其余 0。
    pub const fn breaker_weight_half_units(self) -> u32 {
        match self {
            Self::Transport | Self::Timeout | Self::Protocol | Self::Unknown => 2,
            Self::RateLimit => 1,
            Self::Overloaded => 4,
            _ => 0,
        }
    }
}

impl std::fmt::Display for ErrorClass {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// 是否属于“overload 或上下文溢出”（§4.9.1）。
pub fn is_overload_or_overflow(class: Option<ErrorClass>) -> bool {
    matches!(
        class,
        Some(ErrorClass::Overloaded | ErrorClass::ContextOverflow)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sticky_switch_only_for_overload_and_overflow() {
        assert!(is_overload_or_overflow(Some(ErrorClass::Overloaded)));
        assert!(is_overload_or_overflow(Some(ErrorClass::ContextOverflow)));
        assert!(!is_overload_or_overflow(Some(ErrorClass::RateLimit)));
        assert!(!is_overload_or_overflow(None));
    }

    #[test]
    fn breaker_weights_match_behavior_matrix() {
        assert_eq!(ErrorClass::Auth.breaker_weight_half_units(), 0);
        assert_eq!(ErrorClass::InvalidRequest.breaker_weight_half_units(), 0);
        assert_eq!(ErrorClass::RateLimit.breaker_weight_half_units(), 1);
        assert_eq!(ErrorClass::Transport.breaker_weight_half_units(), 2);
        assert_eq!(ErrorClass::Overloaded.breaker_weight_half_units(), 4);
    }

    #[test]
    fn serde_is_snake_case() {
        let json = serde_json::to_string(&ErrorClass::ContextOverflow).unwrap();
        assert_eq!(json, "\"context_overflow\"");
    }
}
