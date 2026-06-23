use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::paths;

/// On-disk `~/.token-router/config.toml`
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConfigFile {
    #[serde(default)]
    pub gateway: GatewaySection,
    #[serde(default)]
    pub upstream: UpstreamSection,
    #[serde(default)]
    pub agent: HashMap<String, AgentConfig>,
    #[serde(default)]
    pub cli: CliSection,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GatewaySection {
    #[serde(default = "default_listen")]
    pub listen: String,
    /// Routing: `auto` (difficulty-based) | `edge` | `cloud` | `cascade`
    #[serde(default = "default_route")]
    pub route: String,
    /// When `route = auto`: `single` | `cascade` | `split`
    #[serde(default = "default_routing_mode")]
    pub routing_mode: String,
    #[serde(default = "default_profile")]
    pub default_profile: String,
    #[serde(default = "default_ctx_edge_max")]
    pub ctx_edge_max_tokens: u32,
    /// Inbound API key for `/v1/chat/completions` (Bearer or `x-api-key`). Omit to disable auth.
    #[serde(default)]
    pub api_key: Option<String>,
    #[serde(default)]
    pub admin_token: Option<String>,
    #[serde(default = "default_experience_enabled")]
    pub experience_enabled: bool,
    #[serde(default = "default_experience_learning_rate")]
    pub experience_learning_rate: f32,
    #[serde(default = "default_experience_max_bias")]
    pub experience_max_bias: f32,
    #[serde(default = "default_experience_target_fallback")]
    pub experience_target_fallback: f32,
    #[serde(default = "default_cloud_sticky_ttl_secs")]
    pub cloud_sticky_ttl_secs: u64,
    #[serde(default = "default_session_persist_enabled")]
    pub session_persist_enabled: bool,
    /// Fraction of work-step requests that run edge + cloud verification (0.0–1.0).
    #[serde(default = "default_work_verify_sample_rate")]
    pub work_verify_sample_rate: f32,
    /// Runtime auto-tuning from experience.json + stats (see adaptive_*).
    #[serde(default = "default_adaptive_routing_enabled")]
    pub adaptive_routing_enabled: bool,
    #[serde(default = "default_adaptive_min_verified_samples")]
    pub adaptive_min_verified_samples: u64,
    #[serde(default = "default_adaptive_verify_rate_floor")]
    pub adaptive_verify_rate_floor: f32,
    #[serde(default = "default_adaptive_verify_rate_ceiling")]
    pub adaptive_verify_rate_ceiling: f32,
    #[serde(default = "default_adaptive_max_theta_shift")]
    pub adaptive_max_theta_shift: f32,
    #[serde(default = "default_classifier_enabled")]
    pub classifier_enabled: bool,
    #[serde(default = "default_classifier_min_samples")]
    pub classifier_min_samples: u64,
    #[serde(default = "default_classifier_prior_alpha")]
    pub classifier_prior_alpha: f32,
    #[serde(default = "default_classifier_decay_half_life_hours")]
    pub classifier_decay_half_life_hours: f64,
    #[serde(default = "default_classifier_prior_from_heuristic")]
    pub classifier_prior_from_heuristic: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct UpstreamSection {
    #[serde(default)]
    pub edge: Option<UpstreamEndpoint>,
    #[serde(default)]
    pub cloud: Option<UpstreamEndpoint>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpstreamEndpoint {
    #[serde(default)]
    pub base_url: String,
    #[serde(default)]
    pub api_key: Option<String>,
    /// Upstream model id; `auto` keeps the client request model for Flowy routing.
    #[serde(default)]
    pub model: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AgentConfig {
    #[serde(default)]
    pub cloud_token_budget: Option<u64>,
    #[serde(default)]
    pub upstream: AgentUpstreamSection,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AgentUpstreamSection {
    #[serde(default)]
    pub edge: Option<UpstreamEndpoint>,
    #[serde(default)]
    pub cloud: Option<UpstreamEndpoint>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CliSection {
    /// HTTP base URL for talking to the gateway (default derived from `gateway.listen`).
    #[serde(default)]
    pub gateway_url: Option<String>,
}

fn default_listen() -> String {
    "127.0.0.1:11080".to_string()
}

fn default_route() -> String {
    "auto".to_string()
}

fn default_routing_mode() -> String {
    "cascade".to_string()
}

fn default_profile() -> String {
    "balanced".to_string()
}

fn default_ctx_edge_max() -> u32 {
    65_536
}

fn default_experience_enabled() -> bool {
    true
}

fn default_experience_learning_rate() -> f32 {
    0.08
}

fn default_experience_max_bias() -> f32 {
    0.12
}

fn default_experience_target_fallback() -> f32 {
    0.15
}

fn default_cloud_sticky_ttl_secs() -> u64 {
    600
}

fn default_session_persist_enabled() -> bool {
    true
}

fn default_work_verify_sample_rate() -> f32 {
    0.1
}

fn default_adaptive_routing_enabled() -> bool {
    true
}

fn default_adaptive_min_verified_samples() -> u64 {
    20
}

fn default_adaptive_verify_rate_floor() -> f32 {
    0.05
}

fn default_adaptive_verify_rate_ceiling() -> f32 {
    0.45
}

fn default_adaptive_max_theta_shift() -> f32 {
    0.05
}

fn default_classifier_enabled() -> bool {
    true
}

fn default_classifier_min_samples() -> u64 {
    100
}

fn default_classifier_prior_alpha() -> f32 {
    1.0
}

fn default_classifier_decay_half_life_hours() -> f64 {
    168.0
}

fn default_classifier_prior_from_heuristic() -> bool {
    true
}

impl Default for GatewaySection {
    fn default() -> Self {
        Self {
            listen: default_listen(),
            route: default_route(),
            routing_mode: default_routing_mode(),
            default_profile: default_profile(),
            ctx_edge_max_tokens: default_ctx_edge_max(),
            api_key: None,
            admin_token: None,
            experience_enabled: default_experience_enabled(),
            experience_learning_rate: default_experience_learning_rate(),
            experience_max_bias: default_experience_max_bias(),
            experience_target_fallback: default_experience_target_fallback(),
            cloud_sticky_ttl_secs: default_cloud_sticky_ttl_secs(),
            session_persist_enabled: default_session_persist_enabled(),
            work_verify_sample_rate: default_work_verify_sample_rate(),
            adaptive_routing_enabled: default_adaptive_routing_enabled(),
            adaptive_min_verified_samples: default_adaptive_min_verified_samples(),
            adaptive_verify_rate_floor: default_adaptive_verify_rate_floor(),
            adaptive_verify_rate_ceiling: default_adaptive_verify_rate_ceiling(),
            adaptive_max_theta_shift: default_adaptive_max_theta_shift(),
            classifier_enabled: default_classifier_enabled(),
            classifier_min_samples: default_classifier_min_samples(),
            classifier_prior_alpha: default_classifier_prior_alpha(),
            classifier_decay_half_life_hours: default_classifier_decay_half_life_hours(),
            classifier_prior_from_heuristic: default_classifier_prior_from_heuristic(),
        }
    }
}

impl Default for ConfigFile {
    fn default() -> Self {
        Self {
            gateway: GatewaySection::default(),
            upstream: UpstreamSection::default(),
            agent: HashMap::new(),
            cli: CliSection::default(),
        }
    }
}

impl ConfigFile {
    pub fn gateway_http_url(&self) -> String {
        self.cli
            .gateway_url
            .clone()
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| format!("http://{}", self.gateway.listen))
    }

    pub fn pid_file_path(&self) -> anyhow::Result<PathBuf> {
        paths::pid_file()
    }

    pub fn data_dir(&self) -> anyhow::Result<PathBuf> {
        paths::app_dir()
    }
}

pub fn default_config_template() -> String {
    r#"# Flowy Router configuration
# Path: ~/.token-router/config.toml (Linux/macOS) or %USERPROFILE%\.token-router\config.toml (Windows)

[gateway]
listen = "127.0.0.1:11080"
route = "auto"                 # auto | edge | cloud | cascade
routing_mode = "cascade"       # single | cascade | split (when route = auto)
default_profile = "balanced"   # economy | balanced | premium | privacy
ctx_edge_max_tokens = 65536
# api_key = "flowy-local"        # optional: inbound auth when set
# admin_token = "change-me"      # optional: protects POST /v1/admin/shutdown|restart
# experience_enabled = true
# experience_learning_rate = 0.08
# experience_max_bias = 0.12
# session_persist_enabled = true
# work_verify_sample_rate = 0.1   # work 步态云端校验抽样比例 (0.0–1.0)
# adaptive_routing_enabled = true # 根据 experience/stats 运行时微调抽样率与难度阈值
# adaptive_min_verified_samples = 20
# adaptive_verify_rate_floor = 0.05
# adaptive_verify_rate_ceiling = 0.45
# adaptive_max_theta_shift = 0.05
# classifier_enabled = true
# classifier_min_samples = 100
# classifier_prior_alpha = 1.0
# classifier_decay_half_life_hours = 168
# classifier_prior_from_heuristic = true

# [upstream.cloud]
# base_url = "https://api.deepseek.com/v1"
# model = "auto"                 # auto = keep client model; or set e.g. deepseek-chat
# api_key = "sk-..."             # optional: Bearer to cloud upstream when set

# [upstream.edge]
# base_url = "http://127.0.0.1:11434/v1"
# model = "qwen3:8b"             # optional; omit or auto = keep client model

[cli]
# gateway_url = "http://127.0.0.1:11080"
"#
    .to_string()
}

/// Load config from `~/.token-router/config.toml` (file must already exist).
pub fn load() -> anyhow::Result<(ConfigFile, PathBuf)> {
    let path = paths::config_file()?;
    load_from_path(&path)
}

/// Create `~/.token-router/` (and `sessions/`) plus `config.toml` when missing.
///
/// Returns the config path and whether a new template file was written.
pub fn ensure_initialized(path: Option<&Path>) -> anyhow::Result<(PathBuf, bool)> {
    paths::ensure_app_dirs()?;
    let path = match path {
        Some(p) => p.to_path_buf(),
        None => paths::config_file()?,
    };
    let created = !path.exists();
    if created {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(&path, default_config_template())?;
    }
    Ok((path, created))
}

pub fn load_from_path(path: &Path) -> anyhow::Result<(ConfigFile, PathBuf)> {
    if !path.exists() {
        anyhow::bail!(
            "config not found: {}. Run `token-router gateway start` to create it.",
            path.display()
        );
    }

    let raw = fs::read_to_string(path)?;
    let cfg: ConfigFile = toml::from_str(&raw).map_err(|e| {
        anyhow::anyhow!("invalid TOML in {}: {e}", path.display())
    })?;
    Ok((cfg, path.to_path_buf()))
}

pub fn save(path: &Path, cfg: &ConfigFile) -> anyhow::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let raw = toml::to_string_pretty(cfg)?;
    fs::write(path, raw)?;
    Ok(())
}
