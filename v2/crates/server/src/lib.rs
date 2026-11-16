//! axum 组装、鉴权、模型端点、健康检查与观测出口（P0a–P2）。
//!
//! 边界：不直接调用 `core`/`decide`，一律经 `agent`；客户端只通过 HTTP API 访问内核。

#![forbid(unsafe_code)]
