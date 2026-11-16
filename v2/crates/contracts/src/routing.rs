//! 路由契约：候选、档位、虚拟模型、Plan、成本（§4.2–§4.5、§4.9、§9.1）。

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::canonical::{CanonicalRequest, Usage};
use crate::errors::ErrorClass;
use crate::ids::{
    CandidateId, PhysicalModelId, ProfileId, StrategyId, ThinkingLevel, TierId, VirtualModelId,
};
use crate::judge::{JudgeTrace, Question};
use crate::reason::ReasonCode;
use crate::signals::Signals;
use crate::state::{CandidateHealth, LedgerSummary, SessionFacts, StateUpdate};

/// 输入模态。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InputModality {
    Text,
    Image,
    Audio,
}

/// 能力支持度：`Unknown` 表示没有证据，不猜测。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Support {
    #[default]
    Unknown,
    Yes,
    No,
}

/// 客户端请求原因（pi 语义，§4.9.1）：决定是否复用 previous / failed / state。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RequestReason {
    #[default]
    User,
    Continuation,
    Retry,
    Direct,
}

impl RequestReason {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::User => "user",
            Self::Continuation => "continuation",
            Self::Retry => "retry",
            Self::Direct => "direct",
        }
    }
}

/// 客户端选择：虚拟模型 + thinking level。
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Selection {
    pub virtual_model: VirtualModelId,
    pub thinking_level: ThinkingLevel,
}

/// 实际派发：物理模型 + 被 clamp 的 thinking level。
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Dispatch {
    pub physical_model: PhysicalModelId,
    pub provider: String,
    pub thinking_level: ThinkingLevel,
}

/// 对外可选择的模型条目（§3.6）。虚拟模型不能 dispatch 到另一个虚拟模型。
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct VirtualModel {
    pub provider: String,
    pub id: VirtualModelId,
    pub name: String,
    pub thinking_levels: Vec<ThinkingLevel>,
    pub context_window: Option<u32>,
    pub max_tokens: Option<u32>,
    pub input: Vec<InputModality>,
    pub profile: ProfileId,
    pub strategies: Vec<StrategyId>,
    pub candidate_pool: Vec<CandidateId>,
}

/// route 自定义的跨轮状态（**不是 sticky**，§4.9.1）。
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct VirtualModelState {
    pub session_key: crate::ids::SessionKey,
    pub branch_id: crate::ids::BranchId,
    pub virtual_model_id: VirtualModelId,
    /// 必须可 JSON 序列化；内核不解析。
    pub state: serde_json::Value,
    pub version: u64,
}

/// 失败请求的派发信息（Retry 的 StickyFailed 复用依据）。
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct FailedDispatch {
    pub dispatch: Dispatch,
    pub assistant_message: Option<crate::canonical::CanonicalMessage>,
    /// 归一化分类；sticky 判定**只认这个**（§18.4.2）。
    pub error_class: Option<ErrorClass>,
    pub http_status: Option<u16>,
    /// 原始错误文本，只进审计。
    pub error: Option<String>,
    pub stop_reason: Option<String>,
}

impl FailedDispatch {
    /// 是否属于 overload / 上下文溢出（§4.9.1）。
    pub fn is_overload_or_overflow(&self) -> bool {
        crate::errors::is_overload_or_overflow(self.error_class)
    }
}

/// 上行软亲和（HiRoute ContextHold，§4.9.1）。
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct ContextHold {
    pub candidate_id: CandidateId,
    pub virtual_model_id: VirtualModelId,
    /// 候选组/策略组指纹。
    pub group_digest: String,
    pub plan_revision: u64,
    pub profile_id: ProfileId,
    pub ingress_protocol: String,
    pub identity_digest: String,
    pub updated_at: u64,
    pub expires_at: u64,
}

impl ContextHold {
    /// 六元组 + TTL 校验；任一项不匹配即视为失效，交回 Plan。
    pub fn is_valid_for(&self, virtual_model_id: &str, profile_id: &str, now_unix: u64) -> bool {
        self.virtual_model_id == virtual_model_id
            && self.profile_id == profile_id
            && now_unix < self.expires_at
    }
}

/// Plan 的初始派发来源。优先级见 §4.9.1。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DispatchSource {
    StickyFailed,
    StickyPrevious,
    ContextHold,
    #[default]
    Strategy,
    Failover,
    Fallback,
}

impl DispatchSource {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::StickyFailed => "sticky_failed",
            Self::StickyPrevious => "sticky_previous",
            Self::ContextHold => "context_hold",
            Self::Strategy => "strategy",
            Self::Failover => "failover",
            Self::Fallback => "fallback",
        }
    }
}

/// 供应商类型（决定 wire 编码，而不是决定路由）。
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderKind {
    #[default]
    OpenAiCompatible,
    Anthropic,
    Responses,
    Other(String),
}

/// 凭据引用：只存引用，不存明文（§18.4.4）。
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(transparent)]
pub struct CredentialRef(pub String);

impl CredentialRef {
    pub fn env(name: impl Into<String>) -> Self {
        Self(format!("env:{}", name.into()))
    }

    pub fn file(path: impl Into<String>) -> Self {
        Self(format!("file:{}", path.into()))
    }

    pub fn keyring(name: impl Into<String>) -> Self {
        Self(format!("keyring:{}", name.into()))
    }

    /// 引用类型：`env` / `file` / `keyring` / `unknown`。
    pub fn kind(&self) -> &str {
        self.0.split_once(':').map_or("unknown", |(kind, _)| kind)
    }
}

/// 上游端点。
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct UpstreamRef {
    pub provider: ProviderKind,
    pub base_url: String,
    pub model: PhysicalModelId,
    pub credential: Option<CredentialRef>,
}

/// 能力画像。由使用者声明；缺证据记 `Unknown`，不猜。
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct CapabilityProfile {
    pub multimodal: Support,
    pub tools: Support,
    pub reasoning: Support,
    pub input: Vec<InputModality>,
    pub thinking_levels: Vec<ThinkingLevel>,
    pub max_output: Option<u32>,
    pub tags: BTreeMap<String, String>,
}

/// 一个可服务的上游物理候选。
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Candidate {
    pub id: CandidateId,
    /// 只是标签，不是路由结论。
    pub tier: TierId,
    /// 越大越强；由 profile 的 `strength_order` 校验。
    pub strength: u8,
    pub upstream: UpstreamRef,
    pub window: u32,
    pub capability: CapabilityProfile,
    /// 缺价格 = `None`，不猜测。
    pub price: Option<PriceModel>,
    pub enabled: bool,
}

impl Candidate {
    pub fn supports_input(&self, modality: InputModality) -> bool {
        self.capability.input.contains(&modality)
    }

    /// thinking level 是否被该候选支持；空列表视为不支持任何非 `off` 档。
    pub fn supports_thinking_level(&self, level: &str) -> bool {
        level == "off" || self.capability.thinking_levels.iter().any(|l| l == level)
    }
}

/// 价格模型（§9.1）。金额一律整数 micros。
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct PriceModel {
    pub currency: String,
    pub input_micros_per_mtok: u64,
    pub output_micros_per_mtok: u64,
    pub cache_read_micros_per_mtok: Option<u64>,
    pub cache_write_micros_per_mtok: Option<u64>,
    pub request_micros: u64,
}

impl PriceModel {
    /// 按用量估算等价成本。缓存价格缺失时按普通 input 价格保守估算，
    /// 调用方需记 `PRICE_CACHE_ASSUMED`（§9.1）。
    pub fn estimate(&self, usage: &Usage) -> Cost {
        let input_tokens = u128::from(usage.input_tokens);
        let cached = u128::from(usage.cached_tokens.unwrap_or(0));
        let cache_write = u128::from(usage.cache_write_tokens.unwrap_or(0));
        let uncached = input_tokens.saturating_sub(cached);
        let output_tokens = u128::from(usage.output_tokens);

        let input_price = u128::from(self.input_micros_per_mtok);
        let cache_read_price = u128::from(
            self.cache_read_micros_per_mtok
                .unwrap_or(self.input_micros_per_mtok),
        );
        let cache_write_price = u128::from(
            self.cache_write_micros_per_mtok
                .unwrap_or(self.input_micros_per_mtok),
        );
        let output_price = u128::from(self.output_micros_per_mtok);

        let micros = uncached * input_price / 1_000_000
            + cached * cache_read_price / 1_000_000
            + cache_write * cache_write_price / 1_000_000
            + output_tokens * output_price / 1_000_000
            + u128::from(self.request_micros);

        Cost {
            micros: i64::try_from(micros).unwrap_or(i64::MAX),
            currency: self.currency.clone(),
        }
    }
}

/// 等价成本。可为负（切换后更便宜）。不同币种不得相加。
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Cost {
    pub micros: i64,
    pub currency: String,
}

impl Cost {
    pub fn zero(currency: impl Into<String>) -> Self {
        Self {
            micros: 0,
            currency: currency.into(),
        }
    }

    /// 同币种相加；币种不同返回 `None`，由调用方决定如何处理（不静默混算）。
    pub fn checked_add(&self, other: &Cost) -> Option<Cost> {
        if self.currency != other.currency {
            return None;
        }
        Some(Cost {
            micros: self.micros.checked_add(other.micros)?,
            currency: self.currency.clone(),
        })
    }
}

/// 计划执行模式。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlanMode {
    #[default]
    Single,
    Cascade,
    VerifyAfter,
}

impl PlanMode {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Single => "single",
            Self::Cascade => "cascade",
            Self::VerifyAfter => "verify_after",
        }
    }
}

/// 成本闸未知价格策略（§9.4）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UnknownPricePolicy {
    /// 质量升级放行、成本降级拒绝（默认）。
    #[default]
    QualityAllow,
    /// 缺价格一律拒绝切换。
    DenyAll,
}

/// 流式升级时机（§10.6）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StreamEscalation {
    /// 流式轮内不升级，下一轮生效。
    #[default]
    NextTurn,
    /// 只在 tool_call 前缓冲并允许换候选（P2）。
    GateBeforeTool,
    Off,
}

/// profile：阈值、权重、模式与版本指纹。
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct ProfileConfig {
    pub id: ProfileId,
    pub mode: PlanMode,
    pub stream_escalation: StreamEscalation,
    /// 从左到右增强；用于 escalation 目标与 `strength` 排序校验。
    pub strength_order: Vec<TierId>,
    /// `difficulty_gap × difficulty_weight_micros` 的权重（micros 量纲）。
    pub difficulty_weight_micros: i64,
    /// `hard_signal_score × hard_signal_weight_micros` 的权重。
    pub hard_signal_weight_micros: i64,
    pub unknown_price_policy: UnknownPricePolicy,
    /// profile 内容哈希前缀；改价格不应让 ContextHold 失效（§18.4.4）。
    pub revision: String,
    /// 规范化配置的 SHA-256，写入快照与账本，回放先比它。
    pub config_digest: String,
}

/// Plan 中的一个位置。
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Slot {
    pub candidate_id: CandidateId,
    pub tier: TierId,
    pub strength: u8,
    pub window: u32,
    pub thinking_level: ThinkingLevel,
    pub est_cost: Option<Cost>,
}

/// 决策结果。
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Plan {
    pub selection: Selection,
    pub dispatch_source: DispatchSource,
    /// 本轮首选顺序。
    pub preference: Vec<Slot>,
    /// 失败接力顺序；通常是 `preference` 的可用子集。
    pub failover: Vec<Slot>,
    pub mode: PlanMode,
    pub expected_cache_loss: Option<Cost>,
    pub judge: JudgeTrace,
    pub reason_codes: Vec<ReasonCode>,
    /// 决策阶段暂存的状态变更，与 `on_outcome` 一起原子提交。
    pub state_updates: Vec<StateUpdate>,
    /// 每条策略贡献及其归属（P2-11）：账本与 `decision` span 据此追到单个策略。
    pub contributions: Vec<PlanContributionRecord>,
}

impl Plan {
    pub fn preference_ids(&self) -> Vec<&CandidateId> {
        self.preference.iter().map(|s| &s.candidate_id).collect()
    }

    pub fn preferred(&self) -> Option<&Slot> {
        self.preference.first()
    }
}

/// 策略输出：贡献、判官请求与暂存的状态更新。
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct PlanDraft {
    pub contributions: Vec<PlanContribution>,
    pub judge_requests: Vec<JudgeRequest>,
    pub state_updates: Vec<StateUpdate>,
}

/// 单条策略贡献。`agent` 按 `Escalation > CompetenceGuard > Classify > Baseline` 合并。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "contribution", rename_all = "snake_case")]
pub enum PlanContribution {
    Filter {
        exclude: BTreeSet<CandidateId>,
        reason: ReasonCode,
    },
    Prefer {
        order: Vec<CandidateId>,
        reason: ReasonCode,
    },
    Pin {
        candidate: CandidateId,
        reason: ReasonCode,
    },
    Mode {
        mode: PlanMode,
        reason: ReasonCode,
    },
    CacheLoss {
        cost: Cost,
        reason: ReasonCode,
    },
    /// 不改变顺序的理由记录（例如 latch 计数、冷却）；合并器只收集 reason。
    Note {
        reason: ReasonCode,
    },
}

/// 单条贡献的归属记录：哪个策略、什么优先级、贡献了什么。
///
/// 策略本身不感知 `priority`（由 `decide` 注入）；这里只是把合并时的上下文固化，
/// 使一次请求的最终 Plan 可以逐条回溯到来源（P2-11）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PlanContributionRecord {
    pub strategy_id: StrategyId,
    pub priority: u8,
    pub contribution: PlanContribution,
}

impl PlanContributionRecord {
    /// 贡献对应的理由码（用于 span/日志的紧凑展示）。
    pub fn reason(&self) -> &ReasonCode {
        match &self.contribution {
            PlanContribution::Filter { reason, .. }
            | PlanContribution::Prefer { reason, .. }
            | PlanContribution::Pin { reason, .. }
            | PlanContribution::Mode { reason, .. }
            | PlanContribution::CacheLoss { reason, .. }
            | PlanContribution::Note { reason } => reason,
        }
    }
}

/// 判官请求：策略只说“需要”，由 `agent` 统一预算与去重。
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct JudgeRequest {
    pub id: crate::ids::JudgeRequestId,
    pub question: Question,
    /// `false` = 超时可跳过。
    pub required: bool,
    pub priority: u8,
    /// 调用点计算的证据（判官输入需要，且不能由模型自报）。
    pub category: crate::verdict::Category,
    pub evidence: crate::verdict::EvidenceRef,
}

/// 判官预算（与审批预算池分离）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Budget {
    pub max_calls_per_turn: u32,
    pub max_calls_per_minute: u32,
    pub max_calls_per_session: u32,
    pub deadline_ms: u32,
}

/// 策略视图（只读、封闭，§4.2）。策略不得穿透读配置/数据库/环境变量。
#[derive(Debug, Clone, Copy)]
pub struct RequestView<'a> {
    pub canonical: &'a CanonicalRequest,
    pub signals: &'a Signals,
    pub selection: &'a Selection,
    pub virtual_model: &'a VirtualModel,
    pub reason: RequestReason,
    /// 当前 messages 中最近一次成功响应的派发。
    pub previous: Option<&'a Dispatch>,
    /// 重试时失败的请求（含 message / error_class）。
    pub failed: Option<&'a FailedDispatch>,
    /// User 轮的软亲和。
    pub context_hold: Option<&'a ContextHold>,
    pub router_state: Option<&'a VirtualModelState>,
    pub candidates: &'a [Candidate],
    pub health: &'a CandidateHealth,
    pub session: &'a SessionFacts,
    pub ledger: &'a LedgerSummary,
    pub profile: &'a ProfileConfig,
    /// 注入时钟，保证可回放。
    pub now_unix: u64,
}

impl<'a> RequestView<'a> {
    pub fn candidate(&self, id: &str) -> Option<&'a Candidate> {
        self.candidates.iter().find(|c| c.id == id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cost_add_rejects_currency_mismatch() {
        let a = Cost {
            micros: 10,
            currency: "USD".into(),
        };
        let b = Cost {
            micros: 20,
            currency: "CNY".into(),
        };
        assert!(a.checked_add(&b).is_none());
        assert_eq!(a.checked_add(&a).unwrap().micros, 20);
    }

    #[test]
    fn price_estimate_uses_cache_price_when_present() {
        let price = PriceModel {
            currency: "USD".into(),
            input_micros_per_mtok: 1_000_000,
            output_micros_per_mtok: 2_000_000,
            cache_read_micros_per_mtok: Some(100_000),
            cache_write_micros_per_mtok: None,
            request_micros: 0,
        };
        let usage = Usage {
            input_tokens: 1_000_000,
            output_tokens: 0,
            cached_tokens: Some(1_000_000),
            cache_write_tokens: None,
        };
        assert_eq!(price.estimate(&usage).micros, 100_000);
    }

    #[test]
    fn credential_ref_keeps_only_reference() {
        let cred = CredentialRef::env("DEEPSEEK_API_KEY");
        assert_eq!(cred.kind(), "env");
        assert!(!cred.0.contains('='));
    }
}
