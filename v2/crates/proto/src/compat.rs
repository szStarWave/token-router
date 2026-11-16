//! 兼容修复（P0b-7）。
//!
//! 只做**协议形状**修复，不改语义：
//! - 工具调用与结果必须成对；中断/取消时补一条合成结果，否则下一轮请求会被上游拒绝。
//! - 丢弃没有任何内容的消息，避免上游 400。
//!
//! 这些规则来自 v1 `OpenClaw wrapper` 与 `tool_call` 邻接修复的经验；v2 用 canonical
//! 重新实现，不复用 v1 代码。

use tr_contracts::{CanonicalMessage, CanonicalRequest, CanonicalRole, ContentBlock};

/// 合成结果文本（与 Strands 的兜底语义一致）。
pub const INTERRUPTED_TOOL_RESULT: &str = "Tool was interrupted.";

/// 一次修复记录，进入日志/诊断。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CompatibilityFix {
    DroppedOrphanToolResult { tool_use_id: String },
    SynthesizedToolResult { tool_use_id: String },
    DroppedEmptyMessage { index: usize },
}

/// 修复工具调用配对与空消息。
pub fn repair(request: &mut CanonicalRequest) -> Vec<CompatibilityFix> {
    let mut fixes = Vec::new();

    // 1. 收集所有 tool_use id。
    let declared: std::collections::BTreeSet<String> = request
        .messages
        .iter()
        .flat_map(|message| message.blocks.iter())
        .filter_map(|block| match block {
            ContentBlock::ToolUse { id, .. } => Some(id.clone()),
            _ => None,
        })
        .collect();

    // 2. 删除孤儿 tool_result。
    for message in &mut request.messages {
        let before = message.blocks.len();
        message.blocks.retain(|block| match block {
            ContentBlock::ToolResult { tool_use_id, .. } => {
                let keep = declared.contains(tool_use_id);
                if !keep {
                    fixes.push(CompatibilityFix::DroppedOrphanToolResult {
                        tool_use_id: tool_use_id.clone(),
                    });
                }
                keep
            }
            _ => true,
        });
        let _ = before;
    }

    // 3. 为“后面没有结果”的 tool_use 补合成结果（放在同一条 assistant 之后）。
    let mut rebuilt: Vec<CanonicalMessage> = Vec::with_capacity(request.messages.len() + 2);
    for (position, message) in request.messages.iter().enumerate() {
        rebuilt.push(message.clone());

        let unanswered: Vec<String> = message
            .blocks
            .iter()
            .filter_map(|block| match block {
                ContentBlock::ToolUse { id, .. } => Some(id.clone()),
                _ => None,
            })
            .filter(|id| !has_later_result(&request.messages[position + 1..], id))
            .collect();

        // 最后一条消息如果是 assistant 且还没续接，等下一轮再补：此时补会改变语义。
        if unanswered.is_empty() || position + 1 >= request.messages.len() {
            continue;
        }

        for tool_use_id in unanswered {
            fixes.push(CompatibilityFix::SynthesizedToolResult {
                tool_use_id: tool_use_id.clone(),
            });
            rebuilt.push(CanonicalMessage {
                role: CanonicalRole::Tool,
                blocks: vec![ContentBlock::ToolResult {
                    tool_use_id,
                    content: vec![ContentBlock::Text {
                        text: INTERRUPTED_TOOL_RESULT.to_string(),
                        cache_control: None,
                    }],
                    is_error: true,
                }],
                ..CanonicalMessage::default()
            });
        }
    }

    // 4. 删除空消息（保留 system）。
    let mut index = 0;
    rebuilt.retain(|message| {
        let keep = message.role == CanonicalRole::System || !message.blocks.is_empty();
        if !keep {
            fixes.push(CompatibilityFix::DroppedEmptyMessage { index });
        }
        index += 1;
        keep
    });
    request.messages = rebuilt;

    fixes
}

fn has_later_result(messages: &[CanonicalMessage], tool_use_id: &str) -> bool {
    messages.iter().any(|message| {
        message.blocks.iter().any(|block| {
            matches!(block, ContentBlock::ToolResult { tool_use_id: id, .. } if id == tool_use_id)
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn user(text: &str) -> CanonicalMessage {
        CanonicalMessage {
            role: CanonicalRole::User,
            blocks: vec![ContentBlock::Text {
                text: text.to_string(),
                cache_control: None,
            }],
            ..CanonicalMessage::default()
        }
    }

    fn assistant_tool(id: &str) -> CanonicalMessage {
        CanonicalMessage {
            role: CanonicalRole::Assistant,
            blocks: vec![ContentBlock::ToolUse {
                id: id.to_string(),
                name: "read".to_string(),
                arguments: serde_json::json!({}),
            }],
            ..CanonicalMessage::default()
        }
    }

    fn tool_result(id: &str) -> CanonicalMessage {
        CanonicalMessage {
            role: CanonicalRole::Tool,
            blocks: vec![ContentBlock::ToolResult {
                tool_use_id: id.to_string(),
                content: vec![ContentBlock::Text {
                    text: "ok".to_string(),
                    cache_control: None,
                }],
                is_error: false,
            }],
            ..CanonicalMessage::default()
        }
    }

    fn request(messages: Vec<CanonicalMessage>) -> CanonicalRequest {
        CanonicalRequest {
            messages,
            ..CanonicalRequest::default()
        }
    }

    #[test]
    fn drops_orphan_result() {
        let mut req = request(vec![user("hi"), tool_result("missing")]);
        let fixes = repair(&mut req);
        assert_eq!(fixes.len(), 2); // 丢孤儿 + 丢掉空 tool 消息
        assert!(req.messages.len() == 1);
    }

    #[test]
    fn synthesizes_interrupted_result() {
        let mut req = request(vec![
            user("read it"),
            assistant_tool("call_1"),
            user("never mind, answer instead"),
        ]);
        let fixes = repair(&mut req);
        assert!(fixes.iter().any(|fix| matches!(
            fix,
            CompatibilityFix::SynthesizedToolResult { tool_use_id } if tool_use_id == "call_1"
        )));
        assert_eq!(req.messages.len(), 4);
        match &req.messages[2].blocks[0] {
            ContentBlock::ToolResult { is_error, .. } => assert!(is_error),
            other => panic!("expected synthesized result, got {other:?}"),
        }
    }

    #[test]
    fn keeps_paired_conversation_untouched() {
        let mut req = request(vec![
            user("read it"),
            assistant_tool("call_1"),
            tool_result("call_1"),
        ]);
        let fixes = repair(&mut req);
        assert!(fixes.is_empty(), "{fixes:?}");
        assert_eq!(req.messages.len(), 3);
    }

    #[test]
    fn drops_empty_messages() {
        let mut req = request(vec![user("hi"), CanonicalMessage::default()]);
        repair(&mut req);
        assert_eq!(req.messages.len(), 1);
    }
}
