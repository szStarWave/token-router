//! 规范请求/响应形状（§4.1、§10.2、§10.3）。
//!
//! 三种 wire 协议（Chat Completions / Responses / Anthropic Messages）都在 `proto` 里
//! 映射到这里。本模块只描述形状，不做任何策略判断。

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::ids::{PhysicalModelId, ThinkingLevel, VirtualModelId};

/// 客户端来源协议。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IngressProtocol {
    #[default]
    ChatCompletions,
    Responses,
    AnthropicMessages,
}

impl IngressProtocol {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ChatCompletions => "chat_completions",
            Self::Responses => "responses",
            Self::AnthropicMessages => "anthropic_messages",
        }
    }
}

/// 消息角色。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CanonicalRole {
    System,
    #[default]
    User,
    Assistant,
    Tool,
}

/// 统一请求。字段语义见 §4.1；未知 provider 私有字段进 `passthrough`。
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct CanonicalRequest {
    pub messages: Vec<CanonicalMessage>,
    pub tools: Vec<ToolSpec>,
    /// 保留块结构与顺序；合并/展平由 `compat` 在发给上游前完成。
    pub system: Vec<SystemBlock>,
    pub sampling: SamplingParams,
    pub output: OutputParams,
    pub reasoning: Option<ReasoningParams>,
    pub meta: RequestMeta,
    /// 未识别的字段：不得静默丢弃；上游支持时原样带回。
    pub passthrough: BTreeMap<String, Value>,
}

/// 一条消息。`role` 与 `blocks` 组合表达内容；工具调用/结果用专门 block。
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct CanonicalMessage {
    pub role: CanonicalRole,
    pub blocks: Vec<ContentBlock>,
    pub tool_call_id: Option<String>,
    pub name: Option<String>,
}

/// 内容块。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ContentBlock {
    Text {
        text: String,
        cache_control: Option<CacheControl>,
    },
    Image {
        source: MediaSource,
        /// `low` / `high` / `auto`，原样透传。
        detail: Option<String>,
    },
    Audio {
        source: MediaSource,
    },
    ToolUse {
        id: String,
        name: String,
        arguments: Value,
    },
    ToolResult {
        tool_use_id: String,
        content: Vec<ContentBlock>,
        is_error: bool,
    },
    /// Anthropic thinking / OpenAI reasoning 摘要；`signature` 用于跨轮复用。
    Reasoning {
        text: String,
        signature: Option<String>,
        redacted: bool,
    },
}

/// 媒体来源。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum MediaSource {
    Base64 { media_type: String, data: String },
    Url { url: String },
    File { file_id: String },
}

/// 提示缓存标记。
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct CacheControl {
    /// 目前只有 `ephemeral`；保留字符串以便上游扩展。
    pub kind: String,
    pub ttl: Option<String>,
}

/// system 块（保留结构，不在这里合并）。
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct SystemBlock {
    pub text: String,
    pub cache_control: Option<CacheControl>,
}

/// 工具定义。
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct ToolSpec {
    pub name: String,
    pub description: Option<String>,
    /// JSON Schema。
    pub parameters: Value,
    pub strict: Option<bool>,
}

/// 采样参数。
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct SamplingParams {
    pub temperature: Option<f32>,
    pub top_p: Option<f32>,
    pub top_k: Option<u32>,
    pub seed: Option<u64>,
    pub stop: Vec<String>,
    pub presence_penalty: Option<f32>,
    pub frequency_penalty: Option<f32>,
    pub logprobs: Option<bool>,
    pub top_logprobs: Option<u32>,
}

/// 输出参数。
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct OutputParams {
    pub max_tokens: Option<u32>,
    pub stream: bool,
    pub stream_options: Option<StreamOptions>,
    /// `response_format` 原样保留（json_object / json_schema 形状随协议不同）。
    pub response_format: Option<Value>,
    pub n: Option<u32>,
    pub tool_choice: Option<Value>,
    pub parallel_tool_calls: Option<bool>,
}

/// 流式选项。
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct StreamOptions {
    pub include_usage: bool,
}

/// 推理参数。
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct ReasoningParams {
    /// `minimal` / `low` / `medium` / `high` 等，按上游语义透传。
    pub effort: Option<String>,
    pub budget_tokens: Option<u32>,
    pub enabled: Option<bool>,
}

/// 请求元数据：解析后的路由输入 + 原始 id。
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct RequestMeta {
    pub client_id: Option<String>,
    pub ingress_protocol: IngressProtocol,
    /// 客户端请求的虚拟模型；`None` 时由服务端默认虚拟模型补齐。
    pub virtual_model: Option<VirtualModelId>,
    pub thinking_level: Option<ThinkingLevel>,
    pub request_id: Option<String>,
    pub response_id: Option<String>,
    pub previous_response_id: Option<String>,
    /// 工作区/项目路径提示，用于会话键推导（§8.1）。
    pub workspace_hint: Option<String>,
}

/// 统一响应（非流式）。
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct CanonicalResponse {
    pub id: String,
    pub model: PhysicalModelId,
    pub blocks: Vec<ContentBlock>,
    pub finish_reason: FinishReason,
    pub usage: Usage,
}

/// token 用量。缓存字段缺失表示上游未报告，不是 0。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Usage {
    pub input_tokens: u32,
    pub output_tokens: u32,
    pub cached_tokens: Option<u32>,
    pub cache_write_tokens: Option<u32>,
}

/// 结束原因。
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FinishReason {
    #[default]
    Stop,
    Length,
    ToolUse,
    ContentFilter,
    Error,
    Other(String),
}

/// 流式事件（§10.3）。每个 wire 协议有独立状态机，但收敛到这一个序列。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum CanonicalStreamEvent {
    Started {
        id: String,
        model: String,
    },
    ContentDelta {
        text: String,
    },
    ReasoningDelta {
        text: String,
    },
    ToolCallDelta {
        index: u32,
        id: Option<String>,
        name: Option<String>,
        arguments: String,
    },
    Usage {
        input: u32,
        output: u32,
        cached: Option<u32>,
    },
    Finished {
        reason: FinishReason,
    },
    Error {
        message: String,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unknown_provider_fields_survive_round_trip() {
        let mut passthrough = BTreeMap::new();
        passthrough.insert("vendor_flag".to_string(), Value::Bool(true));
        let req = CanonicalRequest {
            passthrough,
            ..CanonicalRequest::default()
        };
        let json = serde_json::to_string(&req).unwrap();
        let back: CanonicalRequest = serde_json::from_str(&json).unwrap();
        assert_eq!(back.passthrough["vendor_flag"], Value::Bool(true));
    }

    #[test]
    fn tool_use_arguments_are_not_flattened() {
        let block = ContentBlock::ToolUse {
            id: "call_1".into(),
            name: "read_file".into(),
            arguments: serde_json::json!({"path": "a.txt", "x-vendor": 1}),
        };
        let back: ContentBlock =
            serde_json::from_str(&serde_json::to_string(&block).unwrap()).unwrap();
        assert_eq!(block, back);
    }

    #[test]
    fn missing_new_fields_deserialize_with_defaults() {
        // 旧数据没有 reasoning 字段时必须仍可读（§18.4.1 序列化约定）。
        let req: CanonicalRequest = serde_json::from_str(r#"{"messages":[]}"#).unwrap();
        assert!(req.reasoning.is_none());
        assert!(!req.output.stream);
    }
}
