//! SQLite 迁移（§18.4.1）。
//!
//! 规则：
//! - 只前进，不提供 down；checksum 不一致拒绝启动；
//! - 库版本高于内核支持版本 → 拒绝启动，不降级写库；
//! - 迁移只允许加表/加列/建索引；语义变更必须代码兼容两版。
//!
//! 目前只有 migration 1（P0a baseline）。新增迁移时：追加 SQL、保持既有迁移文本
//! 一字不改（改文本 = checksum 不匹配 = 老库无法启动）。

use rusqlite::{Connection, params};
use sha2::{Digest, Sha256};

use crate::store::StoreError;

/// 按序号应用的迁移；index + 1 == version。
pub(crate) const MIGRATIONS: &[&str] = &[
    M1_BASELINE,
    M2_LEDGER_JUDGE,
    M3_LEDGER_CONTRIBUTIONS,
    M4_LEDGER_AUTHORITY,
    M5_TOKENS_AUDIT,
];

pub(crate) fn migrate(conn: &mut Connection) -> Result<(), StoreError> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS schema_migrations (
            version    INTEGER PRIMARY KEY,
            applied_at INTEGER NOT NULL,
            checksum   TEXT    NOT NULL
        );",
    )?;

    let applied: Vec<(i64, String)> = {
        let mut statement =
            conn.prepare("SELECT version, checksum FROM schema_migrations ORDER BY version")?;
        statement
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
            .collect::<Result<_, _>>()?
    };

    if let Some((max, _)) = applied.last() {
        let supported = MIGRATIONS.len() as i64;
        if *max > supported {
            return Err(StoreError::SchemaTooNew {
                found: *max,
                supported,
            });
        }
    }

    for (index, sql) in MIGRATIONS.iter().enumerate() {
        let version = (index + 1) as i64;
        let checksum = checksum(sql);

        if let Some((_, existing)) = applied.iter().find(|(applied, _)| *applied == version) {
            if existing != &checksum {
                return Err(StoreError::ChecksumMismatch { version });
            }
            continue;
        }

        let transaction = conn.transaction()?;
        transaction.execute_batch(sql)?;
        transaction.execute(
            "INSERT INTO schema_migrations (version, applied_at, checksum) VALUES (?1, ?2, ?3)",
            params![version, now_millis(), checksum],
        )?;
        transaction.commit()?;
    }

    Ok(())
}

fn checksum(sql: &str) -> String {
    let digest = Sha256::digest(sql.as_bytes());
    let mut out = String::with_capacity(digest.len() * 2);
    for byte in digest {
        use std::fmt::Write as _;
        let _ = write!(out, "{byte:02x}");
    }
    out
}

pub(crate) fn now_millis() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as i64)
        .unwrap_or(0)
}

const M1_BASELINE: &str = r#"
CREATE TABLE sessions (
  session_key   TEXT    NOT NULL,
  branch_id     TEXT    NOT NULL,
  facts_json    TEXT    NOT NULL,
  version       INTEGER NOT NULL,
  created_at    INTEGER NOT NULL,
  updated_at    INTEGER NOT NULL,
  PRIMARY KEY (session_key, branch_id)
) WITHOUT ROWID;

CREATE TABLE strategy_states (
  session_key      TEXT    NOT NULL,
  branch_id        TEXT    NOT NULL,
  task_id          TEXT    NOT NULL,
  virtual_model_id TEXT    NOT NULL,
  strategy_id      TEXT    NOT NULL,
  state_json       TEXT    NOT NULL,
  version          INTEGER NOT NULL,
  updated_at       INTEGER NOT NULL,
  PRIMARY KEY (session_key, branch_id, task_id, virtual_model_id, strategy_id)
) WITHOUT ROWID;

CREATE TABLE virtual_model_states (
  session_key      TEXT    NOT NULL,
  branch_id        TEXT    NOT NULL,
  virtual_model_id TEXT    NOT NULL,
  state_json       TEXT    NOT NULL,
  version          INTEGER NOT NULL,
  updated_at       INTEGER NOT NULL,
  PRIMARY KEY (session_key, branch_id, virtual_model_id)
) WITHOUT ROWID;

CREATE TABLE ledger (
  id                  INTEGER PRIMARY KEY,
  request_id          TEXT    NOT NULL UNIQUE,
  plan_id             TEXT    NOT NULL,
  session_key         TEXT    NOT NULL,
  branch_id           TEXT    NOT NULL,
  task_id             TEXT,
  session_key_source  TEXT    NOT NULL,
  collision_suspect   INTEGER NOT NULL DEFAULT 0,
  ingress_protocol    TEXT    NOT NULL,
  selection_model     TEXT    NOT NULL,
  selection_level     TEXT    NOT NULL,
  dispatch_model      TEXT,
  dispatch_provider   TEXT,
  dispatch_level      TEXT,
  dispatch_source     TEXT,
  plan_mode           TEXT    NOT NULL,
  preference_json     TEXT    NOT NULL,
  failover_json       TEXT    NOT NULL,
  attempts_json       TEXT    NOT NULL,
  switches            INTEGER NOT NULL DEFAULT 0,
  reason_codes_json   TEXT    NOT NULL,
  outcome             TEXT    NOT NULL,
  http_status         INTEGER,
  input_tokens        INTEGER,
  cache_read_tokens   INTEGER,
  cache_write_tokens  INTEGER,
  output_tokens       INTEGER,
  answer_cost_micros  INTEGER,
  routing_cost_micros INTEGER,
  facts_completeness  TEXT    NOT NULL,
  profile_revision    TEXT,
  config_digest       TEXT,
  ttft_ms             INTEGER,
  duration_ms         INTEGER NOT NULL DEFAULT 0,
  created_at          INTEGER NOT NULL
);
CREATE INDEX idx_ledger_created   ON ledger(created_at);
CREATE INDEX idx_ledger_session   ON ledger(session_key, created_at);
CREATE INDEX idx_ledger_selection ON ledger(selection_model, created_at);
CREATE INDEX idx_ledger_outcome   ON ledger(outcome, created_at);
CREATE INDEX idx_ledger_plan      ON ledger(plan_id);

CREATE TABLE shadow_records (
  id                INTEGER PRIMARY KEY,
  request_id        TEXT    NOT NULL,
  selection_model   TEXT    NOT NULL,
  kernel_choice     TEXT    NOT NULL,
  judge_question_id TEXT,
  judge_value_json  TEXT,
  agreement         INTEGER,
  effective_source  TEXT,
  judge_latency_ms  INTEGER,
  judge_cost_micros INTEGER,
  snapshot_digest   TEXT    NOT NULL,
  created_at        INTEGER NOT NULL
);
CREATE INDEX idx_shadow_created ON shadow_records(created_at);
CREATE INDEX idx_shadow_model   ON shadow_records(selection_model, created_at);

CREATE TABLE circuit_breakers (
  candidate_id         TEXT PRIMARY KEY,
  consecutive_failures INTEGER NOT NULL DEFAULT 0,
  opened_count         INTEGER NOT NULL DEFAULT 0,
  open_until           INTEGER,
  half_open_since      INTEGER,
  last_error_class     TEXT,
  last_failure_at      INTEGER,
  updated_at           INTEGER NOT NULL
) WITHOUT ROWID;

CREATE TABLE protocol_state (
  key          TEXT PRIMARY KEY,
  session_key  TEXT    NOT NULL,
  branch_id    TEXT    NOT NULL,
  kind         TEXT    NOT NULL,
  payload_json TEXT    NOT NULL,
  created_at   INTEGER NOT NULL,
  expires_at   INTEGER NOT NULL
) WITHOUT ROWID;
CREATE INDEX idx_protocol_expires ON protocol_state(expires_at);
CREATE INDEX idx_protocol_session ON protocol_state(session_key, branch_id, kind);

CREATE TABLE judge_calls (
  id                 INTEGER PRIMARY KEY,
  request_id         TEXT    NOT NULL,
  question_id        TEXT    NOT NULL,
  service_id         TEXT    NOT NULL,
  revision           INTEGER NOT NULL,
  protocol_digest    TEXT,
  effective_source   TEXT    NOT NULL,
  outcome            TEXT    NOT NULL,
  failure_code       TEXT,
  latency_ms         INTEGER,
  prompt_tokens      INTEGER,
  completion_tokens  INTEGER,
  cost_micros        INTEGER,
  created_at         INTEGER NOT NULL
);
CREATE INDEX idx_judge_calls_created ON judge_calls(created_at);
CREATE INDEX idx_judge_calls_request ON judge_calls(request_id);

CREATE TABLE approvals (
  id          INTEGER PRIMARY KEY,
  request_id  TEXT    NOT NULL,
  session_key TEXT,
  tool_name   TEXT    NOT NULL,
  risk_class  TEXT    NOT NULL,
  decision    TEXT    NOT NULL,
  operator    TEXT,
  wait_ms     INTEGER,
  created_at  INTEGER NOT NULL
);
"#;

/// migration 2：判官审计需要把 trace 落进账本（指标与对账同源）。
const M2_LEDGER_JUDGE: &str = "ALTER TABLE ledger ADD COLUMN judge_json TEXT;";

/// migration 3：每条策略贡献入账（P2-11 决策可追溯）。
const M3_LEDGER_CONTRIBUTIONS: &str = "ALTER TABLE ledger ADD COLUMN contributions_json TEXT;";

/// migration 4：请求授权身份入账（P0a-13；P0–P3 只填 `authority_id`）。
const M4_LEDGER_AUTHORITY: &str = "ALTER TABLE ledger ADD COLUMN authority_json TEXT;";

/// migration 5（P4-019）：scoped token store 与配置审计。
/// 只存 sha256；`config_audit` 只存字段路径与脱敏摘要。
const M5_TOKENS_AUDIT: &str = r#"
CREATE TABLE tokens (
  id            TEXT PRIMARY KEY,
  name          TEXT NOT NULL,
  token_sha256  TEXT NOT NULL UNIQUE,
  prefix        TEXT NOT NULL,
  scopes_json   TEXT NOT NULL,
  created_at    INTEGER NOT NULL,
  last_used_at  INTEGER,
  expires_at    INTEGER,
  revoked_at    INTEGER
);

CREATE TABLE config_audit (
  id            INTEGER PRIMARY KEY,
  at            INTEGER NOT NULL,
  actor         TEXT NOT NULL,
  action        TEXT NOT NULL,
  from_revision TEXT,
  to_revision   TEXT,
  note          TEXT,
  diff_json     TEXT
);

CREATE INDEX idx_ledger_authority
  ON ledger(json_extract(authority_json, '$.authority_id'), created_at);
"#;
