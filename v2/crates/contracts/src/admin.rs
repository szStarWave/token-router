//! 管理 API 契约 DTO（P4-001，契约冻结 v1）。
//!
//! 规则（docs/p4-admin-api-contract.zh.md）：
//! - 字段只增不改；删除/重命名需要 `api_version + 1`；
//! - 客户端必须忽略未知字段（所有类型 `#[serde(default)]` 兼容读取）；
//! - `ts-rs` 导出 TS 类型，`cargo xtask contract-check` 校验生成物无 diff。
//!
//! 本模块是纯类型：零 IO、零 SQL，与 `tr-contracts` 其它模块一致。
//! admin API 的时间统一 Unix 毫秒；金额统一 micros。

use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// 当前管理 API 版本（契约 v1）。
pub const API_VERSION: u32 = 1;

/// 管理端点作用域。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, TS, Serialize, Deserialize)]
#[ts(export, export_to = "admin.ts")]
#[serde(rename_all = "snake_case")]
pub enum AdminScope {
    /// 数据面三协议与 `/v1/models`。
    Inference,
    /// 所有 GET 管理端点。
    AdminRead,
    /// `config/apply`、`config/rollback`、token 管理、shutdown。
    AdminWrite,
    /// 审批写操作。
    AdminApprove,
}

impl AdminScope {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Inference => "inference",
            Self::AdminRead => "admin:read",
            Self::AdminWrite => "admin:write",
            Self::AdminApprove => "admin:approve",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "inference" => Some(Self::Inference),
            "admin:read" | "admin_read" => Some(Self::AdminRead),
            "admin:write" | "admin_write" => Some(Self::AdminWrite),
            "admin:approve" | "admin_approve" => Some(Self::AdminApprove),
            _ => None,
        }
    }
}

/// 配置校验问题（字段路径可映射到表单控件）。
#[derive(Debug, Clone, TS, Serialize, Deserialize)]
#[ts(export, export_to = "admin.ts")]
pub struct ConfigIssue {
    pub path: String,
    pub code: String,
    pub message: String,
}

/// 配置 diff 条目（只存字段路径与脱敏后的旧/新值摘要）。
#[derive(Debug, Clone, TS, Serialize, Deserialize)]
#[ts(export, export_to = "admin.ts")]
pub struct ConfigDiffEntry {
    pub path: String,
    #[ts(as = "Option<String>")]
    pub from: Option<serde_json::Value>,
    #[ts(as = "Option<String>")]
    pub to: Option<serde_json::Value>,
}

/// `GET /v2/admin/status`。
#[derive(Debug, Clone, TS, Serialize, Deserialize)]
#[ts(export, export_to = "admin.ts")]
pub struct StatusResponse {
    pub api_version: u32,
    pub version: String,
    /// Unix 毫秒。
    pub started_at: i64,
    pub uptime_ms: i64,
    pub listen: String,
    /// 配置文件 schema 版本（P4 起为 2）。
    pub schema_version: u32,
    /// DB 迁移版本（migration 5 起）。
    pub db_schema_version: i64,
    pub config_digest: String,
    pub default_virtual_model: String,
    pub modes: ModeSummary,
    pub requests_total: i64,
    pub breakers: BreakerCounts,
}

/// 运行模式摘要。
#[derive(Debug, Clone, TS, Serialize, Deserialize)]
#[ts(export, export_to = "admin.ts")]
pub struct ModeSummary {
    pub judge: String,
    pub cost_gate: String,
    pub approvals: String,
}

/// 熔断计数。
#[derive(Debug, Clone, Default, TS, Serialize, Deserialize)]
#[ts(export, export_to = "admin.ts")]
pub struct BreakerCounts {
    pub closed: u32,
    pub half_open: u32,
    pub open: u32,
}

/// 单个候选的健康状态。
#[derive(Debug, Clone, TS, Serialize, Deserialize)]
#[ts(export, export_to = "admin.ts")]
pub struct CandidateHealthItem {
    pub candidate_id: String,
    /// `closed` | `half_open` | `open`
    pub status: String,
    pub consecutive_failures: u32,
    pub opened_count: u32,
    /// Unix 毫秒或 null。
    #[ts(as = "Option<i64>")]
    pub open_until: Option<u64>,
    #[ts(as = "Option<i64>")]
    pub half_open_since: Option<u64>,
    #[ts(as = "Option<String>")]
    pub last_error_class: Option<String>,
    #[ts(as = "Option<i64>")]
    pub last_failure_at: Option<u64>,
    pub updated_at: i64,
}

/// `GET /v2/admin/health`。
#[derive(Debug, Clone, TS, Serialize, Deserialize)]
#[ts(export, export_to = "admin.ts")]
pub struct HealthResponse {
    pub items: Vec<CandidateHealthItem>,
    pub counts: BreakerCounts,
}

/// 分页信封：游标为 `(created_at, id)` 的 base64 编码。
#[derive(Debug, Clone, TS, Serialize, Deserialize)]
#[ts(export, export_to = "admin.ts")]
pub struct Page<T: TS> {
    pub items: Vec<T>,
    /// 为空表示没有更多页。
    #[ts(as = "Option<String>")]
    pub next_cursor: Option<String>,
}

/// token 维度（账本列表项）。
#[derive(Debug, Clone, Default, TS, Serialize, Deserialize)]
#[ts(export, export_to = "admin.ts")]
pub struct TokenUsage {
    pub input: i64,
    pub cache_read: i64,
    pub output: i64,
}

/// 成本维度（micros）。
#[derive(Debug, Clone, Default, TS, Serialize, Deserialize)]
#[ts(export, export_to = "admin.ts")]
pub struct CostUsage {
    pub answer: i64,
    pub routing: i64,
}

/// `GET /v2/admin/requests` 列表项。
#[derive(Debug, Clone, TS, Serialize, Deserialize)]
#[ts(export, export_to = "admin.ts")]
pub struct RequestSummary {
    pub request_id: String,
    /// Unix 毫秒。
    pub created_at: i64,
    pub session_key: String,
    pub branch_id: String,
    pub ingress_protocol: String,
    pub selection_model: String,
    #[ts(as = "Option<String>")]
    pub dispatch_model: Option<String>,
    #[ts(as = "Option<String>")]
    pub dispatch_source: Option<String>,
    pub outcome: String,
    #[ts(as = "Option<i64>")]
    pub http_status: Option<i64>,
    #[ts(as = "Option<i64>")]
    pub ttft_ms: Option<i64>,
    pub duration_ms: i64,
    pub tokens: TokenUsage,
    pub cost_micros: CostUsage,
    pub reason_codes: Vec<String>,
    pub plan_id: String,
}

/// 一次尝试的脱敏摘要（详情页展开）。
#[derive(Debug, Clone, TS, Serialize, Deserialize)]
#[ts(export, export_to = "admin.ts")]
pub struct AttemptSummary {
    pub candidate_id: String,
    pub outcome: String,
    #[ts(as = "Option<i64>")]
    pub latency_ms: Option<i64>,
    #[ts(as = "Option<String>")]
    pub error_class: Option<String>,
}

/// `GET /v2/admin/requests/{id}`：列表字段 + 决策详情。
#[derive(Debug, Clone, TS, Serialize, Deserialize)]
#[ts(export, export_to = "admin.ts")]
pub struct RequestDetail {
    #[serde(flatten)]
    pub summary: RequestSummary,
    #[ts(as = "Option<String>")]
    pub task_id: Option<String>,
    pub session_key_source: String,
    pub collision_suspect: bool,
    pub selection_level: String,
    #[ts(as = "Option<String>")]
    pub dispatch_provider: Option<String>,
    #[ts(as = "Option<String>")]
    pub dispatch_level: Option<String>,
    pub plan_mode: String,
    #[ts(as = "serde_json::Value")]
    pub preference: serde_json::Value,
    #[ts(as = "serde_json::Value")]
    pub failover: serde_json::Value,
    pub attempts: Vec<AttemptSummary>,
    pub switches: i64,
    pub cache_write_tokens: i64,
    pub facts_completeness: String,
    #[ts(as = "Option<String>")]
    pub profile_revision: Option<String>,
    #[ts(as = "Option<String>")]
    pub config_digest: Option<String>,
    #[ts(as = "Option<serde_json::Value>")]
    pub judge: Option<serde_json::Value>,
    #[ts(as = "serde_json::Value")]
    pub contributions: serde_json::Value,
    #[ts(as = "Option<serde_json::Value>")]
    pub authority: Option<serde_json::Value>,
}

/// 统计延迟分位（毫秒）。
#[derive(Debug, Clone, Default, TS, Serialize, Deserialize)]
#[ts(export, export_to = "admin.ts")]
pub struct LatencySummary {
    pub ttft_p50: Option<i64>,
    pub ttft_p95: Option<i64>,
    pub duration_p95: Option<i64>,
}

/// 判官维度汇总。
#[derive(Debug, Clone, Default, TS, Serialize, Deserialize)]
#[ts(export, export_to = "admin.ts")]
pub struct JudgeSummary {
    pub calls: i64,
    pub failed: i64,
    pub cost_micros: i64,
}

/// `GET /v2/admin/stats/summary`。
#[derive(Debug, Clone, TS, Serialize, Deserialize)]
#[ts(export, export_to = "admin.ts")]
pub struct StatsSummary {
    pub requests: i64,
    pub successes: i64,
    pub success_rate: f64,
    pub upstream_errors: i64,
    pub quality_fails: i64,
    pub cancelled: i64,
    pub rejected: i64,
    pub switches: i64,
    pub tokens: StatsTokenUsage,
    pub cost_micros: CostUsage,
    pub latency: LatencySummary,
    pub judge: JudgeSummary,
}

/// 统计 token 维度（含 cache_write，与列表 TokenUsage 区分）。
#[derive(Debug, Clone, Default, TS, Serialize, Deserialize)]
#[ts(export, export_to = "admin.ts")]
pub struct StatsTokenUsage {
    pub input: i64,
    pub output: i64,
    pub cache_read: i64,
    pub cache_write: i64,
}

/// 序列点。
#[derive(Debug, Clone, TS, Serialize, Deserialize)]
#[ts(export, export_to = "admin.ts")]
pub struct SeriesPoint {
    /// Unix 毫秒（bucket 起点）。
    pub t: i64,
    pub value: i64,
}

/// 序列分组。
#[derive(Debug, Clone, TS, Serialize, Deserialize)]
#[ts(export, export_to = "admin.ts")]
pub struct SeriesGroup {
    pub key: String,
    pub points: Vec<SeriesPoint>,
}

/// `GET /v2/admin/stats/series`。
#[derive(Debug, Clone, TS, Serialize, Deserialize)]
#[ts(export, export_to = "admin.ts")]
pub struct SeriesResponse {
    /// `hour` | `day`
    pub bucket: String,
    /// Unix 毫秒。
    pub from: i64,
    pub to: i64,
    pub group_by: String,
    pub metric: String,
    pub series: Vec<SeriesGroup>,
}

/// 配置审计条目（脱敏）。
#[derive(Debug, Clone, TS, Serialize, Deserialize)]
#[ts(export, export_to = "admin.ts")]
pub struct ConfigAuditEntry {
    pub id: i64,
    /// Unix 毫秒。
    pub at: i64,
    pub actor: String,
    /// `validate` | `apply` | `rollback`
    pub action: String,
    #[ts(as = "Option<String>")]
    pub from_revision: Option<String>,
    #[ts(as = "Option<String>")]
    pub to_revision: Option<String>,
    #[ts(as = "Option<String>")]
    pub note: Option<String>,
    pub diff: Vec<ConfigDiffEntry>,
}

/// 配置文档中的凭据引用（不返回密钥值）。
#[derive(Debug, Clone, TS, Serialize, Deserialize)]
#[ts(export, export_to = "admin.ts")]
pub struct SecretRef {
    pub path: String,
    /// `env:NAME` / `file:path` / `redacted:literal`
    pub reference: String,
}

/// `GET /v2/admin/config` 的 warnings。
#[derive(Debug, Clone, TS, Serialize, Deserialize)]
#[ts(export, export_to = "admin.ts")]
pub struct ConfigWarning {
    pub code: String,
    pub message: String,
}

/// 配置文档顶层结构（脱敏；字段与 `ConfigFile` 一一对应）。
#[derive(Debug, Clone, TS, Serialize, Deserialize)]
#[ts(export, export_to = "admin.ts")]
pub struct ConfigDocument {
    pub schema_version: u32,
    /// 规范化 JSON 的 SHA-256（裸 hex），与账本 `config_digest` 同源。
    pub revision: String,
    pub editable: bool,
    #[ts(as = "serde_json::Value")]
    pub document: serde_json::Value,
    pub secrets: Vec<SecretRef>,
    pub warnings: Vec<ConfigWarning>,
}

/// `POST /v2/admin/config/validate` 响应。
#[derive(Debug, Clone, TS, Serialize, Deserialize)]
#[ts(export, export_to = "admin.ts")]
pub struct ValidateResponse {
    pub valid: bool,
    #[ts(as = "Option<String>")]
    pub revision: Option<String>,
    #[ts(as = "Option<String>")]
    pub normalized_toml: Option<String>,
    pub issues: Vec<ConfigIssue>,
    pub diff: Vec<ConfigDiffEntry>,
}

/// `POST /v2/admin/config/apply` 请求体。
#[derive(Debug, Clone, TS, Serialize, Deserialize)]
#[ts(export, export_to = "admin.ts")]
pub struct ApplyRequest {
    /// 规范化 TOML 文本。
    pub document_toml: String,
    /// 默认 `manual`；由壳决定何时重启。
    #[serde(default)]
    pub restart: Option<String>,
}

/// `POST /v2/admin/config/apply` 响应。
#[derive(Debug, Clone, TS, Serialize, Deserialize)]
#[ts(export, export_to = "admin.ts")]
pub struct ApplyResponse {
    pub revision: String,
    /// P4 恒为 true（不做热加载）。
    pub restart_required: bool,
    #[ts(as = "Option<String>")]
    pub backup_path: Option<String>,
}

/// token 列表项（脱敏：只有 prefix）。
#[derive(Debug, Clone, TS, Serialize, Deserialize)]
#[ts(export, export_to = "admin.ts")]
pub struct TokenInfo {
    pub id: String,
    pub name: String,
    /// 前 8 字符。
    pub prefix: String,
    pub scopes: Vec<AdminScope>,
    /// Unix 毫秒。
    pub created_at: i64,
    #[ts(as = "Option<i64>")]
    pub last_used_at: Option<i64>,
    #[ts(as = "Option<i64>")]
    pub expires_at: Option<i64>,
    #[ts(as = "Option<i64>")]
    pub revoked_at: Option<i64>,
}

/// `POST /v2/admin/tokens` 响应：明文只返回一次。
#[derive(Debug, Clone, TS, Serialize, Deserialize)]
#[ts(export, export_to = "admin.ts")]
pub struct CreatedToken {
    #[serde(flatten)]
    pub info: TokenInfo,
    /// 完整明文；之后任何接口都不再返回。
    pub token: String,
}

/// `POST /v2/admin/tokens` 请求体。
#[derive(Debug, Clone, TS, Serialize, Deserialize)]
#[ts(export, export_to = "admin.ts")]
pub struct CreateTokenRequest {
    pub name: String,
    pub scopes: Vec<AdminScope>,
    /// Unix 毫秒；null 表示不过期。
    #[ts(as = "Option<i64>")]
    pub expires_at: Option<i64>,
}

/// 日志条目（脱敏后）。
#[derive(Debug, Clone, TS, Serialize, Deserialize)]
#[ts(export, export_to = "admin.ts")]
pub struct LogEntry {
    pub seq: i64,
    /// Unix 毫秒。
    pub at: i64,
    pub level: String,
    pub target: String,
    pub message: String,
}

/// `GET /v2/admin/logs`。
#[derive(Debug, Clone, TS, Serialize, Deserialize)]
#[ts(export, export_to = "admin.ts")]
pub struct LogsResponse {
    pub items: Vec<LogEntry>,
    #[ts(as = "Option<i64>")]
    pub next_cursor: Option<i64>,
}

/// `GET /v2/admin/doctor` 检查项。
#[derive(Debug, Clone, TS, Serialize, Deserialize)]
#[ts(export, export_to = "admin.ts")]
pub struct DoctorCheck {
    pub name: String,
    /// `ok` | `warn` | `fail`
    pub status: String,
    #[ts(as = "Option<String>")]
    pub detail: Option<String>,
}

/// `GET /v2/admin/doctor`。
#[derive(Debug, Clone, TS, Serialize, Deserialize)]
#[ts(export, export_to = "admin.ts")]
pub struct DoctorResponse {
    pub checks: Vec<DoctorCheck>,
}

/// 统一错误信封（`error.type` 见契约文档 §1 错误码表）。
#[derive(Debug, Clone, TS, Serialize, Deserialize)]
#[ts(export, export_to = "admin.ts")]
pub struct ApiErrorBody {
    pub r#type: String,
    pub message: String,
    #[ts(as = "Option<String>")]
    pub request_id: Option<String>,
    #[ts(as = "Option<Vec<ConfigIssue>>")]
    pub issues: Option<Vec<ConfigIssue>>,
}

/// 统一错误响应外层。
#[derive(Debug, Clone, TS, Serialize, Deserialize)]
#[ts(export, export_to = "admin.ts")]
pub struct ApiErrorEnvelope {
    pub error: ApiErrorBody,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scope_roundtrip() {
        for scope in [
            AdminScope::Inference,
            AdminScope::AdminRead,
            AdminScope::AdminWrite,
            AdminScope::AdminApprove,
        ] {
            assert_eq!(AdminScope::parse(scope.as_str()), Some(scope));
        }
        assert_eq!(AdminScope::parse("bogus"), None);
    }

    #[test]
    fn api_version_is_frozen() {
        assert_eq!(API_VERSION, 1);
    }
}
