//! 协议编解码、兼容修复与流式事件状态机（P0a–P0b）。
//!
//! 边界：只依赖 `tr-contracts`，不感知策略；把三种 wire 协议映射到
//! `CanonicalRequest` / `CanonicalResponse` / `CanonicalStreamEvent`，不做路由判断。
//!
//! 计划模块：
//! - `chat`：Chat Completions 编解码
//! - `responses`：Responses 编解码 + `previous_response_id`
//! - `anthropic`：Anthropic Messages 编解码（thinking / redacted block）
//! - `compat`：OpenClaw wrapper 剥离、tool_call 邻接修复、末尾清理
//! - `stream`：SSE 状态机与事件序列校验

#![forbid(unsafe_code)]
