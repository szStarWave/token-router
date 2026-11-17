//! Token Router v2 核心契约：纯类型，零 IO，可回放。
//!
//! 本 crate 是 `core` / `decide` / `agent` / `upstream` / `server` 的公共语言，
//! 对应设计文档 §4（核心契约）与 §18.4.1（存储形状）。
//!
//! 约束：
//! - 不依赖任何 Tokio / HTTP / SQLite，保证 `core::decide` 是纯函数；
//! - 所有集合使用 `BTreeMap` / `BTreeSet` 或显式排序，保证决策投影可按字节比较；
//! - 持久化类型带 `#[serde(default)]`，新增字段不得让旧数据无法读取。
//!
//! 判官**服务契约**（`JudgeServiceRef` / 信封 / 问题定义 / 上下文策略）在 P1 落地，
//! 当前只包含决策路径需要的 `JudgeTrace` 与失败码。

#![forbid(unsafe_code)]

pub mod admin;
pub mod canonical;
pub mod errors;
pub mod ids;
pub mod judge;
pub mod ledger;
pub mod provider;
pub mod quota;
pub mod reason;
pub mod routing;
pub mod signals;
pub mod snapshot;
pub mod state;
pub mod verdict;

pub use admin::*;
pub use canonical::*;
pub use errors::*;
pub use ids::*;
pub use judge::*;
pub use ledger::*;
pub use provider::*;
pub use quota::*;
pub use reason::*;
pub use routing::*;
pub use signals::*;
pub use snapshot::*;
pub use state::*;
pub use verdict::*;
