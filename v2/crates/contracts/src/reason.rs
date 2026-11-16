//! 决策理由码（§11.1）。
//!
//! 理由码是**枚举 + 结构化载荷**，不是自由字符串：它进入响应头、账本与指标标签，
//! 因此必须稳定、可枚举、可聚合。

use serde::{Deserialize, Serialize};

macro_rules! reason_kinds {
    ($($variant:ident => $code:literal),* $(,)?) => {
        /// 稳定的理由码集合。新增码 = 新增变体，不复用旧码语义。
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
        pub enum ReasonCodeKind {
            $(
                #[serde(rename = $code)]
                $variant,
            )*
        }

        impl ReasonCodeKind {
            pub const fn as_str(self) -> &'static str {
                match self {
                    $(Self::$variant => $code,)*
                }
            }
        }
    };
}

reason_kinds! {
    // 模型/选择
    ModelNotFound => "MODEL_NOT_FOUND",
    ModelInputUnsupported => "MODEL_INPUT_UNSUPPORTED",
    VirtualModelMissing => "VIRTUAL_MODEL_MISSING",
    VirtualModelChainRejected => "VIRTUAL_MODEL_CHAIN_REJECTED",
    TrustTierUnknown => "TRUST_TIER_UNKNOWN",
    // 策略贡献
    BaselineOrder => "BASELINE_ORDER",
    // Plan 校验
    NoEligibleCandidate => "NO_ELIGIBLE_CANDIDATE",
    CandidatesCoolingDown => "CANDIDATES_COOLING_DOWN",
    PlanFilterWindow => "PLAN_FILTER_WINDOW",
    PlanModeDowngraded => "PLAN_MODE_DOWNGRADED",
    // StickyDispatch
    StickyTargetUnavailable => "STICKY_TARGET_UNAVAILABLE",
    StickyWindowExceeded => "STICKY_WINDOW_EXCEEDED",
    StickyInputUnsupported => "STICKY_INPUT_UNSUPPORTED",
    StickyRetrySwitch => "STICKY_RETRY_SWITCH",
    // ContextHold
    ContextHoldApplied => "CONTEXT_HOLD_APPLIED",
    ContextHoldInvalidated => "CONTEXT_HOLD_INVALIDATED",
    // 成本闸
    PriceUnknown => "PRICE_UNKNOWN",
    PriceAssumed => "PRICE_ASSUMED",
    PriceCacheAssumed => "PRICE_CACHE_ASSUMED",
    GainInputUnknown => "GAIN_INPUT_UNKNOWN",
    SwitchDeniedCacheCost => "SWITCH_DENIED_CACHE_COST",
    SwitchDeniedPriceUnknown => "SWITCH_DENIED_PRICE_UNKNOWN",
    SwitchAllowedQualityPriceUnknown => "SWITCH_ALLOWED_QUALITY_PRICE_UNKNOWN",
    // 判官/棘轮
    JudgeUnavailable => "JUDGE_UNAVAILABLE",
    LatchTargetUnavailable => "LATCH_TARGET_UNAVAILABLE",
    EscalationConfirming => "ESCALATION_CONFIRMING",
    EscalationLatched => "ESCALATION_LATCHED",
    EscalationReleased => "ESCALATION_RELEASED",
    EscalationCooldown => "ESCALATION_COOLDOWN",
    ClassifyPrefer => "CLASSIFY_PREFER",
    CoreFallback => "CORE_FALLBACK",
    // 审批（HITL）
    ApprovalRequired => "APPROVAL_REQUIRED",
    ApprovalDenied => "APPROVAL_DENIED",
    // 状态/协议
    StateCasConflict => "STATE_CAS_CONFLICT",
    StateCasRetry => "STATE_CAS_RETRY",
    ProtocolStateMissing => "PROTOCOL_STATE_MISSING",
    CollisionSuspect => "COLLISION_SUSPECT",
    FieldDropped => "FIELD_DROPPED",
    Cancelled => "CANCELLED",
}

/// 理由码 + 可选细节。细节只进审计与日志，不参与算术。
///
/// 不实现 `Default`：没有 `code` 的理由码在语义上不存在，不能靠反序列化默认值兜底。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReasonCode {
    pub code: ReasonCodeKind,
    pub detail: Option<String>,
}

impl ReasonCode {
    pub const fn new(code: ReasonCodeKind) -> Self {
        Self { code, detail: None }
    }

    pub fn with_detail(code: ReasonCodeKind, detail: impl Into<String>) -> Self {
        Self {
            code,
            detail: Some(detail.into()),
        }
    }
}

impl From<ReasonCodeKind> for ReasonCode {
    fn from(code: ReasonCodeKind) -> Self {
        Self::new(code)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codes_are_uppercase_snake() {
        assert_eq!(
            serde_json::to_string(&ReasonCodeKind::StickyRetrySwitch).unwrap(),
            "\"STICKY_RETRY_SWITCH\""
        );
        assert_eq!(
            ReasonCodeKind::SwitchDeniedPriceUnknown.as_str(),
            "SWITCH_DENIED_PRICE_UNKNOWN"
        );
    }

    #[test]
    fn detail_is_audit_only() {
        let code = ReasonCode::with_detail(ReasonCodeKind::PlanFilterWindow, "window=32768");
        let json = serde_json::to_string(&code).unwrap();
        assert!(json.contains("\"detail\":\"window=32768\""));
    }
}
