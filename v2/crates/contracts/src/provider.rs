//! Provider / Account / 模型目录 DTO（v3 P5-A）。
//!
//! 边界：
//! - 管理面活数据（providers/accounts/模型勾选）存 SQLite，保存时**编译**成
//!   `config.toml` 的 `candidates` 段（内核零感知）；
//! - key 明文只落数据目录凭据文件，API 一律只回 `credential` 引用与指纹后 4 位；
//! - 时间一律 Unix 毫秒；金额一律整数 micros。

use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// provider 状态。
#[derive(Debug, Clone, Copy, PartialEq, Eq, TS, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[ts(export, export_to = "admin.ts")]
pub enum ProviderStatus {
    /// 只定义，还没配任何账号。
    NotConfigured,
    /// 有账号，但目录还没拉取。
    AwaitingCatalog,
    /// 目录就绪。
    Ready,
    /// 最后一次目录刷新失败。
    CatalogError,
}

/// provider 类型（决定 wire 协议与目录拉取方式）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, TS, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[ts(export, export_to = "admin.ts")]
pub enum ProviderKindDto {
    OpenAiCompatible,
    Anthropic,
    Responses,
}

/// 内置厂商模板（`GET /v2/admin/provider-templates`）。
#[derive(Debug, Clone, TS, Serialize, Deserialize)]
#[ts(export, export_to = "admin.ts")]
pub struct ProviderTemplate {
    /// 模板 id（`deepseek`、`openrouter`、`custom-openai`…）。
    pub id: String,
    pub display_name: String,
    pub kind: ProviderKindDto,
    /// 默认 base_url（用户可改）。
    pub base_url: String,
    /// 目录端点；`None` = 该厂商不支持在线目录（如本地推理）。
    #[ts(as = "Option<String>")]
    pub catalog_url: Option<String>,
    /// 默认价格表（micros/MTok）；`None` = 用户手填。
    #[ts(as = "Option<PriceDto>")]
    pub default_price: Option<PriceDto>,
    /// 默认上下文窗口。
    #[ts(as = "Option<u32>")]
    pub default_window: Option<u32>,
    /// 建议默认档位标签。
    pub suggested_tier: String,
}

/// `GET /v2/admin/provider-templates` 响应项（模板库端点 DTO；与服务端内部模板同形）。
pub type ProviderTemplateInfo = ProviderTemplate;

/// 创建/修改 provider 的请求体。
#[derive(Debug, Clone, TS, Serialize, Deserialize)]
#[ts(export, export_to = "admin.ts")]
pub struct PriceDto {
    pub currency: String,
    pub input_micros_per_mtok: u64,
    pub output_micros_per_mtok: u64,
    #[ts(as = "Option<u64>")]
    pub cache_read_micros_per_mtok: Option<u64>,
    #[ts(as = "Option<u64>")]
    pub cache_write_micros_per_mtok: Option<u64>,
    pub request_micros: u64,
}

/// provider 实例（`GET /v2/admin/providers` 列表项）。
#[derive(Debug, Clone, TS, Serialize, Deserialize)]
#[ts(export, export_to = "admin.ts")]
pub struct ProviderInfo {
    pub id: String,
    pub display_name: String,
    pub kind: ProviderKindDto,
    pub base_url: String,
    #[ts(as = "Option<String>")]
    pub catalog_url: Option<String>,
    pub status: ProviderStatus,
    /// 最后一次目录刷新时间（Unix 毫秒）。
    #[ts(as = "Option<i64>")]
    pub catalog_refreshed_at: Option<i64>,
    /// 最后一次目录刷新错误。
    #[ts(as = "Option<String>")]
    pub catalog_error: Option<String>,
    /// 账号数（冗余计数，列表页显示）。
    pub account_count: u32,
    /// 该 provider 编译出的候选数。
    pub candidate_count: u32,
    /// provider 级启停；停用后下次编译的候选为 disabled，账号/目录保留。
    pub enabled: bool,
    pub created_at: i64,
}

/// 创建/修改 provider 的请求体。
#[derive(Debug, Clone, TS, Serialize, Deserialize)]
#[ts(export, export_to = "admin.ts")]
pub struct ProviderUpsertRequest {
    /// 客户端生成（uuid）；不传由服务端生成。
    #[serde(default)]
    #[ts(optional)]
    pub id: Option<String>,
    /// 模板 id；`custom-openai` 等自定义模板也可省略。
    #[serde(default)]
    #[ts(optional)]
    pub template_id: Option<String>,
    #[serde(default)]
    #[ts(optional)]
    pub display_name: Option<String>,
    #[serde(default)]
    #[ts(optional)]
    pub kind: Option<ProviderKindDto>,
    #[serde(default)]
    #[ts(optional)]
    pub base_url: Option<String>,
    #[serde(default)]
    #[ts(optional)]
    pub catalog_url: Option<String>,
    /// provider 级启停；`None` = 创建默认启用 / 更新不变。
    #[serde(default)]
    #[ts(optional)]
    pub enabled: Option<bool>,
}

/// 静态自定义 header 键值对（值非敏感；敏感值走凭据引用）。
#[derive(Debug, Clone, TS, Serialize, Deserialize)]
#[ts(export, export_to = "admin.ts")]
pub struct HeaderPair {
    pub name: String,
    pub value: String,
}

/// 账号（provider 下的一把 key）。
#[derive(Debug, Clone, TS, Serialize, Deserialize)]
#[ts(export, export_to = "admin.ts")]
pub struct AccountInfo {
    pub id: String,
    pub provider_id: String,
    pub label: String,
    /// 凭据引用（`file:credentials/<id>.key`）。**不含明文**。
    pub credential_ref: String,
    /// key 指纹后 4 位；明文不可回读。
    pub fingerprint: String,
    /// 可选代理（`http://host:port`）。
    #[ts(as = "Option<String>")]
    pub proxy_url: Option<String>,
    /// 基础 URL 覆盖（多租户网关等场景）；空 = 用 provider 的。
    #[ts(as = "Option<String>")]
    pub base_url_override: Option<String>,
    /// 自定义请求 header（如 OpenRouter 的 `HTTP-Referer`/`X-Title`）。
    /// 值是**静态非敏感** header；敏感值请走凭据引用。
    #[serde(default)]
    #[ts(optional)]
    pub extra_headers: Option<Vec<HeaderPair>>,
    pub enabled: bool,
    /// 最后一次连通性验证时间。
    #[ts(as = "Option<i64>")]
    pub verified_at: Option<i64>,
    /// 最后一次验证结果（`ok` / 错误摘要）。
    #[ts(as = "Option<String>")]
    pub verify_status: Option<String>,
    pub created_at: i64,
}

/// 创建账号请求体（`POST /v2/admin/providers/{id}/accounts`）。
#[derive(Debug, Clone, TS, Serialize, Deserialize)]
#[ts(export, export_to = "admin.ts")]
pub struct AccountCreateRequest {
    #[serde(default = "default_account_label")]
    pub label: String,
    /// **明文 key**：只在创建请求里出现一次，服务端写入凭据文件后即丢弃。
    /// 与 `credential_ref` 二选一。
    #[serde(default)]
    #[ts(optional)]
    pub key: Option<String>,
    /// 直接引用凭据（`env:NAME` / `file:path`），不复制明文。
    #[serde(default)]
    #[ts(optional)]
    pub credential_ref: Option<String>,
    #[serde(default)]
    #[ts(optional)]
    pub proxy_url: Option<String>,
    #[serde(default)]
    #[ts(optional)]
    pub base_url_override: Option<String>,
    /// 静态自定义 header（非敏感值）。
    #[serde(default)]
    #[ts(optional)]
    pub extra_headers: Option<Vec<HeaderPair>>,
    #[serde(default = "default_some_true")]
    #[ts(optional)]
    pub enabled: Option<bool>,
}

fn default_account_label() -> String {
    "默认账号".to_string()
}

fn default_some_true() -> Option<bool> {
    Some(true)
}

/// 更新账号请求体（`PATCH /v2/admin/accounts/{id}`）。
#[derive(Debug, Clone, TS, Serialize, Deserialize)]
#[ts(export, export_to = "admin.ts")]
pub struct AccountUpdateRequest {
    #[ts(as = "Option<String>")]
    pub label: Option<String>,
    /// 传入则轮换 key（旧凭据文件删除）。
    #[ts(as = "Option<String>")]
    pub key: Option<String>,
    #[ts(as = "Option<String>")]
    pub proxy_url: Option<String>,
    #[ts(as = "Option<String>")]
    pub base_url_override: Option<String>,
    /// 静态自定义 header；传入 = 全量替换（空数组 = 清空）。
    #[serde(default)]
    #[ts(optional)]
    pub extra_headers: Option<Vec<HeaderPair>>,
    #[ts(as = "Option<bool>")]
    pub enabled: Option<bool>,
}

/// 目录模型的用途（选择器分组元数据；不参与候选编译）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, TS, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[ts(export, export_to = "admin.ts")]
pub enum ModelPurposeDto {
    /// 回答（普通对话/补全）。
    Chat,
    /// 判断 LLM（组判断等 LLM 后端）。
    JudgeLlm,
    /// 判断 JEV（JEV 判官）。
    Jev,
}

/// 目录中的模型条目（provider 的模型勾选池）。
#[derive(Debug, Clone, TS, Serialize, Deserialize)]
#[ts(export, export_to = "admin.ts")]
pub struct CatalogModel {
    /// 目录 model id（上游 `/v1/models` 的 `id`）。
    pub model_id: String,
    pub display_name: String,
    /// 用户是否勾选为候选。
    pub selected: bool,
    /// 用途（选择器分组元数据；不参与候选编译）。
    pub purpose: ModelPurposeDto,
    /// 能力标签（编译时写入候选 `capability.tags["caps"]`，供组判断参考）。
    pub caps: Vec<String>,
    /// 模型支持的推理档位（models.dev `reasoning_options.effort` 派生；编译时作为候选默认）。
    #[serde(default)]
    pub thinking_levels: Vec<String>,
    /// 编译出的候选 id（`{provider}~{model}@{account}`）；未勾选为空。
    #[ts(as = "Option<String>")]
    pub candidate_id: Option<String>,
    #[ts(as = "Option<u32>")]
    pub context_window: Option<u32>,
    #[ts(as = "Option<u32>")]
    pub max_output: Option<u32>,
    /// micros / MTok；未目录化 = 手填。
    #[ts(as = "Option<PriceDto>")]
    pub price: Option<PriceDto>,
    /// 元数据来源：`catalog` / `models.dev` / `manual`。
    pub source: String,
    /// 已被用户移除（持久隐藏）；刷新目录不会带回。
    pub hidden: bool,
}

/// 目录响应（`GET /v2/admin/providers/{id}/catalog`）。
#[derive(Debug, Clone, TS, Serialize, Deserialize)]
#[ts(export, export_to = "admin.ts")]
pub struct CatalogResponse {
    pub provider_id: String,
    pub refreshed_at: Option<i64>,
    pub models: Vec<CatalogModel>,
}

/// 目录刷新请求（`POST /v2/admin/providers/{id}/catalog/refresh`）。
#[derive(Debug, Clone, TS, Serialize, Deserialize)]
#[ts(export, export_to = "admin.ts")]
pub struct CatalogRefreshRequest {
    /// 用哪个账号的 key 拉目录；空 = 第一个启用的账号。
    #[serde(default)]
    #[ts(optional)]
    pub account_id: Option<String>,
}

/// 勾选请求（`POST /v2/admin/providers/{id}/catalog/select`）。
#[derive(Debug, Clone, TS, Serialize, Deserialize)]
#[ts(export, export_to = "admin.ts")]
pub struct CatalogSelectRequest {
    pub model_id: String,
    pub selected: bool,
    /// 更新用途元数据（`None` = 不变；不影响 selected 行为）。
    #[serde(default)]
    #[ts(optional)]
    pub purpose: Option<ModelPurposeDto>,
    /// 更新能力标签（`None` = 不变；不影响 selected 行为）。
    #[serde(default)]
    #[ts(optional)]
    pub caps: Option<Vec<String>>,
    /// 上下文窗口（token）：`None` = 不变；`Some(0)` = 清除（未设置）。
    #[serde(default)]
    #[ts(optional)]
    pub context_window: Option<u32>,
    /// 最大输出（token）：`None` = 不变；`Some(0)` = 清除（未设置）。
    #[serde(default)]
    #[ts(optional)]
    pub max_output: Option<u32>,
    /// 价格（micros / MTok）：`None` = 不变；全 0 = 清除（未定价）。
    #[serde(default)]
    #[ts(optional)]
    pub price: Option<PriceDto>,
    /// 覆盖默认档位标签（默认用 provider 的 suggested_tier）。
    #[serde(default)]
    #[ts(optional)]
    pub tier: Option<String>,
    /// 覆盖 strength（0-255，越大越强）。
    #[serde(default)]
    #[ts(optional)]
    pub strength: Option<u8>,
    /// auto-include 通配符（如 `gpt-*`）：新出现在目录里的模型自动勾选。
    /// 传入空串清除。
    #[serde(default)]
    #[ts(optional)]
    pub auto_include_pattern: Option<String>,
    /// 取消 deprecated 标记（上游恢复但刷新没清掉的场合手动恢复）。
    #[serde(default)]
    #[ts(optional)]
    pub clear_deprecated: Option<bool>,
    /// 持久隐藏/恢复目录条目（删除/还原）。
    #[serde(default)]
    #[ts(optional)]
    pub hidden: Option<bool>,
}

/// 候选编译结果（`POST /v2/admin/providers/compile`）。
#[derive(Debug, Clone, TS, Serialize, Deserialize)]
#[ts(export, export_to = "admin.ts")]
pub struct CompileResponse {
    pub revision: String,
    pub previous_revision: String,
    /// 新写入 config 的候选 id 列表。
    pub candidate_ids: Vec<String>,
    /// 编译统计。
    pub candidates_total: u32,
    pub virtual_models_total: u32,
    pub backup_path: Option<String>,
}

/// 编译选项。
#[derive(Debug, Clone, TS, Serialize, Deserialize)]
#[ts(export, export_to = "admin.ts")]
pub struct CompileRequest {
    /// If-Match（config CAS）。
    #[serde(default)]
    #[ts(optional)]
    pub expected_revision: Option<String>,
}

// ── 反向导入（v3 P5-A，roadmap §0.3 吸收项）───────────────────────────────

/// `POST /v2/admin/providers/import`：从本机 agent 配置扫描上游端点草稿。
#[derive(Debug, Clone, TS, Serialize, Deserialize)]
#[ts(export, export_to = "admin.ts")]
pub struct ImportScanRequest {
    /// 显式 home 覆盖（测试用）；空 = 服务端宿主 home。
    #[serde(default)]
    #[ts(optional)]
    pub home: Option<String>,
}

/// 扫描出的导入草稿。
#[derive(Debug, Clone, TS, Serialize, Deserialize)]
#[ts(export, export_to = "admin.ts")]
pub struct ImportedProviderDraft {
    pub display_name: String,
    pub base_url: String,
    /// key 引用（env:/file:）；明文不导入，没有引用时用户手填。
    #[serde(default)]
    #[ts(optional)]
    pub credential_ref: Option<String>,
    /// 来源（agent + 配置文件路径）。
    pub source: String,
}

/// `POST /v2/admin/providers/import` 响应：一键落库。
#[derive(Debug, Clone, TS, Serialize, Deserialize)]
#[ts(export, export_to = "admin.ts")]
pub struct ImportScanResponse {
    pub drafts: Vec<ImportedProviderDraft>,
}

// ── 账户用量（供应商 usage API，v3 余额页）────────────────────────────────

/// `GET /v2/admin/provider-usage` 响应。
#[derive(Debug, Clone, TS, Serialize, Deserialize)]
#[ts(export, export_to = "admin.ts")]
pub struct ProviderUsageResponse {
    pub accounts: Vec<AccountUsageInfo>,
}

/// 单个账号的余额/订阅查询结果（每次查询独立，失败不伪造数值）。
#[derive(Debug, Clone, TS, Serialize, Deserialize)]
#[ts(export, export_to = "admin.ts")]
pub struct AccountUsageInfo {
    pub provider_id: String,
    pub provider_name: String,
    pub account_id: String,
    pub account_label: String,
    /// 凭据指纹后 4 位；明文永不出现。
    pub fingerprint: String,
    pub enabled: bool,
    pub status: UsageStatus,
    /// 计划/计费模式（API 返回时才有，例如 OpenRouter 的免费层/按量）。
    #[serde(default)]
    #[ts(optional)]
    pub plan: Option<String>,
    /// 多币种余额条目（不跨条目求和）。
    pub balances: Vec<BalanceInfo>,
    /// 订阅窗口用量（OpenCode Go 等按窗口计费的厂商）。
    #[serde(default)]
    pub windows: Vec<UsageWindowInfo>,
    /// 键级限额（API 返回时才有）。
    #[serde(default)]
    #[ts(optional)]
    pub limits: Option<UsageLimitInfo>,
    /// 本次查询时间（Unix 毫秒）；0 = 未查询。
    pub fetched_at: i64,
    /// 失败原因（`status = error`）。
    #[serde(default)]
    #[ts(optional)]
    pub error: Option<String>,
}

/// 用量查询状态。
#[derive(Debug, Clone, Copy, PartialEq, Eq, TS, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[ts(export, export_to = "admin.ts")]
pub enum UsageStatus {
    /// 查询成功（至少拿到一个余额/计划字段）。
    Ok,
    /// 该厂商没有公开的 usage/余额接口。
    Unsupported,
    /// 请求或解析失败；`error` 给原因。
    Error,
}

/// 一个币种的余额。金额保留 API 原始字符串，避免浮点误差。
#[derive(Debug, Clone, TS, Serialize, Deserialize)]
#[ts(export, export_to = "admin.ts")]
pub struct BalanceInfo {
    pub currency: String,
    /// 可用余额（API 直接给或有明确派生）。
    #[serde(default)]
    #[ts(optional)]
    pub available: Option<String>,
    /// 已用（如 OpenRouter 的 total_usage）。
    #[serde(default)]
    #[ts(optional)]
    pub used: Option<String>,
    /// 总额/已购（如 OpenRouter 的 total_credits）。
    #[serde(default)]
    #[ts(optional)]
    pub total: Option<String>,
    /// 赠送/代金券部分。
    #[serde(default)]
    #[ts(optional)]
    pub granted: Option<String>,
    /// 充值/现金部分。
    #[serde(default)]
    #[ts(optional)]
    pub topped_up: Option<String>,
}

/// 键级限额（如 OpenRouter key 的 limit / limit_remaining / usage）。
#[derive(Debug, Clone, TS, Serialize, Deserialize)]
#[ts(export, export_to = "admin.ts")]
pub struct UsageLimitInfo {
    /// 限额币种（OpenRouter 为 USD）。
    pub currency: String,
    #[serde(default)]
    #[ts(optional)]
    pub limit: Option<String>,
    #[serde(default)]
    #[ts(optional)]
    pub remaining: Option<String>,
    #[serde(default)]
    #[ts(optional)]
    pub used: Option<String>,
}

/// 订阅窗口用量（如 OpenCode Go 的 rolling / weekly / monthly）。
#[derive(Debug, Clone, TS, Serialize, Deserialize)]
#[ts(export, export_to = "admin.ts")]
pub struct UsageWindowInfo {
    /// 窗口标识（`rolling` / `weekly` / `monthly`）。
    pub label: String,
    /// 已用百分比（厂商原始口径，通常 0–100）。
    pub percent: f64,
    /// 厂商标记为已限流。
    pub limited: bool,
    /// 窗口重置时间（Unix 毫秒）。
    #[serde(default)]
    #[ts(optional)]
    pub resets_at: Option<i64>,
}
