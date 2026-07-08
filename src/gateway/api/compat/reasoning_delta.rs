use serde_json::Value;

const THINK_OPEN_TAG: &str = "<think>";
const THINK_CLOSE_TAG: &str = "</think>";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StreamContentPiece {
    Reasoning(String),
    Text(String),
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
enum InlineThinkMode {
    #[default]
    Detecting,
    Reasoning,
    Text,
}

#[derive(Debug, Default)]
pub struct InlineThinkState {
    mode: InlineThinkMode,
    buffer: String,
}

impl InlineThinkState {
    pub fn push_content(&mut self, delta: &str) -> Vec<StreamContentPiece> {
        if delta.is_empty() {
            return Vec::new();
        }
        match self.mode {
            InlineThinkMode::Text => vec![StreamContentPiece::Text(delta.to_string())],
            InlineThinkMode::Detecting => {
                self.buffer.push_str(delta);
                match leading_think_prefix_decision(&self.buffer) {
                    ThinkPrefixDecision::NeedMore => Vec::new(),
                    ThinkPrefixDecision::Reasoning => {
                        self.mode = InlineThinkMode::Reasoning;
                        self.drain_complete_inline_think()
                    }
                    ThinkPrefixDecision::Text => {
                        self.mode = InlineThinkMode::Text;
                        let text = std::mem::take(&mut self.buffer);
                        if text.is_empty() {
                            Vec::new()
                        } else {
                            vec![StreamContentPiece::Text(text)]
                        }
                    }
                }
            }
            InlineThinkMode::Reasoning => {
                self.buffer.push_str(delta);
                self.drain_complete_inline_think()
            }
        }
    }

    pub fn flush(&mut self) -> Vec<StreamContentPiece> {
        match self.mode {
            InlineThinkMode::Text => Vec::new(),
            InlineThinkMode::Detecting => {
                self.mode = InlineThinkMode::Text;
                let text = std::mem::take(&mut self.buffer);
                if text.is_empty() {
                    Vec::new()
                } else {
                    vec![StreamContentPiece::Text(text)]
                }
            }
            InlineThinkMode::Reasoning => {
                let buffered = std::mem::take(&mut self.buffer);
                self.mode = InlineThinkMode::Text;
                if let Some((reasoning, answer)) = split_leading_think_block(&buffered) {
                    let mut out = Vec::new();
                    if !reasoning.is_empty() {
                        out.push(StreamContentPiece::Reasoning(reasoning));
                    }
                    if !answer.is_empty() {
                        out.push(StreamContentPiece::Text(answer));
                    }
                    return out;
                }
                let reasoning = strip_leading_think_open_tag(&buffered).unwrap_or(buffered);
                if reasoning.trim().is_empty() {
                    Vec::new()
                } else {
                    vec![StreamContentPiece::Reasoning(reasoning)]
                }
            }
        }
    }

    fn drain_complete_inline_think(&mut self) -> Vec<StreamContentPiece> {
        let Some((reasoning, answer)) = split_leading_think_block(&self.buffer) else {
            return Vec::new();
        };
        self.mode = InlineThinkMode::Text;
        self.buffer.clear();
        let mut out = Vec::new();
        if !reasoning.is_empty() {
            out.push(StreamContentPiece::Reasoning(reasoning));
        }
        if !answer.is_empty() {
            out.push(StreamContentPiece::Text(answer));
        }
        out
    }
}

enum ThinkPrefixDecision {
    NeedMore,
    Reasoning,
    Text,
}

fn leading_think_prefix_decision(buffer: &str) -> ThinkPrefixDecision {
    let trimmed = buffer.trim_start();
    if trimmed.is_empty() {
        return ThinkPrefixDecision::NeedMore;
    }
    if trimmed.starts_with(THINK_OPEN_TAG) {
        return ThinkPrefixDecision::Reasoning;
    }
    if THINK_OPEN_TAG.starts_with(trimmed) {
        return ThinkPrefixDecision::NeedMore;
    }
    ThinkPrefixDecision::Text
}

pub fn extract_reasoning_field_text(value: &Value) -> Option<String> {
    for key in ["reasoning_content", "reasoning"] {
        if let Some(text) = value.get(key).and_then(|v| v.as_str()) {
            if !text.is_empty() {
                return Some(text.to_string());
            }
        }
    }

    if let Some(reasoning) = value.get("reasoning") {
        for key in ["content", "text", "summary"] {
            if let Some(text) = reasoning.get(key).and_then(|v| v.as_str()) {
                if !text.is_empty() {
                    return Some(text.to_string());
                }
            }
        }
    }

    if let Some(details) = value.get("reasoning_details") {
        if let Some(text) = extract_reasoning_details_text(details) {
            return Some(text);
        }
    }

    None
}

fn extract_reasoning_details_text(value: &Value) -> Option<String> {
    match value {
        Value::String(text) => (!text.is_empty()).then(|| text.clone()),
        Value::Array(parts) => {
            let text = parts
                .iter()
                .filter_map(extract_reasoning_detail_part_text)
                .filter(|text| !text.is_empty())
                .collect::<Vec<_>>()
                .join("\n\n");
            (!text.is_empty()).then_some(text)
        }
        Value::Object(_) => extract_reasoning_detail_part_text(value),
        _ => None,
    }
}

fn extract_reasoning_detail_part_text(value: &Value) -> Option<String> {
    for key in ["text", "content", "summary"] {
        if let Some(text) = value.get(key).and_then(|v| v.as_str()) {
            if !text.is_empty() {
                return Some(text.to_string());
            }
        }
    }
    if let Some(parts) = value.get("parts").and_then(|v| v.as_array()) {
        let text = parts
            .iter()
            .filter_map(extract_reasoning_detail_part_text)
            .filter(|text| !text.is_empty())
            .collect::<Vec<_>>()
            .join("\n\n");
        return (!text.is_empty()).then_some(text);
    }
    None
}

pub fn extract_chat_delta_reasoning(delta: &Value) -> Option<String> {
    extract_reasoning_field_text(delta)
}

pub fn split_leading_think_block(text: &str) -> Option<(String, String)> {
    let leading_ws_len = text.len() - text.trim_start().len();
    let after_ws = &text[leading_ws_len..];
    if !after_ws.starts_with(THINK_OPEN_TAG) {
        return None;
    }

    let body_start = leading_ws_len + THINK_OPEN_TAG.len();
    let close_relative = text[body_start..].find(THINK_CLOSE_TAG)?;
    let close_start = body_start + close_relative;
    let answer_start = close_start + THINK_CLOSE_TAG.len();

    Some((
        text[body_start..close_start].trim().to_string(),
        strip_think_answer_separator(&text[answer_start..]).to_string(),
    ))
}

fn strip_leading_think_open_tag(text: &str) -> Option<String> {
    let leading_ws_len = text.len() - text.trim_start().len();
    let after_ws = &text[leading_ws_len..];
    after_ws
        .strip_prefix(THINK_OPEN_TAG)
        .map(|value| value.trim().to_string())
}

fn strip_think_answer_separator(text: &str) -> &str {
    text.trim_start_matches(['\r', '\n', '\t', ' '])
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn extracts_reasoning_content_from_delta() {
        let delta = json!({"reasoning_content": "Need context."});
        assert_eq!(
            extract_chat_delta_reasoning(&delta).as_deref(),
            Some("Need context.")
        );
    }

    #[test]
    fn extracts_reasoning_details_from_delta() {
        let delta = json!({"reasoning_details": [{"text": "step one"}]});
        assert_eq!(
            extract_chat_delta_reasoning(&delta).as_deref(),
            Some("step one")
        );
    }

    #[test]
    fn inline_think_splits_reasoning_and_answer() {
        let mut state = InlineThinkState::default();
        let mut out = state.push_content("<think>plan</think>Answer");
        out.extend(state.flush());
        assert_eq!(out.len(), 2);
        assert_eq!(out[0], StreamContentPiece::Reasoning("plan".into()));
        assert_eq!(out[1], StreamContentPiece::Text("Answer".into()));
    }
}
