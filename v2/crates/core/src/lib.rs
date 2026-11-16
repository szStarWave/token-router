//! 信号抽取、策略与 Plan 合并。纯函数：零 async、零 IO。
//!
//! `cargo xtask check-deps` 强制本 crate 不得依赖 tokio / reqwest / rusqlite / axum /
//! tower / hyper；测试里也不允许 `#[tokio::test]`。失去这条约束就失去回放能力。
//!
//! 计划模块：
//! - `signals`：`CoreSignals` 抽取（§4.3）
//! - `strategy`：`Baseline` / `Failover` / `Classify` / `Escalation` / `CompetenceGuard`
//! - `merge`：贡献合并与 Plan 校验不变量（§4.8、§6.3）
//! - `decide`：`decide(&DecisionSnapshot) -> Plan`

#![forbid(unsafe_code)]
