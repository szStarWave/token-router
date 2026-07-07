mod anthropic;
mod codex_history;
mod diagnostics;
mod messages;
mod reasoning;
mod responses_input;

pub use anthropic::{
    apply_anthropic_upstream_options, inject_stream_include_usage,
    strip_leading_anthropic_billing_header, ANTHROPIC_REDACTED_THINKING_PLACEHOLDER,
};

pub use codex_history::CodexChatHistoryStore;
pub use diagnostics::{
    count_assistant_tool_calls_missing_reasoning, truncate_preview, upstream_error_message_hint,
};
pub use messages::{collapse_system_messages, finalize_chat_request_value, finalize_upstream_request};
pub use reasoning::{
    apply_reasoning_options, backfill_tool_call_reasoning_placeholders, infer_reasoning_config,
    model_requires_reasoning_replay, ChatReasoningConfig, TOOL_CALL_REASONING_PLACEHOLDER,
};
pub use responses_input::convert_responses_input_to_messages;

pub fn response_id_from_chat_id(id: Option<&str>) -> String {
    match id.filter(|s| !s.is_empty()) {
        Some(id) if id.starts_with("resp_") => id.to_string(),
        Some(id) => format!("resp_{id}"),
        None => "resp_unknown".into(),
    }
}
