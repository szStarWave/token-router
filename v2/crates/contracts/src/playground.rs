//! Playground DTO（v3 P5-D）。
//!
//! 管理面测试请求：直达物理候选（绕过路由）或走虚拟模型（正常路由）。
//! 计时口径：`ttft_ms` = 从发起到上游第一个流式事件的耗时；
//! `duration_ms` = 到结束的总耗时。非流式上游无法给出 TTFT 时为 `null`。

use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// 一条测试消息。
#[derive(Debug, Clone, TS, Serialize, Deserialize)]
#[ts(export, export_to = "admin.ts")]
pub struct PlaygroundMessage {
    /// `user` / `assistant` / `system`。
    pub role: String,
    pub content: String,
}

/// `POST /v2/admin/playground/execute` 请求体。
#[derive(Debug, Clone, TS, Serialize, Deserialize)]
#[ts(export, export_to = "admin.ts")]
pub struct PlaygroundExecuteRequest {
    /// `candidate:<id>` 直达物理候选；`virtual:<id>` 走该虚拟模型的路由。
    pub target: String,
    #[serde(default)]
    #[ts(optional)]
    pub messages: Option<Vec<PlaygroundMessage>>,
    #[serde(default)]
    #[ts(optional)]
    pub system: Option<String>,
    #[serde(default)]
    #[ts(optional)]
    pub max_tokens: Option<u32>,
    #[serde(default)]
    #[ts(optional)]
    pub temperature: Option<f32>,
}

impl PlaygroundExecuteRequest {
    /// 解析 target。
    pub fn parse_target(&self) -> Option<(&str, &str)> {
        let (kind, id) = self.target.split_once(':')?;
        match kind {
            "candidate" | "virtual" if !id.is_empty() => Some((kind, id)),
            _ => None,
        }
    }
}

/// Playground 执行结果。
#[derive(Debug, Clone, TS, Serialize, Deserialize)]
#[ts(export, export_to = "admin.ts")]
pub struct PlaygroundExecuteResponse {
    pub ok: bool,
    /// 实际服务的模型（候选 id 或虚拟模型 id）。
    pub served_model: String,
    #[serde(default)]
    pub text: String,
    #[serde(default)]
    pub reasoning: String,
    #[serde(default)]
    pub ttft_ms: Option<u64>,
    pub duration_ms: u64,
    #[serde(default)]
    pub input_tokens: u32,
    #[serde(default)]
    pub output_tokens: u32,
    #[serde(default)]
    pub finish_reason: Option<String>,
    #[serde(default)]
    pub error: Option<String>,
}
