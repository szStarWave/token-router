use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use crate::gateway::api::openai::{ChatCompletionRequest, Message, Role};

/// Stable session key for agent loops: hash anchor messages + tool set, not the full growing transcript.
///
/// Hashing every message makes `conversation_key` change each OpenClaw/Hermes turn, which resets
/// `tok_loop_delta` and inflates difficulty → excessive cloud routing.
pub fn conversation_key(req: &ChatCompletionRequest) -> String {
    let mut hasher = DefaultHasher::new();

    if let Some(sys) = req.messages.first() {
        hash_message(sys, &mut hasher);
    }
    if let Some(user) = req.messages.iter().find(|m| m.role == Role::User) {
        hash_message(user, &mut hasher);
    }

    req.tools.len().hash(&mut hasher);
    for tool in &req.tools {
        tool.function.name.hash(&mut hasher);
    }

    format!("conv:{:016x}", hasher.finish())
}

fn hash_message(msg: &Message, hasher: &mut DefaultHasher) {
    format!("{:?}", msg.role).hash(hasher);
    if let Some(ref c) = msg.content {
        c.hash(hasher);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gateway::api::openai::{
        ChatCompletionRequest, FunctionDefinition, Message, Role, ToolDefinition,
    };

    fn growing_request(n: usize) -> ChatCompletionRequest {
        let mut messages = vec![Message {
            role: Role::System,
            content: Some("system prompt".into()),
            content_parts: None,
            tool_calls: None,
            tool_call_id: None,
            reasoning_content: None,
        }];
        for i in 0..n {
            messages.push(Message {
                role: Role::User,
                content: Some(format!("user turn {i}")),
                content_parts: None,
                tool_calls: None,
                tool_call_id: None,
            reasoning_content: None,
            });
            messages.push(Message {
                role: Role::Assistant,
                content: Some(format!("assistant {i}")),
                content_parts: None,
                tool_calls: None,
                tool_call_id: None,
            reasoning_content: None,
            });
        }
        ChatCompletionRequest {
            model: "test".into(),
            messages,
            tools: vec![ToolDefinition {
                tool_type: "function".into(),
                function: FunctionDefinition {
                    name: "exec".into(),
                    description: None,
                    parameters: serde_json::json!({}),
                },
            }],
            stream: false,
            tool_choice: None,
            max_tokens: None,
            ..Default::default()
        }
    }

    #[test]
    fn key_stable_when_transcript_grows() {
        let k1 = conversation_key(&growing_request(2));
        let k2 = conversation_key(&growing_request(10));
        assert_eq!(k1, k2, "session key should not change as messages append");
    }
}
