use std::path::PathBuf;

use crate::config::ConfigFile;
use crate::config::{ensure_initialized, load_from_path, setup::endpoint_configured};

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
    pub api_key: Option<String>,
    pub admin_token: Option<String>,
    pub config_path: PathBuf,
    pub experience: ExperienceSettings,
    pub session_persist_enabled: bool,
    pub cloud_sticky_ttl_secs: u64,
    /// Work-step cloud verification sample rate in `[0.0, 1.0]` (config baseline).
    pub work_verify_sample_rate: f32,
    pub adaptive_routing: AdaptiveRoutingSettings,
}

impl AppConfig {
    /// Load from `~/.flowy-router/config.toml` (initializes app dir + template if missing).
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
            api_key: file.gateway.api_key.filter(|s| !s.is_empty()),
            admin_token: file.gateway.admin_token.filter(|s| !s.is_empty()),
            config_path,
            experience: ExperienceSettings {
                enabled: file.gateway.experience_enabled,
                learning_rate: file.gateway.experience_learning_rate,
                max_bias: file.gateway.experience_max_bias,
                target_fallback: file.gateway.experience_target_fallback,
            },
            session_persist_enabled: file.gateway.session_persist_enabled,
            cloud_sticky_ttl_secs: file.gateway.cloud_sticky_ttl_secs,
            work_verify_sample_rate: file.gateway.work_verify_sample_rate.clamp(0.0, 1.0),
            adaptive_routing: AdaptiveRoutingSettings {
                enabled: file.gateway.adaptive_routing_enabled,
                min_verified_samples: file.gateway.adaptive_min_verified_samples,
                verify_rate_floor: file.gateway.adaptive_verify_rate_floor.clamp(0.0, 1.0),
                verify_rate_ceiling: file.gateway.adaptive_verify_rate_ceiling.clamp(0.0, 1.0),
                max_theta_shift: file.gateway.adaptive_max_theta_shift.max(0.0),
            },
        })
    }

    pub fn gateway_base_url(&self) -> String {
        format!("http://{}", self.listen_addr)
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
