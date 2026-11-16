//! 会话状态（§8.1、§8.2）。
//!
//! 分层：`SessionFacts` 是会话级事实（谁最后被派发、软亲和、缓存锚点）；
//! `StrategyState` 是 `(session, branch, task, virtual_model, strategy)` 级的学习状态；
//! `VirtualModelState` 是 route 自定义状态。三者作用域不同，不得混用。

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

use crate::canonical::Usage;
use crate::errors::ErrorClass;
use crate::ids::{
    BranchId, CandidateId, PhysicalModelId, SessionKey, StrategyId, ThinkingLevel, VirtualModelId,
};
use crate::routing::{ContextHold, Cost, Dispatch};

/// 会话键推导输入（§8.1）。稳定哈希，不用 `DefaultHasher`。
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct SessionKeyInput {
    pub client_id: Option<String>,
    pub workspace_hint: Option<String>,
    pub first_system_hash: String,
    pub first_user_hash: String,
    pub tool_names: Vec<String>,
    /// 服务端下发、客户端带回时优先。
    pub server_session_id: Option<String>,
}

/// 会话键来源，写账本用于审计“为什么这个键”。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionKeySource {
    ServerId,
    #[default]
    Content,
    Fallback,
}

impl SessionKeySource {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ServerId => "server_id",
            Self::Content => "content",
            Self::Fallback => "fallback",
        }
    }
}

/// 会话级事实。
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct SessionFacts {
    pub session_key: SessionKey,
    pub branch_id: BranchId,
    pub selected_virtual_model: Option<VirtualModelId>,
    pub selected_thinking_level: Option<ThinkingLevel>,
    /// 最近一次成功响应的派发（`Continuation` 的 StickyDispatch previous）。
    pub last_successful_dispatch: Option<Dispatch>,
    /// 软亲和，按虚拟模型索引（§4.9.1）。
    pub context_holds: BTreeMap<VirtualModelId, ContextHold>,
    pub last_served_slot: Option<SlotRef>,
    /// 用于 token 增量与缓存损失估计。
    pub last_prompt_tokens: Option<u32>,
    pub cache_anchor: Option<CacheAnchor>,
    pub consecutive_failures: u32,
    /// 上下文溢出后的抑制标记；有释放条件（§8.2），不是永久钉住。
    pub ctx_overflow_pin: bool,
    pub updated_at: u64,
    /// CAS 版本。
    pub version: u64,
}

/// 策略学习状态。`blob` 只由对应策略解析。
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct StrategyState {
    pub branch_id: BranchId,
    pub task_id: String,
    pub virtual_model_id: VirtualModelId,
    pub strategy_id: StrategyId,
    pub state_version: u32,
    pub blob: Value,
    pub updated_at: u64,
    pub version: u64,
}

/// 状态变更来源：`draft/refine` 阶段 vs 执行结果阶段；冲突时 `Outcome` 优先。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StateUpdateOrigin {
    #[default]
    Draft,
    Outcome,
}

/// 一次状态变更（RFC 7386 JSON merge patch），由 `agent` 原子提交。
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct StateUpdate {
    pub strategy_id: StrategyId,
    pub task_id: String,
    pub origin: StateUpdateOrigin,
    pub patch: Value,
}

/// 提示缓存锚点（§9.3）。
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct CacheAnchor {
    pub provider: String,
    pub physical_model: PhysicalModelId,
    /// 前缀哈希；同 provider + 同 model + 同前缀才可沿用缓存比例。
    pub prefix_hash: String,
    pub cached_tokens: u64,
    pub prompt_tokens: u64,
    pub updated_at: u64,
}

/// 被服务的位置引用：候选 + 在 preference 中的下标。
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct SlotRef {
    pub candidate_id: CandidateId,
    pub preference_index: u32,
}

/// 请求最终结果类别。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OutcomeKind {
    #[default]
    Success,
    UpstreamError,
    /// 来自 Verify / `ReleaseCheck`，不是传输错误（§18.4.2 约束 6）。
    QualityFail,
    Cancelled,
    /// 本地策略拒绝（审批拦截），请求没有离开路由器（§12）。
    Rejected,
}

impl OutcomeKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Success => "success",
            Self::UpstreamError => "upstream_error",
            Self::QualityFail => "quality_fail",
            Self::Cancelled => "cancelled",
            Self::Rejected => "rejected",
        }
    }
}

/// 执行结果，交给 `Strategy::on_outcome`。
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Outcome {
    pub request_id: Uuid,
    pub outcome: OutcomeKind,
    pub dispatch: Option<Dispatch>,
    pub slot: Option<SlotRef>,
    pub usage: Option<Usage>,
    pub error_class: Option<ErrorClass>,
    pub stop_reason: Option<String>,
    pub duration_ms: u64,
    pub ttft_ms: Option<u64>,
    pub created_at: u64,
}

/// 单候选健康度（熔断器）。
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct BreakerState {
    pub consecutive_failures: u32,
    pub opened_count: u32,
    pub open_until: Option<u64>,
    pub half_open_since: Option<u64>,
    pub last_error_class: Option<ErrorClass>,
    pub last_failure_at: Option<u64>,
    pub updated_at: u64,
}

impl BreakerState {
    /// 是否仍在冷却窗口内。
    pub fn is_open(&self, now_unix: u64) -> bool {
        self.open_until.is_some_and(|until| now_unix < until)
    }

    /// 是否允许尝试：冷却结束或从未打开。half-open 只放一个探针，由 `agent` 保证。
    pub fn can_attempt(&self, now_unix: u64) -> bool {
        !self.is_open(now_unix)
    }
}

/// 候选健康度快照，进入 `DecisionSnapshot`。
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct CandidateHealth {
    pub candidates: BTreeMap<CandidateId, BreakerState>,
}

impl CandidateHealth {
    pub fn state_of(&self, candidate_id: &str) -> Option<&BreakerState> {
        self.candidates.get(candidate_id)
    }

    pub fn is_open(&self, candidate_id: &str, now_unix: u64) -> bool {
        self.state_of(candidate_id)
            .is_some_and(|state| state.is_open(now_unix))
    }

    pub fn can_attempt(&self, candidate_id: &str, now_unix: u64) -> bool {
        self.state_of(candidate_id)
            .is_none_or(|state| state.can_attempt(now_unix))
    }
}

/// 账本只读聚合，供策略判断；不承载跨请求学习状态。
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct LedgerSummary {
    pub requests: u64,
    pub switches: u64,
    pub strong_requests: u64,
    pub cheap_requests: u64,
    pub answer_cost: Option<Cost>,
    pub routing_cost: Option<Cost>,
    pub last_outcome: Option<OutcomeKind>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn breaker_opens_only_inside_window() {
        let state = BreakerState {
            open_until: Some(1_000),
            ..BreakerState::default()
        };
        assert!(state.is_open(999));
        assert!(!state.can_attempt(999));
        assert!(!state.is_open(1_000));
        assert!(state.can_attempt(1_000));
    }

    #[test]
    fn context_hold_checks_ttl_and_profile() {
        let hold = ContextHold {
            candidate_id: "deepseek-chat".into(),
            virtual_model_id: "auto".into(),
            profile_id: "default".into(),
            expires_at: 100,
            ..ContextHold::default()
        };
        assert!(hold.is_valid_for("auto", "default", 99));
        assert!(!hold.is_valid_for("auto", "default", 100));
        assert!(!hold.is_valid_for("cheap", "default", 99));
    }
}
