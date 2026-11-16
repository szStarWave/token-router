//! `config_audit` 仓储（P4-033）：配置变更审计（脱敏）。
//!
//! 只记录字段路径与脱敏后的旧/新值摘要；不记录密钥值。

use rusqlite::params;
use tr_contracts::{ConfigAuditEntry, ConfigDiffEntry};

use crate::store::{Db, StoreError};

/// 审计写入参数。
pub struct AuditRecord<'a> {
    pub at: i64,
    pub actor: &'a str,
    /// `validate` | `apply` | `rollback`
    pub action: &'a str,
    pub from_revision: Option<&'a str>,
    pub to_revision: Option<&'a str>,
    pub note: Option<&'a str>,
    pub diff: &'a [ConfigDiffEntry],
}

/// 追加一条审计。
pub fn insert(db: &Db, record: &AuditRecord<'_>) -> Result<(), StoreError> {
    let diff_json = serde_json::to_string(record.diff)?;
    let connection = db.write();
    connection.execute(
        "INSERT INTO config_audit (at, actor, action, from_revision, to_revision, note, diff_json)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![
            record.at,
            record.actor,
            record.action,
            record.from_revision,
            record.to_revision,
            record.note,
            diff_json
        ],
    )?;
    Ok(())
}

/// 最近审计（脱敏输出）。
pub fn list(db: &Db, limit: u32) -> Result<Vec<ConfigAuditEntry>, StoreError> {
    let connection = db.read();
    let mut statement = connection.prepare(
        "SELECT id, at, actor, action, from_revision, to_revision, note, diff_json
           FROM config_audit ORDER BY id DESC LIMIT ?1",
    )?;
    let rows = statement.query_map(params![i64::from(limit)], |row| {
        let diff_json: Option<String> = row.get(7)?;
        let diff = diff_json
            .as_deref()
            .and_then(|text| serde_json::from_str::<Vec<ConfigDiffEntry>>(text).ok())
            .unwrap_or_default();
        Ok(ConfigAuditEntry {
            id: row.get(0)?,
            at: row.get(1)?,
            actor: row.get(2)?,
            action: row.get(3)?,
            from_revision: row.get(4)?,
            to_revision: row.get(5)?,
            note: row.get(6)?,
            diff,
        })
    })?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(StoreError::from)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::Db;

    #[test]
    fn insert_and_list_roundtrip() {
        let db = Db::open_in_memory().unwrap();
        let diff = vec![ConfigDiffEntry {
            path: "candidates[1].strength".to_string(),
            from: Some(serde_json::json!(20)),
            to: Some(serde_json::json!(40)),
        }];
        insert(
            &db,
            &AuditRecord {
                at: 1000,
                actor: "ui",
                action: "apply",
                from_revision: Some("rev-a"),
                to_revision: Some("rev-b"),
                note: None,
                diff: &diff,
            },
        )
        .unwrap();
        let entries = list(&db, 10).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].action, "apply");
        assert_eq!(entries[0].diff.len(), 1);
        assert_eq!(entries[0].diff[0].path, "candidates[1].strength");
    }
}
