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

// ---------------------------------------------------------------------------
// 判官输入/答案（§7.2、§7.3）
// ---------------------------------------------------------------------------

/// 判官输入：问题 + 已冻结的上下文 + 调用点计算的证据指纹。
///
/// `evidence` 由调用点计算（step_kind + tool + normalized_error + turn），不信任模型自报。
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct JudgeInput {
    pub question: Question,
    pub category: crate::verdict::Category,
    pub evidence: crate::verdict::EvidenceRef,
    pub context: JudgeContext,
}

/// 判官上下文（P0 只支持固定字符预算，§7.4）。
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct JudgeContext {
    pub system: String,
    pub task: String,
    /// 由旧到新排列的窗口消息（已按单条字符上限截断）。
    pub recent: Vec<String>,
}

/// 判官答案。
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub enum JudgeValue {
    Bool(bool),
    /// 枚举选项 id；映射到 Verdict 由调用点配置决定。
    Choice(String),
    /// 分数；必须配合 `Choices::Score` 的阈值。
    Score(f32),
    /// 保留答案：模型不确定，或置信度低于闸门。
    #[default]
    Uncertain,
}

/// 一次判官回答。
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct JudgeAnswer {
    pub value: JudgeValue,
    pub confidence: Option<f32>,
    /// 自然语言理由：只进审计，不参与算术。
    pub reason: Option<String>,
    /// 本次调用的 token 用量（由适配器从上游响应解析）。
    pub usage: Option<JudgeUsage>,
}

/// 判官失败。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct JudgeFailure {
    pub code: JudgeFailureCode,
    pub message: Option<String>,
}

impl Default for JudgeFailure {
    fn default() -> Self {
        Self {
            code: JudgeFailureCode::SourceUnavailable,
            message: None,
        }
    }
}

/// `judge test` 结果。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct JudgeServiceTestResult {
    pub outcome: TestOutcome,
    pub latency_ms: u32,
    pub failure: Option<JudgeFailure>,
}

impl Default for JudgeServiceTestResult {
    fn default() -> Self {
        Self {
            outcome: TestOutcome::Failed,
            latency_ms: 0,
            failure: None,
        }
    }
}

/// 判官自检结论。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TestOutcome {
    Passed,
    #[default]
    Failed,
}
