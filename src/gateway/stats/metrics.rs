use crate::gateway::api::codex_catalog::is_router_auto_model;
use crate::gateway::api::openai::{ChatCompletionResponse, Usage};

/// Metrics from a single upstream HTTP call (edge or cloud).
#[derive(Debug, Clone)]
pub struct UpstreamCallMetrics {
    pub tier: &'static str,
    pub model: String,
    pub prompt_tokens: u32,
    pub completion_tokens: u32,
    pub cached_tokens: u32,
    pub latency_ms: u64,
    pub ttft_ms: Option<u64>,
    /// Last stream output chunk arrival (ms from start); stream-only.
    pub last_token_ms: Option<u64>,
    /// Microsecond timings for accurate TPS; preferred over ms fields when set.
    pub latency_us: u64,
    pub ttft_us: Option<u64>,
    /// Last stream output chunk arrival (µs from start); stream-only.
    pub last_token_us: Option<u64>,
    pub stream: bool,
}

/// TPS divides by generation time; windows shorter than 1s are treated as 1s.
const MIN_GEN_US: u64 = 1_000_000;

fn effective_latency_us(m: &UpstreamCallMetrics) -> u64 {
    if m.latency_us > 0 {
        m.latency_us
    } else {
        m.latency_ms.saturating_mul(1000)
    }
}

fn effective_ttft_us(m: &UpstreamCallMetrics) -> Option<u64> {
    m.ttft_us.or_else(|| m.ttft_ms.map(|ms| ms.saturating_mul(1000)))
}

fn effective_last_token_us(m: &UpstreamCallMetrics) -> Option<u64> {
    m.last_token_us
        .or_else(|| m.last_token_ms.map(|ms| ms.saturating_mul(1000)))
}

/// Generation window (µs) used for tokens-per-second.
pub fn generation_duration_us(m: &UpstreamCallMetrics) -> u64 {
    let latency_us = effective_latency_us(m);
    let ttft_us = effective_ttft_us(m);
    let last_us = effective_last_token_us(m);

    let span_us = match (ttft_us, last_us) {
        (Some(first), Some(last)) if last > first => last - first,
        _ => 0,
    };

    let fallback_us = latency_us.saturating_sub(ttft_us.unwrap_or(0));

    let gen_us = if m.stream {
        if span_us > 0 {
            span_us
        } else if fallback_us > 0 {
            fallback_us
        } else {
            latency_us
        }
    } else if fallback_us > 0 {
        fallback_us
    } else {
        latency_us
    };

    gen_us.max(MIN_GEN_US)
}

/// Stored as value × 1000; display as `tps_x1000 / 1000.0` tok/s.
pub fn tps_x1000_from_metrics(m: &UpstreamCallMetrics) -> u64 {
    if m.completion_tokens == 0 {
        return 0;
    }
    let gen_us = generation_duration_us(m);
    (m.completion_tokens as u64)
        .saturating_mul(1_000_000_000)
        .saturating_div(gen_us)
}

pub fn normalize_upstream_model(model: &str) -> String {
    let trimmed = model.trim();
    if trimmed.is_empty() {
        "(unknown)".to_string()
    } else {
        trimmed.to_string()
    }
}

/// Prefer the model actually sent upstream; fall back to the response echo when forwarded is empty/auto.
pub fn effective_upstream_model(forwarded: &str, response: &str) -> String {
    let f = forwarded.trim();
    if !f.is_empty() && !is_router_auto_model(f) {
        return normalize_upstream_model(f);
    }
    normalize_upstream_model(response)
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
    pub first_token_us: Option<u64>,
    pub last_token_us: Option<u64>,
    usage: Option<(u32, u32, u32)>,
}

impl StreamChunkAccumulator {
    pub fn on_output_chunk_us(&mut self, elapsed_us: u64) {
        self.chunk_count += 1;
        if self.first_token_us.is_none() {
            self.first_token_us = Some(elapsed_us);
        }
        self.last_token_us = Some(elapsed_us);
        let elapsed_ms = elapsed_us / 1000;
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
/// `now_us` is called once per output chunk so each SSE line gets its own timestamp.
pub fn inspect_sse_bytes<F>(bytes: &[u8], acc: &mut StreamChunkAccumulator, now_us: &mut F)
where
    F: FnMut() -> u64,
{
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
        };
        let Ok(v) = serde_json::from_str::<serde_json::Value>(payload) else {
            continue;
        };
        if let Some(delta) = v.pointer("/choices/0/delta") {
            if delta_has_stream_output(delta) {
                acc.on_output_chunk_us(now_us());
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
        inspect_sse_bytes(chunk, &mut acc, &mut || 500_000);
        assert_eq!(acc.resolve_prompt(0), 100);
        assert_eq!(acc.resolve_completion(), 20);
        assert_eq!(acc.cached_tokens(), 80);
    }

    #[test]
    fn counts_content_delta_chunks() {
        let chunk = br#"data: {"choices":[{"delta":{"content":"hello"}}]}

"#;
        let mut acc = StreamChunkAccumulator::default();
        inspect_sse_bytes(chunk, &mut acc, &mut || 120_000);
        assert_eq!(acc.chunk_count, 1);
        assert_eq!(acc.first_token_ms, Some(120));
        assert_eq!(acc.last_token_ms, Some(120));
        assert_eq!(acc.first_token_us, Some(120_000));
        assert_eq!(acc.last_token_us, Some(120_000));
        assert_eq!(acc.resolve_completion(), 1);
    }

    #[test]
    fn counts_tool_call_delta_chunks() {
        let chunk = br#"data: {"choices":[{"delta":{"tool_calls":[{"index":0,"id":"call_1","function":{"name":"read","arguments":""}}]}}]}

"#;
        let mut acc = StreamChunkAccumulator::default();
        inspect_sse_bytes(chunk, &mut acc, &mut || 80_000);
        assert_eq!(acc.chunk_count, 1);
        assert_eq!(acc.first_token_ms, Some(80));
        assert_eq!(acc.first_token_us, Some(80_000));
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
            &mut || 10_000,
        );
        inspect_sse_bytes(
            br#"data: {"choices":[{"delta":{"content":"b"}}]}

"#,
            &mut acc,
            &mut || 20_000,
        );
        inspect_sse_bytes(
            br#"data: {"usage":{"prompt_tokens":5,"completion_tokens":99}}

"#,
            &mut acc,
            &mut || 30_000,
        );
        assert_eq!(acc.chunk_count, 2);
        assert_eq!(acc.resolve_completion(), 99);
        assert_eq!(acc.first_token_us, Some(10_000));
        assert_eq!(acc.last_token_us, Some(20_000));
    }

    #[test]
    fn effective_upstream_model_prefers_forwarded() {
        assert_eq!(
            effective_upstream_model("deepseek-v4-flash", "Pro/MiniMaxAI/MiniMax-M2.5"),
            "deepseek-v4-flash"
        );
    }

    #[test]
    fn effective_upstream_model_falls_back_to_response() {
        assert_eq!(effective_upstream_model("", "some-model"), "some-model");
        assert_eq!(effective_upstream_model("auto", "some-model"), "some-model");
    }

    #[test]
    fn multi_chunk_frame_counts_each_line_with_distinct_timestamps() {
        let chunk = br#"data: {"choices":[{"delta":{"content":"a"}}]}

data: {"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"arguments":"{"}}]}}]}

"#;
        let mut acc = StreamChunkAccumulator::default();
        let mut i = 0usize;
        let timestamps = [50_000u64, 75_000u64];
        inspect_sse_bytes(chunk, &mut acc, &mut || {
            let v = timestamps[i.min(timestamps.len() - 1)];
            i += 1;
            v
        });
        assert_eq!(acc.chunk_count, 2);
        assert_eq!(acc.first_token_us, Some(50_000));
        assert_eq!(acc.last_token_us, Some(75_000));
    }

    #[test]
    fn tps_not_inflated_when_stream_chunks_share_millisecond() {
        let m = UpstreamCallMetrics {
            tier: "edge",
            model: "test".to_string(),
            prompt_tokens: 10,
            completion_tokens: 952,
            cached_tokens: 0,
            latency_ms: 100,
            ttft_ms: Some(100),
            last_token_ms: Some(100),
            latency_us: 100_000,
            ttft_us: Some(100_000),
            last_token_us: Some(100_000),
            stream: true,
        };
        let tps = tps_x1000_from_metrics(&m) as f64 / 1000.0;
        assert!(
            tps < 2_000.0,
            "expected realistic TPS, got {tps}"
        );
        assert!((tps - 952.0).abs() < 1.0);
    }

    #[test]
    fn tps_uses_actual_duration_when_at_least_one_second() {
        let m = UpstreamCallMetrics {
            tier: "edge",
            model: "test".to_string(),
            prompt_tokens: 10,
            completion_tokens: 100,
            cached_tokens: 0,
            latency_ms: 2500,
            ttft_ms: Some(500),
            last_token_ms: Some(2500),
            latency_us: 2_500_000,
            ttft_us: Some(500_000),
            last_token_us: Some(2_500_000),
            stream: true,
        };
        let tps = tps_x1000_from_metrics(&m) as f64 / 1000.0;
        assert!((tps - 50.0).abs() < 1.0);
    }

    #[test]
    fn tps_uses_microsecond_span_between_chunks() {
        let m = UpstreamCallMetrics {
            tier: "edge",
            model: "test".to_string(),
            prompt_tokens: 10,
            completion_tokens: 100,
            cached_tokens: 0,
            latency_ms: 200,
            ttft_ms: Some(100),
            last_token_ms: Some(150),
            latency_us: 200_000,
            ttft_us: Some(100_000),
            last_token_us: Some(150_000),
            stream: true,
        };
        let tps = tps_x1000_from_metrics(&m) as f64 / 1000.0;
        assert!((tps - 100.0).abs() < 1.0);
    }

    #[test]
    fn tps_non_stream_submillisecond_uses_one_second_floor() {
        let m = UpstreamCallMetrics {
            tier: "cloud",
            model: "test".to_string(),
            prompt_tokens: 10,
            completion_tokens: 50,
            cached_tokens: 0,
            latency_ms: 0,
            ttft_ms: None,
            last_token_ms: None,
            latency_us: 500,
            ttft_us: None,
            last_token_us: None,
            stream: false,
        };
        let tps = tps_x1000_from_metrics(&m) as f64 / 1000.0;
        assert!((tps - 50.0).abs() < 1.0);
    }
}
