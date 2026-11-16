//! 策略模型（§6.1）：三段式 trait 与内置策略。
//!
//! 约束：`draft` / `refine` / `on_outcome` 全部只读；状态变更只能通过
//! `Plan.state_updates` 或 `on_outcome` 的返回值交给 `agent` 原子提交。

pub mod baseline;
pub mod classify;
pub mod escalation;
pub mod failover;

pub use baseline::Baseline;
pub use classify::Classify;
pub use escalation::Escalation;
pub use failover::Failover;

use tr_contracts::{
    Budget, Candidate, Outcome, PlanDraft, RequestView, StateUpdate, StrategyId, StrategyState,
    ThinkingLevel, Verdict,
};

/// 策略触发点。`StrategyConfig.triggers` 可覆盖默认值。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Trigger {
    NewTask,
    UserTurn,
    Continuation,
    Retry,
    Direct,
    ContextRebuild,
    HardSignal,
    Failure,
    ReleaseCheck,
    Ambiguous,
}

/// 策略配置：参数、预算、启停。
///
/// `params` 用 JSON 保存策略自有参数，避免 `core` 为每个策略引入类型依赖；
/// 参数解析失败必须回退到默认值并记 warn，不得让请求失败。
#[derive(Debug, Clone)]
pub struct StrategyConfig {
    pub id: StrategyId,
    pub enabled: bool,
    pub triggers: Vec<Trigger>,
    pub params: serde_json::Value,
    pub budget: Budget,
}

impl StrategyConfig {
    pub fn enabled(id: impl Into<StrategyId>) -> Self {
        Self {
            id: id.into(),
            enabled: true,
            triggers: Vec::new(),
            params: serde_json::Value::Null,
            budget: Budget::default(),
        }
    }

    pub fn disabled(id: impl Into<StrategyId>) -> Self {
        Self {
            enabled: false,
            ..Self::enabled(id)
        }
    }
}

/// 原设计 §6.1 的三段式，外加 `priority`：合并代数（§6.3）需要知道策略优先级。
pub trait Strategy: Send + Sync {
    fn id(&self) -> StrategyId;

    /// 合并顺序：数值越小越优先（`Escalation=10`、`CompetenceGuard=20`、
    /// `Classify=30`、`Baseline=40`、`Failover=50`）。`Failover` 只做过滤。
    fn priority(&self) -> u8;

    /// 本策略关心的触发点；由配置覆盖。
    fn triggers(&self, cfg: &StrategyConfig) -> Vec<Trigger>;

    /// 只读：产生贡献与判官请求。
    fn draft(&self, view: &RequestView, state: &StrategyState, cfg: &StrategyConfig) -> PlanDraft;

    /// 只读：消费判官结论。默认无操作（P0a 没有判官）。
    fn refine(
        &self,
        _view: &RequestView,
        _state: &StrategyState,
        _verdicts: &[Verdict],
        _cfg: &StrategyConfig,
    ) -> PlanDraft {
        PlanDraft::default()
    }

    /// 只读：根据执行结果产出状态更新，由 `agent` 原子提交。
    fn on_outcome(
        &self,
        _outcome: &Outcome,
        _state: &StrategyState,
        _cfg: &StrategyConfig,
    ) -> Vec<StateUpdate> {
        Vec::new()
    }
}

/// thinking level 按候选能力 clamp；未知或空值落到 `off`。
pub(crate) fn clamp_thinking_level(
    candidate: &Candidate,
    requested: &ThinkingLevel,
) -> ThinkingLevel {
    if requested.is_empty() {
        return "off".to_string();
    }
    if candidate.supports_thinking_level(requested) {
        requested.clone()
    } else {
        "off".to_string()
    }
}

/// profile 的档位顺序转成排序权重；未声明的档位排在最后。
pub(crate) fn tier_rank(profile: &tr_contracts::ProfileConfig, tier: &str) -> usize {
    profile
        .strength_order
        .iter()
        .position(|candidate| candidate == tier)
        .unwrap_or(usize::MAX)
}
