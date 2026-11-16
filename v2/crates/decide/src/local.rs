//! 启发式本地判官（§7.1 的常驻兜底）。
//!
//! 不调用任何模型：只看编排层已经算好的信号。它的职责不是"聪明"，而是保证
//! 判官 `off` / 外部服务不可用时，升级机制仍有确定性输入。

use futures_util::future::BoxFuture;
use tr_contracts::{Choices, JudgeAnswer, JudgeFailure, JudgeInput, JudgeValue};

use crate::DecisionModel;

/// 判官可见的本地信号（由 `agent` 从 `CoreSignals` 映射）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct LocalSignals {
    pub tool_error_streak: u32,
    pub repeated_tool_loop: bool,
    pub context_overflow: bool,
    /// 任务变化时抑制升级（避免把上一个任务的故障带过来）。
    pub task_changed: bool,
}

impl LocalSignals {
    fn should_escalate(&self) -> bool {
        if self.task_changed {
            return false;
        }
        self.tool_error_streak >= 2 || self.repeated_tool_loop || self.context_overflow
    }
}

/// 本地启发式模型。
#[derive(Debug, Clone, Copy, Default)]
pub struct LocalDecisionModel {
    pub signals: LocalSignals,
}

impl DecisionModel for LocalDecisionModel {
    fn id(&self) -> String {
        "local-heuristic".to_string()
    }

    fn decide<'a>(
        &'a self,
        input: &'a JudgeInput,
    ) -> BoxFuture<'a, Result<JudgeAnswer, JudgeFailure>> {
        Box::pin(async move {
            let escalate = self.signals.should_escalate();
            let value = match &input.question.choices {
                Choices::Boolean => JudgeValue::Bool(escalate),
                Choices::Score { .. } => JudgeValue::Score(if escalate { 1.0 } else { 0.0 }),
                Choices::Enum(_) => JudgeValue::Uncertain,
            };
            Ok(JudgeAnswer {
                value,
                confidence: Some(0.7),
                reason: Some(if escalate {
                    format!(
                        "local signals: streak={}, loop={}, overflow={}",
                        self.signals.tool_error_streak,
                        self.signals.repeated_tool_loop,
                        self.signals.context_overflow
                    )
                } else {
                    "local signals: no escalation evidence".to_string()
                }),
                usage: None,
            })
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tr_contracts::{Category, EvidenceRef, Question};

    fn input() -> JudgeInput {
        JudgeInput {
            question: Question {
                prompt: "escalate?".to_string(),
                choices: Choices::Boolean,
                min_confidence: 0.6,
            },
            category: Category::ToolFailure,
            evidence: EvidenceRef::default(),
            context: Default::default(),
        }
    }

    fn answer(signals: LocalSignals) -> JudgeAnswer {
        let model = LocalDecisionModel { signals };
        futures_util::FutureExt::now_or_never(model.decide(&input()))
            .expect("ready")
            .expect("answer")
    }

    #[test]
    fn streak_of_two_escalates() {
        let answer = answer(LocalSignals {
            tool_error_streak: 2,
            ..LocalSignals::default()
        });
        assert_eq!(answer.value, JudgeValue::Bool(true));
    }

    #[test]
    fn single_error_does_not_escalate() {
        let answer = answer(LocalSignals {
            tool_error_streak: 1,
            ..LocalSignals::default()
        });
        assert_eq!(answer.value, JudgeValue::Bool(false));
    }

    #[test]
    fn task_change_suppresses_escalation() {
        let answer = answer(LocalSignals {
            tool_error_streak: 5,
            task_changed: true,
            ..LocalSignals::default()
        });
        assert_eq!(answer.value, JudgeValue::Bool(false));
    }
}
