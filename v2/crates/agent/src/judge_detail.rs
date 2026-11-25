//! 判官（含 JEV 决策模型）调用详情摘要。
//!
//! 落库前把 `JudgeInput`/`JudgeAnswer` 压成小 JSON：
//! - 输入：任务文本（截断）、问题提示词（截断）、选项；
//! - 输出：答案类型/取值、置信度、理由（截断）；
//! - 失败：失败码 + 错误消息（由调用点拼）。
//!
//! 不存完整 HTTP 请求体，也不存 system/工具正文（判官上下文本来就只有最后一条 user 消息）。

use serde_json::{Value, json};
use tr_contracts::{Choices, JudgeAnswer, JudgeInput, JudgeValue};

/// 任务文本上限（字符）。
pub const MAX_TASK_CHARS: usize = 4_000;
/// 问题提示词上限（字符）。
pub const MAX_PROMPT_CHARS: usize = 4_000;
/// 答案理由上限（字符）。
pub const MAX_REASON_CHARS: usize = 2_000;

fn truncate(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    let mut out: String = text.chars().take(max).collect();
    out.push('…');
    out
}

/// 选项摘要：枚举展开 `{id,label}`，其他形状给类型与阈值。
fn choices_summary(choices: &Choices) -> Value {
    match choices {
        Choices::Enum(items) => Value::Array(
            items
                .iter()
                .map(|choice| json!({"id": choice.id, "label": truncate(&choice.label, 200)}))
                .collect(),
        ),
        Choices::Boolean => json!({"kind": "boolean"}),
        Choices::Score {
            min,
            max,
            decline_at,
            escalate_at,
        } => json!({
            "kind": "score",
            "min": min,
            "max": max,
            "decline_at": decline_at,
            "escalate_at": escalate_at,
        }),
        Choices::ScoreBelowFloor { floor } => json!({"kind": "score_below_floor", "floor": floor}),
    }
}

/// 调用输入摘要。
pub fn request_summary(input: &JudgeInput) -> Value {
    json!({
        "task": truncate(&input.context.task, MAX_TASK_CHARS),
        "prompt": truncate(&input.question.prompt, MAX_PROMPT_CHARS),
        "choices": choices_summary(&input.question.choices),
    })
}

/// LLM 判官的调用输入：记录真正发出去的 messages（system 规则 + TASK/QUESTION/ALLOWED VALUES）。
///
/// 由 `tr_decide::llm::build_messages` 生成，和实际请求同一个函数，保证「显示的 prompt」
/// 与上游收到的一致；每条内容截断到 [`MAX_PROMPT_CHARS`] * 2。
pub fn llm_messages(messages: &[Value]) -> Value {
    let messages: Vec<Value> = messages
        .iter()
        .map(|message| {
            json!({
                "role": message.get("role").and_then(Value::as_str).unwrap_or("user"),
                "content": truncate(
                    message.get("content").and_then(Value::as_str).unwrap_or(""),
                    MAX_PROMPT_CHARS * 2,
                ),
            })
        })
        .collect();
    json!({ "messages": messages })
}

/// 调用输出摘要（答案形状与置信度）。
pub fn response_summary(answer: &JudgeAnswer) -> Value {
    let (kind, value) = match &answer.value {
        JudgeValue::Bool(value) => ("bool", json!(value)),
        JudgeValue::Choice(choice) => ("choice", json!(choice)),
        JudgeValue::Score(score) => ("score", json!(score)),
        JudgeValue::Uncertain => ("uncertain", Value::Null),
    };
    json!({
        "kind": kind,
        "value": value,
        "confidence": answer.confidence,
        "reason": answer.reason.as_deref().map(|reason| truncate(reason, MAX_REASON_CHARS)),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use tr_contracts::{
        Category, Choice, EvidenceRef, JudgeContext, JudgeQuestionDefinition, Question,
        question_definition,
    };

    fn input() -> JudgeInput {
        let question = Question {
            prompt: "pick one".to_string(),
            choices: Choices::Enum(vec![
                Choice {
                    id: "a".to_string(),
                    label: "candidate a".to_string(),
                },
                Choice {
                    id: "b".to_string(),
                    label: "candidate b".to_string(),
                },
            ]),
            min_confidence: 0.0,
        };
        JudgeInput {
            question,
            category: Category::Other("group_judge".to_string()),
            evidence: EvidenceRef::default(),
            context: JudgeContext {
                system: String::new(),
                task: "hello".to_string(),
                recent: Vec::new(),
            },
        }
    }

    #[test]
    fn request_summary_lists_choices_and_truncates() {
        let summary = request_summary(&input());
        assert_eq!(summary["task"], "hello");
        assert_eq!(summary["prompt"], "pick one");
        let choices = summary["choices"].as_array().expect("choices");
        assert_eq!(choices.len(), 2);
        assert_eq!(choices[0]["id"], "a");
        assert_eq!(choices[1]["label"], "candidate b");
    }

    #[test]
    fn response_summary_flattens_choice() {
        let answer = JudgeAnswer {
            value: JudgeValue::Choice("b".to_string()),
            confidence: Some(0.75),
            reason: Some("b is cheaper".to_string()),
            usage: None,
        };
        let summary = response_summary(&answer);
        assert_eq!(summary["kind"], "choice");
        assert_eq!(summary["value"], "b");
        assert_eq!(summary["confidence"], 0.75);
        assert_eq!(summary["reason"], "b is cheaper");
    }

    #[test]
    fn uncertain_answer_has_null_value() {
        let summary = response_summary(&JudgeAnswer::default());
        assert_eq!(summary["kind"], "uncertain");
        assert!(summary["value"].is_null());
    }

    #[test]
    fn llm_messages_match_what_is_sent() {
        let messages = tr_decide::llm::build_messages(&input());
        let summary = llm_messages(&messages);
        let recorded = summary["messages"].as_array().expect("messages");
        assert_eq!(recorded.len(), 2);
        assert_eq!(recorded[0]["role"], "system");
        assert!(
            recorded[0]["content"]
                .as_str()
                .unwrap()
                .contains("MANDATORY RULES")
        );
        assert_eq!(recorded[1]["role"], "user");
        let user = recorded[1]["content"].as_str().unwrap();
        assert!(user.contains("TASK:\nhello"));
        assert!(user.contains("QUESTION:\npick one"));
        assert!(user.contains("ALLOWED VALUES"));
    }

    #[test]
    fn long_task_is_truncated() {
        let mut input = input();
        input.context.task = "x".repeat(MAX_TASK_CHARS + 5);
        let summary = request_summary(&input);
        assert_eq!(
            summary["task"].as_str().unwrap().chars().count(),
            MAX_TASK_CHARS + 1
        );
    }

    /// 保证 `question_definition` 仍可用于构造判官输入（契约稳定性）。
    #[test]
    fn question_definition_helper_is_usable() {
        let definition = question_definition("q", &input().question);
        assert!(matches!(
            definition,
            JudgeQuestionDefinition::Categorical { .. }
        ));
    }
}
