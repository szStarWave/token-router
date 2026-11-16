//! 决策快照与确定性投影（§4.7）。
//!
//! 快照必须包含 **`RequestView` 的全部输入**，否则“回放同一快照”无法复现
//! sticky / hold 这类依赖上一轮派发的决策。这是对 §4.7 初版的补充：
//! 原字段只有 canonical/signals/candidates/health/session/states/ledger/profile/now/verdicts。

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::canonical::CanonicalRequest;
use crate::ids::StrategyId;
use crate::routing::{
    Candidate, ContextHold, Dispatch, FailedDispatch, ProfileConfig, RequestReason, RequestView,
    Selection, VirtualModel, VirtualModelState,
};
use crate::signals::Signals;
use crate::state::{CandidateHealth, LedgerSummary, SessionFacts, StrategyState};
use crate::verdict::Verdict;

/// 一次决策的完整输入。
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct DecisionSnapshot {
    pub canonical: CanonicalRequest,
    pub signals: Signals,
    pub candidates: Vec<Candidate>,
    pub health: CandidateHealth,
    pub session: SessionFacts,
    pub strategy_states: BTreeMap<StrategyId, StrategyState>,
    pub ledger: LedgerSummary,
    pub profile: ProfileConfig,
    /// 注入时钟。
    pub now_unix: u64,
    /// 必须按 `judge_request_id` 顺序排列（调用方保证）。
    pub verdicts: Vec<Verdict>,

    // ---- RequestView 的其余输入（§4.9）----
    pub selection: Selection,
    pub virtual_model: VirtualModel,
    pub reason: RequestReason,
    pub previous: Option<Dispatch>,
    pub failed: Option<FailedDispatch>,
    pub context_hold: Option<ContextHold>,
    pub router_state: Option<VirtualModelState>,
}

impl DecisionSnapshot {
    /// 还原策略视图，供 `core::decide` 与生产路径共用同一入口。
    pub fn view(&self) -> RequestView<'_> {
        RequestView {
            canonical: &self.canonical,
            signals: &self.signals,
            selection: &self.selection,
            virtual_model: &self.virtual_model,
            reason: self.reason,
            previous: self.previous.as_ref(),
            failed: self.failed.as_ref(),
            context_hold: self.context_hold.as_ref(),
            router_state: self.router_state.as_ref(),
            candidates: &self.candidates,
            health: &self.health,
            session: &self.session,
            ledger: &self.ledger,
            profile: &self.profile,
            now_unix: self.now_unix,
        }
    }

    pub fn strategy_state(&self, strategy_id: &str) -> Option<&StrategyState> {
        self.strategy_states.get(strategy_id)
    }

    /// 确定性投影：清零所有时间戳/请求 id，保留全部决策输入。
    ///
    /// 排除项见 §4.7 规则 4：`judge.latency_ms`、`request_id`、`created_at`、日志 id。
    /// 这里统一处理成“可比较形态”，两次回放必须得到相同字节。
    pub fn projection(&self) -> Self {
        let mut projected = self.clone();
        projected.session.updated_at = 0;
        projected.canonical.meta.request_id = None;
        for state in projected.strategy_states.values_mut() {
            state.updated_at = 0;
        }
        projected
    }

    /// 投影的规范 JSON。所有集合已是 `BTreeMap` / 稳定顺序，序列化即可比较。
    pub fn projection_json(&self) -> serde_json::Result<Vec<u8>> {
        serde_json::to_vec(&self.projection())
    }

    /// 投影指纹（SHA-256 十六进制）。账本 `snapshot_digest` 与 shadow 记录使用它。
    pub fn projection_digest(&self) -> serde_json::Result<String> {
        let bytes = self.projection_json()?;
        Ok(hex(&Sha256::digest(&bytes)))
    }
}

fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;

    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        // 写入 String 不会失败。
        let _ = write!(out, "{byte:02x}");
    }
    out
}
