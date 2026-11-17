//! 配额 DTO（v3 P5-B）。
//!
//! 边界：
//! - `RateLimitSnapshot` 由 `upstream` 从响应头/429 解析，经 `agent` 落库；
//! - `QuotaWindowInfo` 是管理面只读视图；
//! - 金额一律 micros，时间一律 Unix 毫秒。

use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// 一次上游响应的配额观测（429 或带 `x-ratelimit-*` 头的成功响应）。
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct RateLimitSnapshot {
    /// 窗口内剩余额度（请求数或 token，由 `unit` 标注）。
    pub remaining: Option<i64>,
    /// 窗口总额度。
    pub limit: Option<i64>,
    /// 重置时间（Unix 毫秒）。
    pub reset_at: Option<i64>,
    /// 观测到的窗口长度（毫秒）；头里通常给不出，用 `reset_at - now` 反推。
    pub window_ms: Option<i64>,
    /// 计量单位：`requests` / `tokens` / `unknown`。
    pub unit: String,
    /// 本快照来自 429（`true`）还是成功响应的头（`false`）。
    pub from_429: bool,
}

impl RateLimitSnapshot {
    pub fn is_empty(&self) -> bool {
        self.remaining.is_none()
            && self.limit.is_none()
            && self.reset_at.is_none()
            && !self.from_429
    }
}

/// 账号配额窗口状态（管理面 `GET /v2/admin/quotas`）。
#[derive(Debug, Clone, TS, Serialize, Deserialize)]
#[ts(export, export_to = "admin.ts")]
pub struct QuotaWindowInfo {
    /// 候选 id（编译期 `{provider}-{model}-{account}`）。
    pub candidate_id: String,
    /// 账号 id（能关联到 account 时）。
    #[ts(as = "Option<String>")]
    pub account_id: Option<String>,
    /// provider id（derived，用于分组显示）。
    #[ts(as = "Option<String>")]
    pub provider_id: Option<String>,
    #[ts(as = "Option<i64>")]
    pub remaining: Option<i64>,
    #[ts(as = "Option<i64>")]
    pub limit: Option<i64>,
    /// 重置时间（Unix 毫秒）。
    #[ts(as = "Option<i64>")]
    pub reset_at: Option<i64>,
    pub unit: String,
    /// `true` = 本地计数估算（上游没给标准头）。
    pub estimated: bool,
    /// 连续 429 次数。
    pub consecutive_429: u32,
    /// 最近一次观测时间。
    pub updated_at: i64,
}

/// 网关 key 限额（token 上配置）。
#[derive(Debug, Clone, TS, Serialize, Deserialize)]
#[ts(export, export_to = "admin.ts")]
pub struct KeyLimits {
    /// 窗口：`hour` / `day` / `week` / `month` / `hours:<n>`（计划 key 的 5h 节奏）。
    pub window: String,
    /// token 预算（输入+输出）；0 = 不限。
    #[serde(default)]
    pub tokens: u64,
    /// 成本预算 micros；0 = 不限。
    #[serde(default)]
    pub cost_micros: u64,
    /// 只对列出的模型计限额（空 = 全部）。
    #[serde(default)]
    pub models: Vec<String>,
}

impl Default for KeyLimits {
    fn default() -> Self {
        Self {
            window: "day".to_string(),
            tokens: u64::MAX,
            cost_micros: u64::MAX,
            models: Vec::new(),
        }
    }
}

/// 限额命中详情（429 响应体）。
#[derive(Debug, Clone, TS, Serialize, Deserialize)]
#[ts(export, export_to = "admin.ts")]
pub struct KeyLimitHit {
    pub token_id: String,
    pub window: String,
    pub used_tokens: u64,
    pub limit_tokens: u64,
    pub used_cost_micros: u64,
    pub limit_cost_micros: u64,
    /// 窗口重置时间（Unix 毫秒）。
    pub reset_at: i64,
    pub retry_after_secs: u64,
}

/// 窗口长度（毫秒）。
pub fn window_span_millis(window: &str) -> i64 {
    const HOUR: i64 = 3_600_000;
    const DAY: i64 = 24 * HOUR;
    if let Some(hours) = window.strip_prefix("hours:") {
        return hours
            .trim()
            .parse::<i64>()
            .ok()
            .map(|hours| hours.clamp(1, 24 * 31) * HOUR)
            .unwrap_or(DAY);
    }
    match window {
        "hour" => HOUR,
        "week" => 7 * DAY,
        "month" => 30 * DAY,
        _ => DAY,
    }
}

/// 窗口起点（Unix 毫秒）；`window` 语义见 [`KeyLimits::window`]。
pub fn window_start_millis(window: &str, now_millis: i64) -> i64 {
    let span = window_span_millis(window);
    // 对齐到窗口起点（epoch 对齐，不做本地时区）。
    now_millis.div_euclid(span) * span
}

/// 窗口结束时间。
pub fn window_end_millis(window: &str, now_millis: i64) -> i64 {
    window_start_millis(window, now_millis) + window_span_millis(window)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn window_start_aligns() {
        let day = 24 * 3_600_000;
        let now = 1_700_000_000_000;
        assert_eq!(window_start_millis("day", now), now.div_euclid(day) * day);
        assert!(window_start_millis("day", now) <= now);
        assert!(window_start_millis("day", now) > now - day);
    }

    #[test]
    fn hours_window_parses() {
        // 5 小时窗口（计划 key）：起点必须按 5h 对齐。
        let now = 1_700_000_000_000i64;
        let span = 5 * 3_600_000i64;
        let start = window_start_millis("hours:5", now);
        assert_eq!(start % span, 0);
        assert!(start <= now && now - start < span);
    }

    #[test]
    fn empty_snapshot() {
        assert!(RateLimitSnapshot::default().is_empty());
        let with_429 = RateLimitSnapshot {
            from_429: true,
            ..Default::default()
        };
        assert!(!with_429.is_empty());
    }
}

/// `PATCH /v2/admin/tokens/{id}`：设置或清除限额。
#[derive(Debug, Clone, TS, Serialize, Deserialize)]
#[ts(export, export_to = "admin.ts")]
pub struct UpdateTokenLimitsRequest {
    /// 传 `null` 清除限额。
    #[serde(default)]
    #[ts(optional)]
    pub limits: Option<KeyLimits>,
}
