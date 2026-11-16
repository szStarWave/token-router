//! 虚拟模型 route state（P3-9）。
//!
//! 语义（§7）：
//! - 按 `(session, branch, virtual_model)` 存，fork/`/tree` 各自独立；
//! - **不是 sticky**：内核写它只是留下 route 历史，不据此改变 Plan；
//!   只有 route 插件显式读取并返回同一模型时才产生跨轮保持（P3-11）；
//! - compaction 后保留：状态不随消息内容变化，写入时按 `HISTORY_LIMIT` 截断历史，
//!   `last_served` 与 `generation` 永不丢弃。
//!
//! 所有权：state JSON 属于 route。若已有状态没有本模块的 schema 标记，
//! 视为其他 route 插件所有，内核**不覆盖**。

use serde::{Deserialize, Serialize};
use tr_contracts::{
    CandidateId, DispatchSource, PhysicalModelId, RequestReason, ThinkingLevel, VirtualModelState,
};

/// 本模块写入的 state schema 标记。
pub const ROUTE_STATE_SCHEMA: &str = "token-router.route-state/v1";

/// 历史条数上限（compaction）。
pub const HISTORY_LIMIT: usize = 16;

/// 一次成功派发的记录。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RouteStep {
    pub candidate_id: CandidateId,
    pub physical_model: PhysicalModelId,
    pub provider: String,
    pub thinking_level: ThinkingLevel,
    pub request_reason: RequestReason,
    pub dispatch_source: DispatchSource,
    pub switches: u32,
    pub at: u64,
}

/// route state 主体。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RouteState {
    pub schema: String,
    /// 每次成功写入 +1；compaction 不会重置。
    pub generation: u64,
    pub last_served: Option<RouteStep>,
    /// 最近的派发历史，最多 [`HISTORY_LIMIT`] 条（新的在后）。
    pub history: Vec<RouteStep>,
    pub updated_at: u64,
}

impl Default for RouteState {
    fn default() -> Self {
        Self {
            schema: ROUTE_STATE_SCHEMA.to_string(),
            generation: 0,
            last_served: None,
            history: Vec::new(),
            updated_at: 0,
        }
    }
}

impl RouteState {
    /// 从已有 `VirtualModelState` 解析；`None` 表示别家 route 拥有该 state。
    ///
    /// 空对象按“尚未初始化”处理（写第一版），不视为外来状态。
    pub fn from_existing(existing: Option<&VirtualModelState>) -> Option<Self> {
        let Some(existing) = existing else {
            return Some(Self::default());
        };
        if existing
            .state
            .as_object()
            .is_some_and(|object| object.is_empty())
        {
            return Some(Self::default());
        }
        let schema = existing.state.get("schema")?.as_str()?;
        if schema != ROUTE_STATE_SCHEMA {
            return None;
        }
        serde_json::from_value(existing.state.clone()).ok()
    }

    /// 记录一次成功派发，并按压缩策略截断历史。
    pub fn record(&mut self, step: RouteStep) {
        self.generation += 1;
        self.updated_at = step.at;
        self.last_served = Some(step.clone());
        self.history.push(step);
        self.compact();
    }

    /// compaction：只保留最近 `HISTORY_LIMIT` 条；`last_served`/`generation` 保留。
    pub fn compact(&mut self) {
        if self.history.len() > HISTORY_LIMIT {
            let excess = self.history.len() - HISTORY_LIMIT;
            self.history.drain(..excess);
        }
    }
}

/// 计算写入后的 `VirtualModelState`；返回 `None` 表示不应写（外来 route 状态）。
pub fn next_state(
    existing: Option<&VirtualModelState>,
    session_key: &str,
    branch_id: &str,
    virtual_model_id: &str,
    step: RouteStep,
) -> Option<VirtualModelState> {
    let mut state = RouteState::from_existing(existing)?;
    state.record(step);
    Some(VirtualModelState {
        session_key: session_key.to_string(),
        branch_id: branch_id.to_string(),
        virtual_model_id: virtual_model_id.to_string(),
        state: serde_json::to_value(&state).ok()?,
        version: existing.map(|state| state.version).unwrap_or(0),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn step(candidate: &str, at: u64) -> RouteStep {
        RouteStep {
            candidate_id: candidate.to_string(),
            physical_model: format!("physical-{candidate}"),
            provider: "openai".to_string(),
            thinking_level: "off".to_string(),
            request_reason: RequestReason::User,
            dispatch_source: DispatchSource::Strategy,
            switches: 0,
            at,
        }
    }

    #[test]
    fn compaction_keeps_last_served_and_generation() {
        let mut state = RouteState::default();
        for index in 0..(HISTORY_LIMIT + 5) {
            state.record(step(&format!("c{index}"), index as u64 + 1));
        }
        assert_eq!(state.generation, (HISTORY_LIMIT + 5) as u64);
        assert_eq!(state.history.len(), HISTORY_LIMIT);
        assert_eq!(
            state.last_served.as_ref().expect("last").candidate_id,
            "c20"
        );
        assert_eq!(state.history.last().expect("newest").candidate_id, "c20");
        // 最旧的 5 条被压缩掉。
        assert_eq!(
            state.history.first().expect("oldest kept").candidate_id,
            "c5"
        );
    }

    #[test]
    fn foreign_state_is_not_overwritten() {
        let existing = VirtualModelState {
            session_key: "s".to_string(),
            branch_id: "main".to_string(),
            virtual_model_id: "auto".to_string(),
            state: serde_json::json!({"round_robin": 3}),
            version: 1,
        };
        assert!(RouteState::from_existing(Some(&existing)).is_none());
        assert!(
            next_state(Some(&existing), "s", "main", "auto", step("a", 1)).is_none(),
            "must not clobber a plugin-owned state"
        );
    }

    #[test]
    fn empty_state_is_initialized() {
        let existing = VirtualModelState {
            session_key: "s".to_string(),
            branch_id: "main".to_string(),
            virtual_model_id: "auto".to_string(),
            state: serde_json::json!({}),
            version: 0,
        };
        let next = next_state(Some(&existing), "s", "main", "auto", step("a", 1)).expect("state");
        assert_eq!(next.version, 0);
        assert_eq!(next.state["schema"], ROUTE_STATE_SCHEMA);
        assert_eq!(next.state["last_served"]["candidate_id"], "a");
    }

    #[test]
    fn next_state_preserves_version_for_cas() {
        let existing = VirtualModelState {
            session_key: "s".to_string(),
            branch_id: "main".to_string(),
            virtual_model_id: "auto".to_string(),
            state: serde_json::json!({
                "schema": ROUTE_STATE_SCHEMA,
                "generation": 2,
                "last_served": null,
                "history": [],
                "updated_at": 10
            }),
            version: 7,
        };
        let next = next_state(Some(&existing), "s", "main", "auto", step("b", 20)).expect("state");
        assert_eq!(next.version, 7);
        assert_eq!(next.state["generation"], 3);
    }
}
