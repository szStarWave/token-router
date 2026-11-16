//! OpenAI 兼容非流式执行（P0a-9）。
//!
//! 边界：
//! - 只在这里产生 `ErrorClass`（经 `tr_proto::errors::classify`）；
//! - 只在这里解析 `credential` 引用（`env:` / `file:` / `keyring:`）；
//! - 不改健康度、不写账本——那是 `agent` 的职责。
//!
//! P0a 只支持 `ProviderKind::OpenAiCompatible`；其他 provider 显式返回
//! `UnsupportedProvider`，不静默降级成错误的上游调用。

use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use reqwest::Client;
use serde_json::Value;
use tr_contracts::{
    Candidate, CandidateId, CanonicalRequest, CanonicalResponse, CredentialRef, ErrorClass,
    ProviderKind,
};
use tr_proto::chat;

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
}

impl AttemptOutcome {
    pub fn is_success(&self) -> bool {
        self.response.is_some()
    }
}

/// HTTP 上游客户端。
#[derive(Debug, Clone)]
pub struct HttpUpstream {
    client: Client,
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
        Ok(Self { client })
    }

    /// 执行一次非流式请求。
    pub async fn execute(
        &self,
        candidate: &Candidate,
        request: &CanonicalRequest,
    ) -> AttemptOutcome {
        let started = Instant::now();
        match self.try_execute(candidate, request).await {
            Ok(response) => AttemptOutcome {
                candidate_id: candidate.id.clone(),
                http_status: Some(200),
                response: Some(response),
                failure: None,
                duration_ms: started.elapsed().as_millis() as u64,
                ttft_ms: None,
            },
            Err((failure, status)) => AttemptOutcome {
                candidate_id: candidate.id.clone(),
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
    ) -> Result<CanonicalResponse, (AttemptFailure, Option<u16>)> {
        if candidate.upstream.provider != ProviderKind::OpenAiCompatible {
            return Err((
                AttemptFailure {
                    class: ErrorClass::Protocol,
                    status: None,
                    message: Some(format!(
                        "provider {:?} 尚未在 P0a 实现",
                        candidate.upstream.provider
                    )),
                    retry_after_ms: None,
                    cause: AttemptCause::UnsupportedProvider,
                },
                None,
            ));
        }

        let url = endpoint(&candidate.upstream.base_url);
        let wire = physical_request(candidate, request);
        let body = chat::encode_request(&wire);

        let mut builder = self.client.post(&url).json(&body);
        if let Some(reference) = &candidate.upstream.credential {
            let secret = resolve_credential(reference).map_err(|failure| (failure, None))?;
            builder = builder.bearer_auth(secret);
        }

        let response = builder.send().await.map_err(|error| {
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
                },
                Some(status.as_u16()),
            )
        })?;

        if !status.is_success() {
            let body: Value = serde_json::from_str(&text)
                .unwrap_or_else(|_| serde_json::json!({"error": {"message": text}}));
            let classified = tr_proto::errors::classify(status.as_u16(), &headers, &body);
            return Err((
                AttemptFailure {
                    class: classified.class,
                    status: Some(status.as_u16()),
                    message: classified.message,
                    retry_after_ms: classified.retry_after_ms,
                    cause: AttemptCause::HttpStatus,
                },
                Some(status.as_u16()),
            ));
        }

        let value: Value = serde_json::from_str(&text).map_err(|error| {
            (
                AttemptFailure {
                    class: ErrorClass::Protocol,
                    status: Some(status.as_u16()),
                    message: Some(error.to_string()),
                    retry_after_ms: None,
                    cause: AttemptCause::MalformedResponse,
                },
                Some(status.as_u16()),
            )
        })?;

        chat::decode_response(&value).map_err(|error| {
            (
                AttemptFailure {
                    class: ErrorClass::Protocol,
                    status: Some(status.as_u16()),
                    message: Some(error.to_string()),
                    retry_after_ms: None,
                    cause: AttemptCause::MalformedResponse,
                },
                Some(status.as_u16()),
            )
        })
    }
}

/// 把虚拟模型替换成物理模型，并强制非流式（P0a）。
pub fn physical_request(candidate: &Candidate, request: &CanonicalRequest) -> CanonicalRequest {
    let mut wire = request.clone();
    wire.meta.virtual_model = Some(candidate.upstream.model.clone());
    wire.output.stream = false;
    wire
}

/// 拼接 endpoint；已带路径时不再追加。
pub fn endpoint(base_url: &str) -> String {
    let base = base_url.trim_end_matches('/');
    if base.ends_with("/chat/completions") {
        base.to_string()
    } else {
        format!("{base}/chat/completions")
    }
}

fn resolve_credential(reference: &CredentialRef) -> Result<String, AttemptFailure> {
    let (kind, name) = reference
        .0
        .split_once(':')
        .ok_or_else(|| credential_failure("credential 需要 `<kind>:<value>` 形式".to_string()))?;

    match kind {
        "env" => {
            std::env::var(name).map_err(|_| credential_failure(format!("环境变量 {name} 未设置")))
        }
        "file" => std::fs::read_to_string(name)
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
    }
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
            endpoint("http://127.0.0.1:11434/v1"),
            "http://127.0.0.1:11434/v1/chat/completions"
        );
        assert_eq!(
            endpoint("https://api.example.com/v1/chat/completions"),
            "https://api.example.com/v1/chat/completions"
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
    fn missing_env_credential_is_auth_failure() {
        // 用一个几乎不可能存在的变量名，避免受环境干扰。
        let reference = CredentialRef::env("TR_V2_TEST_MISSING_KEY_9F3A");
        let failure = resolve_credential(&reference).expect_err("must fail");
        assert_eq!(failure.class, ErrorClass::Auth);
        assert_eq!(failure.cause, AttemptCause::Credential);
    }
}
