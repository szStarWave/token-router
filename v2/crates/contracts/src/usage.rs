//! 用量聚合 DTO（v3 P5-C）。
//!
//! 维度：`agent`（授权身份）/ `model`（虚拟模型）/ `candidate`（实际物理模型）/
//! `account`（账号，经 candidate→account 映射）/ `session`。
//! 时间一律 Unix 毫秒，金额 micros。

use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// 用量维度。
#[derive(Debug, Clone, Copy, PartialEq, Eq, TS, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[ts(export, export_to = "admin.ts")]
pub enum UsageDimension {
    Agent,
    Model,
    Candidate,
    Account,
    Session,
}

impl UsageDimension {
    pub fn parse(value: &str) -> Self {
        match value {
            "model" => Self::Model,
            "candidate" => Self::Candidate,
            "account" => Self::Account,
            "session" => Self::Session,
            _ => Self::Agent,
        }
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Agent => "agent",
            Self::Model => "model",
            Self::Candidate => "candidate",
            Self::Account => "account",
            Self::Session => "session",
        }
    }
}

/// 一个分组的用量。
#[derive(Debug, Clone, TS, Serialize, Deserialize)]
#[ts(export, export_to = "admin.ts")]
pub struct UsageRow {
    /// 分组键（agent id / 模型名 / 候选名 / 账号 id / 会话 id）。
    pub key: String,
    pub requests: u64,
    pub successes: u64,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_read_tokens: u64,
    pub cache_write_tokens: u64,
    /// 用户流量成本。
    pub answer_cost_micros: i64,
    /// 判官/路由成本。
    pub routing_cost_micros: i64,
    /// 平均 TTFT（毫秒；无样本为 null）。
    #[ts(as = "Option<f64>")]
    pub avg_ttft_ms: Option<f64>,
    /// 缓存命中率 = cache_read / (input + cache_read)；无输入为 null。
    #[ts(as = "Option<f64>")]
    pub cache_hit_ratio: Option<f64>,
}

/// `GET /v2/admin/usage`。
#[derive(Debug, Clone, TS, Serialize, Deserialize)]
#[ts(export, export_to = "admin.ts")]
pub struct UsageResponse {
    pub by: String,
    pub from: i64,
    pub to: i64,
    pub rows: Vec<UsageRow>,
    /// 全窗口合计（不按维度分）。
    pub totals: UsageRow,
}

/// 热力图单元格（按小时墙上时钟）。
#[derive(Debug, Clone, TS, Serialize, Deserialize)]
#[ts(export, export_to = "admin.ts")]
pub struct HeatmapCell {
    /// `YYYY-MM-DD`（UTC）。
    pub day: String,
    /// 0–23。
    pub hour: u32,
    pub requests: u64,
    pub answer_cost_micros: i64,
    pub input_tokens: u64,
    pub output_tokens: u64,
}

/// `GET /v2/admin/usage/heatmap`。
#[derive(Debug, Clone, TS, Serialize, Deserialize)]
#[ts(export, export_to = "admin.ts")]
pub struct HeatmapResponse {
    pub from: i64,
    pub to: i64,
    pub cells: Vec<HeatmapCell>,
    /// 单元格最大值（前端着色用；0 = 无数据）。
    pub max_requests: u64,
}
