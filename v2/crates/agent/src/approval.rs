//! 审批实时拦截（P3-12/13）。
//!
//! 边界（§12）：
//! - 审批与路由共享 `Question`/`DecisionModel` 类型的**方向**，但预算池独立；
//!   审批等待时间不进入判官 deadline（本模块完全没有网络调用）；
//! - 失败方向与路由相反：审批判定缺失/过期 → fail-closed（拦截）；
//! - 只对达到 `min_risk` 的工具生效，低风险工具默认不阻塞；
//! - 审批结论在 `approvals` 表审计，不写进路由账本的 `routing_cost`。
//!
//! 拦截点：请求进入上游之前。工具名来自消息历史里的 `ToolUse` 块；
//! 同一会话内 `approve` 在 TTL 内复用，`deny` 同样在 TTL 内生效。

use std::collections::BTreeSet;

use tr_contracts::{CanonicalRequest, ContentBlock};

use crate::store::approvals::{self, RiskClass};
use crate::store::{Db, StoreError};

/// 审批模式。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ApprovalMode {
    /// 不检查（默认）。
    #[default]
    Off,
    /// 只记录：请求照常转发，账本记 `APPROVAL_REQUIRED`。
    Observe,
    /// 拦截：未批准的高风险工具在本地拒绝，不访问上游。
    Enforce,
}

impl ApprovalMode {
    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "off" => Some(Self::Off),
            "observe" => Some(Self::Observe),
            "enforce" => Some(Self::Enforce),
            _ => None,
        }
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Observe => "observe",
            Self::Enforce => "enforce",
        }
    }
}

/// 审批设置。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApprovalSettings {
    pub mode: ApprovalMode,
    /// 审批结论有效期；过期视为未批准。
    pub ttl_secs: u64,
    /// 达到该风险等级才需要审批。
    pub min_risk: RiskClass,
    /// `enforce` 下是否 fail-closed（默认是）。关掉只用于灰度排查。
    pub fail_closed: bool,
}

impl Default for ApprovalSettings {
    fn default() -> Self {
        Self {
            mode: ApprovalMode::Off,
            ttl_secs: 900,
            min_risk: RiskClass::High,
            fail_closed: true,
        }
    }
}

/// 需要审批的工具及其风险。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingTool {
    pub tool_name: String,
    pub risk: RiskClass,
}

/// 评估结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ApprovalOutcome {
    /// 没有需要审批的工具，或审批有效。
    Clear,
    /// 需要审批（`observe` 或 `enforce` 均返回；由调用方决定是否拦截）。
    Required(PendingTool),
    /// 最近一次结论是拒绝。
    Denied(PendingTool),
}

impl ApprovalOutcome {
    pub fn pending(&self) -> Option<&PendingTool> {
        match self {
            Self::Clear => None,
            Self::Required(tool) | Self::Denied(tool) => Some(tool),
        }
    }
}

/// 收集消息历史里达到阈值的工具（去重，按工具名排序保证确定性）。
pub fn high_risk_tools(request: &CanonicalRequest, min_risk: RiskClass) -> Vec<PendingTool> {
    let mut names: BTreeSet<String> = BTreeSet::new();
    for message in &request.messages {
        for block in &message.blocks {
            if let ContentBlock::ToolUse { name, .. } = block {
                names.insert(name.clone());
            }
        }
    }
    names
        .into_iter()
        .map(|tool_name| PendingTool {
            risk: approvals::classify_tool(&tool_name),
            tool_name,
        })
        .filter(|tool| tool.risk.rank() >= min_risk.rank())
        .collect()
}

/// 判断请求是否需要人工审批。
///
/// 只读本地库，不做网络等待：审批延迟不会进入判官 deadline。
pub fn evaluate(
    db: &Db,
    settings: &ApprovalSettings,
    session_key: &str,
    request: &CanonicalRequest,
    now_unix: u64,
) -> Result<ApprovalOutcome, StoreError> {
    if settings.mode == ApprovalMode::Off {
        return Ok(ApprovalOutcome::Clear);
    }

    let mut required: Option<PendingTool> = None;
    for tool in high_risk_tools(request, settings.min_risk) {
        let decision = approvals::latest_decision(db, session_key, &tool.tool_name)?;
        let fresh = decision
            .as_ref()
            .is_some_and(|(_, at)| now_unix.saturating_sub(*at) <= settings.ttl_secs);
        match (fresh, decision.map(|(decision, _)| decision)) {
            (true, Some(decision)) if decision == "approve" => continue,
            (true, Some(decision)) if decision == "deny" => {
                return Ok(ApprovalOutcome::Denied(tool));
            }
            _ => {
                // 无记录、pending 或已过期：都要重新审批。
                required.get_or_insert(tool);
            }
        }
    }

    Ok(match required {
        Some(tool) => ApprovalOutcome::Required(tool),
        None => ApprovalOutcome::Clear,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use tr_contracts::{CanonicalMessage, CanonicalRole};

    fn request_with_tool(name: &str) -> CanonicalRequest {
        CanonicalRequest {
            messages: vec![CanonicalMessage {
                role: CanonicalRole::Assistant,
                blocks: vec![ContentBlock::ToolUse {
                    id: "call-1".to_string(),
                    name: name.to_string(),
                    arguments: serde_json::json!({}),
                }],
                ..CanonicalMessage::default()
            }],
            ..CanonicalRequest::default()
        }
    }

    #[test]
    fn off_never_requires_approval() {
        let db = Db::open_in_memory().expect("db");
        let outcome = evaluate(
            &db,
            &ApprovalSettings::default(),
            "s",
            &request_with_tool("shell_exec"),
            100,
        )
        .expect("evaluate");
        assert_eq!(outcome, ApprovalOutcome::Clear);
    }

    #[test]
    fn high_risk_without_record_is_required() {
        let db = Db::open_in_memory().expect("db");
        let settings = ApprovalSettings {
            mode: ApprovalMode::Enforce,
            ..ApprovalSettings::default()
        };
        let outcome =
            evaluate(&db, &settings, "s", &request_with_tool("shell_exec"), 100).expect("evaluate");
        assert_eq!(
            outcome,
            ApprovalOutcome::Required(PendingTool {
                tool_name: "shell_exec".to_string(),
                risk: RiskClass::High,
            })
        );
    }

    #[test]
    fn low_risk_tool_is_not_gated() {
        let db = Db::open_in_memory().expect("db");
        let settings = ApprovalSettings {
            mode: ApprovalMode::Enforce,
            ..ApprovalSettings::default()
        };
        let outcome =
            evaluate(&db, &settings, "s", &request_with_tool("read_file"), 100).expect("evaluate");
        assert_eq!(outcome, ApprovalOutcome::Clear);
    }

    #[test]
    fn approve_within_ttl_clears_and_expired_requires_again() {
        let db = Db::open_in_memory().expect("db");
        let settings = ApprovalSettings {
            mode: ApprovalMode::Enforce,
            ttl_secs: 60,
            ..ApprovalSettings::default()
        };
        approvals::insert(
            &db,
            &approvals::ApprovalRecord {
                request_id: "r1".to_string(),
                session_key: Some("s".to_string()),
                tool_name: "shell_exec".to_string(),
                risk_class: "high".to_string(),
                decision: "approve".to_string(),
                operator: Some("tester".to_string()),
                wait_ms: None,
                created_at: 100,
            },
        )
        .expect("insert");

        let fresh =
            evaluate(&db, &settings, "s", &request_with_tool("shell_exec"), 120).expect("evaluate");
        assert_eq!(fresh, ApprovalOutcome::Clear);

        let expired =
            evaluate(&db, &settings, "s", &request_with_tool("shell_exec"), 200).expect("evaluate");
        assert!(matches!(expired, ApprovalOutcome::Required(_)));
    }

    #[test]
    fn deny_wins_over_older_approve() {
        let db = Db::open_in_memory().expect("db");
        let settings = ApprovalSettings {
            mode: ApprovalMode::Enforce,
            ..ApprovalSettings::default()
        };
        for (decision, at) in [("approve", 100u64), ("deny", 110)] {
            approvals::insert(
                &db,
                &approvals::ApprovalRecord {
                    request_id: format!("r-{decision}"),
                    session_key: Some("s".to_string()),
                    tool_name: "shell_exec".to_string(),
                    risk_class: "high".to_string(),
                    decision: decision.to_string(),
                    operator: None,
                    wait_ms: None,
                    created_at: at,
                },
            )
            .expect("insert");
        }
        let outcome =
            evaluate(&db, &settings, "s", &request_with_tool("shell_exec"), 120).expect("evaluate");
        assert!(matches!(outcome, ApprovalOutcome::Denied(_)));
    }
}
