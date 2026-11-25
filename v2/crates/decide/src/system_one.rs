//! SystemOne（JEV）判官 adapter（§7.7.7）。
//!
//! JEV 是**决策模型**，不是 chat 模型：它通过 SystemOne 的 `POST /v1/systemone`
//! 接收 `choice` 问题，返回 `choice` + 可选 `probabilities` / `confidence`。
//!
//! 关键约束：
//! - 只接受 `Choices::Enum`：`JudgeAnswer.value` 恒为 `JudgeValue::Choice(id)`，
//!   与 LLM 判官共享同一个内部契约，agent 侧 `interpret_answer` 不分后端；
//! - `uncertain` 是保留答案，不得作为选项 id；
//! - 只发送最后一条 user 消息（`context.task`），不发 system/recent；
//! - 不重试：解析失败 = `MalformedOutput`，由调用点 fail-open。

use std::time::Duration;

use futures_util::future::BoxFuture;
use serde_json::{Map, Value, json};
use tr_contracts::{
    Choice, Choices, JudgeAnswer, JudgeFailure, JudgeFailureCode, JudgeInput, JudgeUsage,
    JudgeValue,
};

use crate::DecisionModel;

/// SystemOne 判官配置。
#[derive(Debug, Clone)]
pub struct SystemOneJudgeConfig {
    /// 完整 SystemOne 路径（包含 `/v1/systemone`）。
    pub endpoint: String,
    pub model: String,
    pub credential: Option<String>,
    pub timeout: Duration,
    pub service_id: String,
}

/// SystemOne（JEV）判官。
pub struct SystemOneJudgeModel {
    client: reqwest::Client,
    config: SystemOneJudgeConfig,
}

impl SystemOneJudgeModel {
    /// 构造 reqwest client：`timeout` 覆盖请求与响应体读取，禁止重定向。
    pub fn new(config: SystemOneJudgeConfig) -> Result<Self, reqwest::Error> {
        let client = reqwest::Client::builder()
            .timeout(config.timeout)
            .redirect(reqwest::redirect::Policy::none())
            .build()?;
        Ok(Self { client, config })
    }
}

impl DecisionModel for SystemOneJudgeModel {
    fn id(&self) -> String {
        format!("{}:{}", self.config.service_id, self.config.model)
    }

    fn decide<'a>(
        &'a self,
        input: &'a JudgeInput,
    ) -> BoxFuture<'a, Result<JudgeAnswer, JudgeFailure>> {
        Box::pin(async move {
            if self.config.endpoint.is_empty() || self.config.model.is_empty() {
                return Err(config_failure("judge endpoint/model 为空"));
            }
            let choices = enum_choices(&input.question.choices)?;

            let body = request_body(&self.config.model, input, choices);
            let mut request = self.client.post(&self.config.endpoint).json(&body);
            if let Some(secret) = &self.config.credential {
                request = request.bearer_auth(secret);
            }

            let response = request.send().await.map_err(map_transport)?;
            let status = response.status();
            let text = response.text().await.map_err(map_transport)?;
            if !status.is_success() {
                return Err(map_status(status.as_u16(), &text));
            }

            let value: Value = serde_json::from_str(&text).map_err(malformed_err)?;
            parse_response(&value, choices)
        })
    }
}

/// 只接受 `Choices::Enum`，且拒绝保留答案 `uncertain`。
fn enum_choices(choices: &Choices) -> Result<&[Choice], JudgeFailure> {
    let Choices::Enum(choices) = choices else {
        return Err(config_failure("SystemOne 判官只支持 choice 问题"));
    };
    if choices.iter().any(|choice| choice.id == "uncertain") {
        return Err(config_failure("`uncertain` 是保留答案，不能作为选项 id"));
    }
    Ok(choices)
}

/// SystemOne 请求体：一个 `route` choice 问题，state 只含最后一条 user 消息。
fn request_body(model: &str, input: &JudgeInput, choices: &[Choice]) -> Value {
    let criteria: Map<String, Value> = choices
        .iter()
        .map(|choice| (choice.id.clone(), Value::String(choice.label.clone())))
        .collect();
    json!({
        "model": model,
        "state": {"task": input.context.task},
        "questions": {
            "route": {
                "type": "choice",
                "instructions": input.question.prompt,
                "criteria": criteria
            }
        }
    })
}

/// 解析并校验 SystemOne 响应。
fn parse_response(value: &Value, choices: &[Choice]) -> Result<JudgeAnswer, JudgeFailure> {
    let answer = value
        .get("answers")
        .and_then(|answers| answers.get("route"))
        .ok_or_else(|| malformed("响应缺少 answers.route"))?;
    if answer.get("type").and_then(Value::as_str) != Some("choice") {
        return Err(malformed("answers.route.type 不是 choice"));
    }
    let choice = answer
        .get("choice")
        .and_then(Value::as_str)
        .filter(|choice| !choice.is_empty())
        .ok_or_else(|| malformed("answers.route 缺少 choice"))?;
    if !choices.iter().any(|option| option.id == choice) {
        return Err(malformed(format!("选项 `{choice}` 不在允许集合内")));
    }

    let confidence = answer
        .get("probabilities")
        .and_then(|probabilities| probabilities.get(choice))
        .and_then(Value::as_f64)
        .or_else(|| answer.get("confidence").and_then(Value::as_f64))
        .map(|confidence| confidence.clamp(0.0, 1.0) as f32);

    Ok(JudgeAnswer {
        value: JudgeValue::Choice(choice.to_string()),
        confidence,
        reason: answer
            .get("reason")
            .and_then(Value::as_str)
            .map(str::to_string),
        usage: parse_usage(value.get("usage")),
    })
}

/// 解析 SystemOne 的 `usage`：优先 `input_tokens` / `output_tokens`，缺失按 0。
fn parse_usage(value: Option<&Value>) -> Option<JudgeUsage> {
    let object = value?.as_object()?;
    Some(JudgeUsage {
        prompt_tokens: object
            .get("input_tokens")
            .or_else(|| object.get("prompt_tokens"))
            .and_then(Value::as_u64)
            .unwrap_or(0) as u32,
        completion_tokens: object
            .get("output_tokens")
            .or_else(|| object.get("completion_tokens"))
            .and_then(Value::as_u64)
            .unwrap_or(0) as u32,
        cost_micros: 0,
    })
}

fn config_failure(message: &str) -> JudgeFailure {
    JudgeFailure {
        code: JudgeFailureCode::InvalidConfiguration,
        message: Some(message.to_string()),
    }
}

fn malformed(message: impl Into<String>) -> JudgeFailure {
    JudgeFailure {
        code: JudgeFailureCode::MalformedOutput,
        message: Some(message.into()),
    }
}

fn malformed_err(error: serde_json::Error) -> JudgeFailure {
    malformed(error.to_string())
}

fn map_transport(error: reqwest::Error) -> JudgeFailure {
    JudgeFailure {
        code: JudgeFailureCode::TransportUnavailable,
        message: Some(error.to_string()),
    }
}

fn map_status(status: u16, body: &str) -> JudgeFailure {
    let code = match status {
        401 | 403 => JudgeFailureCode::AuthenticationRejected,
        429 => JudgeFailureCode::RateLimited,
        400..=499 => JudgeFailureCode::InputRejected,
        _ => JudgeFailureCode::TransportUnavailable,
    };
    JudgeFailure {
        code,
        message: Some(format!("systemone judge status {status}: {body}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tr_contracts::{Category, EvidenceRef, Question};

    fn choices() -> Vec<Choice> {
        vec![
            Choice {
                id: "coding".to_string(),
                label: "code implementation".to_string(),
            },
            Choice {
                id: "none".to_string(),
                label: "no rule applies".to_string(),
            },
        ]
    }

    fn input(choices: Choices) -> JudgeInput {
        JudgeInput {
            question: Question {
                prompt: "select a rule".to_string(),
                choices,
                min_confidence: 0.0,
            },
            category: Category::Other("group_judge".to_string()),
            evidence: EvidenceRef::default(),
            context: tr_contracts::JudgeContext {
                system: "SECRET".to_string(),
                task: "implement a parser".to_string(),
                recent: vec!["User: earlier".to_string()],
            },
        }
    }

    #[test]
    fn request_body_is_systemone_choice_shape() {
        let body = request_body("jev-1", &input(Choices::Enum(choices())), &choices());
        assert_eq!(body["model"], "jev-1");
        assert_eq!(body["state"]["task"], "implement a parser");
        assert_eq!(body["questions"]["route"]["type"], "choice");
        assert_eq!(body["questions"]["route"]["instructions"], "select a rule");
        assert_eq!(
            body["questions"]["route"]["criteria"]["coding"],
            "code implementation"
        );
        assert!(body["questions"]["route"]["criteria"].get("none").is_some());
        // 不发 system/recent。
        assert!(body.get("messages").is_none());
        assert!(!body.to_string().contains("SECRET"));
        assert!(!body.to_string().contains("earlier"));
    }

    #[test]
    fn rejects_non_enum_choices() {
        let input = input(Choices::Boolean);
        let failure = enum_choices(&input.question.choices).expect_err("must fail");
        assert_eq!(failure.code, JudgeFailureCode::InvalidConfiguration);
    }

    #[test]
    fn reserved_uncertain_is_rejected() {
        let reserved = Choices::Enum(vec![Choice {
            id: "uncertain".to_string(),
            label: "reserved".to_string(),
        }]);
        let failure = enum_choices(&reserved).expect_err("must fail");
        assert_eq!(failure.code, JudgeFailureCode::InvalidConfiguration);
    }

    #[test]
    fn parses_choice_with_probability_confidence_and_usage() {
        let value = json!({
            "answers": {"route": {
                "type": "choice",
                "choice": "coding",
                "probabilities": {"coding": 0.9, "none": 0.1},
                "confidence": 0.7
            }},
            "usage": {"input_tokens": 10, "output_tokens": 2}
        });
        let answer = parse_response(&value, &choices()).expect("parse");
        assert_eq!(answer.value, JudgeValue::Choice("coding".to_string()));
        assert_eq!(answer.confidence, Some(0.9));
        let usage = answer.usage.expect("usage");
        assert_eq!(usage.prompt_tokens, 10);
        assert_eq!(usage.completion_tokens, 2);
    }

    #[test]
    fn confidence_falls_back_and_usage_accepts_prompt_tokens() {
        let value = json!({
            "answers": {"route": {"type": "choice", "choice": "coding", "confidence": 0.6}},
            "usage": {"prompt_tokens": 7, "completion_tokens": 3}
        });
        let answer = parse_response(&value, &choices()).expect("parse");
        assert_eq!(answer.confidence, Some(0.6));
        let usage = answer.usage.expect("usage");
        assert_eq!(usage.prompt_tokens, 7);
        assert_eq!(usage.completion_tokens, 3);
    }

    #[test]
    fn missing_confidence_is_none() {
        let value = json!({
            "answers": {"route": {"type": "choice", "choice": "none"}}
        });
        let answer = parse_response(&value, &choices()).expect("parse");
        assert_eq!(answer.confidence, None);
        assert_eq!(answer.usage, None);
    }

    #[test]
    fn unknown_choice_is_malformed() {
        let value = json!({
            "answers": {"route": {"type": "choice", "choice": "give_up"}}
        });
        let failure = parse_response(&value, &choices()).expect_err("must fail");
        assert_eq!(failure.code, JudgeFailureCode::MalformedOutput);
    }

    #[test]
    fn non_choice_or_empty_choice_is_malformed() {
        let boolean = json!({
            "answers": {"route": {"type": "noul", "noul": 0.8}}
        });
        assert_eq!(
            parse_response(&boolean, &choices())
                .expect_err("must fail")
                .code,
            JudgeFailureCode::MalformedOutput
        );

        let empty = json!({"answers": {"route": {"type": "choice", "choice": ""}}});
        assert_eq!(
            parse_response(&empty, &choices())
                .expect_err("must fail")
                .code,
            JudgeFailureCode::MalformedOutput
        );
    }

    #[test]
    fn maps_status_codes() {
        assert_eq!(
            map_status(401, "").code,
            JudgeFailureCode::AuthenticationRejected
        );
        assert_eq!(
            map_status(403, "").code,
            JudgeFailureCode::AuthenticationRejected
        );
        assert_eq!(map_status(429, "").code, JudgeFailureCode::RateLimited);
        assert_eq!(map_status(400, "").code, JudgeFailureCode::InputRejected);
        assert_eq!(map_status(422, "").code, JudgeFailureCode::InputRejected);
        assert_eq!(
            map_status(503, "").code,
            JudgeFailureCode::TransportUnavailable
        );
    }
}
