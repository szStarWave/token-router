//! 三协议往返测试（P0b-8/5）：fixture 请求必须无损进入 canonical 并编码回去。

use std::path::PathBuf;

use tr_proto::fixture::load_dir;
use tr_proto::{anthropic, chat, responses};

fn fixtures_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures")
}

#[test]
fn every_fixture_request_round_trips() {
    let fixtures = load_dir(&fixtures_dir()).expect("load fixtures");
    let mut failures = Vec::new();

    for fixture in fixtures {
        let protocol = fixture.protocol_kind().expect("protocol");
        let decoded = match protocol {
            tr_contracts::IngressProtocol::ChatCompletions => {
                chat::decode_request(&fixture.request)
            }
            tr_contracts::IngressProtocol::AnthropicMessages => {
                anthropic::decode_request(&fixture.request)
            }
            tr_contracts::IngressProtocol::Responses => responses::decode_request(&fixture.request),
        }
        .unwrap_or_else(|err| panic!("{}: decode failed: {err}", fixture.id));

        let encoded = match protocol {
            tr_contracts::IngressProtocol::ChatCompletions => chat::encode_request(&decoded),
            tr_contracts::IngressProtocol::AnthropicMessages => anthropic::encode_request(&decoded),
            tr_contracts::IngressProtocol::Responses => responses::encode_request(&decoded),
        };

        let decoded_again = match protocol {
            tr_contracts::IngressProtocol::ChatCompletions => chat::decode_request(&encoded),
            tr_contracts::IngressProtocol::AnthropicMessages => anthropic::decode_request(&encoded),
            tr_contracts::IngressProtocol::Responses => responses::decode_request(&encoded),
        }
        .unwrap_or_else(|err| panic!("{}: re-decode failed: {err}", fixture.id));

        if decoded != decoded_again {
            failures.push(fixture.id.clone());
        }
    }

    assert!(failures.is_empty(), "往返不一致的 fixture: {failures:?}");
}

#[test]
fn anthropic_response_keeps_redacted_thinking() {
    let value = serde_json::json!({
        "id": "msg_1",
        "type": "message",
        "role": "assistant",
        "model": "claude-sonnet-4-5",
        "content": [
            {"type": "redacted_thinking", "data": "opaque"},
            {"type": "text", "text": "done"}
        ],
        "stop_reason": "end_turn",
        "usage": {
            "input_tokens": 10,
            "output_tokens": 3,
            "cache_read_input_tokens": 4,
            "cache_creation_input_tokens": 0
        }
    });

    let response = anthropic::decode_response(&value).expect("decode");
    match &response.blocks[0] {
        tr_contracts::ContentBlock::Reasoning {
            signature,
            redacted,
            ..
        } => {
            assert_eq!(signature.as_deref(), Some("opaque"));
            assert!(redacted);
        }
        other => panic!("expected Reasoning, got {other:?}"),
    }
    assert_eq!(response.usage.cached_tokens, Some(4));
}

#[test]
fn responses_response_maps_function_call_to_tool_use() {
    let value = serde_json::json!({
        "id": "resp_1",
        "object": "response",
        "status": "completed",
        "model": "gpt-5-codex",
        "output": [{
            "type": "function_call",
            "id": "fc_1",
            "call_id": "call_1",
            "name": "read_file",
            "arguments": "{\"path\":\"a.txt\"}",
            "status": "completed"
        }],
        "usage": {
            "input_tokens": 30,
            "output_tokens": 8,
            "input_tokens_details": {"cached_tokens": 16}
        }
    });

    let response = responses::decode_response(&value).expect("decode");
    assert_eq!(response.finish_reason, tr_contracts::FinishReason::ToolUse);
    assert_eq!(response.usage.cached_tokens, Some(16));
    match &response.blocks[0] {
        tr_contracts::ContentBlock::ToolUse {
            id,
            name,
            arguments,
        } => {
            assert_eq!(id, "call_1");
            assert_eq!(name, "read_file");
            assert_eq!(arguments["path"], "a.txt");
        }
        other => panic!("expected ToolUse, got {other:?}"),
    }
}
