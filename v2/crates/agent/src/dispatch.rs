//! StickyDispatch + ContextHold 融合（§4.9.1，P3）。
//!
//! 优先级：`Retry.failed > Continuation.previous > ContextHold > Plan`。
//! - Retry/Continuation 是**硬复用**：直接复用上一物理模型；
//! - ContextHold 是**软亲和**：把上一成功候选排到队首，Plan 仍可改选；
//! - `Direct` 不读写 state。
//!
//! 不可用时绝不阻塞请求：回退 Plan 并写明确 reason。

use tr_contracts::{
    Candidate, CandidateHealth, CandidateId, ContextHold, Dispatch, DispatchSource, InputModality,
    ReasonCode, ReasonCodeKind, RequestReason,
};

/// 解析输入。
pub struct DispatchContext<'a> {
    pub reason: RequestReason,
    pub previous: Option<&'a Dispatch>,
    pub hold: Option<&'a ContextHold>,
    pub candidates: &'a [Candidate],
    pub health: &'a CandidateHealth,
    pub now_unix: u64,
    /// 估算输入 token（用于窗口检查）。
    pub prompt_tokens: u32,
    pub required_input: Vec<InputModality>,
    pub virtual_model_id: &'a str,
    pub profile_id: &'a str,
    pub ingress_protocol: &'a str,
    pub identity_digest: &'a str,
    pub group_digest: &'a str,
    pub plan_revision: u64,
    /// route 是否显式要求 Direct 也复用 previous。
    pub direct_opts_into_sticky: bool,
}

/// 解析结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Resolution {
    pub candidate: Option<CandidateId>,
    pub source: DispatchSource,
    pub reasons: Vec<ReasonCode>,
}

/// 决定初始 dispatch。
pub fn resolve(context: &DispatchContext<'_>) -> Resolution {
    let mut reasons = Vec::new();

    if context.reason == RequestReason::Direct {
        if context.direct_opts_into_sticky
            && let Some(previous) = context.previous
            && let Some(resolved) = reusable(context, &previous.physical_model, &mut reasons)
        {
            return Resolution {
                candidate: Some(resolved),
                source: DispatchSource::StickyPrevious,
                reasons,
            };
        }
        return Resolution {
            candidate: None,
            source: DispatchSource::Strategy,
            reasons,
        };
    }

    if matches!(
        context.reason,
        RequestReason::Retry | RequestReason::Continuation
    ) && let Some(previous) = context.previous
        && let Some(resolved) = reusable(context, &previous.physical_model, &mut reasons)
    {
        return Resolution {
            candidate: Some(resolved),
            source: DispatchSource::StickyPrevious,
            reasons,
        };
    }

    // ContextHold：软亲和。
    if let Some(hold) = context.hold {
        if hold.virtual_model_id == context.virtual_model_id
            && hold.profile_id == context.profile_id
            && hold.ingress_protocol == context.ingress_protocol
            && hold.identity_digest == context.identity_digest
            && hold.group_digest == context.group_digest
            && hold.plan_revision == context.plan_revision
            && context.now_unix < hold.expires_at
        {
            if let Some(candidate) = eligible(context, &hold.candidate_id, &mut reasons) {
                reasons.push(ReasonCode::new(ReasonCodeKind::ContextHoldApplied));
                return Resolution {
                    candidate: Some(candidate),
                    source: DispatchSource::ContextHold,
                    reasons,
                };
            }
        } else {
            reasons.push(ReasonCode::new(ReasonCodeKind::ContextHoldInvalidated));
        }
    }

    Resolution {
        candidate: None,
        source: DispatchSource::Strategy,
        reasons,
    }
}

/// previous 是物理模型名；按 `upstream.model` 找回候选。
fn reusable(
    context: &DispatchContext<'_>,
    physical_model: &str,
    reasons: &mut Vec<ReasonCode>,
) -> Option<CandidateId> {
    let candidate = context
        .candidates
        .iter()
        .find(|candidate| candidate.upstream.model == physical_model)?;
    eligible(context, &candidate.id, reasons)
}

fn eligible(
    context: &DispatchContext<'_>,
    candidate_id: &str,
    reasons: &mut Vec<ReasonCode>,
) -> Option<CandidateId> {
    let Some(candidate) = context
        .candidates
        .iter()
        .find(|candidate| candidate.id == candidate_id)
    else {
        reasons.push(ReasonCode::with_detail(
            ReasonCodeKind::StickyTargetUnavailable,
            candidate_id,
        ));
        return None;
    };

    if !candidate.enabled {
        reasons.push(ReasonCode::with_detail(
            ReasonCodeKind::StickyTargetUnavailable,
            candidate_id,
        ));
        return None;
    }
    if context
        .required_input
        .iter()
        .any(|modality| *modality != InputModality::Text && !candidate.supports_input(*modality))
    {
        reasons.push(ReasonCode::with_detail(
            ReasonCodeKind::StickyInputUnsupported,
            candidate_id,
        ));
        return None;
    }
    if candidate.window < context.prompt_tokens {
        reasons.push(ReasonCode::with_detail(
            ReasonCodeKind::StickyWindowExceeded,
            candidate_id,
        ));
        return None;
    }
    if !context.health.can_attempt(&candidate.id, context.now_unix) {
        reasons.push(ReasonCode::with_detail(
            ReasonCodeKind::StickyTargetUnavailable,
            candidate_id,
        ));
        return None;
    }

    Some(candidate.id.clone())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tr_contracts::{CapabilityProfile, ProviderKind, UpstreamRef};

    fn candidate(id: &str, model: &str, strength: u8) -> Candidate {
        Candidate {
            id: id.to_string(),
            tier: "cloud".to_string(),
            strength,
            upstream: UpstreamRef {
                provider: ProviderKind::OpenAiCompatible,
                base_url: "http://x".to_string(),
                model: model.to_string(),
                credential: None,
            },
            window: 128_000,
            capability: CapabilityProfile {
                input: vec![InputModality::Text],
                ..CapabilityProfile::default()
            },
            price: None,
            enabled: true,
        }
    }

    fn context<'a>(
        reason: RequestReason,
        previous: Option<&'a Dispatch>,
        hold: Option<&'a ContextHold>,
        candidates: &'a [Candidate],
        health: &'a CandidateHealth,
    ) -> DispatchContext<'a> {
        DispatchContext {
            reason,
            previous,
            hold,
            candidates,
            health,
            now_unix: 1_700_000_000,
            prompt_tokens: 100,
            required_input: vec![InputModality::Text],
            virtual_model_id: "auto",
            profile_id: "default",
            ingress_protocol: "chat_completions",
            identity_digest: "session",
            group_digest: "group",
            plan_revision: 1,
            direct_opts_into_sticky: false,
        }
    }

    #[test]
    fn continuation_reuses_previous() {
        let candidates = [
            candidate("weak", "weak-physical", 10),
            candidate("strong", "strong-physical", 80),
        ];
        let health = CandidateHealth::default();
        let previous = Dispatch {
            physical_model: "strong-physical".to_string(),
            provider: "OpenAiCompatible".to_string(),
            thinking_level: "off".to_string(),
        };

        let resolution = resolve(&context(
            RequestReason::Continuation,
            Some(&previous),
            None,
            &candidates,
            &health,
        ));
        assert_eq!(resolution.candidate.as_deref(), Some("strong"));
        assert_eq!(resolution.source, DispatchSource::StickyPrevious);
    }

    #[test]
    fn direct_ignores_previous() {
        let candidates = [candidate("strong", "strong-physical", 80)];
        let health = CandidateHealth::default();
        let previous = Dispatch {
            physical_model: "strong-physical".to_string(),
            provider: "OpenAiCompatible".to_string(),
            thinking_level: "off".to_string(),
        };

        let resolution = resolve(&context(
            RequestReason::Direct,
            Some(&previous),
            None,
            &candidates,
            &health,
        ));
        assert_eq!(resolution.candidate, None);
        assert_eq!(resolution.source, DispatchSource::Strategy);
    }

    #[test]
    fn unavailable_previous_falls_back_with_reason() {
        let candidates = [candidate("strong", "strong-physical", 80)];
        let mut health = CandidateHealth::default();
        health.candidates.insert(
            "strong".to_string(),
            tr_contracts::BreakerState {
                open_until: Some(1_700_000_100),
                ..Default::default()
            },
        );
        let previous = Dispatch {
            physical_model: "strong-physical".to_string(),
            provider: "OpenAiCompatible".to_string(),
            thinking_level: "off".to_string(),
        };

        let resolution = resolve(&context(
            RequestReason::Continuation,
            Some(&previous),
            None,
            &candidates,
            &health,
        ));
        assert_eq!(resolution.candidate, None);
        assert!(
            resolution
                .reasons
                .iter()
                .any(|reason| reason.code == ReasonCodeKind::StickyTargetUnavailable)
        );
    }

    #[test]
    fn user_turn_uses_valid_hold() {
        let candidates = [candidate("strong", "strong-physical", 80)];
        let health = CandidateHealth::default();
        let hold = ContextHold {
            candidate_id: "strong".to_string(),
            virtual_model_id: "auto".to_string(),
            group_digest: "group".to_string(),
            plan_revision: 1,
            profile_id: "default".to_string(),
            ingress_protocol: "chat_completions".to_string(),
            identity_digest: "session".to_string(),
            updated_at: 1_699_999_000,
            expires_at: 1_700_000_100,
        };

        let resolution = resolve(&context(
            RequestReason::User,
            None,
            Some(&hold),
            &candidates,
            &health,
        ));
        assert_eq!(resolution.candidate.as_deref(), Some("strong"));
        assert_eq!(resolution.source, DispatchSource::ContextHold);
        assert!(
            resolution
                .reasons
                .iter()
                .any(|reason| reason.code == ReasonCodeKind::ContextHoldApplied)
        );
    }

    #[test]
    fn expired_hold_is_invalidated() {
        let candidates = [candidate("strong", "strong-physical", 80)];
        let health = CandidateHealth::default();
        let hold = ContextHold {
            candidate_id: "strong".to_string(),
            virtual_model_id: "auto".to_string(),
            group_digest: "group".to_string(),
            plan_revision: 1,
            profile_id: "default".to_string(),
            ingress_protocol: "chat_completions".to_string(),
            identity_digest: "session".to_string(),
            updated_at: 1_699_000_000,
            expires_at: 1_699_500_000,
        };

        let resolution = resolve(&context(
            RequestReason::User,
            None,
            Some(&hold),
            &candidates,
            &health,
        ));
        assert_eq!(resolution.candidate, None);
        assert!(
            resolution
                .reasons
                .iter()
                .any(|reason| reason.code == ReasonCodeKind::ContextHoldInvalidated)
        );
    }
}
