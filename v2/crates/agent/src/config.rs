//! 配置加载与目录校验（§18.2、§18.4.4 的 P0a 子集）。
//!
//! v2 自有 schema，不读取 v1 `config.toml`。`schema_version` 只接受当前版本；
//! 未知字段直接报错（带完整路径），避免“配置看着对、行为不对”。

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tr_contracts::{
    Candidate, CandidateId, CapabilityProfile, InputModality, PlanMode, PriceModel, ProfileConfig,
    ProfileId, ProviderKind, StreamEscalation, ThinkingLevel, UnknownPricePolicy, UpstreamRef,
    VirtualModel, VirtualModelId,
};
use tr_decide::budget::Budget;
use tr_decide::llm::StructuredOutput;

use crate::judge::{Backend, JudgeMode, JudgePrice, JudgeSettings};

/// 当前配置 schema 版本。
pub const SCHEMA_VERSION: u32 = 1;

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("toml: {0}")]
    Toml(#[from] toml::de::Error),
    #[error("unsupported schema_version {found}; 本内核只支持 {supported}")]
    SchemaVersion { found: u32, supported: u32 },
    #[error("invalid config: {0}")]
    Invalid(String),
}

/// 校验后的配置目录。
#[derive(Debug, Clone)]
pub struct Catalog {
    pub server: ServerConfig,
    pub default_virtual_model: VirtualModelId,
    pub profiles: BTreeMap<ProfileId, ProfileConfig>,
    pub candidates: BTreeMap<CandidateId, Candidate>,
    pub virtual_models: BTreeMap<VirtualModelId, VirtualModel>,
    /// 客户端不可见/不可路由的虚拟模型（`enabled = false`）。
    pub disabled_virtual_models: BTreeSet<VirtualModelId>,
    /// `/v1/models` 需要同时列出池内成员的组（`list_members = true`）。
    pub list_members: BTreeSet<VirtualModelId>,
    /// 组级判断与规则投影（每个虚拟模型一项；`off` 时 backend 为 Local）。
    pub group_configs: BTreeMap<VirtualModelId, GroupConfig>,
    /// 判官配置（`off` 时 backend 不构造）。
    pub judge: JudgeSettings,
    /// 成本闸模式（P2 默认 shadow）。
    pub cost_gate: crate::gate::CostGateMode,
    /// 信任档位别名 → profile（P3-14）。只换别名，不改虚拟模型与内核。
    pub trust_tiers: BTreeMap<String, ProfileId>,
    /// 本地记录策略（消息正文等）。
    pub recording: RecordingConfig,
    /// 规范化配置的 SHA-256，写入账本与快照。
    pub config_digest: String,
}

impl Catalog {
    pub fn virtual_model(&self, id: &str) -> Option<&VirtualModel> {
        self.virtual_models.get(id)
    }

    /// 虚拟模型是否对客户端可见/可路由。
    pub fn virtual_model_enabled(&self, id: &str) -> bool {
        self.virtual_models.contains_key(id) && !self.disabled_virtual_models.contains(id)
    }

    /// 客户端可见的虚拟模型（按配置顺序）。
    pub fn enabled_virtual_models(&self) -> impl Iterator<Item = &VirtualModel> {
        self.virtual_models
            .values()
            .filter(|model| !self.disabled_virtual_models.contains(&model.id))
    }

    /// 解析 `group/member` 形式的模型选择（`/v1/models` 在 list_members 时列出）。
    ///
    /// `group` 必须是启用的虚拟模型；`member` 匹配池内候选 id 或候选的上游 model 名。
    /// 同一上游模型有多个账号候选时优先取启用的。
    pub fn resolve_member_selection(
        &self,
        requested: &str,
    ) -> Option<(VirtualModelId, CandidateId)> {
        let (group, member) = requested.split_once('/')?;
        if !self.virtual_model_enabled(group) {
            return None;
        }
        let virtual_model = self.virtual_models.get(group)?;
        let mut fallback: Option<CandidateId> = None;
        for candidate_id in &virtual_model.candidate_pool {
            let Some(candidate) = self.candidates.get(candidate_id) else {
                continue;
            };
            if candidate.id != member && candidate.upstream.model != member {
                continue;
            }
            if candidate.enabled {
                return Some((group.to_string(), candidate.id.clone()));
            }
            if fallback.is_none() {
                fallback = Some(candidate.id.clone());
            }
        }
        fallback.map(|id| (group.to_string(), id))
    }

    pub fn candidate(&self, id: &str) -> Option<&Candidate> {
        self.candidates.get(id)
    }

    /// 虚拟模型的组级判断/规则投影（agent 层扩展）。
    pub fn group_config(&self, id: &str) -> Option<&GroupConfig> {
        self.group_configs.get(id)
    }

    /// 某个虚拟模型的候选池（按配置顺序）。
    pub fn pool(&self, virtual_model: &VirtualModel) -> Vec<Candidate> {
        virtual_model
            .candidate_pool
            .iter()
            .filter_map(|id| self.candidates.get(id).cloned())
            .collect()
    }

    pub fn profile_for(&self, virtual_model: &VirtualModel) -> Option<&ProfileConfig> {
        self.profiles.get(&virtual_model.profile)
    }

    /// 按信任档位别名解析 profile（P3-14）。
    ///
    /// 未知/未配置别名回退虚拟模型自己的 profile；返回 `(profile, alias_applied)`。
    pub fn profile_for_trust<'a>(
        &'a self,
        virtual_model: &'a VirtualModel,
        trust_tier: Option<&str>,
    ) -> Option<(&'a ProfileConfig, bool)> {
        if let Some(alias) = trust_tier
            && let Some(profile_id) = self.trust_tiers.get(alias)
            && let Some(profile) = self.profiles.get(profile_id)
        {
            return Some((profile, true));
        }
        self.profile_for(virtual_model)
            .map(|profile| (profile, false))
    }
}

/// 服务端配置。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ServerConfig {
    pub listen: String,
    pub auth_enabled: bool,
    pub auth_mode: String,
    pub admin_token: String,
    /// 入站 key；`auth_enabled = true` 时必填。
    pub inbound_key: String,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            listen: "127.0.0.1:16621".to_string(),
            auth_enabled: false,
            auth_mode: "inbound_key".to_string(),
            admin_token: String::new(),
            inbound_key: String::new(),
        }
    }
}

/// 顶层配置。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConfigFile {
    pub schema_version: u32,
    #[serde(default)]
    pub default_virtual_model: Option<VirtualModelId>,
    #[serde(default)]
    pub server: ServerConfig,
    #[serde(default)]
    pub profiles: BTreeMap<ProfileId, ProfileFile>,
    #[serde(default)]
    pub candidates: Vec<CandidateFile>,
    #[serde(default)]
    pub virtual_models: Vec<VirtualModelFile>,
    #[serde(default)]
    pub judge: JudgeFile,
    #[serde(default)]
    pub cost_gate: CostGateFile,
    /// 信任档位别名（P3-14）：`trial = "profile-id"`。
    #[serde(default)]
    pub trust_tiers: BTreeMap<String, ProfileId>,
    /// 本地记录策略：消息正文是否落库（活动 → 会话 查看轮次消息）。
    #[serde(default)]
    pub recording: RecordingFile,
}

/// 本地记录配置。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct RecordingFile {
    /// 是否把每轮请求的消息与模型回复写入本地库。
    /// 关闭后新请求不再记录（历史记录保留）。
    pub transcript: bool,
}

impl Default for RecordingFile {
    fn default() -> Self {
        Self { transcript: true }
    }
}

/// 运行时记录策略（编译后的 `[recording]`）。
#[derive(Debug, Clone, Copy)]
pub struct RecordingConfig {
    pub transcript: bool,
}

/// 成本闸配置。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct CostGateFile {
    /// `off` | `shadow` | `active`
    pub mode: String,
}

impl Default for CostGateFile {
    fn default() -> Self {
        Self {
            mode: "shadow".to_string(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProfileFile {
    #[serde(default)]
    pub mode: PlanMode,
    #[serde(default)]
    pub stream_escalation: StreamEscalation,
    #[serde(default)]
    pub strength_order: Vec<String>,
    #[serde(default)]
    pub difficulty_weight_micros: i64,
    #[serde(default)]
    pub hard_signal_weight_micros: i64,
    #[serde(default)]
    pub unknown_price_policy: UnknownPricePolicy,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CandidateFile {
    pub id: CandidateId,
    #[serde(default)]
    pub tier: String,
    #[serde(default)]
    pub strength: u8,
    #[serde(default = "default_provider")]
    pub provider: String,
    pub base_url: String,
    pub model: String,
    #[serde(default)]
    pub credential: Option<String>,
    /// 每 key 代理（v3 P5-A，如 http://127.0.0.1:7890）。
    #[serde(default)]
    pub proxy: Option<String>,
    /// 静态自定义 header（非敏感值；随请求附加）。
    #[serde(default)]
    pub extra_headers: BTreeMap<String, String>,
    /// 能力标签（目录元数据；编译时进 capability.tags，供组判断能力模式读取）。
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub caps: Vec<String>,
    #[serde(default)]
    pub window: u32,
    #[serde(default)]
    pub input: Vec<InputModality>,
    #[serde(default)]
    pub thinking_levels: Vec<ThinkingLevel>,
    /// 候选级「关闭思考」：该候选一律按 off 处理（剥离 reasoning + 厂商关闭开关）。
    #[serde(default, skip_serializing_if = "is_false")]
    pub disable_thinking: bool,
    /// 候选级输出上限：请求 `max_tokens` 取 min(客户端值, 上限)，未给时填上限。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_tokens: Option<u32>,
    #[serde(default)]
    pub price: Option<PriceFile>,
    #[serde(default = "default_true")]
    pub enabled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PriceFile {
    #[serde(default)]
    pub currency: String,
    #[serde(default)]
    pub input_micros_per_mtok: u64,
    #[serde(default)]
    pub output_micros_per_mtok: u64,
    #[serde(default)]
    pub cache_read_micros_per_mtok: Option<u64>,
    #[serde(default)]
    pub cache_write_micros_per_mtok: Option<u64>,
    #[serde(default)]
    pub request_micros: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VirtualModelFile {
    #[serde(default = "default_provider")]
    pub provider: String,
    pub id: VirtualModelId,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub thinking_levels: Vec<ThinkingLevel>,
    /// 组级「关闭思考」：本组请求一律按 `off` 处理——剥离推理参数，
    /// 已知厂商（硅基流动/DashScope/智谱/OpenRouter）附带各自的关闭开关。
    #[serde(default, skip_serializing_if = "is_false")]
    pub disable_thinking: bool,
    #[serde(default)]
    pub context_window: Option<u32>,
    /// 上下文窗口提示的来源：`min` 取池内最小、`max` 取池内最大、`custom` 用 `context_window`。
    /// 不写时保持旧语义（有 `context_window` 即自定义，否则取最小）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_window_mode: Option<WindowHintMode>,
    #[serde(default)]
    pub max_tokens: Option<u32>,
    #[serde(default)]
    pub input: Vec<InputModality>,
    pub profile: ProfileId,
    #[serde(default)]
    pub strategies: Vec<String>,
    #[serde(default)]
    pub candidate_pool: Vec<CandidateId>,
    /// `/v1/models` 是否同时列出池内成员（`<group>/<model>`），默认只列本组。
    #[serde(default, skip_serializing_if = "is_false")]
    pub list_members: bool,
    /// 客户端是否可见/可路由；默认 true，默认值不序列化以保持旧配置 digest。
    #[serde(default = "default_true", skip_serializing_if = "is_true")]
    pub enabled: bool,
    /// 组级判断（agent 层扩展）：默认 off，不解析凭据。
    /// 默认值不序列化，避免升级后无组判断的旧配置 `config_digest` 变化。
    #[serde(default, skip_serializing_if = "GroupJudgeFile::is_default")]
    pub judge: GroupJudgeFile,
    /// 组级规则（按配置顺序）；有规则时 judge.mode 必须为 active。
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub rules: Vec<GroupRuleFile>,
}

fn default_provider() -> String {
    "openai".to_string()
}

/// 判官配置（§18.2 `[judge]`）。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct JudgeFile {
    /// `off` | `shadow` | `active`
    pub mode: String,
    pub max_calls_per_turn: u32,
    pub max_calls_per_minute: u32,
    pub max_calls_per_session: u32,
    pub deadline_ms: u32,
    pub service: JudgeServiceFile,
}

impl Default for JudgeFile {
    fn default() -> Self {
        Self {
            mode: "off".to_string(),
            max_calls_per_turn: 1,
            max_calls_per_minute: 30,
            max_calls_per_session: 200,
            deadline_ms: 300,
            service: JudgeServiceFile::default(),
        }
    }
}

/// 判官服务配置。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct JudgeServiceFile {
    /// `local` | `http` | `llm` | `jev`
    pub kind: String,
    pub endpoint: String,
    pub model: String,
    pub service_id: String,
    pub revision: u64,
    pub credential: Option<String>,
    pub timeout_ms: u32,
    /// `json_schema` | `json_mode` | `tool_call`
    pub structured_output: String,
    pub temperature: f32,
    pub max_output_tokens: u32,
    /// 判官调用关闭思考（默认 true）：只看本字段，不受候选/提供商配置影响；
    /// 按 endpoint 主机注入厂商开关（硅基流动 `enable_thinking:false` 等）。
    #[serde(default = "default_true")]
    pub disable_thinking: bool,
    /// 判官计价（micros 量纲）；用于把 token 用量折算进 `routing_cost`。
    pub currency: String,
    pub input_micros_per_mtok: u64,
    pub output_micros_per_mtok: u64,
    pub request_micros: u64,
}

impl Default for JudgeServiceFile {
    fn default() -> Self {
        Self {
            kind: "local".to_string(),
            endpoint: String::new(),
            model: String::new(),
            service_id: "local-judge".to_string(),
            revision: 1,
            credential: None,
            timeout_ms: 300,
            structured_output: "json_mode".to_string(),
            temperature: 0.0,
            max_output_tokens: 256,
            disable_thinking: true,
            currency: "USD".to_string(),
            input_micros_per_mtok: 0,
            output_micros_per_mtok: 0,
            request_micros: 0,
        }
    }
}

fn default_true() -> bool {
    true
}

fn is_true(value: &bool) -> bool {
    *value
}

fn is_false(value: &bool) -> bool {
    !*value
}

/// 虚拟模型上下文窗口提示的来源。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WindowHintMode {
    /// 取池内启用候选的最小窗口（默认行为）。
    Min,
    /// 取池内启用候选的最大窗口。
    Max,
    /// 显式 `context_window`。
    Custom,
}

/// 组级判断配置（挂在 `VirtualModelFile` 上，agent 层扩展）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct GroupJudgeFile {
    /// `off` | `active`（本批明确不接受 `shadow`）。
    pub mode: String,
    pub max_calls_per_turn: u32,
    pub max_calls_per_minute: u32,
    pub max_calls_per_session: u32,
    pub deadline_ms: u32,
    /// 复用判官服务配置；`active` 时只允许 `kind = "llm"` / `"jev"`。
    pub service: JudgeServiceFile,
}

impl Default for GroupJudgeFile {
    fn default() -> Self {
        Self {
            mode: "off".to_string(),
            max_calls_per_turn: 1,
            max_calls_per_minute: 30,
            max_calls_per_session: 200,
            deadline_ms: 300,
            service: JudgeServiceFile::default(),
        }
    }
}

impl GroupJudgeFile {
    fn is_default(&self) -> bool {
        *self == Self::default()
    }
}

/// 组级规则（按配置顺序展示给判断模型）。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GroupRuleFile {
    pub id: String,
    pub intent: String,
    pub prefer: CandidateId,
}

/// 组级规则的运行时投影。
#[derive(Debug, Clone)]
pub struct GroupRule {
    pub id: String,
    pub intent: String,
    pub prefer: CandidateId,
}

/// 组级判断与规则的运行时投影（agent 内部）。
///
/// `digest` 由原始 `GroupJudgeFile + GroupRuleFile` 的规范化 JSON 取 SHA-256；
/// 不序列化含凭据明文的 `JudgeSettings`。
#[derive(Debug, Clone)]
pub struct GroupConfig {
    pub judge: crate::judge::JudgeSettings,
    pub rules: Vec<GroupRule>,
    /// 组级关闭思考：请求一律按 `off` 处理。
    pub disable_thinking: bool,
    pub digest: String,
}

impl ConfigFile {
    pub fn parse(text: &str) -> Result<Self, ConfigError> {
        Ok(toml::from_str(text)?)
    }

    /// 把判官服务里的相对 `file:` 凭据引用解析到配置文件所在目录。
    ///
    /// sidecar 的工作目录不保证是数据目录（由桌面壳/CLI 启动），
    /// `file:credentials/<account>.key` 这类相对引用必须在加载时按配置文件
    /// 位置展开；环境变量与绝对路径保持原样。只改内存中的引用，不回写配置。
    pub fn resolve_file_credentials(&mut self, base: &Path) {
        fn fix(credential: &mut Option<String>, base: &Path) {
            let Some(value) = credential.as_deref() else {
                return;
            };
            let Some(rest) = value.strip_prefix("file:") else {
                return;
            };
            if Path::new(rest).is_absolute() {
                return;
            }
            *credential = Some(format!("file:{}", base.join(rest).display()));
        }

        fix(&mut self.judge.service.credential, base);
        for virtual_model in &mut self.virtual_models {
            fix(&mut virtual_model.judge.service.credential, base);
        }
    }

    /// 校验并转成运行时目录。
    pub fn into_catalog(self) -> Result<Catalog, ConfigError> {
        if self.schema_version != SCHEMA_VERSION {
            return Err(ConfigError::SchemaVersion {
                found: self.schema_version,
                supported: SCHEMA_VERSION,
            });
        }
        if self.virtual_models.is_empty() {
            return Err(ConfigError::Invalid(
                "至少需要一个 [[virtual_models]]".to_string(),
            ));
        }
        if self.server.auth_enabled && self.server.inbound_key.is_empty() {
            return Err(ConfigError::Invalid(
                "auth_enabled = true 时必须设置 server.inbound_key".to_string(),
            ));
        }

        let config_digest = digest_json(&self)?;

        let mut profiles = BTreeMap::new();
        for (id, profile) in &self.profiles {
            profiles.insert(
                id.clone(),
                ProfileConfig {
                    id: id.clone(),
                    mode: profile.mode,
                    stream_escalation: profile.stream_escalation,
                    strength_order: profile.strength_order.clone(),
                    difficulty_weight_micros: profile.difficulty_weight_micros,
                    hard_signal_weight_micros: profile.hard_signal_weight_micros,
                    unknown_price_policy: profile.unknown_price_policy,
                    revision: digest_json(profile)?[..16].to_string(),
                    config_digest: config_digest.clone(),
                },
            );
        }

        let mut candidates = BTreeMap::new();
        for file in &self.candidates {
            if file.window == 0 {
                return Err(ConfigError::Invalid(format!(
                    "candidate `{}` 的 window 必须 > 0",
                    file.id
                )));
            }
            if candidates.contains_key(&file.id) {
                return Err(ConfigError::Invalid(format!(
                    "candidate id `{}` 重复",
                    file.id
                )));
            }
            candidates.insert(file.id.clone(), to_candidate(file));
        }

        let mut virtual_models = BTreeMap::new();
        let mut disabled_virtual_models = BTreeSet::new();
        let mut list_members = BTreeSet::new();
        let mut group_configs = BTreeMap::new();
        for file in &self.virtual_models {
            if !profiles.contains_key(&file.profile) {
                return Err(ConfigError::Invalid(format!(
                    "virtual model `{}` 引用了不存在的 profile `{}`",
                    file.id, file.profile
                )));
            }
            if file.candidate_pool.is_empty() {
                return Err(ConfigError::Invalid(format!(
                    "virtual model `{}` 的 candidate_pool 不能为空",
                    file.id
                )));
            }
            if virtual_models.contains_key(&file.id) {
                return Err(ConfigError::Invalid(format!(
                    "virtual model id `{}` 重复",
                    file.id
                )));
            }
            for candidate_id in &file.candidate_pool {
                if !candidates.contains_key(candidate_id) {
                    return Err(ConfigError::Invalid(format!(
                        "virtual model `{}` 引用了不存在的 candidate `{candidate_id}`",
                        file.id
                    )));
                }
            }
            if file.context_window_mode == Some(WindowHintMode::Custom)
                && file.context_window.is_none()
            {
                return Err(ConfigError::Invalid(format!(
                    "virtual model `{}` 的 context_window_mode = custom 时必须设置 context_window",
                    file.id
                )));
            }
            // 窗口提示：max 取池内最大（优先启用候选），min 交给 pipeline 现算（None），
            // custom 用显式值；未写 mode 时保持旧语义（有值即自定义）。
            let effective_window = match file.context_window_mode {
                Some(WindowHintMode::Max) => pool_window_max(file, &candidates),
                Some(WindowHintMode::Min) => None,
                Some(WindowHintMode::Custom) | None => file.context_window,
            };
            // 组级判断/规则校验与运行时投影（agent 层扩展）。
            group_configs.insert(file.id.clone(), build_group_config(file)?);
            if !file.enabled {
                disabled_virtual_models.insert(file.id.clone());
            }
            if file.list_members {
                list_members.insert(file.id.clone());
            }
            virtual_models.insert(
                file.id.clone(),
                VirtualModel {
                    provider: file.provider.clone(),
                    id: file.id.clone(),
                    name: if file.name.is_empty() {
                        file.id.clone()
                    } else {
                        file.name.clone()
                    },
                    thinking_levels: file.thinking_levels.clone(),
                    context_window: effective_window,
                    max_tokens: file.max_tokens,
                    input: file.input.clone(),
                    profile: file.profile.clone(),
                    strategies: file.strategies.clone(),
                    candidate_pool: file.candidate_pool.clone(),
                },
            );
        }

        let default_virtual_model = match self.default_virtual_model {
            Some(id) if disabled_virtual_models.contains(&id) => {
                return Err(ConfigError::Invalid(format!(
                    "default_virtual_model `{id}` 已停用"
                )));
            }
            Some(id) if virtual_models.contains_key(&id) => id,
            Some(id) => {
                return Err(ConfigError::Invalid(format!(
                    "default_virtual_model `{id}` 不存在"
                )));
            }
            None => {
                if virtual_models.contains_key("auto") && !disabled_virtual_models.contains("auto")
                {
                    "auto".to_string()
                } else {
                    virtual_models
                        .keys()
                        .find(|id| !disabled_virtual_models.contains(*id))
                        .cloned()
                        .ok_or_else(|| {
                            ConfigError::Invalid(
                                "至少需要一个启用的虚拟模型作为默认入口".to_string(),
                            )
                        })?
                }
            }
        };

        // 档位顺序必须覆盖候选实际用到的 tier，否则排序会静默落到“未知档”。
        for (id, profile) in &profiles {
            let known: BTreeSet<&str> = profile.strength_order.iter().map(String::as_str).collect();
            if known.is_empty() {
                continue;
            }
            for virtual_model in virtual_models.values().filter(|vm| &vm.profile == id) {
                for candidate_id in &virtual_model.candidate_pool {
                    let tier = &candidates[candidate_id].tier;
                    if !known.contains(tier.as_str()) {
                        return Err(ConfigError::Invalid(format!(
                            "profile `{id}` 的 strength_order 缺少候选档位 `{tier}`（candidate `{candidate_id}`）"
                        )));
                    }
                }
            }
        }

        let judge = build_judge_settings(&self.judge)?;
        let cost_gate =
            crate::gate::CostGateMode::parse(&self.cost_gate.mode).ok_or_else(|| {
                ConfigError::Invalid(format!(
                    "cost_gate.mode 只能是 off/shadow/active，得到 `{}`",
                    self.cost_gate.mode
                ))
            })?;
        // 信任档位只做别名：目标 profile 必须存在，否则评审时看到的是“静默不生效”。
        for (alias, profile_id) in &self.trust_tiers {
            if !profiles.contains_key(profile_id) {
                return Err(ConfigError::Invalid(format!(
                    "trust_tiers.{alias} 引用了不存在的 profile `{profile_id}`"
                )));
            }
        }

        Ok(Catalog {
            server: self.server,
            default_virtual_model,
            profiles,
            candidates,
            virtual_models,
            disabled_virtual_models,
            list_members,
            group_configs,
            judge,
            cost_gate,
            trust_tiers: self.trust_tiers,
            recording: RecordingConfig {
                transcript: self.recording.transcript,
            },
            config_digest,
        })
    }
}

fn pool_window_max(
    file: &VirtualModelFile,
    candidates: &BTreeMap<CandidateId, Candidate>,
) -> Option<u32> {
    let mut enabled: Option<u32> = None;
    let mut any: Option<u32> = None;
    for candidate in file
        .candidate_pool
        .iter()
        .filter_map(|id| candidates.get(id))
    {
        any = Some(any.map_or(candidate.window, |value| value.max(candidate.window)));
        if candidate.enabled {
            enabled = Some(enabled.map_or(candidate.window, |value| value.max(candidate.window)));
        }
    }
    enabled.or(any)
}

fn build_judge_settings(file: &JudgeFile) -> Result<JudgeSettings, ConfigError> {
    let mode = JudgeMode::parse(&file.mode).ok_or_else(|| {
        ConfigError::Invalid(format!(
            "judge.mode 只能是 off/shadow/active，得到 `{}`",
            file.mode
        ))
    })?;

    let service = &file.service;

    Ok(JudgeSettings {
        mode,
        budget: Budget {
            max_calls_per_turn: file.max_calls_per_turn,
            max_calls_per_minute: file.max_calls_per_minute,
            max_calls_per_session: file.max_calls_per_session,
            deadline_ms: file.deadline_ms,
        },
        backend: build_backend(service)?,
        price: JudgePrice {
            input_micros_per_mtok: service.input_micros_per_mtok,
            output_micros_per_mtok: service.output_micros_per_mtok,
            request_micros: service.request_micros,
        },
        currency: service.currency.clone(),
    })
}

/// 组级判断/规则的校验与运行时投影。
fn build_group_config(file: &VirtualModelFile) -> Result<GroupConfig, ConfigError> {
    let judge_file = &file.judge;
    let mode = match judge_file.mode.as_str() {
        "off" => JudgeMode::Off,
        "active" => JudgeMode::Active,
        other => {
            return Err(ConfigError::Invalid(format!(
                "virtual model `{}` 的 judge.mode 只能是 off/active，得到 `{other}`",
                file.id
            )));
        }
    };

    // 规则存在时必须启用判断；无规则时 off/active 都合法（能力选择模式）。
    if !file.rules.is_empty() && mode != JudgeMode::Active {
        return Err(ConfigError::Invalid(format!(
            "virtual model `{}` 配置了 {} 条规则，必须启用 judge",
            file.id,
            file.rules.len()
        )));
    }

    let mut rule_ids = BTreeSet::new();
    let mut rules = Vec::with_capacity(file.rules.len());
    for rule in &file.rules {
        if rule.id.trim().is_empty() {
            return Err(ConfigError::Invalid(format!(
                "virtual model `{}` 的规则 id 不能为空",
                file.id
            )));
        }
        if !rule_ids.insert(rule.id.as_str()) {
            return Err(ConfigError::Invalid(format!(
                "virtual model `{}` 的规则 id `{}` 重复",
                file.id, rule.id
            )));
        }
        if rule.intent.trim().is_empty() {
            return Err(ConfigError::Invalid(format!(
                "virtual model `{}` 的规则 `{}` 的 intent 不能为空",
                file.id, rule.id
            )));
        }
        if !file.candidate_pool.contains(&rule.prefer) {
            return Err(ConfigError::Invalid(format!(
                "virtual model `{}` 的规则 `{}` 的 prefer `{}` 不在 candidate_pool 中",
                file.id, rule.id, rule.prefer
            )));
        }
        rules.push(GroupRule {
            id: rule.id.clone(),
            intent: rule.intent.clone(),
            prefer: rule.prefer.clone(),
        });
    }

    let backend = if mode == JudgeMode::Active {
        match judge_file.service.kind.as_str() {
            // 复用判官后端构造（含凭据解析与结构化输出校验）。
            "llm" | "jev" => build_backend(&judge_file.service)?,
            other => {
                return Err(ConfigError::Invalid(format!(
                    "virtual model `{}` 的 judge.service.kind 本批只支持 llm/jev，得到 `{other}`",
                    file.id
                )));
            }
        }
    } else {
        // off 时不解析凭据、不要求 endpoint/model。
        Backend::Local
    };

    let judge = JudgeSettings {
        mode,
        budget: Budget {
            max_calls_per_turn: judge_file.max_calls_per_turn,
            max_calls_per_minute: judge_file.max_calls_per_minute,
            max_calls_per_session: judge_file.max_calls_per_session,
            deadline_ms: judge_file.deadline_ms,
        },
        backend,
        price: JudgePrice {
            input_micros_per_mtok: judge_file.service.input_micros_per_mtok,
            output_micros_per_mtok: judge_file.service.output_micros_per_mtok,
            request_micros: judge_file.service.request_micros,
        },
        currency: judge_file.service.currency.clone(),
    };

    // digest 只取原始文件（规范化 JSON），不序列化含凭据明文的 JudgeSettings。
    // 关闭思考改变请求语义，纳入 digest 使旧 ContextHold 失效。
    let digest = digest_json(&(judge_file, &file.rules, file.disable_thinking))?;

    Ok(GroupConfig {
        judge,
        rules,
        disable_thinking: file.disable_thinking,
        digest,
    })
}

/// 从 `JudgeServiceFile` 构造后端（解析凭据引用）。
fn build_backend(service: &JudgeServiceFile) -> Result<Backend, ConfigError> {
    let credential = match &service.credential {
        Some(reference) => resolve_credential(reference)?,
        None => None,
    };

    match service.kind.as_str() {
        "local" => Ok(Backend::Local),
        "http" => {
            if service.endpoint.is_empty() {
                return Err(ConfigError::Invalid(
                    "judge.service.kind = http 时 endpoint 必填".to_string(),
                ));
            }
            Ok(Backend::Http {
                endpoint: service.endpoint.clone(),
                service_id: service.service_id.clone(),
                revision: service.revision,
                credential,
                timeout_ms: service.timeout_ms,
            })
        }
        "llm" => {
            if service.endpoint.is_empty() || service.model.is_empty() {
                return Err(ConfigError::Invalid(
                    "judge.service.kind = llm 时 endpoint/model 必填".to_string(),
                ));
            }
            Ok(Backend::Llm {
                endpoint: service.endpoint.clone(),
                model: service.model.clone(),
                credential,
                structured_output: match service.structured_output.as_str() {
                    "json_schema" => StructuredOutput::JsonSchema,
                    "tool_call" => StructuredOutput::ToolCall,
                    "json_mode" => StructuredOutput::JsonMode,
                    other => {
                        return Err(ConfigError::Invalid(format!(
                            "judge.service.structured_output 不支持 `{other}`"
                        )));
                    }
                },
                temperature: service.temperature,
                max_output_tokens: service.max_output_tokens,
                timeout_ms: service.timeout_ms,
                service_id: service.service_id.clone(),
                disable_thinking: service.disable_thinking,
            })
        }
        "jev" => {
            if service.endpoint.is_empty() || service.model.is_empty() {
                return Err(ConfigError::Invalid(
                    "judge.service.kind = jev 时 endpoint/model 必填".to_string(),
                ));
            }
            if !is_systemone_endpoint(&service.endpoint) {
                return Err(ConfigError::Invalid(
                    "judge.service.kind = jev 时 endpoint 必须是以 /v1/systemone 结尾的完整 SystemOne 路径"
                        .to_string(),
                ));
            }
            Ok(Backend::Jev {
                endpoint: service.endpoint.clone(),
                model: service.model.clone(),
                credential,
                timeout_ms: service.timeout_ms,
                service_id: service.service_id.clone(),
            })
        }
        other => Err(ConfigError::Invalid(format!(
            "judge.service.kind 不支持 `{other}`"
        ))),
    }
}

/// JEV 的 endpoint 必须指向完整 SystemOne 路径。
///
/// query string / fragment 不算 path，所以先截掉再检查；这样
/// `https://host/v1/systemone?trace=1` 仍然合法。
fn is_systemone_endpoint(endpoint: &str) -> bool {
    let without_fragment = endpoint.split('#').next().unwrap_or_default();
    let path = without_fragment.split('?').next().unwrap_or_default();
    path.ends_with("/v1/systemone")
}

/// 解析 `env:` / `file:` 引用；没有前缀时按字面量处理（仅用于测试）。
fn resolve_credential(reference: &str) -> Result<Option<String>, ConfigError> {
    if let Some(name) = reference.strip_prefix("env:") {
        return std::env::var(name)
            .map(Some)
            .map_err(|_| ConfigError::Invalid(format!("判官凭据环境变量 `{name}` 未设置")));
    }
    if let Some(path) = reference.strip_prefix("file:") {
        let value = std::fs::read_to_string(path)?;
        return Ok(Some(value.trim().to_string()));
    }
    Ok(Some(reference.to_string()))
}

/// 从文件加载。
///
/// 判官凭据的相对 `file:` 引用以配置文件所在目录为基准解析（见
/// [`ConfigFile::resolve_file_credentials`]），不依赖进程工作目录。
pub fn load(path: impl AsRef<Path>) -> Result<Catalog, ConfigError> {
    let path = path.as_ref();
    let text = std::fs::read_to_string(path)?;
    let mut file = ConfigFile::parse(&text)?;
    if let Some(base) = config_base_dir(path) {
        file.resolve_file_credentials(&base);
    }
    file.into_catalog()
}

/// 配置文件所在目录（绝对化失败时退回原始 parent）。
fn config_base_dir(path: &Path) -> Option<std::path::PathBuf> {
    if let Ok(absolute) = std::fs::canonicalize(path) {
        if let Some(parent) = absolute.parent() {
            return Some(parent.to_path_buf());
        }
    }
    path.parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .map(Path::to_path_buf)
}

/// proxy/header/caps 进 capability.tags：内核只透传，upstream/组判断消费。
fn candidate_tags(file: &CandidateFile) -> BTreeMap<String, String> {
    let mut tags = BTreeMap::new();
    if let Some(proxy) = &file.proxy {
        tags.insert("proxy_url".to_string(), proxy.clone());
    }
    for (name, value) in &file.extra_headers {
        tags.insert(format!("header:{name}"), value.clone());
    }
    if !file.caps.is_empty() {
        tags.insert("caps".to_string(), file.caps.join(","));
    }
    // 候选级执行策略：upstream 读取（内核不解析）。
    if file.disable_thinking {
        tags.insert("disable_thinking".to_string(), "1".to_string());
    }
    if let Some(limit) = file.max_tokens {
        tags.insert("max_tokens".to_string(), limit.to_string());
    }
    tags
}

fn to_candidate(file: &CandidateFile) -> Candidate {
    Candidate {
        id: file.id.clone(),
        tier: file.tier.clone(),
        strength: file.strength,
        upstream: UpstreamRef {
            provider: parse_provider(&file.provider),
            base_url: file.base_url.clone(),
            model: file.model.clone(),
            credential: file.credential.clone().map(tr_contracts::CredentialRef),
        },
        window: file.window,
        capability: CapabilityProfile {
            input: file.input.clone(),
            thinking_levels: file.thinking_levels.clone(),
            // proxy/自定义 header 经 tags 携带（契约既有扩展位，内核不解析）。
            tags: candidate_tags(file),
            ..CapabilityProfile::default()
        },
        price: file.price.as_ref().map(|price| PriceModel {
            currency: price.currency.clone(),
            input_micros_per_mtok: price.input_micros_per_mtok,
            output_micros_per_mtok: price.output_micros_per_mtok,
            cache_read_micros_per_mtok: price.cache_read_micros_per_mtok,
            cache_write_micros_per_mtok: price.cache_write_micros_per_mtok,
            request_micros: price.request_micros,
        }),
        enabled: file.enabled,
    }
}

fn parse_provider(name: &str) -> ProviderKind {
    match name {
        "openai" | "openai_compatible" => ProviderKind::OpenAiCompatible,
        "anthropic" => ProviderKind::Anthropic,
        "responses" => ProviderKind::Responses,
        other => ProviderKind::Other(other.to_string()),
    }
}

fn digest_json<T: Serialize>(value: &T) -> Result<String, ConfigError> {
    let bytes = serde_json::to_vec(value)
        .map_err(|error| ConfigError::Invalid(format!("配置无法规范化: {error}")))?;
    let digest = Sha256::digest(&bytes);
    let mut out = String::with_capacity(digest.len() * 2);
    for byte in digest {
        use std::fmt::Write as _;
        let _ = write!(out, "{byte:02x}");
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::judge::JudgeMode;

    const MINIMAL: &str = r#"
schema_version = 1

[server]
listen = "127.0.0.1:16621"

[profiles.default]
mode = "cascade"
strength_order = ["local", "cloud"]

[[candidates]]
id = "qwen-local"
tier = "local"
strength = 10
provider = "openai"
base_url = "http://127.0.0.1:11434/v1"
model = "qwen2.5:14b"
window = 32768
input = ["text"]

[[virtual_models]]
provider = "router"
id = "auto"
name = "Auto"
profile = "default"
candidate_pool = ["qwen-local"]
"#;

    const RULE_CODING: &str = r#"
[[virtual_models.rules]]
id = "coding"
intent = "代码实现与调试"
prefer = "qwen-local"
"#;

    fn catalog_from(toml: &str) -> Result<Catalog, ConfigError> {
        ConfigFile::parse(toml)?.into_catalog()
    }

    fn catalog_with(extra: &str) -> Result<Catalog, ConfigError> {
        catalog_from(&format!("{MINIMAL}\n{extra}"))
    }

    fn active_group(judge_extra: &str, rules: &str) -> String {
        format!(
            "{MINIMAL}\n\
             [virtual_models.judge]\n\
             mode = \"active\"\n\
             {judge_extra}\n\
             \n\
             [virtual_models.judge.service]\n\
             kind = \"llm\"\n\
             endpoint = \"http://127.0.0.1:11435/v1\"\n\
             model = \"gemma-2b\"\n\
             {rules}"
        )
    }

    #[test]
    fn minimal_config_loads() {
        let catalog = ConfigFile::parse(MINIMAL)
            .expect("parse")
            .into_catalog()
            .expect("catalog");
        assert_eq!(catalog.default_virtual_model, "auto");
        assert_eq!(catalog.candidates.len(), 1);
        assert_eq!(catalog.config_digest.len(), 64);
    }

    #[test]
    fn unknown_field_is_rejected() {
        let broken = MINIMAL.replace("strength_order", "strength_ordr");
        let error = ConfigFile::parse(&broken).expect_err("must reject");
        assert!(error.to_string().contains("strength_ordr"), "{error}");
    }

    #[test]
    fn schema_version_is_enforced() {
        let future = MINIMAL.replace("schema_version = 1", "schema_version = 99");
        let error = ConfigFile::parse(&future)
            .expect("parse")
            .into_catalog()
            .expect_err("must reject");
        assert!(matches!(
            error,
            ConfigError::SchemaVersion { found: 99, .. }
        ));
    }

    #[test]
    fn missing_candidate_in_pool_is_rejected() {
        let broken = MINIMAL.replace(
            r#"candidate_pool = ["qwen-local"]"#,
            r#"candidate_pool = ["nope"]"#,
        );
        let error = ConfigFile::parse(&broken)
            .expect("parse")
            .into_catalog()
            .expect_err("must reject");
        assert!(error.to_string().contains("nope"), "{error}");
    }

    #[test]
    fn auth_enabled_requires_inbound_key() {
        let broken = MINIMAL.replace(
            r#"listen = "127.0.0.1:16621""#,
            "listen = \"127.0.0.1:16621\"\nauth_enabled = true",
        );
        let error = ConfigFile::parse(&broken)
            .expect("parse")
            .into_catalog()
            .expect_err("must reject");
        assert!(error.to_string().contains("inbound_key"), "{error}");
    }

    #[test]
    fn trust_tier_alias_resolves_profile() {
        let toml = format!("{MINIMAL}\n[trust_tiers]\nstrict = \"default\"\n");
        let catalog = ConfigFile::parse(&toml)
            .expect("parse")
            .into_catalog()
            .expect("catalog");
        let virtual_model = catalog.virtual_model("auto").expect("vm");
        let (profile, applied) = catalog
            .profile_for_trust(virtual_model, Some("strict"))
            .expect("profile");
        assert!(applied);
        assert_eq!(profile.id, "default");

        let (fallback, applied) = catalog
            .profile_for_trust(virtual_model, Some("unknown"))
            .expect("fallback");
        assert!(!applied);
        assert_eq!(fallback.id, "default");
    }

    #[test]
    fn trust_tier_to_missing_profile_is_rejected() {
        let toml = format!("{MINIMAL}\n[trust_tiers]\nstrict = \"nope\"\n");
        let error = ConfigFile::parse(&toml)
            .expect("parse")
            .into_catalog()
            .expect_err("must reject");
        assert!(error.to_string().contains("trust_tiers.strict"), "{error}");
    }

    #[test]
    fn disabled_virtual_model_is_loaded_and_filtered() {
        let toml = format!(
            "{MINIMAL}\n\
             enabled = false\n\
             \n\
             [[virtual_models]]\n\
             provider = \"router\"\n\
             id = \"backup\"\n\
             profile = \"default\"\n\
             candidate_pool = [\"qwen-local\"]\n"
        );
        let catalog = catalog_from(&toml).expect("catalog");
        assert!(!catalog.virtual_model_enabled("auto"));
        assert!(catalog.virtual_model_enabled("backup"));
        assert_eq!(catalog.enabled_virtual_models().count(), 1);

        // 默认入口不能是停用组。
        let default_auto = MINIMAL.replace(
            "schema_version = 1",
            "schema_version = 1\ndefault_virtual_model = \"auto\"",
        ) + "\nenabled = false\n";
        let error = catalog_from(&default_auto).expect_err("disabled default must reject");
        assert!(error.to_string().contains("已停用"), "{error}");
    }

    #[test]
    fn group_judge_off_without_rules_is_valid() {
        let catalog = catalog_with("[virtual_models.judge]\nmode = \"off\"\n").expect("catalog");
        let group = catalog.group_config("auto").expect("group config");
        assert_eq!(group.judge.mode, JudgeMode::Off);
        assert!(group.rules.is_empty());
    }

    #[test]
    fn group_judge_active_without_rules_is_valid() {
        let catalog = catalog_from(&active_group("", "")).expect("catalog");
        let group = catalog.group_config("auto").expect("group config");
        assert_eq!(group.judge.mode, JudgeMode::Active);
        assert!(group.rules.is_empty());
    }

    #[test]
    fn load_resolves_relative_file_credentials_against_config_dir() {
        let dir = std::env::temp_dir().join(format!("tr-agent-cred-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("credentials")).expect("create credentials dir");
        std::fs::write(dir.join("credentials/judge.key"), "sk-relative").expect("write key");

        let toml = format!(
            "{MINIMAL}\n\
             [virtual_models.judge]\n\
             mode = \"active\"\n\
             \n\
             [virtual_models.judge.service]\n\
             kind = \"llm\"\n\
             endpoint = \"http://127.0.0.1:11435/v1\"\n\
             model = \"judge\"\n\
             credential = \"file:credentials/judge.key\"\n"
        );
        let path = dir.join("config.toml");
        std::fs::write(&path, toml).expect("write config");

        // 相对 file: 引用按配置文件所在目录解析，不依赖进程工作目录。
        let catalog = load(&path).expect("relative file credential resolves from config dir");
        let group = catalog.group_config("auto").expect("group config");
        assert_eq!(group.judge.mode, JudgeMode::Active);

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn group_rules_require_active_judge() {
        // judge 缺省 off，但有规则 → 必须启用 judge。
        let error = catalog_with(RULE_CODING).expect_err("must reject");
        assert!(
            error
                .to_string()
                .contains("配置了 1 条规则，必须启用 judge"),
            "{error}"
        );
    }

    #[test]
    fn group_rules_accept_jev_service() {
        let toml = format!(
            "{MINIMAL}\n\
             [virtual_models.judge]\n\
             mode = \"active\"\n\
             \n\
             [virtual_models.judge.service]\n\
             kind = \"jev\"\n\
             endpoint = \"https://api.typesafe.ai/v1/systemone\"\n\
             model = \"systemone\"\n\
             {RULE_CODING}"
        );
        let catalog = catalog_from(&toml).expect("jev catalog");
        let group = catalog.group_config("auto").expect("group config");
        assert!(matches!(group.judge.backend, Backend::Jev { .. }));
        assert_eq!(group.rules.len(), 1);

        // query string 不影响 SystemOne 路径校验。
        let with_query = toml.replace(
            "https://api.typesafe.ai/v1/systemone",
            "https://api.typesafe.ai/v1/systemone?trace=1",
        );
        assert!(catalog_from(&with_query).is_ok());
    }

    #[test]
    fn jev_service_requires_endpoint_and_model() {
        let toml = |service_fields: &str| {
            format!(
                "{MINIMAL}\n\
                 [virtual_models.judge]\n\
                 mode = \"active\"\n\
                 \n\
                 [virtual_models.judge.service]\n\
                 kind = \"jev\"\n\
                 {service_fields}\n\
                 {RULE_CODING}"
            )
        };

        let error = catalog_from(&toml("model = \"systemone\"")).expect_err("must reject");
        assert!(error.to_string().contains("endpoint/model 必填"), "{error}");

        let error = catalog_from(&toml("endpoint = \"https://api.typesafe.ai/v1/systemone\""))
            .expect_err("must reject");
        assert!(error.to_string().contains("endpoint/model 必填"), "{error}");
    }

    #[test]
    fn jev_service_requires_systemone_path() {
        let toml = format!(
            "{MINIMAL}\n\
             [virtual_models.judge]\n\
             mode = \"active\"\n\
             \n\
             [virtual_models.judge.service]\n\
             kind = \"jev\"\n\
             endpoint = \"https://api.typesafe.ai/v1/decisions\"\n\
             model = \"systemone\"\n\
             {RULE_CODING}"
        );
        let error = catalog_from(&toml).expect_err("must reject");
        assert!(error.to_string().contains("/v1/systemone"), "{error}");
    }

    #[test]
    fn group_rules_reject_unknown_service_kind() {
        let toml = format!(
            "{MINIMAL}\n\
             [virtual_models.judge]\n\
             mode = \"active\"\n\
             \n\
             [virtual_models.judge.service]\n\
             kind = \"bogus\"\n\
             endpoint = \"http://127.0.0.1:11435/v1\"\n\
             model = \"gemma-2b\"\n\
             {RULE_CODING}"
        );
        let error = catalog_from(&toml).expect_err("must reject");
        assert!(error.to_string().contains("只支持 llm/jev"), "{error}");
    }

    #[test]
    fn systemone_endpoint_check_ignores_query_and_fragment() {
        assert!(is_systemone_endpoint("https://host/v1/systemone"));
        assert!(is_systemone_endpoint("https://host/v1/systemone?trace=1"));
        assert!(is_systemone_endpoint("https://host/v1/systemone#fragment"));
        assert!(!is_systemone_endpoint("https://host/v1"));
        assert!(!is_systemone_endpoint("https://host/v1/systemone/extra"));
    }

    #[test]
    fn group_rule_prefer_must_be_in_pool() {
        let rules = r#"
[[virtual_models.rules]]
id = "coding"
intent = "代码实现与调试"
prefer = "nope"
"#;
        let error = catalog_from(&active_group("", rules)).expect_err("must reject");
        assert!(error.to_string().contains("prefer `nope`"), "{error}");
        assert!(error.to_string().contains("candidate_pool"), "{error}");
    }

    #[test]
    fn group_rule_ids_must_be_unique() {
        let rules = r#"
[[virtual_models.rules]]
id = "coding"
intent = "第一条"
prefer = "qwen-local"

[[virtual_models.rules]]
id = "coding"
intent = "第二条"
prefer = "qwen-local"
"#;
        let error = catalog_from(&active_group("", rules)).expect_err("must reject");
        assert!(
            error.to_string().contains("规则 id `coding` 重复"),
            "{error}"
        );
    }

    #[test]
    fn group_judge_mode_rejects_shadow() {
        let error =
            catalog_with("[virtual_models.judge]\nmode = \"shadow\"\n").expect_err("must reject");
        assert!(error.to_string().contains("off/active"), "{error}");
    }

    #[test]
    fn group_digest_changes_with_rules_and_budget() {
        let base = catalog_from(&active_group("", "")).expect("base");
        let with_rules = catalog_from(&active_group("", RULE_CODING)).expect("with rules");
        let with_budget =
            catalog_from(&active_group("max_calls_per_minute = 60", "")).expect("with budget");

        let base_digest = &base.group_config("auto").expect("base group").digest;
        let rules_digest = &with_rules.group_config("auto").expect("rules group").digest;
        assert_ne!(base_digest, rules_digest, "规则变化必须改变 digest");

        let budget_group = with_budget.group_config("auto").expect("budget group");
        assert_ne!(base_digest, &budget_group.digest, "预算变化必须改变 digest");
        assert_eq!(budget_group.judge.budget.max_calls_per_minute, 60);
        assert_eq!(
            with_rules
                .group_config("auto")
                .expect("rules group")
                .rules
                .len(),
            1
        );
    }

    #[test]
    fn window_hint_mode_and_member_listing_project() {
        let doc = MINIMAL
            .replace(
                "candidate_pool = [\"qwen-local\"]",
                "candidate_pool = [\"qwen-local\", \"big\"]\ncontext_window_mode = \"max\"\nlist_members = true",
            )
            .replace(
                "[[virtual_models]]",
                "[[candidates]]\nid = \"big\"\ntier = \"cloud\"\nstrength = 20\nprovider = \"openai\"\nbase_url = \"http://127.0.0.1:9/v1\"\nmodel = \"big-model\"\nwindow = 200000\ninput = [\"text\"]\n\n[[virtual_models]]",
            );
        let catalog = catalog_from(&doc).expect("catalog");
        let auto = catalog.virtual_models.get("auto").expect("auto");
        assert_eq!(auto.context_window, Some(200_000), "max 取池内最大窗口");
        assert!(catalog.list_members.contains("auto"));
        let (vm, candidate) = catalog
            .resolve_member_selection("auto/big-model")
            .expect("member resolves");
        assert_eq!(vm, "auto");
        assert_eq!(candidate, "big");
    }

    #[test]
    fn window_hint_mode_custom_requires_value() {
        let doc = MINIMAL.replace(
            "candidate_pool = [\"qwen-local\"]",
            "candidate_pool = [\"qwen-local\"]\ncontext_window_mode = \"custom\"",
        );
        assert!(catalog_from(&doc).is_err());
    }
}
