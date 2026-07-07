use crate::gateway::api::openai::{ChatCompletionResponse, Usage};

/// Metrics from a single upstream HTTP call (edge or cloud).
#[derive(Debug, Clone)]
pub struct UpstreamCallMetrics {
    pub tier: &'static str,
    pub prompt_tokens: u32,
    pub completion_tokens: u32,
    pub cached_tokens: u32,
    pub latency_ms: u64,
    pub ttft_ms: Option<u64>,
    /// Last stream output chunk arrival (ms from start); stream-only.
    pub last_token_ms: Option<u64>,
    pub stream: bool,
}

/// Client-visible response served from edge — counts toward cloud token savings.
#[derive(Debug, Clone)]
pub struct FinalResponseMetrics {
    pub served_tier: &'static str,
    pub cloud_input_saved: u32,
    pub completion_tokens: u32,
}

/// Per-request stream output chunk counter (content or tool_calls deltas).
#[derive(Debug, Clone, Default)]
pub struct StreamChunkAccumulator {
    pub chunk_count: u32,
    pub first_token_ms: Option<u64>,
    pub last_token_ms: Option<u64>,
    usage: Option<(u32, u32, u32)>,
}

impl StreamChunkAccumulator {
    pub fn on_output_chunk(&mut self, elapsed_ms: u64) {
        self.chunk_count += 1;
        if self.first_token_ms.is_none() {
            self.first_token_ms = Some(elapsed_ms);
        }
        self.last_token_ms = Some(elapsed_ms);
    }

    pub fn resolve_completion(&self) -> u32 {
        self.usage
            .map(|(_, completion, _)| completion)
            .filter(|&c| c > 0)
            .unwrap_or(self.chunk_count)
    }

    pub fn resolve_prompt(&self, fallback: u32) -> u32 {
        self.usage
            .map(|(prompt, _, _)| prompt)
            .filter(|&p| p > 0)
            .unwrap_or(fallback)
    }

    pub fn cached_tokens(&self) -> u32 {
        self.usage.map(|(_, _, cached)| cached).unwrap_or(0)
    }
}

pub fn usage_triplet(usage: &Usage) -> (u32, u32, u32) {
    let cached = usage
        .prompt_tokens_details
        .as_ref()
        .map(|d| d.cached_tokens)
        .unwrap_or(0);
    (usage.prompt_tokens, usage.completion_tokens, cached)
}

pub fn tokens_from_response(resp: &ChatCompletionResponse, prompt_fallback: u32) -> (u32, u32, u32) {
    if let Some(usage) = &resp.usage {
        return usage_triplet(usage);
    }
    let completion = resp
        .choices
        .first()
        .and_then(|c| c.message.content.as_ref())
        .map(|t| estimate_tokens(t.len()))
        .unwrap_or(0);
    (prompt_fallback, completion, 0)
}

pub fn estimate_tokens(char_len: usize) -> u32 {
    ((char_len as f64) / 4.0).ceil() as u32
}

fn parse_usage_value(usage: &serde_json::Value) -> (u32, u32, u32) {
    let prompt = usage
        .get("prompt_tokens")
        .and_then(|v| v.as_u64())
        .unwrap_or(0) as u32;
    let completion = usage
        .get("completion_tokens")
        .and_then(|v| v.as_u64())
        .unwrap_or(0) as u32;
    let cached = usage
        .pointer("/prompt_tokens_details/cached_tokens")
        .and_then(|v| v.as_u64())
        .unwrap_or(0) as u32;
    (prompt, completion, cached)
}

fn delta_has_stream_output(delta: &serde_json::Value) -> bool {
    if delta
        .get("content")
        .and_then(|c| c.as_str())
        .is_some_and(|s| !s.is_empty())
    {
        return true;
    }
    delta
        .get("tool_calls")
        .and_then(|t| t.as_array())
        .is_some_and(|calls| !calls.is_empty())
}

/// Parse SSE bytes: count content/tool_calls output chunks and capture usage.
pub fn inspect_sse_bytes(bytes: &[u8], acc: &mut StreamChunkAccumulator, elapsed_ms: u64) {
    let Ok(text) = std::str::from_utf8(bytes) else {
        return;
    };
    for line in text.lines() {
        let Some(payload) = line.strip_prefix("data: ") else {
            continue;
        };
        let payload = payload.trim();
        if payload == "[DONE]" {
            continue;
        }
        let Ok(v) = serde_json::from_str::<serde_json::Value>(payload) else {
            continue;
        };
        if let Some(delta) = v.pointer("/choices/0/delta") {
            if delta_has_stream_output(delta) {
                acc.on_output_chunk(elapsed_ms);
            }
        }
        if let Some(usage) = v.get("usage") {
            acc.usage = Some(parse_usage_value(usage));
        }
    }
}

pub fn sse_has_output(bytes: &[u8]) -> bool {
    let Ok(text) = std::str::from_utf8(bytes) else {
        return false;
    };
    for line in text.lines() {
        let Some(payload) = line.strip_prefix("data: ") else {
            continue;
        };
        if payload.trim() == "[DONE]" {
            continue;
        }
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(payload.trim()) {
            if v.pointer("/choices/0/delta")
                .is_some_and(delta_has_stream_output)
            {
                return true;
            }
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_sse_usage_chunk() {
        let chunk = br#"data: {"usage":{"prompt_tokens":100,"completion_tokens":20,"prompt_tokens_details":{"cached_tokens":80}}}

"#;
        let mut acc = StreamChunkAccumulator::default();
        inspect_sse_bytes(chunk, &mut acc, 500);
        assert_eq!(acc.resolve_prompt(0), 100);
        assert_eq!(acc.resolve_completion(), 20);
        assert_eq!(acc.cached_tokens(), 80);
    }

    #[test]
    fn counts_content_delta_chunks() {
        let chunk = br#"data: {"choices":[{"delta":{"content":"hello"}}]}

"#;
        let mut acc = StreamChunkAccumulator::default();
        inspect_sse_bytes(chunk, &mut acc, 120);
        assert_eq!(acc.chunk_count, 1);
        assert_eq!(acc.first_token_ms, Some(120));
        assert_eq!(acc.last_token_ms, Some(120));
        assert_eq!(acc.resolve_completion(), 1);
    }

    #[test]
    fn counts_tool_call_delta_chunks() {
        let chunk = br#"data: {"choices":[{"delta":{"tool_calls":[{"index":0,"id":"call_1","function":{"name":"read","arguments":""}}]}}]}

"#;
        let mut acc = StreamChunkAccumulator::default();
        inspect_sse_bytes(chunk, &mut acc, 80);
        assert_eq!(acc.chunk_count, 1);
        assert_eq!(acc.first_token_ms, Some(80));
        assert_eq!(acc.resolve_completion(), 1);
        assert!(sse_has_output(chunk));
    }

    #[test]
    fn usage_completion_overrides_chunk_count() {
        let mut acc = StreamChunkAccumulator::default();
        inspect_sse_bytes(
            br#"data: {"choices":[{"delta":{"content":"a"}}]}

"#,
            &mut acc,
            10,
        );
        inspect_sse_bytes(
            br#"data: {"choices":[{"delta":{"content":"b"}}]}

"#,
            &mut acc,
            20,
        );
        inspect_sse_bytes(
            br#"data: {"usage":{"prompt_tokens":5,"completion_tokens":99}}

"#,
            &mut acc,
            30,
        );
        assert_eq!(acc.chunk_count, 2);
        assert_eq!(acc.resolve_completion(), 99);
    }

    #[test]
    fn multi_chunk_frame_counts_each_line() {
        let chunk = br#"data: {"choices":[{"delta":{"content":"a"}}]}

data: {"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"arguments":"{"}}]}}]}

"#;
        let mut acc = StreamChunkAccumulator::default();
        inspect_sse_bytes(chunk, &mut acc, 50);
        assert_eq!(acc.chunk_count, 2);
        assert_eq!(acc.first_token_ms, Some(50));
    }
}
