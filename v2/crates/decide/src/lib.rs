//! 判官（DecisionModel）实现。**唯一允许调用判官模型的 crate**（P1）。
//!
//! 计划模块：
//! - 预算与 deadline、并发额度
//! - 外部判官服务契约（`JudgeServiceRef`、信封、问题定义、上下文策略）
//! - LLM adapter（动态 schema、`Uncertain`、结构化输出）
//! - `LocalDecisionSnapshot` 本地兜底

#![forbid(unsafe_code)]
