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
    /// 判官配置（`off` 时 backend 不构造）。
    pub judge: JudgeSettings,
    /// 成本闸模式（P2 默认 shadow）。
    pub cost_gate: crate::gate::CostGateMode,
    /// 信任档位别名 → profile（P3-14）。只换别名，不改虚拟模型与内核。
    pub trust_tiers: BTreeMap<String, ProfileId>,
    /// 规范化配置的 SHA-256，写入账本与快照。
    pub config_digest: String,
}

impl Catalog {
    pub fn virtual_model(&self, id: &str) -> Option<&VirtualModel> {
        self.virtual_models.get(id)
    }

    pub fn candidate(&self, id: &str) -> Option<&Candidate> {
        self.candidates.get(id)
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
#[derive(Debug, Clone, Serialize, Deserialize)]
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
    #[serde(default)]
    pub window: u32,
    #[serde(default)]
    pub input: Vec<InputModality>,
    #[serde(default)]
    pub thinking_levels: Vec<ThinkingLevel>,
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
    #[serde(default)]
    pub context_window: Option<u32>,
    #[serde(default)]
    pub max_tokens: Option<u32>,
    #[serde(default)]
    pub input: Vec<InputModality>,
    pub profile: ProfileId,
    #[serde(default)]
    pub strategies: Vec<String>,
    #[serde(default)]
    pub candidate_pool: Vec<CandidateId>,
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
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct JudgeServiceFile {
    /// `local` | `http` | `llm`
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

impl ConfigFile {
    pub fn parse(text: &str) -> Result<Self, ConfigError> {
        Ok(toml::from_str(text)?)
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
                    context_window: file.context_window,
                    max_tokens: file.max_tokens,
                    input: file.input.clone(),
                    profile: file.profile.clone(),
                    strategies: file.strategies.clone(),
                    candidate_pool: file.candidate_pool.clone(),
                },
            );
        }

        let default_virtual_model = match self.default_virtual_model {
            Some(id) if virtual_models.contains_key(&id) => id,
            Some(id) => {
                return Err(ConfigError::Invalid(format!(
                    "default_virtual_model `{id}` 不存在"
                )));
            }
            None => {
                if virtual_models.contains_key("auto") {
                    "auto".to_string()
                } else {
                    virtual_models.keys().next().cloned().expect("已检查非空")
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
            judge,
            cost_gate,
            trust_tiers: self.trust_tiers,
            config_digest,
        })
    }
}

fn build_judge_settings(file: &JudgeFile) -> Result<JudgeSettings, ConfigError> {
    let mode = JudgeMode::parse(&file.mode).ok_or_else(|| {
        ConfigError::Invalid(format!(
            "judge.mode 只能是 off/shadow/active，得到 `{}`",
            file.mode
        ))
    })?;

    let service = &file.service;
    let credential = match &service.credential {
        Some(reference) => resolve_credential(reference)?,
        None => None,
    };

    let backend = match service.kind.as_str() {
        "local" => Backend::Local,
        "http" => {
            if service.endpoint.is_empty() {
                return Err(ConfigError::Invalid(
                    "judge.service.kind = http 时 endpoint 必填".to_string(),
                ));
            }
            Backend::Http {
                endpoint: service.endpoint.clone(),
                service_id: service.service_id.clone(),
                revision: service.revision,
                credential,
                timeout_ms: service.timeout_ms,
            }
        }
        "llm" => {
            if service.endpoint.is_empty() || service.model.is_empty() {
                return Err(ConfigError::Invalid(
                    "judge.service.kind = llm 时 endpoint/model 必填".to_string(),
                ));
            }
            Backend::Llm {
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
            }
        }
        other => {
            return Err(ConfigError::Invalid(format!(
                "judge.service.kind 不支持 `{other}`"
            )));
        }
    };

    Ok(JudgeSettings {
        mode,
        budget: Budget {
            max_calls_per_turn: file.max_calls_per_turn,
            max_calls_per_minute: file.max_calls_per_minute,
            max_calls_per_session: file.max_calls_per_session,
            deadline_ms: file.deadline_ms,
        },
        backend,
        price: JudgePrice {
            input_micros_per_mtok: service.input_micros_per_mtok,
            output_micros_per_mtok: service.output_micros_per_mtok,
            request_micros: service.request_micros,
        },
        currency: service.currency.clone(),
    })
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
pub fn load(path: impl AsRef<Path>) -> Result<Catalog, ConfigError> {
    let text = std::fs::read_to_string(path)?;
    ConfigFile::parse(&text)?.into_catalog()
}

/// proxy/header 进 capability.tags：内核只透传，upstream 消费。
fn candidate_tags(file: &CandidateFile) -> BTreeMap<String, String> {
    let mut tags = BTreeMap::new();
    if let Some(proxy) = &file.proxy {
        tags.insert("proxy_url".to_string(), proxy.clone());
    }
    for (name, value) in &file.extra_headers {
        tags.insert(format!("header:{name}"), value.clone());
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
}
