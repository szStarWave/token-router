//! 会话状态、决策编排、账本与事务提交（P0a–P3）。
//!
//! 计划模块：
//! - `store`：SQLite 迁移与仓储（§18.4.1）、CAS
//! - `pipeline`：加载 → 决策 → 执行 → 账本提交
//! - `selection`：虚拟模型解析（§10.5）
//! - `reason`：`RequestReason` 判定
//! - `sticky` / `context_hold`：派发复用与软亲和（§4.9.1）
//! - `health`：熔断器
//! - `shadow`：判官影子记录

#![forbid(unsafe_code)]

pub mod approval;
pub mod config;
pub mod dispatch;
pub mod gate;
pub mod judge;
pub mod pipeline;
pub mod reason;
pub mod route_state;
pub mod session;
pub mod store;
pub mod strategy_quota;
