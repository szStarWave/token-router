use std::collections::HashMap;
use std::path::PathBuf;

use crate::config::ConfigFile;
use crate::config::auth_keys::ResolvedAuthKey;
use crate::config::{ensure_initialized, load_from_path, setup::endpoint_configured};

use crate::gateway::classifier::ClassifierSettings;
use crate::gateway::experience::ExperienceSettings;
use crate::gateway::routing::{Profile, RouteTier, RoutingMode};

#[derive(Debug, Clone)]
pub struct AdaptiveRoutingSettings {
    pub enabled: bool,
    pub min_verified_samples: u64,
    pub verify_rate_floor: f32,
    pub verify_rate_ceiling: f32,
    pub max_theta_shift: f32,
}

#[derive(Debug, Clone, Default)]
pub struct AgentUpstreamConfig {
    pub edge_base_url: Option<String>,
    pub edge_api_key: Option<String>,
    pub edge_model: Option<String>,
    pub cloud_base_url: Option<String>,
    pub cloud_api_key: Option<String>,
    pub cloud_model: Option<String>,
    pub cloud_token_budget: Option<u64>,
}

#[derive(Debug, Clone)]
pub struct ResolvedUpstream {
    pub base_url: Option<String>,
    pub api_key: Option<String>,
    pub model: Option<String>,
}

#[derive(Debug, Clone)]
pub struct AppConfig {
    pub listen_addr: String,
    /// `None` = `route=auto` (difficulty-based); `Some` = fixed tier from config.
    pub fixed_route: Option<RouteTier>,
    pub routing_mode: RoutingMode,
    pub edge_base_url: Option<String>,
    pub edge_api_key: Option<String>,
    pub edge_model: Option<String>,
    pub cloud_base_url: Option<String>,
    pub cloud_api_key: Option<String>,
    pub cloud_model: Option<String>,
    pub default_profile: Profile,
    pub ctx_edge_max_tokens: u32,
    pub data_dir: PathBuf,
    pub pid_file: PathBuf,
    /// Client → Gateway (`/v1/chat/completions`). None = no auth.
    pub inbound_api_keys: Vec<String>,
    /// key value → resolved auth key metadata (when auth enabled).
    pub auth_key_by_value: HashMap<String, ResolvedAuthKey>,
    /// First inbound key for legacy callers.
    pub api_key: Option<String>,
    pub admin_token: Option<String>,
    pub config_path: PathBuf,
    pub experience: ExperienceSettings,
    pub session_persist_enabled: bool,
    pub session_retention_days: u64,
    pub session_cleanup_interval_secs: u64,
    pub cloud_sticky_ttl_secs: u64,
    /// Work-step cloud verification sample rate in `[0.0, 1.0]` (config baseline).
    pub work_verify_sample_rate: f32,
    pub adaptive_routing: AdaptiveRoutingSettings,
    pub classifier: ClassifierSettings,
    pub agents: HashMap<String, AgentUpstreamConfig>,
    pub log_max_size_mb: u64,
    pub log_max_files: usize,
}

impl AppConfig {
    /// Load from `~/.token-router/config.toml` (initializes app dir + template if missing).
    pub fn load() -> anyhow::Result<Self> {
        let (path, _) = ensure_initialized(None)?;
        let (file, config_path) = load_from_path(&path)?;
        Self::from_file(file, config_path)
    }

    /// Load from a custom path (initializes parent dir + template if missing).
    pub fn load_from(path: Option<&std::path::Path>) -> anyhow::Result<Self> {
        let (path, _) = ensure_initialized(path)?;
        let (file, config_path) = load_from_path(&path)?;
        Self::from_file(file, config_path)
    }

    pub fn from_file(file: ConfigFile, config_path: PathBuf) -> anyhow::Result<Self> {
        let data_dir = file.data_dir()?;
        let pid_file = file.pid_file_path()?;
        let default_profile = file
            .gateway
            .default_profile
            .parse()
            .map_err(|()| anyhow::anyhow!("invalid gateway.default_profile"))?;
        let fixed_route = parse_config_route(&file.gateway.route)?;
        let routing_mode = file
            .gateway
            .routing_mode
            .parse()
            .map_err(|()| anyhow::anyhow!("invalid gateway.routing_mode"))?;
        let inbound_api_keys = if file.gateway.auth_enabled {
            crate::config::auth_keys::collect_inbound_api_keys(&file.gateway)
        } else {
            Vec::new()
        };
        let auth_key_by_value = if file.gateway.auth_enabled {
            crate::config::auth_keys::build_auth_key_by_value(&file.gateway)
        } else {
            HashMap::new()
        };
        let api_key = inbound_api_keys.first().cloned();

        Ok(Self {
            listen_addr: file.gateway.listen,
            fixed_route,
            routing_mode,
            edge_base_url: file
                .upstream
                .edge
                .as_ref()
                .filter(|e| endpoint_configured(e))
                .map(|e| e.base_url.clone()),
            edge_api_key: file
                .upstream
                .edge
                .as_ref()
                .and_then(|e| e.api_key.clone())
                .filter(|s| !s.is_empty()),
            edge_model: file.upstream.edge.as_ref().and_then(|e| e.model.clone()),
            cloud_base_url: file
                .upstream
                .cloud
                .as_ref()
                .filter(|e| endpoint_configured(e))
                .map(|e| e.base_url.clone()),
            cloud_api_key: file
                .upstream
                .cloud
                .as_ref()
                .and_then(|e| e.api_key.clone())
                .filter(|s| !s.is_empty()),
            cloud_model: file.upstream.cloud.as_ref().and_then(|e| e.model.clone()),
            default_profile,
            ctx_edge_max_tokens: file.gateway.ctx_edge_max_tokens,
            data_dir,
            pid_file,
            inbound_api_keys,
            auth_key_by_value,
            api_key,
            admin_token: file.gateway.admin_token.filter(|s| !s.is_empty()),
            config_path,
            experience: ExperienceSettings {
                enabled: file.gateway.experience_enabled,
                learning_rate: file.gateway.experience_learning_rate,
                max_bias: file.gateway.experience_max_bias,
                target_fallback: file.gateway.experience_target_fallback,
            },
            session_persist_enabled: file.gateway.session_persist_enabled,
            session_retention_days: file.gateway.session_retention_days,
            session_cleanup_interval_secs: file.gateway.session_cleanup_interval_secs.max(60),
            cloud_sticky_ttl_secs: file.gateway.cloud_sticky_ttl_secs,
            work_verify_sample_rate: file.gateway.work_verify_sample_rate.clamp(0.0, 1.0),
            adaptive_routing: AdaptiveRoutingSettings {
                enabled: file.gateway.adaptive_routing_enabled,
                min_verified_samples: file.gateway.adaptive_min_verified_samples,
                verify_rate_floor: file.gateway.adaptive_verify_rate_floor.clamp(0.0, 1.0),
                verify_rate_ceiling: file.gateway.adaptive_verify_rate_ceiling.clamp(0.0, 1.0),
                max_theta_shift: file.gateway.adaptive_max_theta_shift.max(0.0),
            },
            classifier: ClassifierSettings {
                enabled: file.gateway.classifier_enabled,
                min_samples: file.gateway.classifier_min_samples,
                prior_alpha: file.gateway.classifier_prior_alpha.max(0.0),
                decay_half_life_hours: file.gateway.classifier_decay_half_life_hours.max(0.0),
                prior_from_heuristic: file.gateway.classifier_prior_from_heuristic,
                min_feature_count: 0.5,
            },
            agents: file
                .agent
                .iter()
                .map(|(id, ac)| {
                    let edge = ac.upstream.edge.as_ref();
                    let cloud = ac.upstream.cloud.as_ref();
                    (
                        id.clone(),
                        AgentUpstreamConfig {
                            edge_base_url: edge
                                .filter(|e| endpoint_configured(e))
                                .map(|e| e.base_url.clone()),
                            edge_api_key: edge
                                .and_then(|e| e.api_key.clone())
                                .filter(|s| !s.is_empty()),
                            edge_model: edge.and_then(|e| e.model.clone()),
                            cloud_base_url: cloud
                                .filter(|e| endpoint_configured(e))
                                .map(|e| e.base_url.clone()),
                            cloud_api_key: cloud
                                .and_then(|e| e.api_key.clone())
                                .filter(|s| !s.is_empty()),
                            cloud_model: cloud.and_then(|e| e.model.clone()),
                            cloud_token_budget: ac.cloud_token_budget,
                        },
                    )
                })
                .collect(),
            log_max_size_mb: file.gateway.log_max_size_mb,
            log_max_files: file.gateway.log_max_files.max(1),
        })
    }

    pub fn gateway_base_url(&self) -> String {
        crate::config::setup::client_gateway_http_url(&self.listen_addr)
    }

    pub fn resolve_upstream(&self, agent_id: Option<&str>, tier: &str) -> ResolvedUpstream {
        let agent_cfg = agent_id.and_then(|id| self.agents.get(id));
        match tier {
            "edge" => ResolvedUpstream {
                base_url: agent_cfg
                    .and_then(|a| a.edge_base_url.as_deref())
                    .or(self.edge_base_url.as_deref())
                    .map(String::from),
                api_key: agent_cfg
                    .and_then(|a| a.edge_api_key.as_deref())
                    .or(self.edge_api_key.as_deref())
                    .map(String::from),
                model: agent_cfg
                    .and_then(|a| a.edge_model.as_deref())
                    .or(self.edge_model.as_deref())
                    .map(String::from),
            },
            "cloud" => ResolvedUpstream {
                base_url: agent_cfg
                    .and_then(|a| a.cloud_base_url.as_deref())
                    .or(self.cloud_base_url.as_deref())
                    .map(String::from),
                api_key: agent_cfg
                    .and_then(|a| a.cloud_api_key.as_deref())
                    .or(self.cloud_api_key.as_deref())
                    .map(String::from),
                model: agent_cfg
                    .and_then(|a| a.cloud_model.as_deref())
                    .or(self.cloud_model.as_deref())
                    .map(String::from),
            },
            _ => ResolvedUpstream {
                base_url: None,
                api_key: None,
                model: None,
            },
        }
    }
}

fn parse_config_route(s: &str) -> anyhow::Result<Option<RouteTier>> {
    match s.trim().to_ascii_lowercase().as_str() {
        "auto" => Ok(None),
        "edge" => Ok(Some(RouteTier::Edge)),
        "cloud" => Ok(Some(RouteTier::Cloud)),
        "cascade" => Ok(Some(RouteTier::Cascade)),
        other => anyhow::bail!("invalid gateway.route `{other}` (use auto|edge|cloud|cascade)"),
    }
}
