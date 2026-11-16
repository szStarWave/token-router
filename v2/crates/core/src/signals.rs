//! 信号抽取（§4.3）。
//!
//! `CoreSignals` 是内核稳定字段；这里只做**确定性、零 IO** 的抽取，不调用模型、
//! 不读环境变量。词频/关键词体系属于可插拔 `SignalExtractor`（`Signals.extras`），
//! 不在本模块。
//!
//! token 估算目前用字符启发式（≈4 字符/token，图片固定 85 token），P0a 末替换为
//! `tokenizers` 真实计数；替换前必须保持函数签名与单测形态不变。

use std::collections::BTreeMap;

use tr_contracts::{
    CanonicalMessage, CanonicalRequest, ContentBlock, CoreSignals, LoopInfo, SessionFacts, StepKind,
};

/// 每条消息的固定开销（role/分隔符）。
const MESSAGE_OVERHEAD_TOKENS: u32 = 4;
/// 一张图片的估算 token（OpenAI low detail 量级）。
const IMAGE_TOKENS: u32 = 85;

/// 抽取核心信号。
///
/// `window_hint` 用于计算 `ctx_fill_ratio`：传 `None` 时填 0（未知），
/// 不要假装知道容量。调用点（`decide`）通常传虚拟模型声明或最紧候选的窗口。
pub fn extract_core(
    request: &CanonicalRequest,
    session: Option<&SessionFacts>,
    window_hint: Option<u32>,
) -> CoreSignals {
    let estimated = estimate_input_tokens(request);
    CoreSignals {
        step_kind: detect_step_kind(request),
        consecutive_tool_error_streak: tool_error_streak(request),
        repeated_tool_loop: detect_tool_loop(request),
        assistant_failed_recent: session.is_some_and(|facts| facts.consecutive_failures > 0),
        ctx_fill_ratio: match window_hint {
            Some(window) if window > 0 => (estimated as f32 / window as f32).min(1.0),
            _ => 0.0,
        },
        multimodal: has_multimodal(request),
        // 在飞请求数由 upstream 采集后写回；纯函数里没有这个信息。
        candidate_inflight: BTreeMap::new(),
        lexical: None,
        difficulty_gap: None,
        hard_signal_score: None,
    }
}

/// 估算输入 token。
pub fn estimate_input_tokens(request: &CanonicalRequest) -> u32 {
    let mut units = 0usize;

    for block in &request.system {
        units += block.text.chars().count();
    }
    for message in &request.messages {
        units += message_units(message);
        units += 4 * MESSAGE_OVERHEAD_TOKENS as usize;
    }
    for tool in &request.tools {
        units += tool.name.chars().count();
        units += tool.description.as_deref().unwrap_or("").chars().count();
        // Schema 是 JSON 文本；直接按序列化长度估算，避免引入额外分配路径以外的逻辑。
        units += tool.parameters.to_string().len();
    }

    (units / 4) as u32
}

fn message_units(message: &CanonicalMessage) -> usize {
    message.blocks.iter().map(block_units).sum()
}

fn block_units(block: &ContentBlock) -> usize {
    match block {
        ContentBlock::Text { text, .. } => text.chars().count(),
        ContentBlock::Reasoning { text, .. } => text.chars().count(),
        ContentBlock::Image { .. } | ContentBlock::Audio { .. } => IMAGE_TOKENS as usize * 4,
        ContentBlock::ToolUse {
            name, arguments, ..
        } => name.chars().count() + arguments.to_string().len(),
        ContentBlock::ToolResult { content, .. } => content.iter().map(block_units).sum(),
    }
}

fn has_multimodal(request: &CanonicalRequest) -> bool {
    request.messages.iter().any(|message| {
        message.blocks.iter().any(|block| {
            matches!(
                block,
                ContentBlock::Image { .. } | ContentBlock::Audio { .. }
            )
        })
    })
}

/// 工具错误 streak：从末尾数连续 `is_error` 的 ToolResult。
fn tool_error_streak(request: &CanonicalRequest) -> u32 {
    let mut streak = 0;
    for message in request.messages.iter().rev() {
        let mut saw_result = false;
        for block in message.blocks.iter().rev() {
            if let ContentBlock::ToolResult { is_error, .. } = block {
                saw_result = true;
                if *is_error {
                    streak += 1;
                } else {
                    return streak;
                }
            }
        }
        if !saw_result {
            break;
        }
    }
    streak
}

/// 同一工具连续出现 ≥3 次视为循环。
fn detect_tool_loop(request: &CanonicalRequest) -> Option<LoopInfo> {
    let names: Vec<&str> = request
        .messages
        .iter()
        .flat_map(|message| message.blocks.iter())
        .filter_map(|block| match block {
            ContentBlock::ToolUse { name, .. } => Some(name.as_str()),
            _ => None,
        })
        .collect();

    let last = *names.last()?;
    let repeat_count = names.iter().rev().take_while(|name| **name == last).count();
    if repeat_count >= 3 {
        Some(LoopInfo {
            tool_name: last.to_string(),
            repeat_count: repeat_count as u32,
            window: repeat_count as u32,
        })
    } else {
        None
    }
}

/// 步态推断（P0a 启发式；`step_kind_detail` 留给 `SignalBag`）。
///
/// 规则刻意保守：只能从消息形状推断，不做意图分类。
fn detect_step_kind(request: &CanonicalRequest) -> StepKind {
    let Some(last) = request.messages.last() else {
        return StepKind::Heartbeat;
    };

    let assistant_tool_use_recent = request.messages.iter().rev().take(3).any(|message| {
        message
            .blocks
            .iter()
            .any(|block| matches!(block, ContentBlock::ToolUse { .. }))
    });

    match last.role {
        tr_contracts::CanonicalRole::Tool => StepKind::Tool,
        tr_contracts::CanonicalRole::Assistant => {
            if last
                .blocks
                .iter()
                .any(|block| matches!(block, ContentBlock::ToolUse { .. }))
            {
                StepKind::Tool
            } else {
                StepKind::Final
            }
        }
        tr_contracts::CanonicalRole::User => {
            // 用户消息紧跟在工具结果之后，通常是续接而不是新任务。
            if assistant_tool_use_recent {
                StepKind::Tool
            } else {
                StepKind::DirectChat
            }
        }
        tr_contracts::CanonicalRole::System => StepKind::Background,
    }
}

// `ImagePlaceholder` 仅用于文档占位，避免 `ContentBlock` 匹配时出现魔法分支。
#[allow(dead_code)]
fn _image_tokens_documented() -> u32 {
    IMAGE_TOKENS
}

#[cfg(test)]
mod tests {
    use super::*;
    use tr_contracts::{CanonicalMessage, CanonicalRole, MediaSource};

    fn user_text(text: &str) -> CanonicalMessage {
        CanonicalMessage {
            role: CanonicalRole::User,
            blocks: vec![ContentBlock::Text {
                text: text.to_string(),
                cache_control: None,
            }],
            ..CanonicalMessage::default()
        }
    }

    #[test]
    fn estimates_grow_with_text() {
        let short = CanonicalRequest {
            messages: vec![user_text("hi")],
            ..CanonicalRequest::default()
        };
        let long = CanonicalRequest {
            messages: vec![user_text(&"word ".repeat(400))],
            ..CanonicalRequest::default()
        };
        assert!(estimate_input_tokens(&long) > estimate_input_tokens(&short));
    }

    #[test]
    fn image_counts_as_fixed_tokens() {
        let request = CanonicalRequest {
            messages: vec![CanonicalMessage {
                role: CanonicalRole::User,
                blocks: vec![ContentBlock::Image {
                    source: MediaSource::Url {
                        url: "https://example.com/a.png".into(),
                    },
                    detail: None,
                }],
                ..CanonicalMessage::default()
            }],
            ..CanonicalRequest::default()
        };
        assert!(estimate_input_tokens(&request) >= IMAGE_TOKENS);
    }

    #[test]
    fn tool_errors_streak_from_tail() {
        let request = CanonicalRequest {
            messages: vec![
                CanonicalMessage {
                    role: CanonicalRole::Tool,
                    blocks: vec![ContentBlock::ToolResult {
                        tool_use_id: "a".into(),
                        content: vec![],
                        is_error: true,
                    }],
                    ..CanonicalMessage::default()
                },
                CanonicalMessage {
                    role: CanonicalRole::Tool,
                    blocks: vec![ContentBlock::ToolResult {
                        tool_use_id: "b".into(),
                        content: vec![],
                        is_error: true,
                    }],
                    ..CanonicalMessage::default()
                },
            ],
            ..CanonicalRequest::default()
        };
        assert_eq!(tool_error_streak(&request), 2);
    }

    #[test]
    fn repeated_tool_is_a_loop() {
        let tool = |name: &str| ContentBlock::ToolUse {
            id: format!("id-{name}-{}", name.len()),
            name: name.to_string(),
            arguments: serde_json::json!({}),
        };
        let request = CanonicalRequest {
            messages: vec![
                CanonicalMessage {
                    role: CanonicalRole::Assistant,
                    blocks: vec![tool("read"), tool("read"), tool("read")],
                    ..CanonicalMessage::default()
                },
                user_text("again"),
            ],
            ..CanonicalRequest::default()
        };
        let loop_info = detect_tool_loop(&request).expect("loop");
        assert_eq!(loop_info.tool_name, "read");
        assert_eq!(loop_info.repeat_count, 3);
    }

    #[test]
    fn ctx_fill_ratio_is_unknown_without_window() {
        let request = CanonicalRequest {
            messages: vec![user_text("hello")],
            ..CanonicalRequest::default()
        };
        assert_eq!(extract_core(&request, None, None).ctx_fill_ratio, 0.0);
        assert!(extract_core(&request, None, Some(100)).ctx_fill_ratio > 0.0);
    }
}
