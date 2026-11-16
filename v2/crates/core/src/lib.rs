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

pub mod cache_cost;
pub mod gate;
pub mod merge;
pub mod signals;
pub mod strategy;
pub mod stream_policy;

use tr_contracts::{
    CanonicalRequest, DecisionSnapshot, JudgeRequest, Plan, PlanDraft, SessionFacts, Signals,
    StrategyState, Verdict,
};

use crate::strategy::{Baseline, Classify, Escalation, Failover, Strategy, StrategyConfig};

/// P1 内置策略集：Baseline + Failover + Escalation + Classify（默认 off/shadow 由参数控制）。
pub fn default_strategies() -> Vec<Box<dyn Strategy>> {
    vec![
        Box::new(Baseline),
        Box::new(Failover),
        Box::new(Escalation),
        Box::new(Classify),
    ]
}

/// 决策入口：从快照还原 `RequestView`，跑策略，合并并校验。
///
/// 没有判官结论时等价于 `decide_with_verdicts(.., &[])`。
pub fn decide(snapshot: &DecisionSnapshot) -> Plan {
    decide_with(snapshot, &default_strategies())
}

/// 与 [`decide`] 相同，但允许注入策略集（测试与后续 profile 使用）。
pub fn decide_with(snapshot: &DecisionSnapshot, strategies: &[Box<dyn Strategy>]) -> Plan {
    decide_with_verdicts(snapshot, strategies, &[])
}

/// 两阶段执行：`draft` →（可选 `refine`）→ 合并。
///
/// `refine` 的产出排在 `draft` 之后，因此同一策略的状态更新以 `refine` 为准。
pub fn decide_with_verdicts(
    snapshot: &DecisionSnapshot,
    strategies: &[Box<dyn Strategy>],
    verdicts: &[Verdict],
) -> Plan {
    let view = snapshot.view();
    let mut drafts: Vec<(u8, PlanDraft)> = Vec::with_capacity(strategies.len() * 2);

    for strategy in strategies {
        let id = strategy.id();
        let config = StrategyConfig::enabled(id.clone());
        let state: StrategyState = snapshot.strategy_state(&id).cloned().unwrap_or_default();
        let draft = strategy.draft(&view, &state, &config);
        drafts.push((strategy.priority(), draft));

        if !verdicts.is_empty() {
            let refined = strategy.refine(&view, &state, verdicts, &config);
            drafts.push((strategy.priority(), refined));
        }
    }

    merge::merge(&view, &drafts)
}

/// 只跑 `draft`，收集判官请求（供 `agent` 决定是否咨询判官）。
pub fn draft_all(
    snapshot: &DecisionSnapshot,
    strategies: &[Box<dyn Strategy>],
) -> Vec<JudgeRequest> {
    let view = snapshot.view();
    let mut requests = Vec::new();
    for strategy in strategies {
        let id = strategy.id();
        let config = StrategyConfig::enabled(id.clone());
        let state: StrategyState = snapshot.strategy_state(&id).cloned().unwrap_or_default();
        let draft = strategy.draft(&view, &state, &config);
        requests.extend(draft.judge_requests);
    }
    requests
}

/// 一次 draft 同时产出 Plan 与判官请求，避免 `agent` 重复跑策略。
pub fn decide_and_collect(
    snapshot: &DecisionSnapshot,
    strategies: &[Box<dyn Strategy>],
) -> (Plan, Vec<JudgeRequest>) {
    let view = snapshot.view();
    let mut drafts: Vec<(u8, PlanDraft)> = Vec::with_capacity(strategies.len());
    let mut requests = Vec::new();

    for strategy in strategies {
        let id = strategy.id();
        let config = StrategyConfig::enabled(id.clone());
        let state: StrategyState = snapshot.strategy_state(&id).cloned().unwrap_or_default();
        let draft = strategy.draft(&view, &state, &config);
        requests.extend(draft.judge_requests.iter().cloned());
        drafts.push((strategy.priority(), draft));
    }

    (merge::merge(&view, &drafts), requests)
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
