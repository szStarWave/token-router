//! 信号层（§4.3）。
//!
//! `CoreSignals` 是内核稳定字段，随版本演进；新增信号源实现 `SignalExtractor`
//! 写入 `extras: SignalBag`，不改 `CoreSignals`。

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::canonical::CanonicalRequest;
use crate::ids::{CandidateId, SignalId};
use crate::state::SessionFacts;

/// 决策信号：稳定核心 + 可插拔扩展。
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Signals {
    pub core: CoreSignals,
    /// 稳定排序（`BTreeMap`），保证决策投影可字节比较。
    pub extras: SignalBag,
}

/// 内核字段。新增字段必须同时更新 §4.3 与投影 golden。
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct CoreSignals {
    pub step_kind: StepKind,
    pub consecutive_tool_error_streak: u32,
    pub repeated_tool_loop: Option<LoopInfo>,
    pub assistant_failed_recent: bool,
    /// 输入 token / 候选窗口；0 表示未知。
    pub ctx_fill_ratio: f32,
    pub multimodal: bool,
    /// 每个候选的在飞请求数（替代 v1 的 edge_load）。
    pub candidate_inflight: BTreeMap<CandidateId, u32>,
    /// 词频/关键词只作为 hint，整体可关闭。
    pub lexical: Option<LexicalHint>,
    /// 与上一次服务候选的难度差，成本闸输入。
    pub difficulty_gap: Option<f32>,
    /// 硬信号归一化分数 [0,1]，成本闸输入。
    pub hard_signal_score: Option<f32>,
}

/// v2 只保留 7 种步态（§15.2 收敛映射）；更细的区分进 `SignalBag.step_kind_detail`。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StepKind {
    #[default]
    DirectChat,
    Heartbeat,
    Tool,
    Final,
    Recovery,
    Plan,
    Background,
}

/// 工具循环观测。
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct LoopInfo {
    pub tool_name: String,
    pub repeat_count: u32,
    /// 观察窗口（最近 N 次工具调用）。
    pub window: u32,
}

/// 词频/关键词提示。只影响模糊带边界，不直接改阈值。
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct LexicalHint {
    pub difficulty_hint: Option<f32>,
    pub matched_terms: Vec<String>,
    pub source: String,
}

/// 可插拔信号取值。保持封闭集合，避免“任意 JSON 当信号”。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub enum SignalValue {
    Bool(bool),
    Int(i64),
    Float(f32),
    Text(String),
    Json(Value),
}

impl From<bool> for SignalValue {
    fn from(v: bool) -> Self {
        Self::Bool(v)
    }
}

impl From<i64> for SignalValue {
    fn from(v: i64) -> Self {
        Self::Int(v)
    }
}

impl From<f32> for SignalValue {
    fn from(v: f32) -> Self {
        Self::Float(v)
    }
}

impl From<String> for SignalValue {
    fn from(v: String) -> Self {
        Self::Text(v)
    }
}

impl From<&str> for SignalValue {
    fn from(v: &str) -> Self {
        Self::Text(v.to_string())
    }
}

/// 扩展信号包。
pub type SignalBag = BTreeMap<SignalId, SignalValue>;

/// 扩展信号源。注册后由 `core::signals` 统一调用，结果写入 `Signals.extras`。
pub trait SignalExtractor: Send + Sync {
    fn id(&self) -> SignalId;

    fn extract(&self, req: &CanonicalRequest, prev: Option<&SessionFacts>) -> SignalBag;
}
