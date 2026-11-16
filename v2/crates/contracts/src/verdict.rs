//! 判官结论（§4.6）。
//!
//! 三态封闭：`Escalate` / `Decline` / `Uncertain`。自然语言理由只进审计，
//! 不参与算术；`confidence` 只作为闸门。

use serde::{Deserialize, Serialize};

/// 一条判官结论。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "verdict", rename_all = "snake_case")]
pub enum Verdict {
    Escalate {
        category: Category,
        /// `None` = 下一个更强的可用候选；P3 多档时用 profile 映射。
        target_strength: Option<u8>,
        evidence: EvidenceRef,
        confidence: f32,
    },
    Decline {
        reason: DeclineReason,
    },
    Uncertain {
        reason: String,
    },
}

/// 粗粒度分类。细分类（Repetition / Drift / ...）只作为 detail，避免 latch 抖动。
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Category {
    ToolFailure,
    #[default]
    QualityUncertain,
    ContextOverflow,
    PolicyRisk,
    Latency,
    Other(String),
}

/// 证据引用。指纹由调用点计算，不信任模型的布尔值。
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct EvidenceRef {
    /// 典型组成：`step_kind + tool_name + normalized_error + turn_index`。
    pub fingerprint: String,
    /// 给审计看的摘要，不参与算术。
    pub summary: String,
}

/// 不升级的理由。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeclineReason {
    /// 没有新证据（指纹已见过）。
    NoNewEvidence,
    /// 判官认为当前候选仍可完成。
    Sufficient,
    /// 证据冲突。
    Conflicting,
    /// 置信度不足，按 Uncertain 处理前的显式拒绝。
    LowConfidence,
    /// 策略/预算禁止。
    Policy,
    Other(String),
}
