//! provider 客户端、流式转发、候选接力与上游错误归一化（P0a–P0b）。
//!
//! `ErrorClass` 只在这里产生一次（§18.4.2）；下游不得再匹配错误文本。

#![forbid(unsafe_code)]
