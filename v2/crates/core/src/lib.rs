//! 信号抽取、策略与 Plan 合并。纯函数：零 async、零 IO。
//!
//! `cargo xtask check-deps` 强制本 crate 不得依赖 tokio / reqwest / rusqlite / axum /
//! tower / hyper；测试里也不允许 `#[tokio::test]`。失去这条约束就失去回放能力。
//!
//! 入口：
//! - [`decide`]：`DecisionSnapshot -> Plan`，回放与生产共用；
//! - [`merge`]：贡献合并与 §4.8 校验；
//! - [`signals`]：`CoreSignals` 抽取（token 估算当前为字符启发式）。

#![forbid(unsafe_code)]

pub mod merge;
pub mod signals;
pub mod strategy;

use tr_contracts::{
    CanonicalRequest, DecisionSnapshot, Plan, SessionFacts, Signals, StrategyState,
};

use crate::strategy::{Baseline, Failover, Strategy, StrategyConfig};

/// P0a 内置策略集：Baseline + Failover（无判官、无状态学习）。
pub fn default_strategies() -> Vec<Box<dyn Strategy>> {
    vec![Box::new(Baseline), Box::new(Failover)]
}

/// 决策入口：从快照还原 `RequestView`，跑策略，合并并校验。
///
/// P0a 不消费 `PlanDraft.judge_requests`：`agent` 在没有 `decide` 实现时直接丢弃，
/// 并在 P1 接入判官后改用两阶段调用。
pub fn decide(snapshot: &DecisionSnapshot) -> Plan {
    decide_with(snapshot, &default_strategies())
}

/// 与 [`decide`] 相同，但允许注入策略集（测试与后续 profile 使用）。
pub fn decide_with(snapshot: &DecisionSnapshot, strategies: &[Box<dyn Strategy>]) -> Plan {
    let view = snapshot.view();
    let mut drafts = Vec::with_capacity(strategies.len());

    for strategy in strategies {
        let id = strategy.id();
        let config = StrategyConfig::enabled(id.clone());
        let state: StrategyState = snapshot.strategy_state(&id).cloned().unwrap_or_default();
        let draft = strategy.draft(&view, &state, &config);
        drafts.push((strategy.priority(), draft));
    }

    merge::merge(&view, &drafts)
}

/// 组装 `Signals`：核心字段 + 空的扩展信号包。
///
/// `window_hint` 取虚拟模型声明的 `context_window`，或候选池里最紧的窗口；
/// 都没有就传 `None`（`ctx_fill_ratio = 0`，表示未知而不是“空”）。
pub fn build_signals(
    request: &CanonicalRequest,
    session: Option<&SessionFacts>,
    window_hint: Option<u32>,
) -> Signals {
    Signals {
        core: signals::extract_core(request, session, window_hint),
        extras: Default::default(),
    }
}
