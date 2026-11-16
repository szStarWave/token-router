//! 版本化决策服务客户端（HiRoute 信封，§7.5）。
//!
//! 只发一个版本化请求、只读一个版本化响应；`revision` / `protocol_digest` 不匹配
//! 属于配置错误，运行期不重试。

use std::time::Duration;

use futures_util::future::BoxFuture;
use serde_json::{Value, json};
use tr_contracts::{JudgeAnswer, JudgeFailure, JudgeFailureCode, JudgeInput, JudgeValue};

use crate::DecisionModel;

/// 服务配置。
#[derive(Debug, Clone)]
pub struct HttpJudgeConfig {
    pub endpoint: String,
    pub service_id: String,
    pub revision: u64,
    /// 已解析的密钥（不落盘；由 `agent` 从 `credential` 引用解析）。
    pub credential: Option<String>,
    pub timeout: Duration,
}

/// 外部决策服务。
pub struct HttpJudgeModel {
    client: reqwest::Client,
    config: HttpJudgeConfig,
}

impl HttpJudgeModel {
    pub fn new(config: HttpJudgeConfig) -> Result<Self, reqwest::Error> {
        let client = reqwest::Client::builder().timeout(config.timeout).build()?;
        Ok(Self { client, config })
    }
}

impl DecisionModel for HttpJudgeModel {
    fn id(&self) -> String {
        format!("{}@{}", self.config.service_id, self.config.revision)
    }

    fn decide<'a>(
        &'a self,
        input: &'a JudgeInput,
    ) -> BoxFuture<'a, Result<JudgeAnswer, JudgeFailure>> {
        Box::pin(async move {
            if self.config.endpoint.is_empty() {
                return Err(JudgeFailure {
                    code: JudgeFailureCode::InvalidConfiguration,
                    message: Some("judge endpoint 为空".to_string()),
                });
            }

            let envelope = json!({
                "schema": "token-router.judge-request/v1",
                "service_id": self.config.service_id,
                "revision": self.config.revision,
                "question": input.question,
                "category": input.category,
                "evidence": input.evidence,
                "context": input.context
            });

            let mut request = self.client.post(&self.config.endpoint).json(&envelope);
            if let Some(secret) = &self.config.credential {
                request = request.bearer_auth(secret);
            }

            let response = request.send().await.map_err(map_transport)?;
            let status = response.status();
            let text = response.text().await.map_err(map_transport)?;

            if !status.is_success() {
                return Err(map_status(status.as_u16(), &text));
            }

            let value: Value = serde_json::from_str(&text).map_err(|error| JudgeFailure {
                code: JudgeFailureCode::MalformedOutput,
                message: Some(error.to_string()),
            })?;
            let answer = value.get("answer").ok_or_else(|| JudgeFailure {
                code: JudgeFailureCode::MalformedOutput,
                message: Some("响应缺少 answer".to_string()),
            })?;
            parse_answer(answer)
        })
    }
}

fn parse_answer(value: &Value) -> Result<JudgeAnswer, JudgeFailure> {
    let judged = match value.get("kind").and_then(Value::as_str) {
        Some("bool") => JudgeValue::Bool(
            value
                .get("value")
                .and_then(Value::as_bool)
                .ok_or_else(|| malformed("bool value 缺失"))?,
        ),
        Some("choice") => JudgeValue::Choice(
            value
                .get("value")
                .and_then(Value::as_str)
                .ok_or_else(|| malformed("choice value 缺失"))?
                .to_string(),
        ),
        Some("score") => JudgeValue::Score(
            value
                .get("value")
                .and_then(Value::as_f64)
                .ok_or_else(|| malformed("score value 缺失"))? as f32,
        ),
        Some("uncertain") | None => JudgeValue::Uncertain,
        Some(other) => {
            return Err(malformed(format!("未知 answer kind `{other}`")));
        }
    };

    Ok(JudgeAnswer {
        value: judged,
        confidence: value
            .get("confidence")
            .and_then(Value::as_f64)
            .map(|confidence| confidence.clamp(0.0, 1.0) as f32),
        reason: value
            .get("reason")
            .and_then(Value::as_str)
            .map(str::to_string),
    })
}

fn map_transport(error: reqwest::Error) -> JudgeFailure {
    JudgeFailure {
        code: if error.is_timeout() {
            JudgeFailureCode::Timeout
        } else {
            JudgeFailureCode::TransportUnavailable
        },
        message: Some(error.to_string()),
    }
}

fn map_status(status: u16, body: &str) -> JudgeFailure {
    let code = match status {
        401 | 403 => JudgeFailureCode::AuthenticationRejected,
        408 | 504 => JudgeFailureCode::Timeout,
        429 => JudgeFailureCode::RateLimited,
        400 | 422 => JudgeFailureCode::InputRejected,
        500..=599 => JudgeFailureCode::SourceUnavailable,
        _ => JudgeFailureCode::TransportUnavailable,
    };
    JudgeFailure {
        code,
        message: Some(format!("service status {status}: {body}")),
    }
}

fn malformed(message: impl Into<String>) -> JudgeFailure {
    JudgeFailure {
        code: JudgeFailureCode::MalformedOutput,
        message: Some(message.into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_typed_answers() {
        let answer = parse_answer(&json!({"kind": "bool", "value": true, "confidence": 0.9}))
            .expect("parse");
        assert_eq!(answer.value, JudgeValue::Bool(true));
        assert_eq!(answer.confidence, Some(0.9));

        let answer = parse_answer(&json!({"kind": "score", "value": 0.42})).expect("parse score");
        assert!(matches!(answer.value, JudgeValue::Score(score) if (score - 0.42).abs() < 1e-6));

        let answer = parse_answer(&json!({"kind": "uncertain"})).expect("parse uncertain");
        assert_eq!(answer.value, JudgeValue::Uncertain);
    }

    #[test]
    fn rejects_unknown_kind() {
        let failure = parse_answer(&json!({"kind": "nope"})).expect_err("must fail");
        assert_eq!(failure.code, JudgeFailureCode::MalformedOutput);
    }

    #[test]
    fn status_maps_to_failure_codes() {
        assert_eq!(
            map_status(401, "").code,
            JudgeFailureCode::AuthenticationRejected
        );
        assert_eq!(map_status(429, "").code, JudgeFailureCode::RateLimited);
        assert_eq!(map_status(400, "").code, JudgeFailureCode::InputRejected);
        assert_eq!(
            map_status(503, "").code,
            JudgeFailureCode::SourceUnavailable
        );
    }
}
