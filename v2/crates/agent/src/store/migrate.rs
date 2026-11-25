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
    M6_PROVIDERS,
    M7_ACCOUNT_HEADERS,
    M8_QUOTA_LIMITS,
    M9_COMPILED_CANDIDATES,
    M10_DROP_APPROVALS,
    M11_PROVIDER_ENABLED,
    M12_MODEL_PURPOSE_CAPS,
    M13_CATALOG_HIDDEN,
    M14_MODEL_THINKING_LEVELS,
    M15_REQUEST_TRANSCRIPTS,
    M16_JUDGE_CALL_DETAIL,
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

/// migration 6（v3 P5-A）：Provider / Account / 模型目录勾选。
/// 明文 key 不入库存（凭据文件），目录只存勾选与元数据。
const M6_PROVIDERS: &str = r#"
CREATE TABLE providers (
  id                  TEXT PRIMARY KEY,
  display_name        TEXT NOT NULL,
  kind                TEXT NOT NULL,
  base_url            TEXT NOT NULL,
  catalog_url         TEXT,
  template_id         TEXT,
  status              TEXT NOT NULL DEFAULT 'not_configured',
  catalog_refreshed_at INTEGER,
  catalog_error       TEXT,
  created_at          INTEGER NOT NULL
);

CREATE TABLE accounts (
  id                TEXT PRIMARY KEY,
  provider_id       TEXT NOT NULL REFERENCES providers(id) ON DELETE CASCADE,
  label             TEXT NOT NULL,
  credential_ref    TEXT NOT NULL,
  fingerprint       TEXT NOT NULL,
  proxy_url         TEXT,
  base_url_override TEXT,
  enabled           INTEGER NOT NULL DEFAULT 1,
  verified_at       INTEGER,
  verify_status     TEXT,
  created_at        INTEGER NOT NULL
);

CREATE INDEX idx_accounts_provider ON accounts(provider_id);

CREATE TABLE catalog_models (
  provider_id    TEXT NOT NULL REFERENCES providers(id) ON DELETE CASCADE,
  model_id       TEXT NOT NULL,
  display_name   TEXT NOT NULL DEFAULT '',
  selected       INTEGER NOT NULL DEFAULT 0,
  context_window INTEGER,
  max_output     INTEGER,
  price_json     TEXT,
  source         TEXT NOT NULL DEFAULT 'catalog',
  tier_override  TEXT,
  strength       INTEGER,
  PRIMARY KEY (provider_id, model_id)
);
"#;

/// migration 7（v3 P5-A 吸收项）：账号自定义 header；目录 auto-include 通配符
/// 与 deprecated 标记。
const M7_ACCOUNT_HEADERS: &str = r#"
ALTER TABLE accounts ADD COLUMN extra_headers_json TEXT;

ALTER TABLE catalog_models ADD COLUMN auto_include_pattern TEXT;
ALTER TABLE catalog_models ADD COLUMN deprecated INTEGER NOT NULL DEFAULT 0;
ALTER TABLE catalog_models ADD COLUMN deprecated_detected_at INTEGER;
"#;

/// migration 8（v3 P5-B）：上游配额观测与网关 key 限额。
const M8_QUOTA_LIMITS: &str = r#"
CREATE TABLE quota_windows (
  candidate_id     TEXT PRIMARY KEY,
  account_id       TEXT,
  provider_id      TEXT,
  remaining        INTEGER,
  limit_value      INTEGER,
  reset_at         INTEGER,
  unit             TEXT NOT NULL DEFAULT 'unknown',
  estimated        INTEGER NOT NULL DEFAULT 0,
  consecutive_429  INTEGER NOT NULL DEFAULT 0,
  updated_at       INTEGER NOT NULL
);

CREATE INDEX idx_quota_reset ON quota_windows(reset_at);

ALTER TABLE tokens ADD COLUMN limits_json TEXT;
"#;

/// migration 9（v3 P5-A 修正）：记录「上次编译写进 config 的候选 id」。
///
/// 编译是「全量重建 compiled 候选 + 原样保留手写候选」，但 config 里没有来源标记。
/// 早先靠 id 前缀猜测，猜不出来就退化成「现有全部候选都算手写」，于是第二次编译
/// 同一 id 出现两次 → `candidate id 重复` 422。改为把上次产物落库，编译前精确移除。
const M9_COMPILED_CANDIDATES: &str = r#"
CREATE TABLE compiled_candidates (
  candidate_id  TEXT PRIMARY KEY,
  compiled_at   INTEGER NOT NULL
);
"#;

/// migration 10：移除人工审批（HITL）模块。
///
/// 审批闸与产品定位冲突——本产品是"每个请求选哪个模型"的路由网关，
/// 不做请求内容级的高危工具拦截。整个模块（approval.rs、admin 端点、
/// `admin:approve` scope、[approvals] 配置段）已删除，审计表一并清掉。
const M10_DROP_APPROVALS: &str = r#"
DROP TABLE IF EXISTS approvals;
"#;

/// migration 11（v3 供应商页）：provider 级停用开关。
///
/// 停用后不删除账号/目录；下次编译时其候选以 `enabled = false` 写入配置，
/// 已有路由组继续引用但不会再被派发。
const M11_PROVIDER_ENABLED: &str = r#"
ALTER TABLE providers ADD COLUMN enabled INTEGER NOT NULL DEFAULT 1;
"#;

/// migration 12（M2 增强）：目录模型的用途与能力标签。
///
/// 只是目录/UI 元数据 + 编译时写入候选 capability tags；不改变候选编译的
/// 「全部勾选模型都进候选」语义，也不把 purpose 写进候选。
const M12_MODEL_PURPOSE_CAPS: &str = r#"
ALTER TABLE catalog_models ADD COLUMN purpose TEXT NOT NULL DEFAULT 'chat';
ALTER TABLE catalog_models ADD COLUMN caps_json TEXT;
"#;

/// migration 13：目录模型持久删除（隐藏）。
///
/// 删除不物理删行：刷新目录时保留 hidden，避免同一模型被上游目录重新带回。
/// 用户可在目录页「显示已移除」后恢复。
const M13_CATALOG_HIDDEN: &str = r#"
ALTER TABLE catalog_models ADD COLUMN hidden INTEGER NOT NULL DEFAULT 0;
"#;

/// migration 14：目录模型的推理档位（models.dev 自动填充；编译成候选默认值）。
const M14_MODEL_THINKING_LEVELS: &str = r#"
ALTER TABLE catalog_models ADD COLUMN thinking_levels_json TEXT;
"#;

/// migration 15：每轮请求的消息与模型回复（活动 → 会话 查看轮次消息）。
///
/// 只存展平后的文本（图片/音频变占位符，不存 base64）；与账本同键 `request_id`，
/// 会话维度走 `session_key` 索引。由配置 `[recording] transcript` 控制是否写入。
const M15_REQUEST_TRANSCRIPTS: &str = r#"
CREATE TABLE request_transcripts (
  request_id  TEXT PRIMARY KEY,
  session_key TEXT    NOT NULL,
  created_at  INTEGER NOT NULL,
  input_json  TEXT    NOT NULL,
  output_json TEXT    NOT NULL,
  truncated   INTEGER NOT NULL DEFAULT 0
);

CREATE INDEX idx_request_transcripts_session
  ON request_transcripts(session_key, created_at);
"#;

/// migration 16：判官（含 JEV 决策模型）调用的详细输入/输出。
///
/// 只存结构化摘要：任务文本（截断）、问题提示词、选项；答案（choice/bool/score、
/// 置信度、理由）与失败信息。不存完整 HTTP 请求体。
const M16_JUDGE_CALL_DETAIL: &str = r#"
ALTER TABLE judge_calls ADD COLUMN request_json TEXT;
ALTER TABLE judge_calls ADD COLUMN response_json TEXT;
ALTER TABLE judge_calls ADD COLUMN failure_message TEXT;
"#;
