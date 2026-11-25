//! 供应商账户用量（余额 / 订阅）查询。
//!
//! 只查询**厂商自己公开的 usage 接口**，没有公开接口的厂商返回 `unsupported`，
//! 不猜测、不用本地账本冒充余额。每次查询按账号独立成败，失败只影响该账号。
//!
//! 已适配（字段以厂商文档为准）：
//! - DeepSeek：`GET {origin}/user/balance` → `balance_infos[]`
//! - Moonshot / Kimi：`GET {base}/users/me/balance` → `data.available_balance` 等
//! - OpenRouter：`GET {base}/credits`（余额）+ `GET {base}/key`（计划/键限额）

use std::time::Duration;

use futures_util::future::join_all;
use serde_json::Value;
use tr_agent::store;
use tr_contracts::{
    AccountInfo, AccountUsageInfo, BalanceInfo, ProviderInfo, ProviderUsageResponse,
    UsageLimitInfo, UsageStatus, UsageWindowInfo,
};

use crate::error::AdminApiError;
use crate::provider_service::read_credential_required;
use crate::state::ControlState;

/// 余额查询缓存 TTL：60s 内重复打开页面不重复打厂商接口。
const CACHE_TTL_MS: i64 = 60_000;
const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);

/// 支持的 usage 接口适配器。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum UsageAdapter {
    DeepSeek,
    Moonshot,
    OpenRouter,
    /// OpenCode Zen / Go 订阅用量：`{origin}/zen/go/v1/usage`。
    OpenCodeGo,
    /// OpenAI 旧版计费接口（new-api/one-api 等中转站普遍实现）：
    /// `{base}/dashboard/billing/{subscription,usage}`。
    OpenAiBilling,
}

/// 解析后的用量（与 HTTP 解耦，便于单测）。
#[derive(Debug, Clone, Default)]
struct ParsedUsage {
    plan: Option<String>,
    balances: Vec<BalanceInfo>,
    windows: Vec<UsageWindowInfo>,
    limits: Option<UsageLimitInfo>,
}

/// 一次查询的结论：拿到数据，或该厂商/该 key 没有可用接口。
enum FetchOutcome {
    Supported(ParsedUsage),
    Unsupported(String),
}

/// 查询全部账号的用量。`refresh = true` 跳过缓存。
pub async fn list_account_usage(
    state: &ControlState,
    refresh: bool,
) -> Result<ProviderUsageResponse, AdminApiError> {
    let db = state.pipeline.db();
    let providers = store::providers::list_providers(db)
        .map_err(|error| AdminApiError::internal(error.to_string()))?;
    let now = now_millis();
    let cache = state.usage_cache();
    let mut slots = Vec::new();

    for provider in providers {
        let template_id = store::providers::find_provider_template_id(db, &provider.id)
            .ok()
            .flatten();
        let adapter = adapter_for(template_id.as_deref(), &provider.base_url);
        let accounts = store::providers::list_accounts(db, &provider.id)
            .map_err(|error| AdminApiError::internal(error.to_string()))?;
        for account in accounts {
            let cached = if refresh {
                None
            } else {
                cache
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .get(&account.id)
                    .filter(|(fetched_at, _)| now - *fetched_at < CACHE_TTL_MS)
                    .map(|(_, info)| info.clone())
            };
            match cached {
                Some(info) => slots.push(Slot::Ready(info)),
                None => slots.push(Slot::Fetch {
                    provider: provider.clone(),
                    account,
                    adapter,
                }),
            }
        }
    }

    let data_dir = state.data_dir.clone();
    let accounts = join_all(slots.into_iter().map(|slot| {
        let data_dir = data_dir.clone();
        async move {
            match slot {
                Slot::Ready(info) => info,
                Slot::Fetch {
                    provider,
                    account,
                    adapter,
                } => fetch_account(data_dir.as_deref(), &provider, account, adapter).await,
            }
        }
    }))
    .await;

    if let Ok(mut guard) = cache.lock() {
        for info in &accounts {
            guard.insert(info.account_id.clone(), (now, info.clone()));
        }
    }
    Ok(ProviderUsageResponse { accounts })
}

enum Slot {
    /// 命中缓存，直接回放。
    Ready(AccountUsageInfo),
    /// 需要真实查询。
    Fetch {
        provider: ProviderInfo,
        account: AccountInfo,
        adapter: Option<UsageAdapter>,
    },
}

/// 模板 id 优先；自定义/导入的给主机名兜底；其余 http(s) 端点尝试通用的
/// OpenAI 旧版计费接口（很多中转站实现，404 会回到「未提供余额接口」）。
fn adapter_for(template_id: Option<&str>, base_url: &str) -> Option<UsageAdapter> {
    match template_id {
        Some("deepseek") => return Some(UsageAdapter::DeepSeek),
        Some("kimi") => return Some(UsageAdapter::Moonshot),
        Some("openrouter") => return Some(UsageAdapter::OpenRouter),
        Some("opencode") | Some("opencode-go") => return Some(UsageAdapter::OpenCodeGo),
        _ => {}
    }
    let url = reqwest::Url::parse(base_url).ok()?;
    if !matches!(url.scheme(), "http" | "https") {
        return None;
    }
    if let Some(host) = url.host_str().map(str::to_ascii_lowercase) {
        if host.contains("deepseek.com") {
            return Some(UsageAdapter::DeepSeek);
        }
        if host.contains("moonshot.cn") || host.contains("moonshot.ai") || host.contains("kimi.com")
        {
            return Some(UsageAdapter::Moonshot);
        }
        if host.contains("openrouter.ai") {
            return Some(UsageAdapter::OpenRouter);
        }
        if host.contains("opencode.ai") {
            return Some(UsageAdapter::OpenCodeGo);
        }
    }
    Some(UsageAdapter::OpenAiBilling)
}

async fn fetch_account(
    data_dir: Option<&std::path::Path>,
    provider: &ProviderInfo,
    account: AccountInfo,
    adapter: Option<UsageAdapter>,
) -> AccountUsageInfo {
    let empty = AccountUsageInfo {
        provider_id: provider.id.clone(),
        provider_name: provider.display_name.clone(),
        account_id: account.id.clone(),
        account_label: account.label.clone(),
        fingerprint: account.fingerprint.clone(),
        enabled: account.enabled,
        status: UsageStatus::Ok,
        plan: None,
        balances: Vec::new(),
        windows: Vec::new(),
        limits: None,
        fetched_at: now_millis(),
        error: None,
    };

    let Some(adapter) = adapter else {
        return AccountUsageInfo {
            status: UsageStatus::Unsupported,
            error: Some("base_url 不是 http(s) 端点".to_string()),
            ..empty
        };
    };

    let key = match read_credential_required(data_dir, &account) {
        Ok(key) if !key.trim().is_empty() => key,
        Ok(_) => return failed(empty, "缺少凭据".to_string()),
        Err(error) => return failed(empty, error),
    };

    match fetch(adapter, provider, &account, &key).await {
        Ok(FetchOutcome::Supported(parsed)) => AccountUsageInfo {
            plan: parsed.plan,
            balances: parsed.balances,
            windows: parsed.windows,
            limits: parsed.limits,
            ..empty
        },
        Ok(FetchOutcome::Unsupported(reason)) => AccountUsageInfo {
            status: UsageStatus::Unsupported,
            error: Some(reason),
            ..empty
        },
        Err(error) => failed(empty, error),
    }
}

fn failed(mut info: AccountUsageInfo, error: String) -> AccountUsageInfo {
    info.status = UsageStatus::Error;
    info.error = Some(error);
    info
}

async fn fetch(
    adapter: UsageAdapter,
    provider: &ProviderInfo,
    account: &AccountInfo,
    key: &str,
) -> Result<FetchOutcome, String> {
    let client = build_client(account)?;
    match adapter {
        UsageAdapter::DeepSeek => {
            let url = format!("{}/user/balance", origin(&provider.base_url)?);
            let value = get_json(&client, &url, key, account).await?;
            Ok(FetchOutcome::Supported(parse_deepseek(&value)?))
        }
        UsageAdapter::Moonshot => {
            let url = format!("{}/users/me/balance", v1_base(&provider.base_url));
            let value = get_json(&client, &url, key, account).await?;
            Ok(FetchOutcome::Supported(parse_moonshot(
                &value,
                &provider.base_url,
            )?))
        }
        UsageAdapter::OpenRouter => {
            let base = provider.base_url.trim_end_matches('/');
            let credits = get_json(&client, &format!("{base}/credits"), key, account).await?;
            let mut parsed = parse_openrouter_credits(&credits)?;
            // 计划/键限额是加分项：失败不影响余额结果。
            if let Ok(key_info) = get_json(&client, &format!("{base}/key"), key, account).await {
                apply_openrouter_key(&mut parsed, &key_info);
            }
            Ok(FetchOutcome::Supported(parsed))
        }
        UsageAdapter::OpenCodeGo => fetch_opencode_go(&client, provider, account, key).await,
        UsageAdapter::OpenAiBilling => fetch_openai_billing(&client, provider, account, key).await,
    }
}

/// OpenCode Zen / Go 订阅用量：`{origin}/zen/go/v1/usage`。
///
/// 返回 rolling / weekly / monthly 三个窗口的已用百分比与重置时间；
/// 非 OpenCode Go 订阅返回 403（OpenCode 自己也把 403 当作「不可用」）。
async fn fetch_opencode_go(
    client: &reqwest::Client,
    provider: &ProviderInfo,
    account: &AccountInfo,
    key: &str,
) -> Result<FetchOutcome, String> {
    let url = format!("{}/zen/go/v1/usage", origin(&provider.base_url)?);
    let (status, text) = get_text(client, &url, key, account).await?;
    match status.as_u16() {
        200..=299 => Ok(FetchOutcome::Supported(parse_opencode_usage(&text)?)),
        401 => Err("认证失败：OpenCode key 无效或已失效".to_string()),
        403 => Ok(FetchOutcome::Unsupported(
            "该 key 未开通 OpenCode Go（可能需要 x-opencode-org-id 请求头）".to_string(),
        )),
        404 => Ok(FetchOutcome::Unsupported(
            "厂商未提供 /zen/go/v1/usage 接口".to_string(),
        )),
        other => Err(format!("HTTP {other}")),
    }
}

/// OpenAI 旧版计费接口：`/dashboard/billing/subscription` + `/usage`。
///
/// 中转站（new-api/one-api）普遍实现；OpenAI 官方只对浏览器会话开放，
/// 普通 API key 会得到 401/403 —— 这两种情况都归为「该接口不可用」，不伪装成故障。
async fn fetch_openai_billing(
    client: &reqwest::Client,
    provider: &ProviderInfo,
    account: &AccountInfo,
    key: &str,
) -> Result<FetchOutcome, String> {
    let base = provider.base_url.trim_end_matches('/');
    let (status, text) = get_text(
        client,
        &format!("{base}/dashboard/billing/subscription"),
        key,
        account,
    )
    .await?;
    match status.as_u16() {
        200..=299 => {
            let value: Value = serde_json::from_str(&text)
                .map_err(|error| format!("subscription 响应不是合法 JSON: {error}"))?;
            let total = parse_billing_limit(&value)?;
            let (start, end) = billing_window(now_millis());
            let used = match get_text(
                client,
                &format!("{base}/dashboard/billing/usage?start_date={start}&end_date={end}"),
                key,
                account,
            )
            .await
            {
                Ok((usage_status, usage_text)) if (200..=299).contains(&usage_status.as_u16()) => {
                    serde_json::from_str::<Value>(&usage_text)
                        .ok()
                        .and_then(|value| parse_billing_usage(&value))
                }
                _ => None,
            };
            Ok(FetchOutcome::Supported(billing_balance(total, used)))
        }
        401 | 403 => Ok(FetchOutcome::Unsupported(
            "该接口不接受当前 key（可能需要管理员 key）".to_string(),
        )),
        404 => Ok(FetchOutcome::Unsupported(
            "厂商未提供 /dashboard/billing 接口".to_string(),
        )),
        other => Err(format!("HTTP {other}")),
    }
}

fn build_client(account: &AccountInfo) -> Result<reqwest::Client, String> {
    let mut builder = reqwest::Client::builder().timeout(REQUEST_TIMEOUT);
    if let Some(proxy) = &account.proxy_url {
        builder = builder
            .proxy(reqwest::Proxy::all(proxy).map_err(|error| format!("代理配置无效: {error}"))?);
    }
    builder
        .build()
        .map_err(|error| format!("HTTP client 构建失败: {error}"))
}

async fn get_json(
    client: &reqwest::Client,
    url: &str,
    key: &str,
    account: &AccountInfo,
) -> Result<Value, String> {
    let (status, text) = get_text(client, url, key, account).await?;
    if !status.is_success() {
        return Err(format!(
            "HTTP {}: {}",
            status.as_u16(),
            truncate(text.trim(), 160)
        ));
    }
    serde_json::from_str(&text).map_err(|error| format!("响应不是合法 JSON: {error}"))
}

/// GET 文本 + 状态码（计费接口需要按 401/404 区分「没有接口」与真故障）。
async fn get_text(
    client: &reqwest::Client,
    url: &str,
    key: &str,
    account: &AccountInfo,
) -> Result<(reqwest::StatusCode, String), String> {
    let mut request = client.get(url).bearer_auth(key);
    if let Some(headers) = &account.extra_headers {
        for header in headers {
            request = request.header(&header.name, &header.value);
        }
    }
    let response = request.send().await.map_err(|error| {
        if error.is_timeout() {
            "请求超时".to_string()
        } else {
            format!("请求失败: {error}")
        }
    })?;
    let status = response.status();
    let text = response.text().await.unwrap_or_default();
    Ok((status, text))
}

/// `https://host/v1` 不存在时补上 `/v1`（Moonshot 的 balance 在 v1 下）。
fn v1_base(base_url: &str) -> String {
    let base = base_url.trim_end_matches('/');
    if base.ends_with("/v1") {
        base.to_string()
    } else {
        format!("{base}/v1")
    }
}

fn origin(base_url: &str) -> Result<String, String> {
    let url = reqwest::Url::parse(base_url).map_err(|error| format!("base_url 无效: {error}"))?;
    Ok(url.origin().ascii_serialization())
}

fn money(value: Option<&Value>) -> Option<String> {
    match value {
        Some(Value::String(text)) if !text.trim().is_empty() => Some(text.trim().to_string()),
        Some(Value::Number(number)) => number.as_f64().map(trim_decimal),
        _ => None,
    }
}

/// 金额统一去掉无意义的尾零（`20.0` → `20`，`12.340000` → `12.34`）。
fn trim_decimal(value: f64) -> String {
    let mut text = format!("{value:.6}");
    while text.contains('.') && text.ends_with('0') {
        text.pop();
    }
    if text.ends_with('.') {
        text.pop();
    }
    text
}

fn decimal_sub(left: &str, right: &str) -> Option<String> {
    let left: f64 = left.parse().ok()?;
    let right: f64 = right.parse().ok()?;
    Some(trim_decimal(left - right))
}

fn truncate(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        text.to_string()
    } else {
        format!("{}…", text.chars().take(max).collect::<String>())
    }
}

fn parse_deepseek(value: &Value) -> Result<ParsedUsage, String> {
    let infos = value
        .get("balance_infos")
        .and_then(Value::as_array)
        .ok_or_else(|| "缺少 balance_infos".to_string())?;
    let balances: Vec<BalanceInfo> = infos
        .iter()
        .filter_map(|item| {
            let currency = item.get("currency").and_then(Value::as_str)?;
            Some(BalanceInfo {
                currency: currency.to_string(),
                available: money(item.get("total_balance")),
                used: None,
                total: None,
                granted: money(item.get("granted_balance")),
                topped_up: money(item.get("topped_up_balance")),
            })
        })
        .collect();
    if balances.is_empty() {
        return Err("balance_infos 为空".to_string());
    }
    Ok(ParsedUsage {
        balances,
        ..ParsedUsage::default()
    })
}

fn parse_moonshot(value: &Value, base_url: &str) -> Result<ParsedUsage, String> {
    let code = value.get("code").and_then(Value::as_i64).unwrap_or(-1);
    if code != 0 {
        let message = value
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or("code != 0");
        return Err(message.to_string());
    }
    let data = value.get("data").ok_or_else(|| "缺少 data".to_string())?;
    let currency = data
        .get("currency")
        .and_then(Value::as_str)
        .map(str::to_string)
        .unwrap_or_else(|| {
            if base_url.contains(".cn") {
                "CNY".to_string()
            } else {
                "USD".to_string()
            }
        });
    Ok(ParsedUsage {
        balances: vec![BalanceInfo {
            currency,
            available: money(data.get("available_balance")),
            used: None,
            total: None,
            granted: money(data.get("voucher_balance")),
            topped_up: money(data.get("cash_balance")),
        }],
        ..ParsedUsage::default()
    })
}

fn parse_openrouter_credits(value: &Value) -> Result<ParsedUsage, String> {
    let data = value.get("data").ok_or_else(|| "缺少 data".to_string())?;
    let total = money(data.get("total_credits"));
    let used = money(data.get("total_usage"));
    if total.is_none() && used.is_none() {
        return Err("credits 响应缺少余额字段".to_string());
    }
    let available = match (total.as_deref(), used.as_deref()) {
        (Some(total), Some(used)) => decimal_sub(total, used),
        _ => None,
    };
    Ok(ParsedUsage {
        balances: vec![BalanceInfo {
            currency: "USD".to_string(),
            available,
            used,
            total,
            granted: None,
            topped_up: None,
        }],
        ..ParsedUsage::default()
    })
}

fn apply_openrouter_key(parsed: &mut ParsedUsage, value: &Value) {
    let Some(data) = value.get("data") else {
        return;
    };
    if let Some(free) = data.get("is_free_tier").and_then(Value::as_bool) {
        parsed.plan = Some(if free {
            "free_tier".to_string()
        } else {
            "pay_as_you_go".to_string()
        });
    }
    let limit = money(data.get("limit"));
    let remaining = money(data.get("limit_remaining"));
    let used = money(data.get("usage"));
    if limit.is_some() || remaining.is_some() || used.is_some() {
        parsed.limits = Some(UsageLimitInfo {
            currency: "USD".to_string(),
            limit,
            remaining,
            used,
        });
    }
}

/// OpenCode Go：`{usage:{rolling,weekly,monthly}}`，每个窗口是
/// `{status:"ok"|"rate-limited", percent, resetsAt}`。
fn parse_opencode_usage(text: &str) -> Result<ParsedUsage, String> {
    let value: Value =
        serde_json::from_str(text).map_err(|error| format!("usage 响应不是合法 JSON: {error}"))?;
    let usage = value.get("usage").ok_or_else(|| "缺少 usage".to_string())?;
    let mut windows = Vec::new();
    for label in ["rolling", "weekly", "monthly"] {
        let Some(item) = usage.get(label) else {
            continue;
        };
        windows.push(UsageWindowInfo {
            label: label.to_string(),
            percent: item.get("percent").and_then(Value::as_f64).unwrap_or(0.0),
            limited: item.get("status").and_then(Value::as_str) == Some("rate-limited"),
            resets_at: item.get("resetsAt").and_then(epoch_millis),
        });
    }
    if windows.is_empty() {
        return Err("usage 响应没有窗口数据".to_string());
    }
    Ok(ParsedUsage {
        plan: Some("opencode_go".to_string()),
        windows,
        ..ParsedUsage::default()
    })
}

fn epoch_millis(value: &Value) -> Option<i64> {
    match value {
        Value::Number(number) => {
            let raw = number.as_i64()?;
            Some(if raw >= 100_000_000_000 {
                raw
            } else {
                raw * 1_000
            })
        }
        Value::String(text) => parse_iso_millis(text),
        _ => None,
    }
}

/// RFC3339 → Unix 毫秒（只覆盖 OpenCode 返回的 `YYYY-MM-DDTHH:MM:SS[.fff][Z|±HH:MM]`）。
fn parse_iso_millis(text: &str) -> Option<i64> {
    let text = text.trim();
    if text.len() < 19 {
        return None;
    }
    let year: i64 = text.get(0..4)?.parse().ok()?;
    let month: i64 = text.get(5..7)?.parse().ok()?;
    let day: i64 = text.get(8..10)?.parse().ok()?;
    let hour: i64 = text.get(11..13)?.parse().ok()?;
    let minute: i64 = text.get(14..16)?.parse().ok()?;
    let second: i64 = text.get(17..19)?.parse().ok()?;
    let mut millis =
        (days_from_civil(year, month, day) * 86_400 + hour * 3_600 + minute * 60 + second) * 1_000;

    let mut rest = &text[19..];
    if let Some(fraction) = rest.strip_prefix('.') {
        let digits: String = fraction
            .chars()
            .take_while(|ch| ch.is_ascii_digit())
            .collect();
        if !digits.is_empty() {
            let mut ms: i64 = digits.parse().ok()?;
            for _ in digits.len()..3 {
                ms *= 10;
            }
            millis += ms;
        }
        rest = &fraction[digits.len()..];
    }
    if let Some(offset) = rest.strip_prefix('+') {
        millis -= offset_millis(offset)?;
    } else if let Some(offset) = rest.strip_prefix('-') {
        millis += offset_millis(offset)?;
    }
    Some(millis)
}

fn offset_millis(text: &str) -> Option<i64> {
    let (hours, minutes) = text.split_once(':')?;
    let hours: i64 = hours.parse().ok()?;
    let minutes: i64 = minutes.parse().ok()?;
    Some((hours * 60 + minutes) * 60_000)
}

/// 公历日期 → 1970-01-01 起的天数（Hinnant civil_from_days 的逆运算）。
fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let year = year - i64::from(month <= 2);
    let era = if year >= 0 { year } else { year - 399 } / 400;
    let yoe = year - era * 400;
    let mp = (month + 9) % 12;
    let doy = (153 * mp + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

fn parse_billing_limit(value: &Value) -> Result<String, String> {
    for key in ["system_hard_limit_usd", "hard_limit_usd", "soft_limit_usd"] {
        if let Some(text) = money(value.get(key)) {
            return Ok(text);
        }
    }
    Err("subscription 响应缺少额度字段".to_string())
}

/// 旧版 usage 接口的 `total_usage` 是美分。
fn parse_billing_usage(value: &Value) -> Option<String> {
    let cents = match value.get("total_usage") {
        Some(Value::Number(number)) => number.as_f64(),
        Some(Value::String(text)) => text.trim().parse::<f64>().ok(),
        _ => None,
    }?;
    Some(format_cents(cents))
}

fn format_cents(cents: f64) -> String {
    trim_decimal(cents / 100.0)
}

fn billing_balance(total: String, used: Option<String>) -> ParsedUsage {
    let available = used.as_deref().and_then(|used| decimal_sub(&total, used));
    ParsedUsage {
        balances: vec![BalanceInfo {
            currency: "USD".to_string(),
            available,
            used,
            total: Some(total),
            granted: None,
            topped_up: None,
        }],
        ..ParsedUsage::default()
    }
}

/// 计费接口的日期窗口：本月 1 日 → 今天（UTC）。
fn billing_window(now_millis: i64) -> (String, String) {
    let (year, month, day) = utc_date(now_millis);
    (
        format!("{year:04}-{month:02}-01"),
        format!("{year:04}-{month:02}-{day:02}"),
    )
}

/// 毫秒时间戳 → UTC 年月日（civil_from_days，不引入 chrono）。
fn utc_date(now_millis: i64) -> (i64, i64, i64) {
    let z = now_millis.div_euclid(86_400_000) + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let mut year = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    if month <= 2 {
        year += 1;
    }
    (year, month, day)
}

fn now_millis() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn amount(value: &str) -> Option<String> {
        Some(value.to_string())
    }

    #[test]
    fn deepseek_balances_and_components() {
        let parsed = parse_deepseek(&json!({
            "is_available": true,
            "balance_infos": [{
                "currency": "CNY",
                "total_balance": "110.00",
                "granted_balance": "10.00",
                "topped_up_balance": "100.00"
            }]
        }))
        .expect("parse");
        assert_eq!(parsed.balances.len(), 1);
        assert_eq!(parsed.balances[0].currency, "CNY");
        assert_eq!(parsed.balances[0].available, amount("110.00"));
        assert_eq!(parsed.balances[0].granted, amount("10.00"));
        assert_eq!(parsed.balances[0].topped_up, amount("100.00"));
        assert!(parsed.plan.is_none());
    }

    #[test]
    fn moonshot_balance_and_error_code() {
        let parsed = parse_moonshot(
            &json!({
                "code": 0,
                "data": {
                    "available_balance": 12.5,
                    "voucher_balance": 2.5,
                    "cash_balance": 10
                }
            }),
            "https://api.moonshot.cn/v1",
        )
        .expect("parse");
        assert_eq!(parsed.balances[0].currency, "CNY");
        assert_eq!(parsed.balances[0].available, amount("12.5"));
        assert_eq!(parsed.balances[0].granted, amount("2.5"));
        assert_eq!(parsed.balances[0].topped_up, amount("10"));

        let error = parse_moonshot(
            &json!({"code": 401, "message": "bad key"}),
            "https://x.cn/v1",
        )
        .expect_err("code must fail");
        assert_eq!(error, "bad key");
    }

    #[test]
    fn openrouter_credits_derive_available_and_plan() {
        let mut parsed = parse_openrouter_credits(&json!({
            "data": {"total_credits": "10.5", "total_usage": "1.25"}
        }))
        .expect("parse");
        assert_eq!(parsed.balances[0].currency, "USD");
        assert_eq!(parsed.balances[0].available, Some("9.25".to_string()));
        assert_eq!(parsed.balances[0].used, Some("1.25".to_string()));
        assert_eq!(parsed.balances[0].total, Some("10.5".to_string()));

        apply_openrouter_key(
            &mut parsed,
            &json!({"data": {"is_free_tier": true, "limit": 5, "limit_remaining": 3, "usage": 2}}),
        );
        assert_eq!(parsed.plan.as_deref(), Some("free_tier"));
        let limits = parsed.limits.expect("limits");
        assert_eq!(limits.limit, Some("5".to_string()));
        assert_eq!(limits.remaining, Some("3".to_string()));
        assert_eq!(limits.used, Some("2".to_string()));
    }

    #[test]
    fn adapter_follows_template_then_host() {
        assert_eq!(
            adapter_for(Some("deepseek"), "https://proxy.example/v1"),
            Some(UsageAdapter::DeepSeek)
        );
        assert_eq!(
            adapter_for(None, "https://api.moonshot.cn/v1"),
            Some(UsageAdapter::Moonshot)
        );
        assert_eq!(
            adapter_for(None, "https://opencode.ai/zen/v1"),
            Some(UsageAdapter::OpenCodeGo)
        );
        assert_eq!(
            adapter_for(None, "https://api.siliconflow.cn/v1"),
            Some(UsageAdapter::OpenAiBilling),
            "未知厂商先探测通用计费接口，404 再回落为「未提供」"
        );
        assert_eq!(
            adapter_for(None, "file:///tmp/base"),
            None,
            "非 http(s) 端点不探测"
        );
    }

    #[test]
    fn billing_parses_limit_usage_and_window() {
        let total = parse_billing_limit(&json!({"hard_limit_usd": 20})).expect("limit");
        assert_eq!(total, "20");
        let used = parse_billing_usage(&json!({"total_usage": 1234})).expect("usage");
        assert_eq!(used, "12.34");
        let parsed = billing_balance(total, Some(used));
        assert_eq!(parsed.balances[0].currency, "USD");
        assert_eq!(parsed.balances[0].total.as_deref(), Some("20"));
        assert_eq!(parsed.balances[0].used.as_deref(), Some("12.34"));
        assert_eq!(parsed.balances[0].available.as_deref(), Some("7.66"));

        assert_eq!(
            billing_window(0),
            ("1970-01-01".to_string(), "1970-01-01".to_string())
        );
    }

    #[test]
    fn opencode_go_windows_are_parsed() {
        let parsed = parse_opencode_usage(
            &json!({
                "usage": {
                    "rolling": {
                        "status": "ok",
                        "percent": 12.5,
                        "resetsAt": "2026-11-23T10:00:00Z"
                    },
                    "weekly": {
                        "status": "rate-limited",
                        "percent": 100,
                        "resetsAt": 1_795_420_000_000i64
                    },
                    "monthly": {
                        "status": "ok",
                        "percent": 40,
                        "resetsAt": "2026-12-01T00:00:00+08:00"
                    }
                }
            })
            .to_string(),
        )
        .expect("parse");

        assert_eq!(parsed.plan.as_deref(), Some("opencode_go"));
        assert_eq!(parsed.windows.len(), 3);
        assert_eq!(parsed.windows[0].label, "rolling");
        assert_eq!(parsed.windows[0].percent, 12.5);
        assert!(parsed.windows[0].resets_at.is_some());
        assert_eq!(parsed.windows[1].label, "weekly");
        assert!(parsed.windows[1].limited);
        assert_eq!(parsed.windows[1].percent, 100.0);
        assert_eq!(parsed.windows[2].label, "monthly");
        assert!(!parsed.windows[2].limited);

        assert_eq!(
            parse_iso_millis("2024-01-01T00:00:00Z"),
            Some(1_704_067_200_000)
        );
        assert_eq!(
            parse_iso_millis("2024-01-01T08:00:00+08:00"),
            Some(1_704_067_200_000)
        );
    }

    #[test]
    fn endpoint_shapes() {
        assert_eq!(
            origin("https://api.deepseek.com/v1").expect("origin"),
            "https://api.deepseek.com"
        );
        assert_eq!(
            v1_base("https://api.moonshot.cn/v1"),
            "https://api.moonshot.cn/v1"
        );
        assert_eq!(
            v1_base("https://api.moonshot.cn"),
            "https://api.moonshot.cn/v1"
        );
    }
}
