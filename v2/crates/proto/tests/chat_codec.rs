//! Chat 编解码往返测试（P0a-12）：fixture 必须无损。

use std::path::PathBuf;

use tr_proto::chat::{decode_request, decode_response, encode_request};
use tr_proto::fixture::load_dir;

fn fixtures_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/chat")
}

#[test]
fn every_chat_request_round_trips() {
    let fixtures = load_dir(&fixtures_dir()).expect("load chat fixtures");
    assert!(!fixtures.is_empty());

    for fixture in fixtures {
        let decoded = decode_request(&fixture.request)
            .unwrap_or_else(|err| panic!("{}: decode failed: {err}", fixture.id));
        let encoded = encode_request(&decoded);
        let decoded_again = decode_request(&encoded)
            .unwrap_or_else(|err| panic!("{}: re-decode failed: {err}", fixture.id));
        assert_eq!(
            decoded, decoded_again,
            "{}: canonical 往返不一致\nencoded={encoded}",
            fixture.id
        );
    }
}

#[test]
fn unknown_top_level_field_is_preserved() {
    let value = serde_json::json!({
        "model": "auto",
        "messages": [{"role": "user", "content": "hi"}],
        "vendor_extension": {"flag": true}
    });
    let request = decode_request(&value).expect("decode");
    assert_eq!(request.passthrough["vendor_extension"]["flag"], true);

    let encoded = encode_request(&request);
    assert_eq!(encoded["vendor_extension"]["flag"], true);
}

#[test]
fn tool_arguments_are_parsed_as_json() {
    let value = serde_json::json!({
        "model": "auto",
        "messages": [{
            "role": "assistant",
            "content": null,
            "tool_calls": [{
                "id": "call_1",
                "type": "function",
                "function": {"name": "read_file", "arguments": "{\"path\":\"a.txt\"}"}
            }]
        }]
    });
    let request = decode_request(&value).expect("decode");
    let block = &request.messages[0].blocks[0];
    match block {
        tr_contracts::ContentBlock::ToolUse { arguments, .. } => {
            assert_eq!(arguments["path"], "a.txt");
        }
        other => panic!("expected ToolUse, got {other:?}"),
    }
}

#[test]
fn response_round_trips() {
    let value = serde_json::json!({
        "id": "chatcmpl-1",
        "object": "chat.completion",
        "model": "deepseek-chat",
        "choices": [{
            "index": 0,
            "message": {"role": "assistant", "content": "hello"},
            "finish_reason": "stop"
        }],
        "usage": {"prompt_tokens": 7, "completion_tokens": 2, "total_tokens": 9}
    });
    let response = decode_response(&value).expect("decode response");
    assert_eq!(response.usage.input_tokens, 7);
    assert_eq!(response.model, "deepseek-chat");
}
