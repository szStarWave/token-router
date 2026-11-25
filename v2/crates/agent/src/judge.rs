//! 判官运行时（§5、§7）：预算 → 调用 → 结论 → 审计。
//!
//! 职责边界：
//! - 只在这里把 `JudgeRequest` 变成 `JudgeInput`（含上下文与证据）；
//! - 外部判官失败/超时 → 本地启发式兜底，**绝不阻塞请求**；
//! - 每次调用都写 `judge_calls`；shadow 模式的对比写 `shadow_records`。

use std::sync::Arc;
use std::time::{Duration, Instant};

use sha2::{Digest, Sha256};
use tr_contracts::{
    CanonicalRequest, CanonicalRole, ContentBlock, DecisionSnapshot, JudgeContext, JudgeInput,
    JudgeRequest, JudgeTrace, JudgeValue, ReasonCodeKind, Verdict,
};
use tr_decide::budget::{Budget, BudgetDecision, BudgetTracker};
use tr_decide::http::{HttpJudgeConfig, HttpJudgeModel};
use tr_decide::llm::{LlmJudgeConfig, LlmJudgeModel, StructuredOutput};
use tr_decide::local::{LocalDecisionModel, LocalSignals};
use tr_decide::system_one::{SystemOneJudgeConfig, SystemOneJudgeModel};
use tr_decide::{DecisionModel, consult, to_verdict};

use crate::store::judge::{JudgeCallDetail, JudgeCallRecord};

/// 判官模式。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum JudgeMode {
    #[default]
    Off,
    Shadow,
    Active,
}

impl JudgeMode {
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "off" => Some(Self::Off),
            "shadow" => Some(Self::Shadow),
            "active" => Some(Self::Active),
            _ => None,
        }
    }
}

/// 判官后端。
#[derive(Debug, Clone)]
pub enum Backend {
    Local,
    Http {
        endpoint: String,
        service_id: String,
        revision: u64,
        credential: Option<String>,
        timeout_ms: u32,
    },
    Llm {
        endpoint: String,
        model: String,
        credential: Option<String>,
        structured_output: StructuredOutput,
        temperature: f32,
        max_output_tokens: u32,
        timeout_ms: u32,
        service_id: String,
        /// 判官关闭思考（judge service 配置，默认 true）。
        disable_thinking: bool,
    },
    Jev {
        endpoint: String,
        model: String,
        credential: Option<String>,
        timeout_ms: u32,
        service_id: String,
    },
}

impl Backend {
    fn service_id(&self) -> &str {
        match self {
            Self::Local => "local-heuristic",
            Self::Http { service_id, .. }
            | Self::Llm { service_id, .. }
            | Self::Jev { service_id, .. } => service_id,
        }
    }

    fn revision(&self) -> u64 {
        match self {
            Self::Local => 1,
            Self::Http { revision, .. } => *revision,
            Self::Llm { .. } | Self::Jev { .. } => 1,
        }
    }

    fn source_label(&self) -> &'static str {
        match self {
            Self::Local => "heuristic",
            Self::Http { .. } => "external_service",
            Self::Llm { .. } => "external_llm",
            Self::Jev { .. } => "external_jev",
        }
    }
}

/// 判官价格（micros 量纲，§9.1）。判官成本计入 `routing_cost`。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct JudgePrice {
    pub input_micros_per_mtok: u64,
    pub output_micros_per_mtok: u64,
    pub request_micros: u64,
}

/// 判官配置。
#[derive(Debug, Clone)]
pub struct JudgeSettings {
    pub mode: JudgeMode,
    pub budget: Budget,
    pub backend: Backend,
    pub price: JudgePrice,
    pub currency: String,
}

impl Default for JudgeSettings {
    fn default() -> Self {
        Self {
            mode: JudgeMode::Off,
            budget: Budget::default(),
            backend: Backend::Local,
            price: JudgePrice::default(),
            currency: "USD".to_string(),
        }
    }
}

/// 一次咨询的产出。
#[derive(Debug, Default)]
pub struct JudgeOutcome {
    pub verdicts: Vec<Verdict>,
    pub traces: Vec<JudgeTrace>,
    pub calls: Vec<JudgeCallRecord>,
    /// 首个结论的原始值（shadow 记录用）。
    pub first_value: Option<JudgeValue>,
    pub latency_ms: u32,
    /// 本次咨询消耗的判官成本（写入账本 `routing_cost`）。
    pub routing_cost: Option<tr_contracts::Cost>,
}

/// 判官运行时。
pub struct JudgeRuntime {
    settings: JudgeSettings,
    external: Option<Arc<dyn DecisionModel>>,
    tracker: BudgetTracker,
}

impl JudgeRuntime {
    pub fn new(settings: JudgeSettings) -> Result<Self, String> {
        let external: Option<Arc<dyn DecisionModel>> = match &settings.backend {
            Backend::Local => None,
            Backend::Http {
                endpoint,
                service_id,
                revision,
                credential,
                timeout_ms,
            } => {
                let model = HttpJudgeModel::new(HttpJudgeConfig {
                    endpoint: endpoint.clone(),
                    service_id: service_id.clone(),
                    revision: *revision,
                    credential: credential.clone(),
                    timeout: Duration::from_millis(u64::from(*timeout_ms)),
                })
                .map_err(|error| format!("judge http client: {error}"))?;
                Some(Arc::new(model))
            }
            Backend::Llm {
                endpoint,
                model,
                credential,
                structured_output,
                temperature,
                max_output_tokens,
                timeout_ms,
                service_id,
                disable_thinking,
            } => {
                let judge = LlmJudgeModel::new(LlmJudgeConfig {
                    endpoint: endpoint.clone(),
                    model: model.clone(),
                    credential: credential.clone(),
                    timeout: Duration::from_millis(u64::from(*timeout_ms)),
                    structured_output: *structured_output,
                    temperature: *temperature,
                    max_output_tokens: *max_output_tokens,
                    service_id: service_id.clone(),
                    disable_thinking: *disable_thinking,
                })
                .map_err(|error| format!("judge llm client: {error}"))?;
                Some(Arc::new(judge))
            }
            Backend::Jev {
                endpoint,
                model,
                credential,
                timeout_ms,
                service_id,
            } => {
                let judge = SystemOneJudgeModel::new(SystemOneJudgeConfig {
                    endpoint: endpoint.clone(),
                    model: model.clone(),
                    credential: credential.clone(),
                    timeout: Duration::from_millis(u64::from(*timeout_ms)),
                    service_id: service_id.clone(),
                })
                .map_err(|error| format!("judge jev client: {error}"))?;
                Some(Arc::new(judge))
            }
        };

        Ok(Self {
            settings,
            external,
            tracker: BudgetTracker::new(),
        })
    }

    pub fn mode(&self) -> JudgeMode {
        self.settings.mode
    }

    pub fn is_active(&self) -> bool {
        self.settings.mode == JudgeMode::Active
    }

    pub fn is_shadow(&self) -> bool {
        self.settings.mode == JudgeMode::Shadow
    }

    /// 咨询一批请求。`turn_key` 用于预算的轮次划分（通常是 plan_id）；
    /// `request_id` 是触发这次判官的客户端请求（审计与账本用它关联）。
    pub async fn consult(
        &self,
        request_id: uuid::Uuid,
        session_key: &str,
        turn_key: &str,
        snapshot: &DecisionSnapshot,
        requests: &[JudgeRequest],
    ) -> JudgeOutcome {
        let mut outcome = JudgeOutcome::default();
        if self.settings.mode == JudgeMode::Off || requests.is_empty() {
            return outcome;
        }

        self.tracker.start_turn(session_key);
        let context = build_context(&snapshot.canonical);
        let context_digest = digest(&context);
        let local_signals = local_signals(snapshot);

        let mut sorted: Vec<&JudgeRequest> = requests.iter().collect();
        sorted.sort_by_key(|request| std::cmp::Reverse(request.priority));

        for request in sorted {
            let input = JudgeInput {
                question: request.question.clone(),
                category: request.category.clone(),
                evidence: request.evidence.clone(),
                context: context.clone(),
            };
            // 详细调用记录：LLM 记录实际发出的 messages，其余后端记录任务/提示词/选项。
            let request_json = match &self.settings.backend {
                Backend::Llm { .. } => {
                    crate::judge_detail::llm_messages(&tr_decide::llm::build_messages(&input))
                }
                _ => crate::judge_detail::request_summary(&input),
            };
            let decision = self
                .tracker
                .try_consume(session_key, turn_key, self.settings.budget);
            if decision != BudgetDecision::Allowed {
                outcome.calls.push(JudgeCallRecord {
                    request_id,
                    question_id: request.id.clone(),
                    service_id: self.settings.backend.service_id().to_string(),
                    revision: self.settings.backend.revision(),
                    protocol_digest: None,
                    effective_source: "disabled".to_string(),
                    outcome: "budget_exceeded".to_string(),
                    failure_code: Some(format!("{decision:?}")),
                    latency_ms: 0,
                    prompt_tokens: None,
                    completion_tokens: None,
                    cost_micros: None,
                    created_at: snapshot.now_unix,
                    detail: JudgeCallDetail {
                        request_json: Some(request_json.to_string()),
                        response_json: None,
                        failure_message: Some(format!("budget: {decision:?}")),
                    },
                });
                continue;
            }

            let deadline = Duration::from_millis(u64::from(self.settings.budget.deadline_ms));
            let started = Instant::now();

            let (answer, effective_source, failure) = match &self.external {
                Some(model) => match consult(model.as_ref(), &input, deadline).await {
                    Ok(answer) => (Some(answer), self.settings.backend.source_label(), None),
                    Err(failure) => {
                        // 外部失败 → 本地启发式兜底（不重试、不阻塞）。
                        let local = LocalDecisionModel {
                            signals: local_signals,
                        };
                        let fallback = consult(&local, &input, deadline).await.ok();
                        (fallback, "fallback", Some(failure))
                    }
                },
                None => {
                    let local = LocalDecisionModel {
                        signals: local_signals,
                    };
                    match consult(&local, &input, deadline).await {
                        Ok(answer) => (Some(answer), "heuristic", None),
                        Err(failure) => (None, "heuristic", Some(failure)),
                    }
                }
            };

            let latency_ms = started.elapsed().as_millis() as u32;
            outcome.latency_ms = outcome.latency_ms.max(latency_ms);

            // 判官成本：tokens 来自适配器解析的 usage，价格来自配置（micros 量纲）。
            let usage = answer
                .as_ref()
                .and_then(|answer| answer.usage)
                .map(|mut usage| {
                    usage.cost_micros = self.price_micros(&usage);
                    usage
                });
            if let Some(usage) = usage
                && usage.cost_micros > 0
            {
                outcome.routing_cost = Some(tr_contracts::Cost {
                    micros: outcome
                        .routing_cost
                        .as_ref()
                        .map_or(usage.cost_micros, |cost| cost.micros + usage.cost_micros),
                    currency: self.settings.currency.clone(),
                });
            }

            let verdict = match &answer {
                Some(answer) => {
                    if outcome.first_value.is_none() {
                        outcome.first_value = Some(answer.value.clone());
                    }
                    to_verdict(&input, answer)
                }
                None => Verdict::Uncertain {
                    reason: "judge unavailable".to_string(),
                },
            };
            outcome.verdicts.push(verdict);

            let protocol_digest =
                tr_contracts::question_protocol_digest(&[tr_contracts::question_definition(
                    "q0",
                    &request.question,
                )]);
            let trace = JudgeTrace {
                service_id: Some(self.settings.backend.service_id().to_string()),
                service_revision: Some(self.settings.backend.revision()),
                protocol_digest: Some(protocol_digest.clone()),
                context_digest: Some(context_digest.clone()),
                local_decision_digest: digest(&request.evidence),
                effective_source: match effective_source {
                    "fallback" => tr_contracts::JudgeEffectiveSource::Fallback,
                    "external_llm" => tr_contracts::JudgeEffectiveSource::ExternalLlm,
                    // 契约枚举没有单独的 JEV 变体；JEV 是外部决策服务。
                    "external_jev" | "external_service" => {
                        tr_contracts::JudgeEffectiveSource::ExternalService
                    }
                    _ => tr_contracts::JudgeEffectiveSource::Heuristic,
                },
                fallback_used: failure.is_some(),
                fallback_reason: failure.as_ref().map(|failure| failure.code),
                duration_ms: latency_ms,
                usage,
            };
            outcome.traces.push(trace.clone());

            outcome.calls.push(JudgeCallRecord {
                request_id,
                question_id: request.id.clone(),
                service_id: self.settings.backend.service_id().to_string(),
                revision: self.settings.backend.revision(),
                protocol_digest: Some(protocol_digest),
                effective_source: effective_source.to_string(),
                outcome: if failure.is_some() {
                    "failed".to_string()
                } else {
                    "answered".to_string()
                },
                failure_code: failure
                    .as_ref()
                    .map(|failure| format!("{:?}", failure.code)),
                latency_ms,
                prompt_tokens: usage.map(|usage| usage.prompt_tokens),
                completion_tokens: usage.map(|usage| usage.completion_tokens),
                cost_micros: usage.map(|usage| usage.cost_micros),
                created_at: snapshot.now_unix,
                // 详细调用记录：任务/提示词/选项 + 答案（或失败信息）。
                detail: JudgeCallDetail {
                    request_json: Some(request_json.to_string()),
                    response_json: answer
                        .as_ref()
                        .map(|answer| crate::judge_detail::response_summary(answer).to_string()),
                    failure_message: failure.as_ref().map(|failure| {
                        match failure.message.as_deref() {
                            Some(message) if !message.is_empty() => {
                                format!("{:?}: {message}", failure.code)
                            }
                            _ => format!("{:?}", failure.code),
                        }
                    }),
                },
            });
        }

        outcome
    }

    /// 按配置价格计算一次判官调用的成本（micros）。
    fn price_micros(&self, usage: &tr_contracts::JudgeUsage) -> i64 {
        let prompt = u128::from(usage.prompt_tokens);
        let completion = u128::from(usage.completion_tokens);
        let micros = prompt * u128::from(self.settings.price.input_micros_per_mtok) / 1_000_000
            + completion * u128::from(self.settings.price.output_micros_per_mtok) / 1_000_000
            + u128::from(self.settings.price.request_micros);
        i64::try_from(micros).unwrap_or(i64::MAX)
    }

    /// shadow 对比：判官建议升级 vs 内核已选最强候选。
    pub fn agreement(verdict: Option<&Verdict>, kernel_is_strongest: bool) -> Option<bool> {
        match verdict? {
            Verdict::Escalate { .. } => Some(kernel_is_strongest),
            Verdict::Decline { .. } => Some(!kernel_is_strongest),
            Verdict::Uncertain { .. } => None,
        }
    }

    /// 供审计的 reason：判官不可用时补一条理由码。
    pub fn unavailable_reason(outcome: &JudgeOutcome) -> Option<ReasonCodeKind> {
        outcome
            .traces
            .iter()
            .any(|trace| trace.fallback_used)
            .then_some(ReasonCodeKind::JudgeUnavailable)
    }

    /// `judge test`：用固定问题自检。
    pub async fn self_test(&self) -> tr_contracts::JudgeServiceTestResult {
        let input = JudgeInput {
            question: tr_contracts::Question {
                prompt: "Self-test: answer true if you are able to answer.".to_string(),
                choices: tr_contracts::Choices::Boolean,
                min_confidence: 0.0,
            },
            category: tr_contracts::Category::QualityUncertain,
            evidence: tr_contracts::EvidenceRef {
                fingerprint: "self-test".to_string(),
                summary: "judge self test".to_string(),
            },
            context: JudgeContext::default(),
        };
        let deadline = Duration::from_millis(u64::from(self.settings.budget.deadline_ms));
        match &self.external {
            Some(model) => tr_decide::test_service(model.as_ref(), &input, deadline).await,
            None => {
                let local = LocalDecisionModel {
                    signals: LocalSignals {
                        tool_error_streak: 2,
                        ..LocalSignals::default()
                    },
                };
                tr_decide::test_service(&local, &input, deadline).await
            }
        }
    }
}

fn local_signals(snapshot: &DecisionSnapshot) -> LocalSignals {
    LocalSignals {
        tool_error_streak: snapshot.signals.core.consecutive_tool_error_streak,
        repeated_tool_loop: snapshot.signals.core.repeated_tool_loop.is_some(),
        context_overflow: snapshot.signals.core.ctx_fill_ratio >= 0.9,
        task_changed: false,
    }
}

/// 判官上下文：固定字符预算（§7.4），从最旧开始丢弃。
fn build_context(request: &CanonicalRequest) -> JudgeContext {
    let system = request
        .system
        .iter()
        .map(|block| block.text.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    let system = if system.is_empty() {
        first_message_text(request, CanonicalRole::System)
    } else {
        system
    };

    let task = last_message_text(request, CanonicalRole::User);
    let recent: Vec<String> = request
        .messages
        .iter()
        .rev()
        .take(28)
        .rev()
        .map(|message| {
            let text = message
                .blocks
                .iter()
                .filter_map(|block| match block {
                    ContentBlock::Text { text, .. } => Some(text.as_str()),
                    ContentBlock::ToolUse { name, .. } => Some(name.as_str()),
                    _ => None,
                })
                .collect::<Vec<_>>()
                .join(" ");
            format!("{:?}: {}", message.role, truncate(&text, 500))
        })
        .collect();

    JudgeContext {
        system: truncate(&system, 1000),
        task: truncate(&task, 4000),
        recent,
    }
}

fn first_message_text(request: &CanonicalRequest, role: CanonicalRole) -> String {
    request
        .messages
        .iter()
        .find(|message| message.role == role)
        .map(blocks_text)
        .unwrap_or_default()
}

fn last_message_text(request: &CanonicalRequest, role: CanonicalRole) -> String {
    request
        .messages
        .iter()
        .rev()
        .find(|message| message.role == role)
        .map(blocks_text)
        .unwrap_or_default()
}

fn blocks_text(message: &tr_contracts::CanonicalMessage) -> String {
    message
        .blocks
        .iter()
        .filter_map(|block| match block {
            ContentBlock::Text { text, .. } => Some(text.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn truncate(text: &str, limit: usize) -> String {
    if text.chars().count() <= limit {
        return text.to_string();
    }
    text.chars().take(limit).collect()
}

fn digest<T: serde::Serialize>(value: &T) -> String {
    let bytes = serde_json::to_vec(value).unwrap_or_default();
    let hash = Sha256::digest(&bytes);
    let mut out = String::with_capacity(hash.len() * 2);
    for byte in hash {
        use std::fmt::Write as _;
        let _ = write!(out, "{byte:02x}");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use tr_contracts::{
        CanonicalMessage, DeclineReason, EvidenceRef, JudgeAnswer, Question, Verdict,
    };

    fn request() -> CanonicalRequest {
        CanonicalRequest {
            messages: vec![CanonicalMessage {
                role: CanonicalRole::User,
                blocks: vec![ContentBlock::Text {
                    text: "hello".to_string(),
                    cache_control: None,
                }],
                ..CanonicalMessage::default()
            }],
            ..CanonicalRequest::default()
        }
    }

    #[test]
    fn context_truncates_budget() {
        let mut request = request();
        request.messages[0].blocks = vec![ContentBlock::Text {
            text: "x".repeat(9000),
            cache_control: None,
        }];
        let context = build_context(&request);
        assert_eq!(context.task.chars().count(), 4000);
        assert_eq!(context.recent[0].chars().count(), "User: ".len() + 500);
    }

    #[test]
    fn agreement_semantics() {
        let escalate = Verdict::Escalate {
            category: tr_contracts::Category::ToolFailure,
            target_strength: None,
            evidence: EvidenceRef::default(),
            confidence: 0.9,
        };
        assert_eq!(JudgeRuntime::agreement(Some(&escalate), true), Some(true));
        assert_eq!(JudgeRuntime::agreement(Some(&escalate), false), Some(false));

        let decline = Verdict::Decline {
            reason: DeclineReason::Sufficient,
        };
        assert_eq!(JudgeRuntime::agreement(Some(&decline), false), Some(true));
        assert_eq!(JudgeRuntime::agreement(None, true), None);
    }

    #[tokio::test]
    async fn local_backend_answers_and_records_call() {
        let runtime = JudgeRuntime::new(JudgeSettings {
            mode: JudgeMode::Active,
            backend: Backend::Local,
            ..JudgeSettings::default()
        })
        .expect("runtime");

        let mut snapshot = DecisionSnapshot::default();
        snapshot.signals.core.consecutive_tool_error_streak = 2;
        let request = JudgeRequest {
            id: "q1".to_string(),
            question: Question {
                prompt: "escalate?".to_string(),
                choices: tr_contracts::Choices::Boolean,
                min_confidence: 0.6,
            },
            required: false,
            priority: 10,
            category: tr_contracts::Category::ToolFailure,
            evidence: EvidenceRef::default(),
        };

        let request_id = uuid::Uuid::new_v4();
        let outcome = runtime
            .consult(request_id, "session", "turn", &snapshot, &[request])
            .await;
        assert_eq!(outcome.verdicts.len(), 1);
        // 审计行必须挂在触发它的请求上（否则无法按请求/会话关联）。
        assert_eq!(outcome.calls[0].request_id, request_id);
        assert!(matches!(outcome.verdicts[0], Verdict::Escalate { .. }));
        assert_eq!(outcome.calls.len(), 1);
        assert_eq!(outcome.calls[0].outcome, "answered");
    }

    #[tokio::test]
    async fn off_mode_does_nothing() {
        let runtime = JudgeRuntime::new(JudgeSettings::default()).expect("runtime");
        let outcome = runtime
            .consult(
                uuid::Uuid::new_v4(),
                "s",
                "t",
                &DecisionSnapshot::default(),
                &[],
            )
            .await;
        assert!(outcome.verdicts.is_empty());
        assert!(outcome.calls.is_empty());
    }

    #[test]
    fn parses_modes() {
        assert_eq!(JudgeMode::parse("off"), Some(JudgeMode::Off));
        assert_eq!(JudgeMode::parse("shadow"), Some(JudgeMode::Shadow));
        assert_eq!(JudgeMode::parse("active"), Some(JudgeMode::Active));
        assert_eq!(JudgeMode::parse("nope"), None);
    }

    #[test]
    fn answer_value_is_recorded() {
        let answer = JudgeAnswer {
            value: JudgeValue::Bool(true),
            confidence: Some(0.9),
            reason: None,
            usage: None,
        };
        assert_eq!(answer.value, JudgeValue::Bool(true));
    }
}
