//! 组级判断（M1）：在 agent 层为单个虚拟模型的候选组选一个规则/候选。
//!
//! 与全局判官（[`crate::judge`]）分离：
//! - 独立预算（turn/minute/session + deadline），**不复用** `JudgeRuntime::consult`；
//! - 只接受 LLM / JEV 后端；规则/能力两种选择模式；
//! - 失败、超时、非法答案、`none` 一律 fail-open：返回原因码，不阻塞请求；
//! - 不可回放：只落 `judge_calls` / 账本，不改变 `core` 语义。

use std::sync::Arc;
use std::time::{Duration, Instant};

use tr_contracts::{
    Candidate, CandidateId, CanonicalRequest, CanonicalRole, Category, Choice, Choices,
    ContentBlock, Cost, EvidenceRef, InputModality, JudgeAnswer, JudgeContext, JudgeInput,
    JudgeUsage, JudgeValue, Question,
};
use tr_decide::budget::{BudgetDecision, BudgetTracker};
use tr_decide::llm::{LlmJudgeConfig, LlmJudgeModel};
use tr_decide::system_one::{SystemOneJudgeConfig, SystemOneJudgeModel};
use tr_decide::{DecisionModel, consult};

use crate::config::GroupRule;
use crate::judge::{Backend, JudgeMode, JudgeSettings};
use crate::store::judge::{JudgeCallDetail, JudgeCallRecord};

/// `none` 是保留答案：判官认为没有规则/候选适用。
const NONE: &str = "none";

/// 组判断的选择结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GroupSelection {
    /// 规则命中：使用规则声明的 `prefer` 候选。
    Rule { id: String, prefer: CandidateId },
    /// 能力选择：直接使用候选 id。
    Candidate(CandidateId),
}

/// 一次组判断的产出。
#[derive(Debug)]
pub struct GroupJudgeOutcome {
    /// 命中时的选择；失败/`none` 时为 `None`。
    pub selection: Option<GroupSelection>,
    /// 审计记录（无论成败都写一条）。
    pub call: JudgeCallRecord,
    /// 本次调用成本（写入账本 `routing_cost`）。
    pub cost: Option<Cost>,
    pub latency_ms: u32,
    /// 失败描述（只进日志，不进入用户错误路径）。
    pub failure: Option<String>,
    /// 判官明确表示没有匹配项（合法答案，不是失败）。
    pub no_match: bool,
}

/// 组级判断运行时。
pub struct GroupJudgeRuntime {
    settings: JudgeSettings,
    external: Option<Arc<dyn DecisionModel>>,
    tracker: BudgetTracker,
}

impl GroupJudgeRuntime {
    /// `off` → 无模型；`active + llm/jev` → 构造对应判官；其他组合拒绝。
    pub fn new(settings: JudgeSettings) -> Result<Self, String> {
        let external: Option<Arc<dyn DecisionModel>> = match settings.mode {
            JudgeMode::Off => None,
            JudgeMode::Shadow => {
                return Err("组判断不支持 shadow，只能是 off/active".to_string());
            }
            JudgeMode::Active => match &settings.backend {
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
                    .map_err(|error| format!("group judge llm client: {error}"))?;
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
                    .map_err(|error| format!("group judge jev client: {error}"))?;
                    Some(Arc::new(judge))
                }
                // 配置层已拦截；运行时防御性拒绝。
                Backend::Local | Backend::Http { .. } => {
                    return Err("组判断 active 时只支持 llm/jev 后端".to_string());
                }
            },
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

    /// 选择规则或候选。
    ///
    /// `rules` 非空 → 规则模式；为空 → 能力模式。
    #[allow(clippy::too_many_arguments)]
    pub async fn select(
        &self,
        request_id: uuid::Uuid,
        session_key: &str,
        turn_key: &str,
        vm_id: &str,
        candidates: &[Candidate],
        rules: &[GroupRule],
        canonical: &CanonicalRequest,
        now_unix: u64,
    ) -> GroupJudgeOutcome {
        let started = Instant::now();
        let question = build_question(candidates, rules);
        let protocol_digest =
            tr_contracts::question_protocol_digest(&[tr_contracts::question_definition(
                "group_judge",
                &question,
            )]);
        let input = JudgeInput {
            question,
            category: Category::Other("group_judge".to_string()),
            evidence: EvidenceRef {
                fingerprint: format!("group:{vm_id}:{}", rules.len()),
                summary: "group judge selection".to_string(),
            },
            // 隐私：只发最后一条 user 消息；system/tool 内容一律不进判官上下文。
            context: JudgeContext {
                system: String::new(),
                task: truncate(&last_user_text(canonical), 4000),
                recent: Vec::new(),
            },
        };
        // 详细调用记录：LLM 记录实际发出的 messages（与上游同一构造），
        // JEV 记录任务/提示词/选项；输出在拿到答案后补。
        let request_json = match &self.settings.backend {
            Backend::Llm { .. } => {
                crate::judge_detail::llm_messages(&tr_decide::llm::build_messages(&input))
                    .to_string()
            }
            _ => crate::judge_detail::request_summary(&input).to_string(),
        };

        self.tracker.start_turn(session_key);
        let decision = self
            .tracker
            .try_consume(session_key, turn_key, self.settings.budget);
        if decision != BudgetDecision::Allowed {
            let latency_ms = started.elapsed().as_millis() as u32;
            let failure = format!("group judge budget exceeded: {decision:?}");
            return GroupJudgeOutcome {
                selection: None,
                call: self.call_record(
                    request_id,
                    vm_id,
                    now_unix,
                    "unavailable",
                    Some(format!("{decision:?}")),
                    latency_ms,
                    None,
                    protocol_digest,
                    "disabled",
                    JudgeCallDetail {
                        request_json: Some(request_json.clone()),
                        response_json: None,
                        failure_message: Some(failure.clone()),
                    },
                ),
                cost: None,
                latency_ms,
                failure: Some(failure),
                no_match: false,
            };
        }

        let Some(model) = &self.external else {
            let latency_ms = started.elapsed().as_millis() as u32;
            let failure = "group judge model not configured".to_string();
            return GroupJudgeOutcome {
                selection: None,
                call: self.call_record(
                    request_id,
                    vm_id,
                    now_unix,
                    "unavailable",
                    Some("NoModel".to_string()),
                    latency_ms,
                    None,
                    protocol_digest,
                    "disabled",
                    JudgeCallDetail {
                        request_json: Some(request_json.clone()),
                        response_json: None,
                        failure_message: Some(failure.clone()),
                    },
                ),
                cost: None,
                latency_ms,
                failure: Some(failure),
                no_match: false,
            };
        };

        let deadline = Duration::from_millis(u64::from(self.settings.budget.deadline_ms));
        let result = consult(model.as_ref(), &input, deadline).await;
        let latency_ms = started.elapsed().as_millis() as u32;

        let (selection, no_match, failure, outcome_label) = match &result {
            Ok(answer) => {
                let (selection, no_match, failure) =
                    interpret_answer(vm_id, answer, candidates, rules);
                let label = if selection.is_some() {
                    "selected"
                } else if no_match {
                    "no_match"
                } else {
                    "unavailable"
                };
                (selection, no_match, failure, label)
            }
            Err(error) => (
                None,
                false,
                Some(format!(
                    "{:?}: {}",
                    error.code,
                    error.message.as_deref().unwrap_or("")
                )),
                "unavailable",
            ),
        };

        let usage = result
            .as_ref()
            .ok()
            .and_then(|answer| answer.usage)
            .map(|mut usage| {
                usage.cost_micros = self.price_micros(&usage);
                usage
            });
        let cost = usage.map(|usage| Cost {
            micros: usage.cost_micros,
            currency: self.settings.currency.clone(),
        });

        let response_json = result
            .as_ref()
            .ok()
            .map(|answer| crate::judge_detail::response_summary(answer).to_string());
        GroupJudgeOutcome {
            selection,
            call: self.call_record(
                request_id,
                vm_id,
                now_unix,
                outcome_label,
                failure.as_ref().and_then(|_| failure_code(&result)),
                latency_ms,
                usage,
                protocol_digest,
                backend_source_label(&self.settings.backend),
                JudgeCallDetail {
                    request_json: Some(request_json),
                    response_json,
                    failure_message: failure.clone(),
                },
            ),
            cost,
            latency_ms,
            failure,
            no_match,
        }
    }

    /// 按配置价格计算一次组判断的成本（micros）。
    fn price_micros(&self, usage: &JudgeUsage) -> i64 {
        let prompt = u128::from(usage.prompt_tokens);
        let completion = u128::from(usage.completion_tokens);
        let micros = prompt * u128::from(self.settings.price.input_micros_per_mtok) / 1_000_000
            + completion * u128::from(self.settings.price.output_micros_per_mtok) / 1_000_000
            + u128::from(self.settings.price.request_micros);
        i64::try_from(micros).unwrap_or(i64::MAX)
    }

    #[allow(clippy::too_many_arguments)]
    fn call_record(
        &self,
        request_id: uuid::Uuid,
        vm_id: &str,
        now_unix: u64,
        outcome: &str,
        failure_code: Option<String>,
        latency_ms: u32,
        usage: Option<JudgeUsage>,
        protocol_digest: String,
        effective_source: &str,
        detail: JudgeCallDetail,
    ) -> JudgeCallRecord {
        JudgeCallRecord {
            request_id,
            question_id: format!("group:{vm_id}"),
            service_id: backend_service_id(&self.settings.backend),
            revision: 1,
            protocol_digest: Some(protocol_digest),
            effective_source: effective_source.to_string(),
            outcome: outcome.to_string(),
            failure_code,
            latency_ms,
            prompt_tokens: usage.map(|usage| usage.prompt_tokens),
            completion_tokens: usage.map(|usage| usage.completion_tokens),
            cost_micros: usage.map(|usage| usage.cost_micros),
            created_at: now_unix,
            detail,
        }
    }
}

fn backend_service_id(backend: &Backend) -> String {
    match backend {
        Backend::Local => "local-heuristic".to_string(),
        Backend::Http { service_id, .. }
        | Backend::Llm { service_id, .. }
        | Backend::Jev { service_id, .. } => service_id.clone(),
    }
}

/// 审计里的来源标识：LLM 与 JEV 只在传输层不同，但审计保留后端区分。
fn backend_source_label(backend: &Backend) -> &'static str {
    match backend {
        Backend::Jev { .. } => "external_jev",
        _ => "external_llm",
    }
}

fn failure_code(result: &Result<JudgeAnswer, tr_contracts::JudgeFailure>) -> Option<String> {
    match result {
        Ok(_) => Some("MalformedOutput".to_string()),
        Err(error) => Some(format!("{:?}", error.code)),
    }
}

/// 把合法答案映射为选择；非法形状按失败处理（fail-open）。
fn interpret_answer(
    vm_id: &str,
    answer: &JudgeAnswer,
    candidates: &[Candidate],
    rules: &[GroupRule],
) -> (Option<GroupSelection>, bool, Option<String>) {
    match &answer.value {
        JudgeValue::Choice(choice) if choice == NONE => (None, true, None),
        JudgeValue::Choice(choice) => {
            if rules.is_empty() {
                if let Some(candidate) = candidates
                    .iter()
                    .find(|candidate| candidate.id == *choice && candidate.enabled)
                {
                    return (
                        Some(GroupSelection::Candidate(candidate.id.clone())),
                        false,
                        None,
                    );
                }
                (
                    None,
                    false,
                    Some(format!(
                        "group judge chose unknown or disabled candidate `{choice}` for `{vm_id}`"
                    )),
                )
            } else if let Some(rule) = rules.iter().find(|rule| rule.id == *choice) {
                if !candidates
                    .iter()
                    .any(|candidate| candidate.id == rule.prefer && candidate.enabled)
                {
                    return (
                        None,
                        false,
                        Some(format!(
                            "group rule `{}` prefers unavailable candidate `{}` for `{vm_id}`",
                            rule.id, rule.prefer
                        )),
                    );
                }
                (
                    Some(GroupSelection::Rule {
                        id: rule.id.clone(),
                        prefer: rule.prefer.clone(),
                    }),
                    false,
                    None,
                )
            } else {
                (
                    None,
                    false,
                    Some(format!(
                        "group judge chose unknown rule `{choice}` for `{vm_id}`"
                    )),
                )
            }
        }
        // 明确不确定：合法但不可决策。
        JudgeValue::Uncertain => (None, true, None),
        other => (
            None,
            false,
            Some(format!("unexpected group judge value: {other:?}")),
        ),
    }
}

fn build_question(candidates: &[Candidate], rules: &[GroupRule]) -> Question {
    if rules.is_empty() {
        capability_question(candidates)
    } else {
        rules_question(rules)
    }
}

/// 规则模式：按配置顺序展示规则，答案必须是规则 id 或 `none`。
fn rules_question(rules: &[GroupRule]) -> Question {
    let mut prompt = String::from(
        "Select the routing rule that best matches the task. Rules are listed in precedence \
order; choose the first rule id whose intent applies. If no rule applies, answer \"none\".\n\
RULES:\n",
    );
    for (index, rule) in rules.iter().enumerate() {
        prompt.push_str(&format!("{}. {}: {}\n", index + 1, rule.id, rule.intent));
    }

    let mut choices: Vec<Choice> = rules
        .iter()
        .map(|rule| Choice {
            id: rule.id.clone(),
            label: rule.intent.clone(),
        })
        .collect();
    choices.push(Choice {
        id: NONE.to_string(),
        label: "no rule applies".to_string(),
    });

    Question {
        prompt,
        choices: Choices::Enum(choices),
        min_confidence: 0.0,
    }
}

/// 能力模式：展示候选能力摘要，答案必须是候选 id 或 `none`。
fn capability_question(candidates: &[Candidate]) -> Question {
    let mut prompt = String::from(
        "Select the candidate model that best fits the task. Choose exactly one candidate id \
listed below. If no candidate is appropriate, answer \"none\".\nCANDIDATES:\n",
    );
    let mut choices = Vec::with_capacity(candidates.len() + 1);
    for candidate in candidates.iter().filter(|candidate| candidate.enabled) {
        prompt.push_str(&format!(
            "- {}: tier={}, strength={}, window={}, input=[{}], thinking=[{}], tags={{{}}}, price={}\n",
            candidate.id,
            candidate.tier,
            candidate.strength,
            candidate.window,
            modalities(&candidate.capability.input),
            if candidate.capability.thinking_levels.is_empty() {
                String::new()
            } else {
                candidate.capability.thinking_levels.join(",")
            },
            candidate
                .capability
                .tags
                .iter()
                .map(|(key, value)| format!("{key}={value}"))
                .collect::<Vec<_>>()
                .join(";"),
            price_summary(candidate),
        ));
        choices.push(Choice {
            id: candidate.id.clone(),
            label: format!(
                "{} ({}, strength {})",
                candidate.id, candidate.tier, candidate.strength
            ),
        });
    }
    choices.push(Choice {
        id: NONE.to_string(),
        label: "no candidate is appropriate".to_string(),
    });

    Question {
        prompt,
        choices: Choices::Enum(choices),
        min_confidence: 0.0,
    }
}

fn modalities(input: &[InputModality]) -> String {
    input
        .iter()
        .map(|modality| match modality {
            InputModality::Text => "text",
            InputModality::Image => "image",
            InputModality::Audio => "audio",
        })
        .collect::<Vec<_>>()
        .join(",")
}

fn price_summary(candidate: &Candidate) -> String {
    match &candidate.price {
        Some(price) => format!(
            "{} input {} / output {} micros per Mtok, request {} micros",
            price.currency,
            price.input_micros_per_mtok,
            price.output_micros_per_mtok,
            price.request_micros
        ),
        None => "unknown".to_string(),
    }
}

/// 最后一条 user 消息（只取 text 块）。
fn last_user_text(request: &CanonicalRequest) -> String {
    request
        .messages
        .iter()
        .rev()
        .find(|message| message.role == CanonicalRole::User)
        .map(|message| {
            message
                .blocks
                .iter()
                .filter_map(|block| match block {
                    ContentBlock::Text { text, .. } => Some(text.as_str()),
                    _ => None,
                })
                .collect::<Vec<_>>()
                .join("\n")
        })
        .unwrap_or_default()
}

fn truncate(text: &str, limit: usize) -> String {
    if text.chars().count() <= limit {
        return text.to_string();
    }
    text.chars().take(limit).collect()
}
