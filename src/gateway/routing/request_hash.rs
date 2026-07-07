use sha2::{Digest, Sha256};

use crate::gateway::api::openai::ChatCompletionRequest;
use serde_json::Value;

/// Canonical SHA-256 of the request body with the last two messages removed.
pub fn request_context_hash(req: &ChatCompletionRequest) -> String {
    let mut body = req.clone();
    let n = body.messages.len();
    if n > 2 {
        body.messages.truncate(n - 2);
    } else {
        body.messages.clear();
    }
    let value = serde_json::to_value(&body).expect("request serializable");
    let canonical = sort_json_keys(value);
    sha256_hex(&canonical.to_string())
}

fn sha256_hex(input: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(input.as_bytes());
    format!("{:x}", hasher.finalize())
}

fn sort_json_keys(value: Value) -> Value {
    match value {
        Value::Object(map) => {
            let mut keys: Vec<_> = map.keys().cloned().collect();
            keys.sort();
            let mut out = serde_json::Map::new();
            for key in keys {
                if let Some(v) = map.get(&key) {
                    out.insert(key, sort_json_keys(v.clone()));
                }
            }
            Value::Object(out)
        }
        Value::Array(items) => Value::Array(items.into_iter().map(sort_json_keys).collect()),
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gateway::api::openai::{
        ChatCompletionRequest, FunctionDefinition, Message, Role, ToolDefinition,
    };

    fn base_req() -> ChatCompletionRequest {
        ChatCompletionRequest {
            model: "gpt-4o".into(),
            messages: vec![
                Message {
                    role: Role::System,
                    content: Some("sys".into()),
                    content_parts: None,
                    tool_calls: None,
                    tool_call_id: None,
            reasoning_content: None,
                },
                Message {
                    role: Role::User,
                    content: Some("u1".into()),
                    content_parts: None,
                    tool_calls: None,
                    tool_call_id: None,
            reasoning_content: None,
                },
            ],
            tools: vec![ToolDefinition {
                tool_type: "function".into(),
                function: FunctionDefinition {
                    name: "exec".into(),
                    description: None,
                    parameters: serde_json::json!({"z": 1, "a": 2}),
                },
            }],
            stream: false,
            tool_choice: None,
            max_tokens: Some(100),
            max_completion_tokens: None,
            store: None,
            stream_options: None,
            thinking: None,
            reasoning_effort: None,
            temperature: None,
            top_p: None,
            parallel_tool_calls: None,
        }
    }

    #[test]
    fn hash_ignores_last_two_messages() {
        let mut a = base_req();
        a.messages.push(Message {
            role: Role::Assistant,
            content: Some("mid".into()),
            content_parts: None,
            tool_calls: None,
            tool_call_id: None,
            reasoning_content: None,
        });
        a.messages.push(Message {
            role: Role::User,
            content: Some("tail-a".into()),
            content_parts: None,
            tool_calls: None,
            tool_call_id: None,
            reasoning_content: None,
        });
        let h0 = request_context_hash(&a);
        a.messages[2].content = Some("mid-changed".into());
        a.messages[3].content = Some("tail-b".into());
        assert_eq!(request_context_hash(&a), h0);
    }

    #[test]
    fn canonical_hash_stable_key_order() {
        let mut a = base_req();
        a.tools[0].function.parameters =
            serde_json::json!({"b": 1, "a": 2, "nested": {"z": 0, "y": 1}});
        let mut b = base_req();
        b.tools[0].function.parameters =
            serde_json::json!({"nested": {"y": 1, "z": 0}, "a": 2, "b": 1});
        assert_eq!(request_context_hash(&a), request_context_hash(&b));
    }
}
