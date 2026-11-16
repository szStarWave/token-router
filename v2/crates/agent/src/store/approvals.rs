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
    pub session_key: Option<String>,
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
            request_id, session_key, tool_name, risk_class, decision, operator, wait_ms, created_at
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        params![
            record.request_id,
            record.session_key,
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

/// 最近一次针对 `(session, tool)` 的审批结论与时间（P3-12/13 实时拦截）。
///
/// `pending` 不是结论：它表示“已拦截、等待人工”，不应让后续请求放行。
/// `session_key IS NULL` 的记录视为运维显式设置的全局审批。
pub fn latest_decision(
    db: &Db,
    session_key: &str,
    tool_name: &str,
) -> Result<Option<(String, u64)>, StoreError> {
    let connection = db.read();
    let mut statement = connection.prepare(
        "SELECT decision, created_at FROM approvals
          WHERE (session_key = ?1 OR session_key IS NULL) AND tool_name = ?2
          ORDER BY id DESC LIMIT 1",
    )?;
    let mut rows = statement.query(params![session_key, tool_name])?;
    let Some(row) = rows.next()? else {
        return Ok(None);
    };
    let decision: String = row.get(0)?;
    let created_at: i64 = row.get(1)?;
    Ok(Some((decision, created_at.max(0) as u64)))
}

/// 最近审批记录。
pub fn list(db: &Db, limit: u32) -> Result<Vec<Value>, StoreError> {
    let connection = db.read();
    let mut statement = connection.prepare(
        "SELECT request_id, session_key, tool_name, risk_class, decision, operator, wait_ms, created_at
           FROM approvals ORDER BY id DESC LIMIT ?1",
    )?;
    let mut rows = statement.query(params![i64::from(limit)])?;
    let mut out = Vec::new();
    while let Some(row) = rows.next()? {
        out.push(json!({
            "request_id": row.get::<_, String>(0)?,
            "session_key": row.get::<_, Option<String>>(1)?,
            "tool_name": row.get::<_, String>(2)?,
            "risk_class": row.get::<_, String>(3)?,
            "decision": row.get::<_, String>(4)?,
            "operator": row.get::<_, Option<String>>(5)?,
            "wait_ms": row.get::<_, Option<i64>>(6)?,
            "created_at": row.get::<_, i64>(7)?,
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

    /// 风险等级：数值越大越危险。
    pub const fn rank(self) -> u8 {
        match self {
            Self::Low => 0,
            Self::Medium => 1,
            Self::High => 2,
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "low" => Some(Self::Low),
            "medium" => Some(Self::Medium),
            "high" => Some(Self::High),
            _ => None,
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
