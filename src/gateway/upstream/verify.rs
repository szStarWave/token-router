use std::collections::HashSet;

use crate::gateway::api::openai::ChatCompletionResponse;

/// Cloud validates edge output: tool names must match; text replies need loose agreement.
pub fn cloud_verifies_edge(edge: &ChatCompletionResponse, cloud: &ChatCompletionResponse) -> bool {
    let Some(edge_choice) = edge.choices.first() else {
        return false;
    };
    let Some(cloud_choice) = cloud.choices.first() else {
        return false;
    };

    let edge_tools = edge_choice.message.tool_calls.as_deref().unwrap_or(&[]);
    let cloud_tools = cloud_choice.message.tool_calls.as_deref().unwrap_or(&[]);

    if !edge_tools.is_empty() || !cloud_tools.is_empty() {
        if edge_tools.is_empty() || cloud_tools.is_empty() {
            return false;
        }
        let edge_names: Vec<_> = edge_tools.iter().map(|t| t.function.name.as_str()).collect();
        let cloud_names: Vec<_> = cloud_tools.iter().map(|t| t.function.name.as_str()).collect();
        return edge_names == cloud_names;
    }

    let edge_text = edge_choice.message.content.as_deref().unwrap_or("").trim();
    let cloud_text = cloud_choice.message.content.as_deref().unwrap_or("").trim();
    if edge_text.len() < 8 || cloud_text.len() < 8 {
        return false;
    }
    if crate::gateway::routing::response_has_uncertainty(cloud_text) {
        return false;
    }

    text_responses_compatible(edge_text, cloud_text)
}

fn text_responses_compatible(edge: &str, cloud: &str) -> bool {
    let edge_words: HashSet<_> = edge.split_whitespace().collect();
    let cloud_words: HashSet<_> = cloud.split_whitespace().collect();
    if edge_words.is_empty() || cloud_words.is_empty() {
        return false;
    }
    let inter = edge_words.intersection(&cloud_words).count();
    let union = edge_words.union(&cloud_words).count();
    inter as f32 / union as f32 >= 0.12
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gateway::api::openai::{
        ChatCompletionResponse, Choice, FunctionCallPayload, Message, Role, ToolCall,
    };

    fn resp_with_tools(names: &[&str]) -> ChatCompletionResponse {
        ChatCompletionResponse {
            id: "1".into(),
            object: "chat.completion".into(),
            created: 0,
            model: "m".into(),
            choices: vec![Choice {
                index: 0,
                message: Message {
                    role: Role::Assistant,
                    content: None,
                    content_parts: None,
                    tool_calls: Some(
                        names
                            .iter()
                            .enumerate()
                            .map(|(i, name)| ToolCall {
                                id: format!("call_{i}"),
                                call_type: "function".into(),
                                function: FunctionCallPayload {
                                    name: (*name).into(),
                                    arguments: "{}".into(),
                                },
                            })
                            .collect(),
                    ),
                    tool_call_id: None,
                },
                finish_reason: "tool_calls".into(),
            }],
            usage: None,
            token_router_meta: None,
        }
    }

    fn resp_with_text(text: &str) -> ChatCompletionResponse {
        ChatCompletionResponse {
            id: "1".into(),
            object: "chat.completion".into(),
            created: 0,
            model: "m".into(),
            choices: vec![Choice {
                index: 0,
                message: Message {
                    role: Role::Assistant,
                    content: Some(text.into()),
                    content_parts: None,
                    tool_calls: None,
                    tool_call_id: None,
                },
                finish_reason: "stop".into(),
            }],
            usage: None,
            token_router_meta: None,
        }
    }

    #[test]
    fn matching_tool_names_verify() {
        let edge = resp_with_tools(&["exec", "read"]);
        let cloud = resp_with_tools(&["exec", "read"]);
        assert!(cloud_verifies_edge(&edge, &cloud));
    }

    #[test]
    fn mismatched_tool_names_fail() {
        let edge = resp_with_tools(&["exec"]);
        let cloud = resp_with_tools(&["read"]);
        assert!(!cloud_verifies_edge(&edge, &cloud));
    }

    #[test]
    fn similar_text_verify() {
        let edge = resp_with_text("Use ffmpeg to convert the audio file to wav format.");
        let cloud = resp_with_text("Convert the audio to wav using ffmpeg on the file.");
        assert!(cloud_verifies_edge(&edge, &cloud));
    }
}
