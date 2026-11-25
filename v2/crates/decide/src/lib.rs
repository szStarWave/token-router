//! 判官（DecisionModel）实现。**唯一允许调用判官模型的 crate**（P1）。
//!
//! 四道闸（§7）：
//! 1. 只有本 crate 能发判官请求；
//! 2. 预算与 deadline 硬生效，超限降级不重试；
//! 3. 失败一律 `JudgeFailure`，不进入用户错误路径；
//! 4. 自然语言理由只进审计，不参与算术。
//!
//! 实现：
//! - [`local::LocalDecisionModel`]：启发式本地模型（常驻兜底）；
//! - [`http::HttpJudgeModel`]：版本化决策服务（HiRoute 信封）；
//! - [`llm::LlmJudgeModel`]：用 LLM 判（Strands `LLMDecisionModel` 形状，结构化输出）；
//! - [`budget::BudgetTracker`]：turn/minute/session 三级预算。

#![forbid(unsafe_code)]

pub mod budget;
pub mod http;
pub mod llm;
pub mod local;
pub mod system_one;

use std::time::Duration;

use futures_util::future::BoxFuture;
use tr_contracts::{
    Category, Choices, JudgeAnswer, JudgeFailure, JudgeFailureCode, JudgeInput,
    JudgeServiceTestResult, JudgeValue, TestOutcome, Verdict,
};

/// 判官模型抽象。
pub trait DecisionModel: Send + Sync {
    /// 稳定 id（写 `JudgeTrace.service_id`）。
    fn id(&self) -> String;

    /// 回答一个问题。实现必须自己保证不 panic，并映射所有失败为 `JudgeFailure`。
    fn decide<'a>(
        &'a self,
        input: &'a JudgeInput,
    ) -> BoxFuture<'a, Result<JudgeAnswer, JudgeFailure>>;
}

/// 带 deadline 的调用。超时按 `Timeout` 失败，不重试。
pub async fn consult(
    model: &dyn DecisionModel,
    input: &JudgeInput,
    deadline: Duration,
) -> Result<JudgeAnswer, JudgeFailure> {
    match tokio::time::timeout(deadline, model.decide(input)).await {
        Ok(result) => result,
        Err(_) => Err(JudgeFailure {
            code: JudgeFailureCode::Timeout,
            message: Some(format!(
                "judge deadline exceeded ({}ms)",
                deadline.as_millis()
            )),
        }),
    }
}

/// `judge test`：跑一次固定问题并给出结论。
pub async fn test_service(
    model: &dyn DecisionModel,
    input: &JudgeInput,
    deadline: Duration,
) -> JudgeServiceTestResult {
    let started = std::time::Instant::now();
    let outcome = consult(model, input, deadline).await;
    let latency_ms = started.elapsed().as_millis() as u32;
    match outcome {
        Ok(answer) if !matches!(answer.value, JudgeValue::Uncertain) => JudgeServiceTestResult {
            outcome: TestOutcome::Passed,
            latency_ms,
            failure: None,
        },
        Ok(_) => JudgeServiceTestResult {
            outcome: TestOutcome::Failed,
            latency_ms,
            failure: Some(JudgeFailure {
                code: JudgeFailureCode::MalformedOutput,
                message: Some("judge 对自检问题返回 Uncertain".to_string()),
            }),
        },
        Err(failure) => JudgeServiceTestResult {
            outcome: TestOutcome::Failed,
            latency_ms,
            failure: Some(failure),
        },
    }
}

/// 把答案映射为结论（§7.3）。
///
/// 映射规则是**封闭**的：Boolean/Score 由问题声明决定，Enum 需要调用点提供映射，
/// 未提供时按 `Uncertain`（不猜）。
pub fn to_verdict(input: &JudgeInput, answer: &JudgeAnswer) -> Verdict {
    let confidence = answer.confidence.unwrap_or(0.0);
    if input.question.min_confidence > 0.0 && confidence < input.question.min_confidence {
        return Verdict::Uncertain {
            reason: format!(
                "confidence {confidence:.2} < min_confidence {:.2}",
                input.question.min_confidence
            ),
        };
    }

    let evidence = input.evidence.clone();
    let category: Category = input.category.clone();

    match (&answer.value, &input.question.choices) {
        (JudgeValue::Bool(true), Choices::Boolean) => Verdict::Escalate {
            category,
            target_strength: None,
            evidence,
            confidence,
        },
        (JudgeValue::Bool(false), Choices::Boolean) => Verdict::Decline {
            reason: tr_contracts::DeclineReason::Sufficient,
        },
        (
            JudgeValue::Score(score),
            Choices::Score {
                decline_at,
                escalate_at,
                ..
            },
        ) => {
            if *score <= *decline_at {
                Verdict::Decline {
                    reason: tr_contracts::DeclineReason::Sufficient,
                }
            } else if *score >= *escalate_at {
                Verdict::Escalate {
                    category,
                    target_strength: None,
                    evidence,
                    confidence,
                }
            } else {
                Verdict::Uncertain {
                    reason: format!("score {score:.2} in uncertain band"),
                }
            }
        }
        (JudgeValue::Choice(_), Choices::Enum(_)) => Verdict::Uncertain {
            reason: "enum mapping 未在调用点声明".to_string(),
        },
        (JudgeValue::Score(score), Choices::ScoreBelowFloor { floor }) => {
            // 胜任度方向相反：分数越低越需要升级。
            if *score < *floor {
                Verdict::Escalate {
                    category,
                    target_strength: None,
                    evidence,
                    confidence,
                }
            } else {
                Verdict::Decline {
                    reason: tr_contracts::DeclineReason::Sufficient,
                }
            }
        }
        (JudgeValue::Uncertain, _) => Verdict::Uncertain {
            reason: answer
                .reason
                .clone()
                .unwrap_or_else(|| "judge uncertain".to_string()),
        },
        _ => Verdict::Uncertain {
            reason: "answer shape does not match question choices".to_string(),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tr_contracts::{EvidenceRef, JudgeValue, Question};

    fn input(choices: Choices, min_confidence: f32) -> JudgeInput {
        JudgeInput {
            question: Question {
                prompt: "escalate?".to_string(),
                choices,
                min_confidence,
            },
            category: Category::ToolFailure,
            evidence: EvidenceRef {
                fingerprint: "fp".to_string(),
                summary: "tool failed twice".to_string(),
            },
            context: Default::default(),
        }
    }

    #[test]
    fn boolean_true_escalates() {
        let answer = JudgeAnswer {
            value: JudgeValue::Bool(true),
            confidence: Some(0.9),
            reason: None,
            usage: None,
        };
        assert!(matches!(
            to_verdict(&input(Choices::Boolean, 0.6), &answer),
            Verdict::Escalate { .. }
        ));
    }

    #[test]
    fn low_confidence_becomes_uncertain() {
        let answer = JudgeAnswer {
            value: JudgeValue::Bool(true),
            confidence: Some(0.3),
            reason: None,
            usage: None,
        };
        assert!(matches!(
            to_verdict(&input(Choices::Boolean, 0.6), &answer),
            Verdict::Uncertain { .. }
        ));
    }

    #[test]
    fn score_thresholds_are_exclusive_band() {
        let question = input(
            Choices::Score {
                min: 0.0,
                max: 1.0,
                decline_at: 0.3,
                escalate_at: 0.7,
            },
            0.0,
        );
        let answer = |score: f32| JudgeAnswer {
            value: JudgeValue::Score(score),
            confidence: Some(1.0),
            reason: None,
            usage: None,
        };
        assert!(matches!(
            to_verdict(&question, &answer(0.2)),
            Verdict::Decline { .. }
        ));
        assert!(matches!(
            to_verdict(&question, &answer(0.5)),
            Verdict::Uncertain { .. }
        ));
        assert!(matches!(
            to_verdict(&question, &answer(0.9)),
            Verdict::Escalate { .. }
        ));
    }

    #[test]
    fn enum_without_mapping_is_uncertain() {
        let answer = JudgeAnswer {
            value: JudgeValue::Choice("retry".to_string()),
            confidence: Some(1.0),
            reason: None,
            usage: None,
        };
        assert!(matches!(
            to_verdict(&input(Choices::Enum(Vec::new()), 0.0), &answer),
            Verdict::Uncertain { .. }
        ));
    }
}
