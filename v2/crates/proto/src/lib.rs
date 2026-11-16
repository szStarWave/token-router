//! 协议编解码、兼容修复与流式事件状态机（P0a–P0b）。
//!
//! 边界：只依赖 `tr-contracts`，不感知策略；把三种 wire 协议映射到
//! `CanonicalRequest` / `CanonicalResponse` / `CanonicalStreamEvent`，不做路由判断。
//!
//! 当前已落地（S0）：
//! - [`coverage`]：字段覆盖探针，用来回答“canonical 到底丢不丢字段”；
//! - [`errors`]：HTTP 错误归一化为 `ErrorClass`（§18.4.2）；
//! - [`sse`]：三协议流式事件 → `CanonicalStreamEvent`。
//!
//! P0a 再补类型化 codec（`chat` / `responses` / `anthropic`）与 `compat`。

#![forbid(unsafe_code)]

pub mod chat;
pub mod coverage;
pub mod errors;
pub mod fixture;
pub mod sse;

/// 编解码错误。形状问题必须报错，不能静默丢弃字段。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CodecError {
    MissingField(&'static str),
    Invalid(&'static str),
    Unsupported(String),
}

impl std::fmt::Display for CodecError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::MissingField(field) => write!(f, "missing field: {field}"),
            Self::Invalid(detail) => write!(f, "invalid shape: {detail}"),
            Self::Unsupported(value) => write!(f, "unsupported value: {value}"),
        }
    }
}

impl std::error::Error for CodecError {}
