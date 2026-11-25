//! 上游执行：按候选的 `provider` 选 wire 协议（Chat Completions / Responses /
//! Anthropic Messages），非流式与流式共用同一套端点、鉴权、代理与自定义 header。
//!
//! 边界：
//! - 只在这里产生 `ErrorClass`（经 `tr_proto::errors::classify`）；
//! - 只在这里解析 `credential` 引用（`env:` / `file:` / `keyring:`）；
//! - 不改健康度、不写账本——那是 `agent` 的职责。
//!
//! 未知 provider 显式返回 `UnsupportedProvider`，不静默降级成错误的上游调用。

use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use reqwest::Client;
use serde_json::{Value, json};
use tr_contracts::{
    Candidate, CandidateId, CanonicalRequest, CanonicalResponse, CredentialRef, ErrorClass,
    IngressProtocol, ProviderKind,
};
use tr_proto::{anthropic, chat, responses};

/// 失败原因（审计用，不参与路由算术）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AttemptCause {
    Transport,
    HttpStatus,
    Credential,
    UnsupportedProvider,
    MalformedResponse,
}

/// 一次 attempt 的失败。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AttemptFailure {
    pub class: ErrorClass,
    pub status: Option<u16>,
    pub message: Option<String>,
    pub retry_after_ms: Option<u64>,
    pub cause: AttemptCause,
    /// 配额观测（429 或带限流头）；无观测为 `None`。装箱避免 Err 变大。
    pub rate_limit: Option<Box<tr_contracts::RateLimitSnapshot>>,
}

/// 一次 attempt 的结果。
#[derive(Debug, Clone)]
pub struct AttemptOutcome {
    pub candidate_id: CandidateId,
    pub response: Option<CanonicalResponse>,
    pub failure: Option<AttemptFailure>,
    pub http_status: Option<u16>,
    pub duration_ms: u64,
    pub ttft_ms: Option<u64>,
    /// 成功响应的限流头观测（失败时在 `failure.rate_limit`）。
    pub rate_limit: Option<Box<tr_contracts::RateLimitSnapshot>>,
}

impl AttemptOutcome {
    pub fn is_success(&self) -> bool {
        self.response.is_some()
    }
}

/// HTTP 上游客户端。
#[derive(Debug, Clone)]
pub struct HttpUpstream {
    pub(crate) client: Client,
    /// 单次请求超时（每 key 代理时按候选重建 client 需要）。
    pub(crate) timeout: Duration,
}

fn now_millis() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as i64)
        .unwrap_or(0)
}

/// 默认客户端：120s 单次请求超时（P0a 非流式）。
///
/// 构建失败只可能是 TLS 后端初始化失败，属于进程无法自愈的错误，直接 panic
/// 好过让调用方拿到一个“永远失败”的客户端。
impl Default for HttpUpstream {
    fn default() -> Self {
        Self::new(Duration::from_secs(120)).expect("build reqwest client")
    }
}

impl HttpUpstream {
    pub fn new(timeout: Duration) -> Result<Self, reqwest::Error> {
        let client = Client::builder().timeout(timeout).build()?;
        Ok(Self { client, timeout })
    }

    /// 构造发往该候选的 POST：协议端点 + 自定义 header + 协议鉴权 + 代理。
    ///
    /// 非流式与流式共用，避免两条路径的 header/代理行为漂移。
    pub(crate) fn post_for(
        &self,
        candidate: &Candidate,
    ) -> Result<reqwest::RequestBuilder, AttemptFailure> {
        let url = endpoint(&candidate.upstream.provider, &candidate.upstream.base_url);
        // 每 key 代理（tags 的 `proxy_url`）：有代理时按请求换 client
        // （共享 client 无法按请求换代理）。
        let mut builder = match candidate.capability.tags.get("proxy_url") {
            Some(proxy_url) => {
                let proxy = reqwest::Proxy::all(proxy_url)
                    .map_err(|error| credential_failure(format!("代理配置无效: {error}")))?;
                let client = Client::builder()
                    .timeout(self.timeout)
                    .proxy(proxy)
                    .build()
                    .map_err(|error| {
                        credential_failure(format!("代理 client 构建失败: {error}"))
                    })?;
                client.post(&url)
            }
            None => self.client.post(&url),
        };

        // 静态自定义 header（tags 的 `header:<name>`），先于鉴权，便于覆盖默认值。
        for (key, value) in &candidate.capability.tags {
            if let Some(name) = key.strip_prefix("header:") {
                builder = builder.header(name, value);
            }
        }

        apply_auth(
            builder,
            &candidate.upstream.provider,
            candidate.upstream.credential.as_ref(),
        )
    }

    /// 执行一次非流式请求。
    pub async fn execute(
        &self,
        candidate: &Candidate,
        request: &CanonicalRequest,
    ) -> AttemptOutcome {
        let started = Instant::now();
        match self.try_execute(candidate, request).await {
            Ok((response, rate_limit)) => AttemptOutcome {
                candidate_id: candidate.id.clone(),
                http_status: Some(200),
                response: Some(response),
                failure: None,
                duration_ms: started.elapsed().as_millis() as u64,
                ttft_ms: None,
                rate_limit,
            },
            Err((failure, status)) => AttemptOutcome {
                candidate_id: candidate.id.clone(),
                rate_limit: failure.rate_limit.clone(),
                response: None,
                failure: Some(failure),
                http_status: status,
                duration_ms: started.elapsed().as_millis() as u64,
                ttft_ms: None,
            },
        }
    }

    async fn try_execute(
        &self,
        candidate: &Candidate,
        request: &CanonicalRequest,
    ) -> Result<
        (
            CanonicalResponse,
            Option<Box<tr_contracts::RateLimitSnapshot>>,
        ),
        (AttemptFailure, Option<u16>),
    > {
        let protocol = wire_protocol(&candidate.upstream.provider).ok_or_else(|| {
            (
                unsupported_provider(&candidate.upstream.provider, "非流式"),
                None,
            )
        })?;

        let wire = physical_request(candidate, request);
        let mut body = encode_wire(protocol, &wire);
        apply_thinking_disable(candidate, request, &mut body);

        let response = self
            .post_for(candidate)
            .map_err(|failure| (failure, None))?
            .json(&body)
            .send()
            .await
            .map_err(|error| {
                let class = if error.is_timeout() {
                    ErrorClass::Timeout
                } else {
                    ErrorClass::Transport
                };
                (
                    AttemptFailure {
                        class,
                        status: None,
                        message: Some(error.to_string()),
                        retry_after_ms: None,
                        cause: AttemptCause::Transport,
                        rate_limit: None,
                    },
                    None,
                )
            })?;

        let status = response.status();
        let headers: BTreeMap<String, String> = response
            .headers()
            .iter()
            .filter_map(|(name, value)| {
                value
                    .to_str()
                    .ok()
                    .map(|value| (name.as_str().to_string(), value.to_string()))
            })
            .collect();
        let text = response.text().await.map_err(|error| {
            (
                AttemptFailure {
                    class: ErrorClass::Transport,
                    status: Some(status.as_u16()),
                    message: Some(error.to_string()),
                    retry_after_ms: None,
                    cause: AttemptCause::Transport,
                    rate_limit: None,
                },
                Some(status.as_u16()),
            )
        })?;

        if !status.is_success() {
            let body: Value = serde_json::from_str(&text)
                .unwrap_or_else(|_| serde_json::json!({"error": {"message": text}}));
            let classified = tr_proto::errors::classify(status.as_u16(), &headers, &body);
            let rate_limit =
                crate::rate_limit::parse_rate_limit(&headers, status.as_u16(), now_millis())
                    .map(Box::new);
            return Err((
                AttemptFailure {
                    class: classified.class,
                    status: Some(status.as_u16()),
                    message: classified.message,
                    retry_after_ms: classified.retry_after_ms,
                    cause: AttemptCause::HttpStatus,
                    rate_limit,
                },
                Some(status.as_u16()),
            ));
        }
        let rate_limit =
            crate::rate_limit::parse_rate_limit(&headers, status.as_u16(), now_millis())
                .map(Box::new);

        let value: Value = serde_json::from_str(&text).map_err(|error| {
            (
                AttemptFailure {
                    class: ErrorClass::Protocol,
                    status: Some(status.as_u16()),
                    message: Some(error.to_string()),
                    retry_after_ms: None,
                    cause: AttemptCause::MalformedResponse,
                    rate_limit: None,
                },
                Some(status.as_u16()),
            )
        })?;

        decode_wire(protocol, &value)
            .map(|response| (response, rate_limit))
            .map_err(|error| {
                (
                    AttemptFailure {
                        class: ErrorClass::Protocol,
                        status: Some(status.as_u16()),
                        message: Some(error.to_string()),
                        retry_after_ms: None,
                        cause: AttemptCause::MalformedResponse,
                        rate_limit: None,
                    },
                    Some(status.as_u16()),
                )
            })
    }
}

/// 把虚拟模型替换成物理模型，并强制非流式（P0a）。
///
/// 同时按候选声明的推理档位收口 reasoning：候选声明了 `thinking_levels`
/// 但请求的档位不在其中时，不把 reasoning 发给上游（等价 off）。
/// 空声明 = 未声明，原样放行（不猜测能力）。
pub fn physical_request(candidate: &Candidate, request: &CanonicalRequest) -> CanonicalRequest {
    let mut wire = request.clone();
    wire.meta.virtual_model = Some(candidate.upstream.model.clone());
    wire.output.stream = false;
    if !reasoning_supported(candidate, &wire) {
        wire.reasoning = None;
    }
    // 候选级输出上限：客户端值更小就保留，未给则填上限（截断证据由 agent 记账）。
    if let Some(limit) = candidate_max_tokens(candidate) {
        wire.output.max_tokens = Some(match wire.output.max_tokens {
            Some(requested) => requested.min(limit),
            None => limit,
        });
    }
    wire
}

/// 候选级输出上限（编译期写入 `capability.tags`）。
pub fn candidate_max_tokens(candidate: &Candidate) -> Option<u32> {
    candidate
        .capability
        .tags
        .get("max_tokens")
        .and_then(|value| value.parse::<u32>().ok())
        .filter(|value| *value > 0)
}

/// 候选级关闭思考（编译期写入 `capability.tags`）。
pub(crate) fn candidate_thinking_disabled(candidate: &Candidate) -> bool {
    candidate
        .capability
        .tags
        .get("disable_thinking")
        .is_some_and(|value| value == "1" || value.eq_ignore_ascii_case("true"))
}

fn reasoning_supported(candidate: &Candidate, request: &CanonicalRequest) -> bool {
    // 显式关闭（组级 `disable_thinking` / 客户端 off / 候选级关闭）都不发推理参数。
    if thinking_disabled(request) || candidate_thinking_disabled(candidate) {
        return false;
    }
    let declared = &candidate.capability.thinking_levels;
    if declared.is_empty() {
        return true;
    }
    let requested = request.meta.thinking_level.as_deref().or_else(|| {
        request
            .reasoning
            .as_ref()
            .and_then(|params| params.effort.as_deref())
    });
    let Some(requested) = requested.filter(|level| !level.is_empty()) else {
        return true;
    };
    declared
        .iter()
        .any(|level| level.eq_ignore_ascii_case(requested))
}

/// 显式关闭思考：`off` / `none` / `disabled`（大小写不敏感，`meta` 优先于 `reasoning.effort`）。
pub(crate) fn thinking_disabled(request: &CanonicalRequest) -> bool {
    let level = request.meta.thinking_level.as_deref().or_else(|| {
        request
            .reasoning
            .as_ref()
            .and_then(|params| params.effort.as_deref())
    });
    let Some(level) = level.map(str::trim).filter(|level| !level.is_empty()) else {
        return false;
    };
    matches!(
        level.to_ascii_lowercase().as_str(),
        "off" | "none" | "disabled"
    )
}

/// 显式关闭思考时补上厂商自己的关闭开关（只看已在文档/实测确认的约定）。
///
/// - 硅基流动 / DashScope（阿里百炼）：`enable_thinking: false`
/// - 智谱 GLM / Z.ai：`thinking: { type: "disabled" }`
/// - OpenRouter：`reasoning: { enabled: false }`
///
/// 其余厂商只剥离推理参数，不发送未确认的字段（避免 400）；Anthropic / Responses
/// 不携带 thinking 即为关闭，无需注入。
pub(crate) fn apply_thinking_disable(
    candidate: &Candidate,
    request: &CanonicalRequest,
    body: &mut Value,
) {
    if !(thinking_disabled(request) || candidate_thinking_disabled(candidate)) {
        return;
    }
    if wire_protocol(&candidate.upstream.provider) != Some(IngressProtocol::ChatCompletions) {
        return;
    }
    let Some(host) = base_host(&candidate.upstream.base_url) else {
        return;
    };
    match thinking_switch(&host) {
        Some(ThinkingSwitch::EnableThinking) => {
            body["enable_thinking"] = Value::Bool(false);
        }
        Some(ThinkingSwitch::ThinkingObject) => {
            body["thinking"] = json!({ "type": "disabled" });
        }
        Some(ThinkingSwitch::ReasoningObject) => {
            body["reasoning"] = json!({ "enabled": false });
        }
        None => {}
    }
}

enum ThinkingSwitch {
    EnableThinking,
    ThinkingObject,
    ReasoningObject,
}

fn thinking_switch(host: &str) -> Option<ThinkingSwitch> {
    let host = host.to_ascii_lowercase();
    if host.ends_with("siliconflow.cn")
        || host.ends_with("siliconflow.com")
        || host.contains("dashscope")
    {
        return Some(ThinkingSwitch::EnableThinking);
    }
    if host.ends_with("bigmodel.cn") || host == "z.ai" || host.ends_with(".z.ai") {
        return Some(ThinkingSwitch::ThinkingObject);
    }
    if host == "openrouter.ai" || host.ends_with(".openrouter.ai") {
        return Some(ThinkingSwitch::ReasoningObject);
    }
    None
}

/// 从 base_url 取 host（忽略 scheme、端口、路径；IPv6 按原样透传）。
fn base_host(base_url: &str) -> Option<String> {
    let rest = base_url
        .split_once("://")
        .map_or(base_url, |(_, rest)| rest);
    let authority = rest.split(['/', '?', '#']).next().unwrap_or_default();
    let authority = authority.rsplit('@').next().unwrap_or(authority);
    let host = match authority.rsplit_once(':') {
        Some((host, port)) if port.chars().all(|c| c.is_ascii_digit()) => host,
        _ => authority,
    };
    (!host.is_empty()).then(|| host.to_string())
}

/// 候选 `provider` → 上游 wire 协议；未知 provider 返回 `None`（显式失败）。
pub(crate) fn wire_protocol(kind: &ProviderKind) -> Option<IngressProtocol> {
    match kind {
        ProviderKind::OpenAiCompatible => Some(IngressProtocol::ChatCompletions),
        ProviderKind::Anthropic => Some(IngressProtocol::AnthropicMessages),
        ProviderKind::Responses => Some(IngressProtocol::Responses),
        ProviderKind::Other(_) => None,
    }
}

pub(crate) fn encode_wire(protocol: IngressProtocol, request: &CanonicalRequest) -> Value {
    match protocol {
        IngressProtocol::ChatCompletions => chat::encode_request(request),
        IngressProtocol::AnthropicMessages => anthropic::encode_request(request),
        IngressProtocol::Responses => responses::encode_request(request),
    }
}

pub(crate) fn decode_wire(
    protocol: IngressProtocol,
    value: &Value,
) -> Result<CanonicalResponse, tr_proto::CodecError> {
    match protocol {
        IngressProtocol::ChatCompletions => chat::decode_response(value),
        IngressProtocol::AnthropicMessages => anthropic::decode_response(value),
        IngressProtocol::Responses => responses::decode_response(value),
    }
}

pub(crate) fn unsupported_provider(kind: &ProviderKind, path: &str) -> AttemptFailure {
    AttemptFailure {
        class: ErrorClass::Protocol,
        status: None,
        message: Some(format!("provider {kind:?} 尚未支持{path}执行")),
        retry_after_ms: None,
        cause: AttemptCause::UnsupportedProvider,
        rate_limit: None,
    }
}

/// 拼接端点；已带目标路径时不再追加。
///
/// - OpenAI 兼容：`{base}/chat/completions`
/// - Responses：`{base}/responses`
/// - Anthropic：`{base}/v1/messages`（base 已带 `/v1` 时只补 `/messages`）
pub fn endpoint(kind: &ProviderKind, base_url: &str) -> String {
    let base = base_url.trim_end_matches('/');
    match kind {
        ProviderKind::Anthropic => {
            if base.ends_with("/messages") {
                base.to_string()
            } else if base.ends_with("/v1") {
                format!("{base}/messages")
            } else {
                format!("{base}/v1/messages")
            }
        }
        ProviderKind::Responses => {
            if base.ends_with("/responses") {
                base.to_string()
            } else {
                format!("{base}/responses")
            }
        }
        _ => {
            if base.ends_with("/chat/completions") {
                base.to_string()
            } else {
                format!("{base}/chat/completions")
            }
        }
    }
}

/// 数据目录基準（v3 P5-A）：`file:` **相对路径**凭据（`file:credentials/<id>.key`）
/// 按数据目录解析，而不是进程 cwd——桌面端 supervisor spawn 的 sidecar cwd 不可假设。
/// 由 server 启动时调用一次；测试可显式设置或用 `None` 清空。
static DATA_DIR: std::sync::OnceLock<std::path::PathBuf> = std::sync::OnceLock::new();

/// 设置 `file:` 相对路径凭据的解析基准（数据目录）。进程内只生效第一次调用。
pub fn set_data_dir(path: impl Into<std::path::PathBuf>) {
    let _ = DATA_DIR.set(path.into());
}

/// 解析 `file:` 凭据路径：相对路径按数据目录基准，找不到再回退 cwd（兼容旧用法）。
fn resolve_file_path(name: &str) -> std::path::PathBuf {
    resolve_file_path_with_base(name, DATA_DIR.get().map(|path| path.as_path()))
}

fn resolve_file_path_with_base(name: &str, base: Option<&std::path::Path>) -> std::path::PathBuf {
    let raw = std::path::Path::new(name);
    if raw.is_absolute() {
        return raw.to_path_buf();
    }
    if let Some(base) = base {
        let candidate = base.join(name);
        if candidate.is_file() {
            return candidate;
        }
    }
    raw.to_path_buf()
}

pub(crate) fn resolve_credential(reference: &CredentialRef) -> Result<String, AttemptFailure> {
    let (kind, name) = reference
        .0
        .split_once(':')
        .ok_or_else(|| credential_failure("credential 需要 `<kind>:<value>` 形式".to_string()))?;

    match kind {
        "env" => {
            std::env::var(name).map_err(|_| credential_failure(format!("环境变量 {name} 未设置")))
        }
        "file" => std::fs::read_to_string(resolve_file_path(name))
            .map(|content| content.trim().to_string())
            .map_err(|error| credential_failure(format!("读取凭据文件失败: {error}"))),
        "keyring" => Err(credential_failure(
            "keyring 凭据尚未实现（P0a 只支持 env/file）".to_string(),
        )),
        other => Err(credential_failure(format!("未知凭据类型 `{other}`"))),
    }
}

fn credential_failure(message: String) -> AttemptFailure {
    AttemptFailure {
        class: ErrorClass::Auth,
        status: None,
        message: Some(message),
        retry_after_ms: None,
        cause: AttemptCause::Credential,
        rate_limit: None,
    }
}

/// 按协议挂鉴权：Anthropic 用 `x-api-key` + 版本头，其余用 Bearer。
fn apply_auth(
    builder: reqwest::RequestBuilder,
    kind: &ProviderKind,
    reference: Option<&CredentialRef>,
) -> Result<reqwest::RequestBuilder, AttemptFailure> {
    let Some(reference) = reference else {
        return Ok(builder);
    };
    let secret = resolve_credential(reference)?;
    Ok(match kind {
        ProviderKind::Anthropic => builder
            .header("x-api-key", secret)
            .header("anthropic-version", "2023-06-01"),
        _ => builder.bearer_auth(secret),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use tr_contracts::{
        CanonicalMessage, CanonicalRole, CapabilityProfile, ContentBlock, PriceModel, UpstreamRef,
    };

    fn candidate(provider: ProviderKind, base_url: &str) -> Candidate {
        Candidate {
            id: "c1".to_string(),
            tier: "local".to_string(),
            strength: 10,
            upstream: UpstreamRef {
                provider,
                base_url: base_url.to_string(),
                model: "physical-model".to_string(),
                credential: None,
            },
            window: 32_768,
            capability: CapabilityProfile::default(),
            price: Some(PriceModel::default()),
            enabled: true,
        }
    }

    fn request() -> CanonicalRequest {
        CanonicalRequest {
            messages: vec![CanonicalMessage {
                role: CanonicalRole::User,
                blocks: vec![ContentBlock::Text {
                    text: "hi".to_string(),
                    cache_control: None,
                }],
                ..CanonicalMessage::default()
            }],
            ..CanonicalRequest::default()
        }
    }

    #[test]
    fn endpoint_appends_path_once() {
        assert_eq!(
            endpoint(&ProviderKind::OpenAiCompatible, "http://127.0.0.1:11434/v1"),
            "http://127.0.0.1:11434/v1/chat/completions"
        );
        assert_eq!(
            endpoint(
                &ProviderKind::OpenAiCompatible,
                "https://api.example.com/v1/chat/completions"
            ),
            "https://api.example.com/v1/chat/completions"
        );
    }

    #[test]
    fn endpoint_follows_wire_protocol() {
        assert_eq!(
            endpoint(&ProviderKind::Responses, "https://api.openai.com/v1"),
            "https://api.openai.com/v1/responses"
        );
        assert_eq!(
            endpoint(
                &ProviderKind::Responses,
                "https://api.openai.com/v1/responses"
            ),
            "https://api.openai.com/v1/responses"
        );
        assert_eq!(
            endpoint(&ProviderKind::Anthropic, "https://api.anthropic.com"),
            "https://api.anthropic.com/v1/messages"
        );
        assert_eq!(
            endpoint(&ProviderKind::Anthropic, "https://api.anthropic.com/v1"),
            "https://api.anthropic.com/v1/messages"
        );
        assert_eq!(
            endpoint(&ProviderKind::Anthropic, "https://proxy/v1/messages"),
            "https://proxy/v1/messages"
        );
    }

    #[test]
    fn physical_request_replaces_virtual_model() {
        let wire = physical_request(
            &candidate(ProviderKind::OpenAiCompatible, "http://x"),
            &request(),
        );
        assert_eq!(wire.meta.virtual_model.as_deref(), Some("physical-model"));
        assert!(!wire.output.stream);
    }

    #[test]
    fn physical_request_drops_reasoning_not_declared() {
        let mut with_levels = candidate(ProviderKind::OpenAiCompatible, "http://x");
        with_levels.capability.thinking_levels = vec!["low".to_string()];

        let mut asked = request();
        asked.meta.thinking_level = Some("high".to_string());
        asked.reasoning = Some(tr_contracts::ReasoningParams {
            effort: Some("high".to_string()),
            ..Default::default()
        });

        let wire = physical_request(&with_levels, &asked);
        assert!(wire.reasoning.is_none(), "未声明的档位不应透传给上游");

        asked.meta.thinking_level = Some("low".to_string());
        asked.reasoning = Some(tr_contracts::ReasoningParams {
            effort: Some("low".to_string()),
            ..Default::default()
        });
        let wire = physical_request(&with_levels, &asked);
        assert!(wire.reasoning.is_some(), "已声明的档位应原样放行");
    }

    #[test]
    fn physical_request_keeps_reasoning_when_levels_undeclared() {
        let mut asked = request();
        asked.meta.thinking_level = Some("high".to_string());
        asked.reasoning = Some(tr_contracts::ReasoningParams {
            effort: Some("high".to_string()),
            ..Default::default()
        });
        // 空声明 = 未声明：不猜测能力，保持既有透传行为。
        let wire = physical_request(
            &candidate(ProviderKind::OpenAiCompatible, "http://x"),
            &asked,
        );
        assert!(wire.reasoning.is_some());
    }

    #[test]
    fn explicit_off_strips_reasoning_even_when_levels_undeclared() {
        let mut asked = request();
        asked.meta.thinking_level = Some("off".to_string());
        asked.reasoning = Some(tr_contracts::ReasoningParams {
            effort: Some("high".to_string()),
            ..Default::default()
        });
        let wire = physical_request(
            &candidate(ProviderKind::OpenAiCompatible, "http://x"),
            &asked,
        );
        assert!(wire.reasoning.is_none(), "显式关闭时不应发推理参数");
    }

    #[test]
    fn off_synonyms_are_recognized() {
        for level in ["off", "NONE", "disabled"] {
            let mut asked = request();
            asked.meta.thinking_level = Some(level.to_string());
            assert!(thinking_disabled(&asked), "{level} 应视为关闭");
        }
        let mut asked = request();
        asked.meta.thinking_level = Some("high".to_string());
        assert!(!thinking_disabled(&asked));
    }

    #[test]
    fn vendor_thinking_switch_is_injected_for_known_hosts() {
        let mut asked = request();
        asked.meta.thinking_level = Some("off".to_string());
        let cases = [
            ("https://api.siliconflow.cn/v1", "enable_thinking"),
            (
                "https://dashscope.aliyuncs.com/compatible-mode/v1",
                "enable_thinking",
            ),
            ("https://open.bigmodel.cn/api/paas/v4", "thinking"),
            ("https://openrouter.ai/api/v1", "reasoning"),
        ];
        for (base_url, key) in cases {
            let mut body = json!({"model": "m", "messages": []});
            apply_thinking_disable(
                &candidate(ProviderKind::OpenAiCompatible, base_url),
                &asked,
                &mut body,
            );
            assert!(body.get(key).is_some(), "{base_url} 应注入 {key}: {body}");
        }
    }

    #[test]
    fn unknown_host_and_other_protocols_get_no_switch() {
        let mut asked = request();
        asked.meta.thinking_level = Some("off".to_string());

        let mut body = json!({"model": "m", "messages": []});
        apply_thinking_disable(
            &candidate(ProviderKind::OpenAiCompatible, "https://api.example.com/v1"),
            &asked,
            &mut body,
        );
        assert!(body.get("enable_thinking").is_none(), "未知厂商不注入");

        let mut body = json!({"model": "m", "messages": []});
        apply_thinking_disable(
            &candidate(ProviderKind::Anthropic, "https://api.siliconflow.cn/v1"),
            &asked,
            &mut body,
        );
        assert!(body.get("enable_thinking").is_none(), "非 Chat wire 不注入");
    }

    #[test]
    fn base_host_ignores_scheme_port_and_path() {
        assert_eq!(
            base_host("https://api.siliconflow.cn/v1").as_deref(),
            Some("api.siliconflow.cn")
        );
        assert_eq!(
            base_host("http://127.0.0.1:8000/v1/chat/completions").as_deref(),
            Some("127.0.0.1")
        );
        assert_eq!(
            base_host("api.siliconflow.cn/v1").as_deref(),
            Some("api.siliconflow.cn")
        );
    }

    #[test]
    fn candidate_disable_thinking_strips_and_injects_switch() {
        let mut with_tag = candidate(
            ProviderKind::OpenAiCompatible,
            "https://api.siliconflow.cn/v1",
        );
        with_tag
            .capability
            .tags
            .insert("disable_thinking".to_string(), "1".to_string());

        let mut asked = request();
        asked.reasoning = Some(tr_contracts::ReasoningParams {
            effort: Some("high".to_string()),
            ..Default::default()
        });
        let wire = physical_request(&with_tag, &asked);
        assert!(wire.reasoning.is_none(), "候选级关闭思考应剥离 reasoning");

        let mut body = json!({"model": "m", "messages": []});
        apply_thinking_disable(&with_tag, &asked, &mut body);
        assert_eq!(body["enable_thinking"], false);
    }

    #[test]
    fn candidate_max_tokens_clamps_request() {
        let mut limited = candidate(ProviderKind::OpenAiCompatible, "http://x");
        limited
            .capability
            .tags
            .insert("max_tokens".to_string(), "256".to_string());

        let mut asked = request();
        asked.output.max_tokens = Some(1024);
        assert_eq!(
            physical_request(&limited, &asked).output.max_tokens,
            Some(256)
        );

        asked.output.max_tokens = Some(128);
        assert_eq!(
            physical_request(&limited, &asked).output.max_tokens,
            Some(128)
        );

        asked.output.max_tokens = None;
        assert_eq!(
            physical_request(&limited, &asked).output.max_tokens,
            Some(256)
        );
    }

    #[test]
    fn missing_env_credential_is_auth_failure() {
        // 用一个几乎不可能存在的变量名，避免受环境干扰。
        let reference = CredentialRef::env("TR_V2_TEST_MISSING_KEY_9F3A");
        let failure = resolve_credential(&reference).expect_err("must fail");
        assert_eq!(failure.class, ErrorClass::Auth);
        assert_eq!(failure.cause, AttemptCause::Credential);
    }
}

#[cfg(test)]
mod data_dir_tests {
    use super::*;

    /// v3 P5-A 回归：`file:credentials/<id>.key` 相对路径按数据目录解析，
    /// 不依赖进程 cwd（桌面端 supervisor spawn 的 sidecar 场景）。
    #[test]
    fn file_credential_resolves_relative_to_data_dir() {
        // 独立临时目录 + 相对子路径；OnceLock 进程级只 set 一次，
        // 因此用「绝对路径 + 数据目录内的文件」验证解析逻辑本身。
        let base = std::env::temp_dir().join(format!("tr2-cred-test-{}", std::process::id()));
        std::fs::create_dir_all(base.join("credentials")).unwrap();
        std::fs::write(base.join("credentials").join("acc.key"), "  sk-test-1234  ").unwrap();

        // 绝对路径形式总是可用（既有行为不回归）。
        let absolute = CredentialRef(format!(
            "file:{}",
            base.join("credentials").join("acc.key").display()
        ));
        let resolved = resolve_credential(&absolute).unwrap();
        assert_eq!(resolved, "sk-test-1234", "绝对路径 + trim");

        // 相对路径解析函数：有数据目录基准时命中。
        let relative = resolve_file_path_with_base("credentials/acc.key", Some(&base));
        assert!(relative.is_file(), "相对路径应按数据目录基准解析");

        // 没有基准时保持 cwd 行为（原样返回）。
        let fallback = resolve_file_path_with_base("credentials/acc.key", None);
        assert_eq!(fallback, std::path::PathBuf::from("credentials/acc.key"));

        let _ = std::fs::remove_dir_all(&base);
    }
}
