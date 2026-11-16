//! `approvals` 仓储（P3-13）：审批审计。
//!
//! 审批与路由共享类型/配置，但**预算池独立**：人类等待不占判官 deadline。

use rusqlite::params;
use serde_json::{Value, json};

use crate::store::{Db, StoreError};

/// 一条审批记录。
#[derive(Debug, Clone)]
pub struct ApprovalRecord {
    pub request_id: String,
    pub tool_name: String,
    pub risk_class: String,
    pub decision: String,
    pub operator: Option<String>,
    pub wait_ms: Option<u64>,
    pub created_at: u64,
}

/// 写入审批。
pub fn insert(db: &Db, record: &ApprovalRecord) -> Result<(), StoreError> {
    let connection = db.write();
    connection.execute(
        "INSERT INTO approvals (
            request_id, tool_name, risk_class, decision, operator, wait_ms, created_at
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![
            record.request_id,
            record.tool_name,
            record.risk_class,
            record.decision,
            record.operator,
            record.wait_ms.map(|value| value as i64),
            record.created_at as i64
        ],
    )?;
    Ok(())
}

/// 最近审批记录。
pub fn list(db: &Db, limit: u32) -> Result<Vec<Value>, StoreError> {
    let connection = db.read();
    let mut statement = connection.prepare(
        "SELECT request_id, tool_name, risk_class, decision, operator, wait_ms, created_at
           FROM approvals ORDER BY id DESC LIMIT ?1",
    )?;
    let mut rows = statement.query(params![i64::from(limit)])?;
    let mut out = Vec::new();
    while let Some(row) = rows.next()? {
        out.push(json!({
            "request_id": row.get::<_, String>(0)?,
            "tool_name": row.get::<_, String>(1)?,
            "risk_class": row.get::<_, String>(2)?,
            "decision": row.get::<_, String>(3)?,
            "operator": row.get::<_, Option<String>>(4)?,
            "wait_ms": row.get::<_, Option<i64>>(5)?,
            "created_at": row.get::<_, i64>(6)?,
        }));
    }
    Ok(out)
}

/// 工具风险分级（审批规则兜底）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RiskClass {
    Low,
    Medium,
    High,
}

impl RiskClass {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
        }
    }
}

/// 按工具名做保守分级：**未知工具按 High**，宁可多问一次。
pub fn classify_tool(tool_name: &str) -> RiskClass {
    let name = tool_name.to_ascii_lowercase();
    if name.contains("read")
        || name.contains("list")
        || name.contains("search")
        || name.contains("get")
    {
        RiskClass::Low
    } else if name.contains("write")
        || name.contains("delete")
        || name.contains("remove")
        || name.contains("exec")
        || name.contains("shell")
        || name.contains("run")
        || name.contains("install")
    {
        RiskClass::High
    } else if name.contains("http") || name.contains("fetch") || name.contains("browser") {
        RiskClass::Medium
    } else {
        RiskClass::High
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_are_low() {
        assert_eq!(classify_tool("read_file"), RiskClass::Low);
        assert_eq!(classify_tool("list_files"), RiskClass::Low);
    }

    #[test]
    fn writes_and_shell_are_high() {
        assert_eq!(classify_tool("write_file"), RiskClass::High);
        assert_eq!(classify_tool("shell_exec"), RiskClass::High);
    }

    #[test]
    fn network_is_medium() {
        assert_eq!(classify_tool("http_fetch"), RiskClass::Medium);
    }

    #[test]
    fn unknown_tool_is_high() {
        assert_eq!(classify_tool("mystery"), RiskClass::High);
    }
}
