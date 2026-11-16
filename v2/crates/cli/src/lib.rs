//! 命令行：配置校验与决策回放（P0a-6）。
//!
//! `replay` 是唯一允许直接调用 `core::decide(&DecisionSnapshot)` 的入口：
//! 回放不执行 IO、不触发判官、不写库。

#![forbid(unsafe_code)]

use std::path::Path;

use tr_contracts::{DecisionSnapshot, Plan};

#[derive(Debug, thiserror::Error)]
pub enum ReplayError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
}

/// 从文件读取快照并重放决策。
pub fn replay_file(path: impl AsRef<Path>) -> Result<Plan, ReplayError> {
    let text = std::fs::read_to_string(path)?;
    replay_str(&text)
}

/// 从 JSON 字符串重放。
pub fn replay_str(json: &str) -> Result<Plan, ReplayError> {
    let snapshot: DecisionSnapshot = serde_json::from_str(json)?;
    Ok(tr_core::decide(&snapshot))
}

/// 回放投影指纹（用于比对“同一快照两次决策是否一致”）。
pub fn replay_digest(snapshot: &DecisionSnapshot) -> Result<String, serde_json::Error> {
    snapshot.projection_digest()
}
