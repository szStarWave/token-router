//! 命令行：配置、诊断、回放、评测、capture（P0a 起逐步补齐）。
//!
//! `replay` 是唯一允许直接调用 `core::decide(&DecisionSnapshot)` 的入口：
//! 回放不执行 IO、不触发判官、不写库。

#![forbid(unsafe_code)]
