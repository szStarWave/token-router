//! 判官问题、结论与审计（§7.3、§7.5）。
//!
//! 策略只说“这里需要判官”，由 `agent` 统一预算与去重；`core` 不感知 `decide`。
//! 完整的外部判官服务契约（`JudgeServiceRef` / 信封 / 上下文策略）在 P1 落地。

use serde::{Deserialize, Serialize};

use crate::routing::Cost;

/// 策略内部问题。
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Question {
    pub prompt: String,
    pub choices: Choices,
    /// 低于该置信度视为 `Uncertain`；`0.0` 表示不设闸门。
    pub min_confidence: f32,
}

/// 允许的答案形状。
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Choices {
    /// `true` = Escalate，`false` = Decline。
    #[default]
    Boolean,
    /// 选项到结论的映射由策略配置声明。
    Enum(Vec<Choice>),
    /// Score 仅 P3 启用；要求 `decline_at < escalate_at`，分数越高越倾向升级。
    Score {
        min: f32,
        max: f32,
        decline_at: f32,
        escalate_at: f32,
    },
}

/// 枚举选项。`uncertain` 是保留答案，不得作为选项 id。
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Choice {
    pub id: String,
    pub label: String,
}

/// 判官调用审计。写入 `LedgerRecord.judge` 与 `DecisionSnapshot` 之外的账本路径。
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct JudgeTrace {
    pub service_id: Option<String>,
    pub service_revision: Option<u64>,
    pub protocol_digest: Option<String>,
    pub context_digest: Option<String>,
    /// 本地结论（`LocalDecisionSnapshot`）的指纹：外部失败时回退到它。
    pub local_decision_digest: String,
    pub effective_source: JudgeEffectiveSource,
    pub fallback_used: bool,
    pub fallback_reason: Option<JudgeFailureCode>,
    pub duration_ms: u32,
    pub usage: Option<JudgeUsage>,
}

/// 结论实际来源。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JudgeEffectiveSource {
    #[default]
    Heuristic,
    LocalRules,
    ExternalService,
    ExternalLlm,
    /// 外部判官失败，回退本地快照。
    Fallback,
}

impl JudgeEffectiveSource {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Heuristic => "heuristic",
            Self::LocalRules => "local_rules",
            Self::ExternalService => "external_service",
            Self::ExternalLlm => "external_llm",
            Self::Fallback => "fallback",
        }
    }
}

/// 判官调用的 token 与成本；成本计入 `routing_cost`，不占用户流量。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct JudgeUsage {
    pub prompt_tokens: u32,
    pub completion_tokens: u32,
    pub cost_micros: i64,
}

/// 失败码（§7.5.5）。配置类在启动或 `judge test` 失败；运行期直接回退，不重试。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JudgeFailureCode {
    InvalidConfiguration,
    SourceNotFound,
    SourceLineageMismatch,
    SourceDisabled,
    SourceUnavailable,
    NoUsableCredentials,
    RevisionConflict,
    Timeout,
    AuthenticationRejected,
    RateLimited,
    InputRejected,
    MalformedOutput,
    ProtocolDigestMismatch,
    TransportUnavailable,
}

impl JudgeFailureCode {
    /// 是否属于“配置/源”类：运行期应直接禁用该服务，而不是逐请求重试。
    pub const fn is_config_failure(self) -> bool {
        matches!(
            self,
            Self::InvalidConfiguration
                | Self::SourceNotFound
                | Self::SourceLineageMismatch
                | Self::SourceDisabled
                | Self::NoUsableCredentials
                | Self::RevisionConflict
        )
    }
}

/// 判官成本（显式保留，便于 `JudgeUsage.cost_micros` 与 `Cost` 汇合）。
pub fn judge_cost(usage: &JudgeUsage, currency: &str) -> Cost {
    Cost {
        micros: usage.cost_micros,
        currency: currency.to_string(),
    }
}
