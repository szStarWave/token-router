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
    pub key: String,
    #[serde(default)]
    #[ts(optional)]
    pub proxy_url: Option<String>,
    #[serde(default)]
    #[ts(optional)]
    pub base_url_override: Option<String>,
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
    #[ts(as = "Option<bool>")]
    pub enabled: Option<bool>,
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
    /// 覆盖默认档位标签（默认用 provider 的 suggested_tier）。
    #[serde(default)]
    #[ts(optional)]
    pub tier: Option<String>,
    /// 覆盖 strength（0-255，越大越强）。
    #[serde(default)]
    #[ts(optional)]
    pub strength: Option<u8>,
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
    /// 编译后是否为每个 provider 生成/更新一个聚合虚拟模型
    /// （`auto-{provider}`，含其全部候选）。默认 true。
    #[serde(default = "default_some_true")]
    #[ts(optional)]
    pub create_virtual_models: Option<bool>,
    /// If-Match（config CAS）。
    #[serde(default)]
    #[ts(optional)]
    pub expected_revision: Option<String>,
}
