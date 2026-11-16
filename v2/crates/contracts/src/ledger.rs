//! 账本记录（§9.2）。逐请求一行，是"为什么选它 / 花了多少"的唯一事实来源。
//!
//! 账本必须覆盖失败请求（§8.4）：决策失败时 `served_*` 为 `None`，但记录仍要写。

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::canonical::{IngressProtocol, Usage};
use crate::errors::ErrorClass;
use crate::ids::{
    BranchId, CandidateId, PhysicalModelId, SessionKey, ThinkingLevel, VirtualModelId,
};
use crate::judge::JudgeTrace;
use crate::reason::ReasonCode;
use crate::routing::{Cost, DispatchSource, PlanMode, RequestReason};
use crate::state::{OutcomeKind, SessionKeySource, SlotRef};

/// 账本事实完整度。部分覆盖不得当作完整和（§9.2）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FactsCompleteness {
    Complete,
    Partial,
    #[default]
    Unknown,
}

/// 单次候选尝试。落库为 `attempts_json`，P2 再拆表。
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct AttemptRecord {
    pub candidate_id: CandidateId,
    pub error_class: Option<ErrorClass>,
    pub http_status: Option<u16>,
    /// 是否已向客户端提交字节；提交后不得再换候选（§10.6）。
    pub committed: bool,
    pub duration_ms: u64,
    pub ttft_ms: Option<u64>,
    pub usage: Option<Usage>,
}

/// 逐请求账本。
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct LedgerRecord {
    pub request_id: Uuid,
    /// 与响应头 `x-token-router-plan-id` 关联。
    pub plan_id: String,
    pub session_key: SessionKey,
    pub branch_id: BranchId,
    pub task_id: String,
    pub session_key_source: SessionKeySource,
    pub collision_suspect: bool,
    pub ingress_protocol: IngressProtocol,
    pub selected_virtual_model: Option<VirtualModelId>,
    pub selected_thinking_level: Option<ThinkingLevel>,
    pub request_reason: RequestReason,
    pub plan_mode: PlanMode,
    /// 最终计划顺序，用于事后分析"为什么选它"。
    pub preference: Vec<CandidateId>,
    pub served_slot: Option<SlotRef>,
    pub served_physical_model: Option<PhysicalModelId>,
    pub served_thinking_level: Option<ThinkingLevel>,
    pub dispatch_source: DispatchSource,
    pub switches: u32,
    pub attempts: Vec<AttemptRecord>,
    pub judge: Option<JudgeTrace>,
    pub cache_hit_tokens: Option<u64>,
    /// 用户流量成本。
    pub answer_cost: Option<Cost>,
    /// 判官/分类器成本，不占用户 token 限额。
    pub routing_cost: Option<Cost>,
    /// `answer_cost + routing_cost`；币种不一致时为 `None`。
    pub cost_equiv: Option<Cost>,
    pub facts_completeness: FactsCompleteness,
    pub reason_codes: Vec<ReasonCode>,
    pub outcome: OutcomeKind,
    pub http_status: Option<u16>,
    pub duration_ms: u64,
    pub created_at: u64,
}
