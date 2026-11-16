//! provider 客户端、流式转发、候选接力与上游错误归一化（P0a–P0b）。
//!
//! P0a 只实现 OpenAI 兼容的**非流式**执行；流式、Responses、Anthropic 在 P0b。
//!
//! 错误归一化只在这里发生（通过 `tr_proto::errors`），写回健康度由 `agent` 负责。

#![forbid(unsafe_code)]

pub mod http;
pub mod stream;

pub use http::{AttemptFailure, AttemptOutcome, HttpUpstream};
pub use stream::{StreamAttempt, StreamItem};
