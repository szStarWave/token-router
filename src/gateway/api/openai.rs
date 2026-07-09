use serde::{Deserialize, Deserializer, Serialize};
use serde_json::Value;

use crate::gateway::api::compat::finalize_upstream_request;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    System,
    User,
    Assistant,
    Tool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatCompletionRequest {
    pub model: String,
    pub messages: Vec<Message>,
    #[serde(default)]
    pub tools: Vec<ToolDefinition>,
    #[serde(default)]
    pub stream: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_choice: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_tokens: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_completion_tokens: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub store: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stream_options: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thinking: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning_effort: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enable_thinking: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning_split: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub top_p: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parallel_tool_calls: Option<bool>,
}

impl Default for ChatCompletionRequest {
    fn default() -> Self {
        Self {
            model: String::new(),
            messages: Vec::new(),
            tools: Vec::new(),
            stream: false,
            tool_choice: None,
            max_tokens: None,
            max_completion_tokens: None,
            store: None,
            stream_options: None,
            thinking: None,
            reasoning_effort: None,
            enable_thinking: None,
            reasoning_split: None,
            reasoning: None,
            temperature: None,
            top_p: None,
            parallel_tool_calls: None,
        }
    }
}

impl ChatCompletionRequest {
    /// Normalize messages for upstream providers that only accept string `content`,
    /// merge system prompts, and apply thinking-model compatibility fixes.
    pub fn for_upstream(&self, upstream_base: Option<&str>) -> Self {
        finalize_upstream_request(self.clone(), upstream_base)
    }

    /// Edge-side normalization (same as cloud; edge models also benefit from merged system).
    pub fn for_edge_upstream(&self, upstream_base: Option<&str>) -> Self {
        self.for_upstream(upstream_base)
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct Message {
    pub role: Role,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_parts: Option<Vec<ContentPart>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<ToolCall>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning_content: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum ContentField {
    Text(String),
    Parts(Vec<ContentPart>),
}

#[derive(Debug, Deserialize)]
struct MessageRaw {
    role: Role,
    #[serde(default)]
    content: Option<ContentField>,
    #[serde(default)]
    tool_calls: Option<Vec<ToolCall>>,
    #[serde(default)]
    tool_call_id: Option<String>,
    #[serde(default)]
    reasoning_content: Option<String>,
}

impl From<MessageRaw> for Message {
    fn from(raw: MessageRaw) -> Self {
        let (content, content_parts) = match raw.content {
            None => (None, None),
            Some(ContentField::Text(text)) => (Some(text), None),
            Some(ContentField::Parts(parts)) => {
                let text = join_text_parts(&parts);
                let content = if text.is_empty() { None } else { Some(text) };
                (content, Some(parts))
            }
        };
        Self {
            role: raw.role,
            content,
            content_parts,
            tool_calls: raw.tool_calls,
            tool_call_id: raw.tool_call_id,
            reasoning_content: raw.reasoning_content,
        }
    }
}

impl<'de> Deserialize<'de> for Message {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        MessageRaw::deserialize(deserializer).map(Into::into)
    }
}

impl Message {
    pub fn normalized_for_upstream(&self) -> Self {
        let mut msg = self.clone();
        if msg.content.as_deref().unwrap_or("").is_empty() {
            if let Some(parts) = &msg.content_parts {
                let text = join_text_parts(parts);
                if !text.is_empty() {
                    msg.content = Some(text);
                }
            }
        }
        msg.content_parts = None;
        if msg.role == Role::Assistant
            && msg
                .tool_calls
                .as_ref()
                .is_some_and(|calls| !calls.is_empty())
            && msg.content.as_deref().unwrap_or("").is_empty()
        {
            msg.content = None;
        }
        msg
    }
}

fn join_text_parts(parts: &[ContentPart]) -> String {
    parts
        .iter()
        .filter_map(|part| part.text.as_ref())
        .cloned()
        .collect::<Vec<_>>()
        .join("\n")
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContentPart {
    #[serde(rename = "type")]
    pub part_type: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image_url: Option<ImageUrl>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImageUrl {
    pub url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolDefinition {
    #[serde(rename = "type")]
    pub tool_type: String,
    pub function: FunctionDefinition,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FunctionDefinition {
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub parameters: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCall {
    pub id: String,
    #[serde(rename = "type")]
    pub call_type: String,
    pub function: FunctionCallPayload,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FunctionCallPayload {
    pub name: String,
    pub arguments: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Usage {
    #[serde(default)]
    pub prompt_tokens: u32,
    #[serde(default)]
    pub completion_tokens: u32,
    #[serde(default)]
    pub total_tokens: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt_tokens_details: Option<PromptTokenDetails>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PromptTokenDetails {
    #[serde(default)]
    pub cached_tokens: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatCompletionResponse {
    pub id: String,
    pub object: String,
    pub created: u64,
    pub model: String,
    pub choices: Vec<Choice>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage: Option<Usage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub token_router_meta: Option<TokenRouterMeta>,
    /// Model sent to upstream after config override; not serialized to clients.
    #[serde(skip, default)]
    pub upstream_forwarded_model: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Choice {
    pub index: u32,
    pub message: Message,
    pub finish_reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenRouterMeta {
    pub route: String,
    pub fallback: bool,
    pub difficulty_score: f32,
    pub step_kind: String,
    pub reason_codes: Vec<String>,
    pub tokens_in: u32,
    pub tokens_out: u32,
    pub input_ratio: f32,
    pub cloud_input_saved: u32,
    pub profile: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deserializes_string_content() {
        let msg: Message = serde_json::from_str(
            r#"{"role":"user","content":"hello"}"#,
        )
        .unwrap();
        assert_eq!(msg.content.as_deref(), Some("hello"));
        assert!(msg.content_parts.is_none());
    }

    #[test]
    fn deserializes_array_content() {
        let msg: Message = serde_json::from_str(
            r#"{"role":"user","content":[{"type":"text","text":"你好"}]}"#,
        )
        .unwrap();
        assert_eq!(msg.content.as_deref(), Some("你好"));
        assert!(msg.content_parts.is_some());
    }

    #[test]
    fn normalizes_array_content_for_upstream() {
        let msg: Message = serde_json::from_str(
            r#"{"role":"user","content":[{"type":"text","text":"你好"}]}"#,
        )
        .unwrap();
        let normalized = msg.normalized_for_upstream();
        assert_eq!(normalized.content.as_deref(), Some("你好"));
        assert!(normalized.content_parts.is_none());
    }

    #[test]
    fn deserializes_openclaw_style_request() {
        let req: ChatCompletionRequest = serde_json::from_str(
            r#"{
                "model":"MiniMax-M2.5",
                "messages":[
                    {"role":"system","content":"You are a personal assistant"},
                    {"role":"user","content":[{"type":"text","text":"你好"}]}
                ],
                "stream":true,
                "store":false,
                "max_completion_tokens":8192
            }"#,
        )
        .unwrap();
        assert_eq!(req.messages.len(), 2);
        assert_eq!(req.messages[1].content.as_deref(), Some("你好"));
        assert_eq!(req.max_completion_tokens, Some(8192));
    }
}
