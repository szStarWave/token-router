//! 把规范请求/响应展平成可读消息（供 `request_transcripts` 落库）。
//!
//! 只展平文本：图片/音频变占位符（不存 base64），工具调用/结果与思考块按文本展开。
//! 单条与单轮都有字符上限，超出部分截断并标记 `truncated`，避免一条大请求撑爆本地库。

use tr_contracts::admin::TranscriptMessage;
use tr_contracts::{
    CanonicalMessage, CanonicalRequest, CanonicalResponse, CanonicalRole, ContentBlock,
};

/// 单条消息上限（字符）。
pub const MAX_MESSAGE_CHARS: usize = 20_000;
/// 单轮（输入或输出各自）上限（字符）。
pub const MAX_TURN_CHARS: usize = 200_000;

/// 流式回复里累积的一次工具调用。
#[derive(Debug, Clone, Default)]
pub struct StreamToolCall {
    pub name: String,
    pub arguments: String,
}

struct Budget {
    remaining: usize,
    truncated: bool,
}

impl Budget {
    fn new() -> Self {
        Self {
            remaining: MAX_TURN_CHARS,
            truncated: false,
        }
    }

    /// 先按单条上限截断，再扣本轮剩余额度。
    fn take(&mut self, text: String) -> String {
        let text = truncate(text, MAX_MESSAGE_CHARS, &mut self.truncated);
        let out = truncate(text, self.remaining, &mut self.truncated);
        self.remaining = self.remaining.saturating_sub(out.chars().count());
        out
    }
}

fn truncate(text: String, limit: usize, truncated: &mut bool) -> String {
    let count = text.chars().count();
    if count <= limit {
        return text;
    }
    *truncated = true;
    if limit == 0 {
        return String::new();
    }
    let mut out: String = text.chars().take(limit).collect();
    out.push('…');
    out
}

fn role_str(role: CanonicalRole) -> &'static str {
    match role {
        CanonicalRole::System => "system",
        CanonicalRole::User => "user",
        CanonicalRole::Assistant => "assistant",
        CanonicalRole::Tool => "tool",
    }
}

/// 内容块 → 可读文本（媒体只留占位符）。
fn blocks_text(blocks: &[ContentBlock]) -> String {
    let mut parts: Vec<String> = Vec::with_capacity(blocks.len());
    for block in blocks {
        match block {
            ContentBlock::Text { text, .. } => parts.push(text.clone()),
            ContentBlock::Image { .. } => parts.push("[图片]".to_string()),
            ContentBlock::Audio { .. } => parts.push("[音频]".to_string()),
            ContentBlock::ToolUse {
                name, arguments, ..
            } => parts.push(format!("[工具调用 {name}] {arguments}")),
            ContentBlock::ToolResult {
                content, is_error, ..
            } => {
                let inner = blocks_text(content);
                let mark = if *is_error { "（错误）" } else { "" };
                parts.push(format!("[工具结果{mark}] {inner}"));
            }
            ContentBlock::Reasoning { text, .. } => parts.push(format!("[思考] {text}")),
        }
    }
    parts.join("\n")
}

fn message(role: &str, name: Option<String>, text: String) -> TranscriptMessage {
    TranscriptMessage {
        role: role.to_string(),
        name,
        text,
    }
}

/// 请求 → 这一轮发给上游的消息（system 块合并在最前）。
pub fn request_messages(request: &CanonicalRequest) -> (Vec<TranscriptMessage>, bool) {
    let mut budget = Budget::new();
    let mut messages: Vec<TranscriptMessage> = Vec::new();

    let system = request
        .system
        .iter()
        .map(|block| block.text.clone())
        .collect::<Vec<_>>()
        .join("\n\n");
    let system = budget.take(system);
    if !system.trim().is_empty() {
        messages.push(message("system", None, system));
    }

    for CanonicalMessage {
        role, blocks, name, ..
    } in &request.messages
    {
        let text = budget.take(blocks_text(blocks));
        if text.is_empty() {
            continue;
        }
        messages.push(message(role_str(*role), name.clone(), text));
    }

    (messages, budget.truncated)
}

/// 非流式响应 → 一条 assistant 回复。
pub fn response_messages(response: &CanonicalResponse) -> (Vec<TranscriptMessage>, bool) {
    let mut budget = Budget::new();
    let text = budget.take(blocks_text(&response.blocks));
    let messages = if text.is_empty() {
        Vec::new()
    } else {
        vec![message("assistant", None, text)]
    };
    (messages, budget.truncated)
}

/// 流式回复 → 一条 assistant 回复（正文 + 工具调用展开）。
pub fn stream_reply(text: &str, tool_calls: &[StreamToolCall]) -> (Vec<TranscriptMessage>, bool) {
    let mut budget = Budget::new();
    let mut parts: Vec<String> = Vec::new();
    if !text.is_empty() {
        parts.push(text.to_string());
    }
    for call in tool_calls {
        if call.name.is_empty() && call.arguments.is_empty() {
            continue;
        }
        parts.push(format!("[工具调用 {}] {}", call.name, call.arguments));
    }
    let text = budget.take(parts.join("\n"));
    let messages = if text.is_empty() {
        Vec::new()
    } else {
        vec![message("assistant", None, text)]
    };
    (messages, budget.truncated)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tr_contracts::SystemBlock;

    fn text_block(text: &str) -> ContentBlock {
        ContentBlock::Text {
            text: text.to_string(),
            cache_control: None,
        }
    }

    #[test]
    fn request_puts_system_first_and_keeps_roles() {
        let request = CanonicalRequest {
            system: vec![SystemBlock {
                text: "be brief".to_string(),
                cache_control: None,
            }],
            messages: vec![
                CanonicalMessage {
                    role: CanonicalRole::User,
                    blocks: vec![text_block("hello")],
                    ..CanonicalMessage::default()
                },
                CanonicalMessage {
                    role: CanonicalRole::Tool,
                    blocks: vec![ContentBlock::ToolResult {
                        tool_use_id: "t1".to_string(),
                        content: vec![text_block("ok")],
                        is_error: false,
                    }],
                    ..CanonicalMessage::default()
                },
            ],
            ..CanonicalRequest::default()
        };
        let (messages, truncated) = request_messages(&request);
        assert!(!truncated);
        assert_eq!(messages.len(), 3);
        assert_eq!(messages[0].role, "system");
        assert_eq!(messages[1].role, "user");
        assert_eq!(messages[2].role, "tool");
        assert!(messages[2].text.contains("[工具结果] ok"));
    }

    #[test]
    fn media_becomes_placeholder_not_base64() {
        let request = CanonicalRequest {
            messages: vec![CanonicalMessage {
                role: CanonicalRole::User,
                blocks: vec![ContentBlock::Image {
                    source: tr_contracts::MediaSource::Base64 {
                        media_type: "image/png".to_string(),
                        data: "QUJD".repeat(1000),
                    },
                    detail: None,
                }],
                ..CanonicalMessage::default()
            }],
            ..CanonicalRequest::default()
        };
        let (messages, _) = request_messages(&request);
        assert_eq!(messages[0].text, "[图片]");
        assert!(!messages[0].text.contains("QUJD"));
    }

    #[test]
    fn long_message_is_truncated_and_flagged() {
        let request = CanonicalRequest {
            messages: vec![CanonicalMessage {
                role: CanonicalRole::User,
                blocks: vec![text_block(&"x".repeat(MAX_MESSAGE_CHARS + 10))],
                ..CanonicalMessage::default()
            }],
            ..CanonicalRequest::default()
        };
        let (messages, truncated) = request_messages(&request);
        assert!(truncated);
        assert_eq!(messages[0].text.chars().count(), MAX_MESSAGE_CHARS + 1);
        assert!(messages[0].text.ends_with('…'));
    }

    #[test]
    fn stream_reply_includes_tool_calls() {
        let (messages, truncated) = stream_reply(
            "hello",
            &[StreamToolCall {
                name: "read_file".to_string(),
                arguments: "{\"path\":\"a\"}".to_string(),
            }],
        );
        assert!(!truncated);
        assert_eq!(messages.len(), 1);
        assert!(messages[0].text.starts_with("hello"));
        assert!(messages[0].text.contains("[工具调用 read_file]"));
    }

    #[test]
    fn empty_turn_produces_no_messages() {
        let (messages, truncated) = response_messages(&CanonicalResponse::default());
        assert!(messages.is_empty());
        assert!(!truncated);
    }
}
