//! 上游配额头解析（v3 P5-B）。
//!
//! 支持三类来源（按优先级）：
//! 1. OpenAI 风格：`x-ratelimit-{limit,remaining,reset}-{requests,tokens,request,token}`；
//! 2. 通用：`x-ratelimit-limit` / `-remaining` / `-reset`；
//! 3. Anthropic：`anthropic-ratelimit-{tokens,requests}-{limit,remaining,reset}`。
//!
//! reset 支持三种编码：epoch 秒、`6m0s`/`1s` 形式的时长、RFC3339 时间戳。
//! 解析不出来字段留 `None`，绝不做猜测。

use tr_contracts::RateLimitSnapshot;

/// 从响应头解析配额观测；`status` 为 HTTP 状态码（429 时即使没有头也返回快照）。
pub fn parse_rate_limit(
    headers: &std::collections::BTreeMap<String, String>,
    status: u16,
    now_millis: i64,
) -> Option<RateLimitSnapshot> {
    let get = |name: &str| {
        headers
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.as_str())
    };

    // ── 剩余额度（按 token 优先，再 requests，再通用）────────────────────
    let (remaining, unit) = first_int(
        &[
            ("anthropic-ratelimit-tokens-remaining", "tokens"),
            ("x-ratelimit-remaining-tokens", "tokens"),
            ("x-ratelimit-remaining-requests", "requests"),
            ("anthropic-ratelimit-requests-remaining", "requests"),
            ("x-ratelimit-remaining", "unknown"),
        ],
        &get,
    )
    .map_or((None, "unknown"), |(value, unit)| (Some(value), unit));

    let (limit, _) = first_int(
        &[
            ("anthropic-ratelimit-tokens-limit", "tokens"),
            ("x-ratelimit-limit-tokens", "tokens"),
            ("x-ratelimit-limit-requests", "requests"),
            ("anthropic-ratelimit-requests-limit", "requests"),
            ("x-ratelimit-limit", "unknown"),
        ],
        &get,
    )
    .map_or((None, ""), |(value, _)| (Some(value), ""));

    let reset_at = first_reset(
        &[
            "anthropic-ratelimit-tokens-reset",
            "anthropic-ratelimit-requests-reset",
            "x-ratelimit-reset-tokens",
            "x-ratelimit-reset-requests",
            "x-ratelimit-reset",
            "retry-after",
        ],
        &get,
        now_millis,
    );

    let snapshot = RateLimitSnapshot {
        remaining,
        limit,
        reset_at,
        window_ms: reset_at.map(|reset| (reset - now_millis).max(0)),
        unit: unit.to_string(),
        from_429: status == 429,
    };
    if snapshot.is_empty() {
        None
    } else {
        Some(snapshot)
    }
}

fn first_int<'a>(
    candidates: &[(&str, &'a str)],
    get: &impl Fn(&str) -> Option<&'a str>,
) -> Option<(i64, &'a str)> {
    for (name, unit) in candidates {
        if let Some(raw) = get(name) {
            if let Ok(value) = raw.trim().parse::<i64>() {
                return Some((value, unit));
            }
        }
    }
    None
}

fn first_reset<'a>(
    candidates: &[&str],
    get: &impl Fn(&str) -> Option<&'a str>,
    now_millis: i64,
) -> Option<i64> {
    for name in candidates {
        let Some(raw) = get(name) else { continue };
        if let Some(reset) = parse_reset(raw, now_millis) {
            return Some(reset);
        }
    }
    None
}

fn parse_reset(raw: &str, now_millis: i64) -> Option<i64> {
    let raw = raw.trim();
    if raw.is_empty() {
        return None;
    }
    // epoch 秒（>= 1e9 视为绝对时间）。
    if let Ok(value) = raw.parse::<i64>() {
        if value >= 1_000_000_000 {
            return Some(value * 1000);
        }
        // 小整数：秒数（Retry-After 语义）。
        return Some(now_millis + value * 1000);
    }
    // 时长：`1s` / `6m0s` / `1h2m3s` / `500ms`。
    if let Some(duration_ms) = parse_duration_ms(raw) {
        return Some(now_millis + duration_ms);
    }
    // RFC3339（Anthropic）：`2026-11-17T12:34:56Z`，可带小数秒。
    parse_rfc3339_millis(raw)
}

fn parse_duration_ms(raw: &str) -> Option<i64> {
    // Go duration 风格子集；不含符号与小数点以外的复杂格式。
    let mut total = 0f64;
    let mut number = String::new();
    let mut saw_unit = false;
    let mut chars = raw.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch.is_ascii_digit() || ch == '.' {
            number.push(ch);
            continue;
        }
        let unit = if ch == 'm' {
            // `ms` 还是 `m`。
            if chars.peek() == Some(&'s') {
                chars.next();
                "ms"
            } else {
                "m"
            }
        } else {
            match ch {
                'h' => "h",
                's' => "s",
                _ => return None,
            }
        };
        let value: f64 = number.parse().ok()?;
        number.clear();
        saw_unit = true;
        total += match unit {
            "h" => value * 3_600_000.0,
            "m" => value * 60_000.0,
            "s" => value * 1000.0,
            "ms" => value,
            _ => return None,
        };
    }
    if !number.is_empty() || !saw_unit {
        return None;
    }
    Some(total as i64)
}

fn parse_rfc3339_millis(raw: &str) -> Option<i64> {
    // 只接受 `YYYY-MM-DDTHH:MM:SS[.fff](Z|±HH:MM)`。
    let bytes = raw.as_bytes();
    if bytes.len() < 20 {
        return None;
    }
    let year: i64 = raw.get(0..4)?.parse().ok()?;
    let month: i64 = raw.get(5..7)?.parse().ok()?;
    let day: i64 = raw.get(8..10)?.parse().ok()?;
    let hour: i64 = raw.get(11..13)?.parse().ok()?;
    let minute: i64 = raw.get(14..16)?.parse().ok()?;
    let second: i64 = raw.get(17..19)?.parse().ok()?;
    if bytes.get(4) != Some(&b'-') || bytes.get(7) != Some(&b'-') {
        return None;
    }
    if !matches!(bytes.get(10), Some(b'T') | Some(b't') | Some(b' ')) {
        return None;
    }
    if bytes.get(13) != Some(&b':') || bytes.get(16) != Some(&b':') {
        return None;
    }

    // 时区偏移。
    let mut offset_secs = 0i64;
    let tz_start = if bytes.get(19) == Some(&b'.') {
        raw.find(['Z', 'z', '+', '-']).filter(|index| *index > 19)?
    } else {
        19
    };
    match bytes.get(tz_start) {
        Some(b'Z') | Some(b'z') | None => {}
        Some(&sign) if sign == b'+' || sign == b'-' => {
            let rest = raw.get(tz_start + 1..)?;
            let (hh, mm) = rest.split_once(':')?;
            let offset =
                hh.trim().parse::<i64>().ok()? * 3600 + mm.trim().parse::<i64>().ok()? * 60;
            offset_secs = if sign == b'-' { -offset } else { offset };
        }
        _ => return None,
    }

    // civil → days since epoch（Howard Hinnant 算法）。
    let y = if month <= 2 { year - 1 } else { year };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = (month + 9) % 12;
    let doy = (153 * mp + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    let secs = days * 86_400 + hour * 3600 + minute * 60 + second - offset_secs;
    Some(secs * 1000)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    fn headers(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs
            .iter()
            .map(|(key, value)| (key.to_string(), value.to_string()))
            .collect()
    }

    const NOW: i64 = 1_700_000_000_000;

    #[test]
    fn openai_style_requests_window() {
        let parsed = parse_rate_limit(
            &headers(&[
                ("x-ratelimit-limit-requests", "10000"),
                ("x-ratelimit-remaining-requests", "999"),
                ("x-ratelimit-reset-requests", "6m0s"),
            ]),
            200,
            NOW,
        )
        .expect("snapshot");
        assert_eq!(parsed.remaining, Some(999));
        assert_eq!(parsed.limit, Some(10000));
        assert_eq!(parsed.unit, "requests");
        assert_eq!(parsed.reset_at, Some(NOW + 360_000));
        assert!(!parsed.from_429);
    }

    #[test]
    fn anthropic_style_tokens_window_rfc3339() {
        let parsed = parse_rate_limit(
            &headers(&[
                ("anthropic-ratelimit-tokens-limit", "400000"),
                ("anthropic-ratelimit-tokens-remaining", "250000"),
                ("anthropic-ratelimit-tokens-reset", "2026-11-17T12:00:00Z"),
            ]),
            200,
            NOW,
        )
        .expect("snapshot");
        assert_eq!(parsed.unit, "tokens");
        assert_eq!(parsed.remaining, Some(250_000));
        // 1970 epoch 基准校验：2026-11-17T12:00:00Z = 1794916800。
        assert_eq!(parsed.reset_at, Some(1_794_916_800_000));
    }

    #[test]
    fn bare_429_without_headers_still_reports() {
        let parsed = parse_rate_limit(&BTreeMap::new(), 429, NOW).expect("429 snapshot");
        assert!(parsed.from_429);
        assert_eq!(parsed.remaining, None);
    }

    #[test]
    fn no_headers_no_snapshot() {
        assert!(parse_rate_limit(&BTreeMap::new(), 200, NOW).is_none());
    }

    #[test]
    fn retry_after_seconds() {
        let parsed = parse_rate_limit(&headers(&[("retry-after", "30")]), 429, NOW).unwrap();
        assert_eq!(parsed.reset_at, Some(NOW + 30_000));
    }

    #[test]
    fn duration_formats() {
        assert_eq!(parse_duration_ms("1s"), Some(1000));
        assert_eq!(parse_duration_ms("500ms"), Some(500));
        assert_eq!(parse_duration_ms("1h2m3s"), Some(3_723_000));
        assert_eq!(parse_duration_ms("6m0s"), Some(360_000));
        assert_eq!(parse_duration_ms("abc"), None);
    }
}
