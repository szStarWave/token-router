use super::file::{ConfigFile, GatewaySection, UpstreamEndpoint, UpstreamSection};
use serde::{Deserialize, Serialize};

pub const CLOUD_MODEL_AUTO: &str = "auto";
/// Agent id used for global (non-agent) cloud token budget in config `[agent]`.
pub const DEFAULT_CLOUD_BUDGET_AGENT_ID: &str = "__default__";

/// Gateway routing / experience / adaptive settings (hot-updatable via `/v1/admin/setup`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GatewayConfigView {
    pub route: String,
    pub routing_mode: String,
    pub default_profile: String,
    pub ctx_edge_max_tokens: u32,
    pub experience_enabled: bool,
    pub experience_learning_rate: f32,
    pub experience_max_bias: f32,
    pub experience_target_fallback: f32,
    pub cloud_cache_decay_half_life_secs: u64,
    pub cloud_cache_boost_max: f32,
    pub request_route_cache_enabled: bool,
    pub request_route_cache_retention_days: u64,
    pub request_route_cache_cleanup_interval_secs: u64,
    pub session_persist_enabled: bool,
    pub work_verify_sample_rate: f32,
    pub adaptive_routing_enabled: bool,
    pub adaptive_min_verified_samples: u64,
    pub adaptive_verify_rate_floor: f32,
    pub adaptive_verify_rate_ceiling: f32,
    pub adaptive_max_theta_shift: f32,
    pub classifier_enabled: bool,
    pub classifier_min_samples: u64,
    pub classifier_prior_alpha: f32,
    pub classifier_decay_half_life_hours: f64,
    pub classifier_prior_from_heuristic: bool,
    pub listen_port: u16,
    pub listen_lan: bool,
    pub auth_enabled: bool,
    pub api_key_set: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub api_key_preview: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct GatewayConfigPatch {
    pub route: Option<String>,
    pub routing_mode: Option<String>,
    pub default_profile: Option<String>,
    pub ctx_edge_max_tokens: Option<u32>,
    pub experience_enabled: Option<bool>,
    pub experience_learning_rate: Option<f32>,
    pub experience_max_bias: Option<f32>,
    pub experience_target_fallback: Option<f32>,
    #[serde(alias = "cloud_sticky_ttl_secs")]
    pub cloud_cache_decay_half_life_secs: Option<u64>,
    pub cloud_cache_boost_max: Option<f32>,
    pub request_route_cache_enabled: Option<bool>,
    pub request_route_cache_retention_days: Option<u64>,
    pub request_route_cache_cleanup_interval_secs: Option<u64>,
    pub session_persist_enabled: Option<bool>,
    pub work_verify_sample_rate: Option<f32>,
    pub adaptive_routing_enabled: Option<bool>,
    pub adaptive_min_verified_samples: Option<u64>,
    pub adaptive_verify_rate_floor: Option<f32>,
    pub adaptive_verify_rate_ceiling: Option<f32>,
    pub adaptive_max_theta_shift: Option<f32>,
    pub classifier_enabled: Option<bool>,
    pub classifier_min_samples: Option<u64>,
    pub classifier_prior_alpha: Option<f32>,
    pub classifier_decay_half_life_hours: Option<f64>,
    pub classifier_prior_from_heuristic: Option<bool>,
    pub listen_port: Option<u16>,
    pub listen_lan: Option<bool>,
    pub auth_enabled: Option<bool>,
    pub api_key: Option<String>,
}

impl GatewayConfigPatch {
    pub fn is_empty(&self) -> bool {
        self.route.is_none()
            && self.routing_mode.is_none()
            && self.default_profile.is_none()
            && self.ctx_edge_max_tokens.is_none()
            && self.experience_enabled.is_none()
            && self.experience_learning_rate.is_none()
            && self.experience_max_bias.is_none()
            && self.experience_target_fallback.is_none()
            && self.cloud_cache_decay_half_life_secs.is_none()
            && self.cloud_cache_boost_max.is_none()
            && self.request_route_cache_enabled.is_none()
            && self.request_route_cache_retention_days.is_none()
            && self.request_route_cache_cleanup_interval_secs.is_none()
            && self.session_persist_enabled.is_none()
            && self.work_verify_sample_rate.is_none()
            && self.adaptive_routing_enabled.is_none()
            && self.adaptive_min_verified_samples.is_none()
            && self.adaptive_verify_rate_floor.is_none()
            && self.adaptive_verify_rate_ceiling.is_none()
            && self.adaptive_max_theta_shift.is_none()
            && self.classifier_enabled.is_none()
            && self.classifier_min_samples.is_none()
            && self.classifier_prior_alpha.is_none()
            && self.classifier_decay_half_life_hours.is_none()
            && self.classifier_prior_from_heuristic.is_none()
            && self.listen_port.is_none()
            && self.listen_lan.is_none()
            && self.auth_enabled.is_none()
            && self.api_key.is_none()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpstreamSetupView {
    pub gateway: GatewayConfigView,
    pub edge: Option<UpstreamEndpointView>,
    pub cloud: Option<UpstreamEndpointView>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct UpstreamSetupUpdate {
    #[serde(default)]
    pub agent_id: Option<String>,
    /// Partial gateway settings; omit or `null` to leave unchanged.
    #[serde(default)]
    pub gateway: Option<GatewayConfigPatch>,
    #[serde(default)]
    pub edge: Option<UpstreamEndpointPatch>,
    #[serde(default)]
    pub cloud: Option<UpstreamEndpointPatch>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpstreamEndpointView {
    pub configured: bool,
    pub base_url: String,
    pub model: Option<String>,
    pub upstream_model: Option<String>,
    pub api_key_set: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub token_budget: Option<u64>,
    /// Whether the configured `token_budget` limit is actively enforced.
    #[serde(default)]
    pub token_quota_enabled: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct UpstreamEndpointPatch {
    /// Set to empty string to clear `base_url`.
    pub base_url: Option<String>,
    /// Omit to keep existing; empty string clears the key.
    pub api_key: Option<String>,
    pub model: Option<String>,
    pub upstream_model: Option<String>,
    /// When true, remove this tier entirely (edge only).
    #[serde(default)]
    pub clear: bool,
    /// Cloud token budget (5 h window); `None`=unchanged, `Some(None)`=clear, `Some(Some(n))`=set.
    #[serde(default)]
    pub token_budget: Option<Option<u64>>,
}

pub fn endpoint_configured(ep: &UpstreamEndpoint) -> bool {
    !ep.base_url.trim().is_empty()
}

pub const LISTEN_PORT_MIN: u16 = 1024;
pub const LISTEN_PORT_MAX: u16 = 65535;
#[cfg(feature = "desktop")]
pub const DEFAULT_LISTEN_PORT: u16 = 11088;
#[cfg(not(feature = "desktop"))]
pub const DEFAULT_LISTEN_PORT: u16 = 16621;

pub fn listen_port_from_addr(listen: &str) -> u16 {
    listen
        .rsplit_once(':')
        .and_then(|(_, port)| port.parse().ok())
        .unwrap_or(DEFAULT_LISTEN_PORT)
}

pub fn listen_lan_from_addr(listen: &str) -> bool {
    let host = listen
        .rsplit_once(':')
        .map(|(host, _)| host.trim())
        .unwrap_or(listen.trim());
    matches!(host, "0.0.0.0" | "::" | "[::]")
}

pub fn build_listen_addr(port: u16, lan: bool) -> String {
    let host = if lan { "0.0.0.0" } else { "127.0.0.1" };
    format!("{host}:{port}")
}

/// Best-effort primary LAN IPv4 for display when gateway binds on `0.0.0.0`.
pub fn primary_lan_ipv4() -> Option<String> {
    use std::net::IpAddr;
    match local_ip_address::local_ip().ok()? {
        IpAddr::V4(ip) if !ip.is_loopback() => Some(ip.to_string()),
        _ => None,
    }
}

/// HTTP base URL reachable from other devices on the LAN (`None` when LAN bind is off).
pub fn lan_client_http_url(listen: &str) -> Option<String> {
    if !listen_lan_from_addr(listen) {
        return None;
    }
    let ip = primary_lan_ipv4()?;
    Some(format!("http://{}:{}", ip, listen_port_from_addr(listen)))
}

/// HTTP base URL for local clients (desktop UI, CLI) to reach `gateway.listen`.
/// Wildcard bind addresses like `0.0.0.0` are mapped to loopback.
pub fn client_gateway_http_url(listen: &str) -> String {
    let port = listen_port_from_addr(listen);
    let host = listen
        .rsplit_once(':')
        .map(|(host, _)| host.trim())
        .unwrap_or(listen.trim());
    let client_host = if listen_lan_from_addr(listen) {
        "127.0.0.1"
    } else {
        host
    };
    format!("http://{client_host}:{port}")
}

fn is_wildcard_bind_host(host: &str) -> bool {
    matches!(host.trim(), "0.0.0.0" | "::" | "[::]")
}

fn ensure_http_scheme(url: &str) -> String {
    let trimmed = url.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    if trimmed.contains("://") {
        trimmed.to_string()
    } else {
        format!("http://{trimmed}")
    }
}

fn parse_http_authority(url: &str) -> Option<(String, String, String, String)> {
    let with_scheme = ensure_http_scheme(url);
    if with_scheme.is_empty() {
        return None;
    }
    let (scheme, rest) = with_scheme.split_once("://")?;
    let (authority, path_and_query) = match rest.split_once('/') {
        Some((auth, path)) => (auth.to_string(), format!("/{path}")),
        None => (rest.to_string(), String::new()),
    };

    let (host, port_suffix) = if let Some((bracket_host, port)) = authority.rsplit_once("]:") {
        let host = bracket_host.trim_start_matches('[').to_string();
        (host, format!(":{port}"))
    } else if let Some((host, port)) = authority.rsplit_once(':') {
        if host.contains('.') || host.contains(':') {
            (host.to_string(), format!(":{port}"))
        } else {
            (authority.clone(), String::new())
        }
    } else {
        (authority.clone(), String::new())
    };

    Some((scheme.to_string(), host, port_suffix, path_and_query))
}

fn should_map_local_service_host_to_loopback(host: &str) -> bool {
    if is_wildcard_bind_host(host) {
        return true;
    }
    if host.eq_ignore_ascii_case("localhost") {
        return false;
    }
    if let Some(lan) = primary_lan_ipv4() {
        if host == lan {
            return true;
        }
    }
    use std::net::IpAddr;
    match host.parse::<IpAddr>() {
        Ok(IpAddr::V4(v4)) => v4.is_private() || v4.is_link_local(),
        Ok(IpAddr::V6(v6)) if v6.is_loopback() => false,
        Ok(IpAddr::V6(_)) => false,
        Err(_) => false,
    }
}

/// Normalize an HTTP(S) URL for local client access.
/// Wildcard bind hosts and local LAN addresses are mapped to loopback.
pub fn normalize_client_http_url(url: &str) -> String {
    let trimmed = url.trim();
    if trimmed.is_empty() {
        return url.to_string();
    }

    let Some((scheme, host, port_suffix, path_and_query)) = parse_http_authority(trimmed) else {
        return url.to_string();
    };

    if !should_map_local_service_host_to_loopback(&host) {
        if trimmed.contains("://") {
            return trimmed.to_string();
        }
        return ensure_http_scheme(trimmed);
    }

    format!("{scheme}://127.0.0.1{port_suffix}{path_and_query}")
}

pub fn normalize_listen_port(port: u16) -> Result<u16, String> {
    if (LISTEN_PORT_MIN..=LISTEN_PORT_MAX).contains(&port) {
        Ok(port)
    } else {
        Err(format!(
            "listen_port must be between {LISTEN_PORT_MIN} and {LISTEN_PORT_MAX}"
        ))
    }
}

pub fn gateway_view_from_section(g: &GatewaySection) -> GatewayConfigView {
    GatewayConfigView {
        route: g.route.clone(),
        routing_mode: g.routing_mode.clone(),
        default_profile: g.default_profile.clone(),
        ctx_edge_max_tokens: g.ctx_edge_max_tokens,
        experience_enabled: g.experience_enabled,
        experience_learning_rate: g.experience_learning_rate,
        experience_max_bias: g.experience_max_bias,
        experience_target_fallback: g.experience_target_fallback,
        cloud_cache_decay_half_life_secs: g.cloud_cache_decay_half_life_secs,
        cloud_cache_boost_max: g.cloud_cache_boost_max,
        request_route_cache_enabled: g.request_route_cache_enabled,
        request_route_cache_retention_days: g.request_route_cache_retention_days,
        request_route_cache_cleanup_interval_secs: g.request_route_cache_cleanup_interval_secs,
        session_persist_enabled: g.session_persist_enabled,
        work_verify_sample_rate: g.work_verify_sample_rate,
        adaptive_routing_enabled: g.adaptive_routing_enabled,
        adaptive_min_verified_samples: g.adaptive_min_verified_samples,
        adaptive_verify_rate_floor: g.adaptive_verify_rate_floor,
        adaptive_verify_rate_ceiling: g.adaptive_verify_rate_ceiling,
        adaptive_max_theta_shift: g.adaptive_max_theta_shift,
        classifier_enabled: g.classifier_enabled,
        classifier_min_samples: g.classifier_min_samples,
        classifier_prior_alpha: g.classifier_prior_alpha,
        classifier_decay_half_life_hours: g.classifier_decay_half_life_hours,
        classifier_prior_from_heuristic: g.classifier_prior_from_heuristic,
        listen_port: listen_port_from_addr(&g.listen),
        listen_lan: listen_lan_from_addr(&g.listen),
        auth_enabled: g.auth_enabled,
        api_key_set: !crate::config::auth_keys::collect_inbound_api_keys(g).is_empty(),
        api_key_preview: g
            .api_keys
            .first()
            .map(|entry| entry.key.as_str())
            .or(g.api_key.as_deref())
            .and_then(mask_gateway_api_key),
    }
}

pub fn view_from_config(file: &ConfigFile) -> UpstreamSetupView {
    let agent_cfg = file.agent.get(DEFAULT_CLOUD_BUDGET_AGENT_ID);
    let token_quota_enabled = agent_cfg.map(cloud_token_quota_enabled).unwrap_or(false);
    let cloud_budget = agent_cfg.and_then(cloud_token_budget_display);
    UpstreamSetupView {
        gateway: gateway_view_from_section(&file.gateway),
        edge: file.upstream.edge.as_ref().map(endpoint_view),
        cloud: file.upstream.cloud.as_ref().map(|ep| {
            let mut view = endpoint_view(ep);
            view.token_budget = cloud_budget;
            view.token_quota_enabled = token_quota_enabled;
            view
        }),
        agent_id: None,
    }
}

pub fn view_from_config_for_agent(file: &ConfigFile, agent_id: &str) -> UpstreamSetupView {
    let agent_cfg = file.agent.get(agent_id);
    let budget = agent_cfg.and_then(cloud_token_budget_display);
    let token_quota_enabled = agent_cfg.map(cloud_token_quota_enabled).unwrap_or(false);
    UpstreamSetupView {
        gateway: gateway_view_from_section(&file.gateway),
        edge: agent_cfg
            .and_then(|a| a.upstream.edge.as_ref())
            .map(endpoint_view),
        cloud: agent_cfg
            .and_then(|a| a.upstream.cloud.as_ref())
            .map(endpoint_view)
            .or_else(|| {
                if budget.is_some() || token_quota_enabled {
                    Some(UpstreamEndpointView {
                        configured: false,
                        base_url: String::new(),
                        model: None,
                        upstream_model: None,
                        api_key_set: false,
                        token_budget: budget,
                        token_quota_enabled,
                    })
                } else {
                    None
                }
            })
            .map(|mut v| {
                v.token_budget = budget;
                v.token_quota_enabled = token_quota_enabled;
                v
            }),
        agent_id: Some(agent_id.to_string()),
    }
}

/// Minimum allowed `ctx_edge_max_tokens` (overflow gate uses 80% of this).
pub const CTX_EDGE_MAX_TOKENS_MIN: u32 = 4_096;
/// Maximum allowed `ctx_edge_max_tokens`.
pub const CTX_EDGE_MAX_TOKENS_MAX: u32 = 2_000_000;

pub fn normalize_ctx_edge_max_tokens(value: u32) -> Result<u32, String> {
    if (CTX_EDGE_MAX_TOKENS_MIN..=CTX_EDGE_MAX_TOKENS_MAX).contains(&value) {
        Ok(value)
    } else {
        Err(format!(
            "ctx_edge_max_tokens must be between {CTX_EDGE_MAX_TOKENS_MIN} and {CTX_EDGE_MAX_TOKENS_MAX}"
        ))
    }
}

pub fn is_setup_validation_error(msg: &str) -> bool {
    msg.contains("ctx_edge_max_tokens")
        || msg.contains("invalid gateway.")
        || msg.contains("must be between")
        || msg.contains("must be in [")
        || msg.contains("listen_port")
}

fn cloud_token_budget_display(agent: &super::file::AgentConfig) -> Option<u64> {
    match agent.cloud_token_budget {
        Some(n) if n > 0 => Some(n),
        _ => agent.cloud_token_budget_saved.filter(|&n| n > 0),
    }
}

fn cloud_token_quota_enabled(agent: &super::file::AgentConfig) -> bool {
    agent.cloud_token_budget.unwrap_or(0) > 0
}

fn apply_cloud_token_budget(agent: &mut super::file::AgentConfig, budget: Option<u64>) {
    match budget {
        Some(0) => {
            if agent.cloud_token_budget.unwrap_or(0) > 0 {
                agent.cloud_token_budget_saved = agent.cloud_token_budget;
            }
            agent.cloud_token_budget = Some(0);
        }
        Some(n) => {
            agent.cloud_token_budget = Some(n);
            agent.cloud_token_budget_saved = Some(n);
        }
        None => {
            agent.cloud_token_budget = None;
            agent.cloud_token_budget_saved = None;
        }
    }
}

fn agent_has_budget_data(agent: &super::file::AgentConfig) -> bool {
    agent.cloud_token_budget.is_some() || agent.cloud_token_budget_saved.is_some()
}

fn apply_global_cloud_token_budget(file: &mut ConfigFile, patch: &Option<UpstreamEndpointPatch>) {
    let Some(cloud_patch) = patch else {
        return;
    };
    let Some(budget) = cloud_patch.token_budget else {
        return;
    };
    let agent = file
        .agent
        .entry(DEFAULT_CLOUD_BUDGET_AGENT_ID.to_string())
        .or_default();
    apply_cloud_token_budget(agent, budget);
    prune_agent_if_empty(file, DEFAULT_CLOUD_BUDGET_AGENT_ID);
}

fn prune_agent_if_empty(file: &mut ConfigFile, agent_id: &str) {
    let Some(agent) = file.agent.get(agent_id) else {
        return;
    };
    if agent.upstream.edge.is_none()
        && agent.upstream.cloud.is_none()
        && !agent_has_budget_data(agent)
    {
        file.agent.remove(agent_id);
    }
}

fn endpoint_view(ep: &UpstreamEndpoint) -> UpstreamEndpointView {
    UpstreamEndpointView {
        configured: endpoint_configured(ep),
        base_url: ep.base_url.clone(),
        model: ep.model.clone(),
        upstream_model: ep.upstream_model.clone(),
        api_key_set: ep
            .api_key
            .as_ref()
            .is_some_and(|k| !k.trim().is_empty()),
        token_budget: None,
        token_quota_enabled: false,
    }
}

/// Default upstream block: cloud model `auto`, edge unset.
pub fn apply_default_upstream(file: &mut ConfigFile) {
    file.upstream = UpstreamSection {
        cloud: Some(UpstreamEndpoint {
            base_url: String::new(),
            api_key: None,
            model: Some(CLOUD_MODEL_AUTO.to_string()),
            upstream_model: None,
        }),
        edge: None,
    };
}

pub fn apply_setup_patch(file: &mut ConfigFile, patch: &UpstreamSetupUpdate) -> Result<(), String> {
    if let Some(gw) = &patch.gateway {
        if !gw.is_empty() {
            apply_gateway_patch(&mut file.gateway, gw)?;
        }
    }
    if let Some(ref agent_id) = patch.agent_id {
        let agent_id = agent_id.trim();
        if agent_id.is_empty() {
            apply_tier_patch_section(&mut file.upstream.edge, &patch.edge);
            apply_tier_patch_section(&mut file.upstream.cloud, &patch.cloud);
            apply_global_cloud_token_budget(file, &patch.cloud);
        } else {
            if let Some(ref cloud_patch) = patch.cloud {
                if let Some(budget) = cloud_patch.token_budget {
                    let agent = file.agent.entry(agent_id.to_string()).or_default();
                    apply_cloud_token_budget(agent, budget);
                }
            }
            let agent = file.agent.entry(agent_id.to_string()).or_default();
            apply_tier_patch_section(&mut agent.upstream.edge, &patch.edge);
            apply_tier_patch_section(&mut agent.upstream.cloud, &patch.cloud);
            if agent.upstream.edge.is_none()
                && agent.upstream.cloud.is_none()
                && !agent_has_budget_data(agent)
            {
                file.agent.remove(agent_id);
            }
        }
    } else {
        if let Some(edge) = &patch.edge {
            apply_tier_patch(&mut file.upstream.edge, edge);
        }
        if let Some(cloud) = &patch.cloud {
            apply_tier_patch(&mut file.upstream.cloud, cloud);
        }
        apply_global_cloud_token_budget(file, &patch.cloud);
    }
    Ok(())
}

fn apply_tier_patch_section(slot: &mut Option<UpstreamEndpoint>, patch: &Option<UpstreamEndpointPatch>) {
    if let Some(p) = patch {
        apply_tier_patch(slot, p);
    }
}

/// Backward-compatible alias.
pub fn apply_upstream_patch(file: &mut ConfigFile, patch: &UpstreamSetupUpdate) -> Result<(), String> {
    apply_setup_patch(file, patch)
}

fn apply_gateway_patch(g: &mut GatewaySection, patch: &GatewayConfigPatch) -> Result<(), String> {
    if let Some(route) = &patch.route {
        validate_route(route)?;
        g.route = route.trim().to_ascii_lowercase();
    }
    if let Some(mode) = &patch.routing_mode {
        validate_routing_mode(mode)?;
        g.routing_mode = mode.trim().to_ascii_lowercase();
    }
    if let Some(profile) = &patch.default_profile {
        validate_profile(profile)?;
        g.default_profile = profile.trim().to_ascii_lowercase();
    }
    if let Some(value) = patch.ctx_edge_max_tokens {
        g.ctx_edge_max_tokens = normalize_ctx_edge_max_tokens(value)?;
    }
    if let Some(v) = patch.experience_enabled {
        g.experience_enabled = v;
    }
    if let Some(v) = patch.experience_learning_rate {
        g.experience_learning_rate = clamp_unit_f32(v, "experience_learning_rate")?;
    }
    if let Some(v) = patch.experience_max_bias {
        g.experience_max_bias = clamp_range_f32(v, 0.0, 1.0, "experience_max_bias")?;
    }
    if let Some(v) = patch.experience_target_fallback {
        g.experience_target_fallback = clamp_range_f32(v, 0.0, 1.0, "experience_target_fallback")?;
    }
    if let Some(v) = patch.cloud_cache_decay_half_life_secs {
        g.cloud_cache_decay_half_life_secs =
            clamp_range_u64(v, 0, 604_800, "cloud_cache_decay_half_life_secs")?;
    }
    if let Some(v) = patch.cloud_cache_boost_max {
        g.cloud_cache_boost_max = clamp_range_f32(v, 0.0, 1.0, "cloud_cache_boost_max")?;
    }
    if let Some(v) = patch.request_route_cache_enabled {
        g.request_route_cache_enabled = v;
    }
    if let Some(v) = patch.request_route_cache_retention_days {
        g.request_route_cache_retention_days =
            clamp_range_u64(v, 0, 365, "request_route_cache_retention_days")?;
    }
    if let Some(v) = patch.request_route_cache_cleanup_interval_secs {
        g.request_route_cache_cleanup_interval_secs =
            clamp_range_u64(v, 60, 86_400, "request_route_cache_cleanup_interval_secs")?;
    }
    if let Some(v) = patch.session_persist_enabled {
        g.session_persist_enabled = v;
    }
    if let Some(v) = patch.work_verify_sample_rate {
        g.work_verify_sample_rate = clamp_range_f32(v, 0.0, 1.0, "work_verify_sample_rate")?;
    }
    if let Some(v) = patch.adaptive_routing_enabled {
        g.adaptive_routing_enabled = v;
    }
    if let Some(v) = patch.adaptive_min_verified_samples {
        g.adaptive_min_verified_samples = clamp_range_u64(v, 1, 1_000_000, "adaptive_min_verified_samples")?;
    }
    if let Some(v) = patch.adaptive_verify_rate_floor {
        g.adaptive_verify_rate_floor = clamp_range_f32(v, 0.0, 1.0, "adaptive_verify_rate_floor")?;
    }
    if let Some(v) = patch.adaptive_verify_rate_ceiling {
        g.adaptive_verify_rate_ceiling =
            clamp_range_f32(v, 0.0, 1.0, "adaptive_verify_rate_ceiling")?;
    }
    if let Some(v) = patch.adaptive_max_theta_shift {
        g.adaptive_max_theta_shift = clamp_range_f32(v, 0.0, 0.5, "adaptive_max_theta_shift")?;
    }
    if let Some(v) = patch.classifier_enabled {
        g.classifier_enabled = v;
    }
    if let Some(v) = patch.classifier_min_samples {
        g.classifier_min_samples = v;
    }
    if let Some(v) = patch.classifier_prior_alpha {
        g.classifier_prior_alpha = clamp_range_f32(v, 0.0, 100.0, "classifier_prior_alpha")?;
    }
    if let Some(v) = patch.classifier_decay_half_life_hours {
        g.classifier_decay_half_life_hours = v.max(0.0);
    }
    if let Some(v) = patch.classifier_prior_from_heuristic {
        g.classifier_prior_from_heuristic = v;
    }
    let mut listen_port = listen_port_from_addr(&g.listen);
    let mut listen_lan = listen_lan_from_addr(&g.listen);
    if let Some(port) = patch.listen_port {
        listen_port = normalize_listen_port(port)?;
    }
    if let Some(lan) = patch.listen_lan {
        listen_lan = lan;
    }
    if patch.listen_port.is_some() || patch.listen_lan.is_some() {
        g.listen = build_listen_addr(listen_port, listen_lan);
    }
    if let Some(v) = patch.auth_enabled {
        g.auth_enabled = v;
    }
    if let Some(key) = &patch.api_key {
        let k = key.trim();
        g.api_key = if k.is_empty() {
            None
        } else {
            Some(normalize_gateway_api_key(k)?)
        };
    }
    if g.adaptive_verify_rate_floor > g.adaptive_verify_rate_ceiling {
        return Err(
            "adaptive_verify_rate_floor must be <= adaptive_verify_rate_ceiling".into(),
        );
    }
    Ok(())
}

pub fn normalize_gateway_api_key(key: &str) -> Result<String, String> {
    let k = key.trim();
    if k.is_empty() {
        return Err("gateway.api_key cannot be empty".into());
    }
    if !k.starts_with("token-") {
        return Err("gateway.api_key must start with `token-`".into());
    }
    let suffix = &k[6..];
    if suffix.len() < 32 {
        return Err("gateway.api_key suffix must be at least 32 characters".into());
    }
    if !suffix.chars().all(|c| c.is_ascii_alphanumeric()) {
        return Err("gateway.api_key suffix must be alphanumeric".into());
    }
    Ok(k.to_string())
}

/// Masked preview for UI: `token-abc` + `x` * (len - 6) + last 3 suffix chars.
pub fn mask_gateway_api_key(key: &str) -> Option<String> {
    let k = key.trim();
    if k.is_empty() || !k.starts_with("token-") {
        return None;
    }
    let suffix = &k[6..];
    if suffix.is_empty() {
        return None;
    }
    if suffix.len() <= 6 {
        return Some(format!("token-{}", "x".repeat(suffix.len())));
    }
    let head = &suffix[..3];
    let tail = &suffix[suffix.len() - 3..];
    let masked_len = suffix.len().saturating_sub(6);
    Some(format!("token-{head}{}{tail}", "x".repeat(masked_len)))
}

fn validate_route(s: &str) -> Result<(), String> {
    match s.trim().to_ascii_lowercase().as_str() {
        "auto" | "edge" | "cloud" | "cascade" => Ok(()),
        other => Err(format!(
            "invalid gateway.route `{other}` (use auto|edge|cloud|cascade)"
        )),
    }
}

fn validate_routing_mode(s: &str) -> Result<(), String> {
    match s.trim().to_ascii_lowercase().as_str() {
        "single" | "cascade" | "split" => Ok(()),
        other => Err(format!(
            "invalid gateway.routing_mode `{other}` (use single|cascade|split)"
        )),
    }
}

fn validate_profile(s: &str) -> Result<(), String> {
    match s.trim().to_ascii_lowercase().as_str() {
        "economy" | "balanced" | "premium" | "privacy" => Ok(()),
        other => Err(format!(
            "invalid gateway.default_profile `{other}` (use economy|balanced|premium|privacy)"
        )),
    }
}

fn clamp_unit_f32(v: f32, name: &str) -> Result<f32, String> {
    clamp_range_f32(v, 0.0, 1.0, name)
}

fn clamp_range_f32(v: f32, min: f32, max: f32, name: &str) -> Result<f32, String> {
    if v.is_finite() && (min..=max).contains(&v) {
        Ok(v)
    } else {
        Err(format!("{name} must be in [{min}, {max}]"))
    }
}

fn clamp_range_u64(v: u64, min: u64, max: u64, name: &str) -> Result<u64, String> {
    if (min..=max).contains(&v) {
        Ok(v)
    } else {
        Err(format!("{name} must be between {min} and {max}"))
    }
}

fn apply_tier_patch(slot: &mut Option<UpstreamEndpoint>, patch: &UpstreamEndpointPatch) {
    if patch.clear {
        *slot = None;
        return;
    }

    let entry = slot.get_or_insert_with(|| UpstreamEndpoint {
        base_url: String::new(),
        api_key: None,
        model: None,
        upstream_model: None,
    });

    if let Some(url) = &patch.base_url {
        entry.base_url = url.trim().to_string();
    }
    if let Some(model) = &patch.model {
        let m = model.trim();
        entry.model = if m.is_empty() { None } else { Some(m.to_string()) };
    }
    if let Some(upstream_model) = &patch.upstream_model {
        let m = upstream_model.trim();
        entry.upstream_model = if m.is_empty() { None } else { Some(m.to_string()) };
    }
    if let Some(key) = &patch.api_key {
        let k = key.trim();
        entry.api_key = if k.is_empty() { None } else { Some(k.to_string()) };
    }

    if !endpoint_configured(entry)
        && entry.api_key.is_none()
        && entry.model.is_none()
    {
        *slot = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::file::{load_from_path, save};

    #[test]
    fn default_setup_cloud_auto_edge_empty() {
        let mut file = ConfigFile::default();
        apply_default_upstream(&mut file);
        assert!(file.upstream.edge.is_none());
        let cloud = file.upstream.cloud.as_ref().unwrap();
        assert_eq!(cloud.model.as_deref(), Some("auto"));
        assert!(cloud.base_url.is_empty());
        let view = view_from_config(&file);
        assert!(!view.cloud.as_ref().unwrap().configured);
        assert_eq!(view.cloud.as_ref().unwrap().model.as_deref(), Some("auto"));
        assert_eq!(view.gateway.route, "auto");
    }

    #[test]
    fn patch_cloud_token_budget_global() {
        let mut file = ConfigFile::default();
        apply_default_upstream(&mut file);
        apply_setup_patch(
            &mut file,
            &UpstreamSetupUpdate {
                cloud: Some(UpstreamEndpointPatch {
                    token_budget: Some(Some(500_000)),
                    ..Default::default()
                }),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(
            file.agent
                .get(DEFAULT_CLOUD_BUDGET_AGENT_ID)
                .and_then(|a| a.cloud_token_budget),
            Some(500_000)
        );
        let view = view_from_config(&file);
        assert_eq!(view.cloud.as_ref().unwrap().token_budget, Some(500_000));
        assert!(view.cloud.as_ref().unwrap().token_quota_enabled);

        apply_setup_patch(
            &mut file,
            &UpstreamSetupUpdate {
                cloud: Some(UpstreamEndpointPatch {
                    token_budget: Some(Some(0)),
                    ..Default::default()
                }),
                ..Default::default()
            },
        )
        .unwrap();
        let agent = file.agent.get(DEFAULT_CLOUD_BUDGET_AGENT_ID).unwrap();
        assert_eq!(agent.cloud_token_budget, Some(0));
        assert_eq!(agent.cloud_token_budget_saved, Some(500_000));
        let view = view_from_config(&file);
        assert_eq!(view.cloud.as_ref().unwrap().token_budget, Some(500_000));
        assert!(!view.cloud.as_ref().unwrap().token_quota_enabled);

        apply_setup_patch(
            &mut file,
            &UpstreamSetupUpdate {
                cloud: Some(UpstreamEndpointPatch {
                    token_budget: Some(None),
                    ..Default::default()
                }),
                ..Default::default()
            },
        )
        .unwrap();
        assert!(!file.agent.contains_key(DEFAULT_CLOUD_BUDGET_AGENT_ID));
        let view = view_from_config(&file);
        assert_eq!(view.cloud.as_ref().unwrap().token_budget, None);
        assert!(!view.cloud.as_ref().unwrap().token_quota_enabled);
    }

    #[test]
    fn patch_cloud_url() {
        let mut file = ConfigFile::default();
        apply_default_upstream(&mut file);
        apply_setup_patch(
            &mut file,
            &UpstreamSetupUpdate {
                cloud: Some(UpstreamEndpointPatch {
                    base_url: Some("https://api.deepseek.com/v1".into()),
                    ..Default::default()
                }),
                ..Default::default()
            },
        )
        .unwrap();
        assert!(endpoint_configured(file.upstream.cloud.as_ref().unwrap()));
    }

    #[test]
    fn patch_ctx_edge_max_tokens() {
        let mut file = ConfigFile::default();
        apply_setup_patch(
            &mut file,
            &UpstreamSetupUpdate {
                gateway: Some(GatewayConfigPatch {
                    ctx_edge_max_tokens: Some(32_768),
                    ..Default::default()
                }),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(file.gateway.ctx_edge_max_tokens, 32_768);
        let view = view_from_config(&file);
        assert_eq!(view.gateway.ctx_edge_max_tokens, 32_768);
    }

    #[test]
    fn patch_ctx_edge_max_tokens_rejects_out_of_range() {
        let mut file = ConfigFile::default();
        assert!(apply_setup_patch(
            &mut file,
            &UpstreamSetupUpdate {
                gateway: Some(GatewayConfigPatch {
                    ctx_edge_max_tokens: Some(100),
                    ..Default::default()
                }),
                ..Default::default()
            },
        )
        .is_err());
    }

    #[test]
    fn client_gateway_http_url_maps_wildcard_bind_to_loopback() {
        assert_eq!(
            client_gateway_http_url("0.0.0.0:11080"),
            "http://127.0.0.1:11080"
        );
        assert_eq!(
            client_gateway_http_url("127.0.0.1:11080"),
            "http://127.0.0.1:11080"
        );
    }

    #[test]
    fn normalize_client_http_url_maps_wildcard_bind_to_loopback() {
        assert_eq!(
            normalize_client_http_url("http://0.0.0.0:8080"),
            "http://127.0.0.1:8080"
        );
        assert_eq!(
            normalize_client_http_url("http://0.0.0.0:8080/v1"),
            "http://127.0.0.1:8080/v1"
        );
        assert_eq!(
            normalize_client_http_url("0.0.0.0:8080/v1"),
            "http://127.0.0.1:8080/v1"
        );
        assert_eq!(
            normalize_client_http_url("http://127.0.0.1:8080/v1"),
            "http://127.0.0.1:8080/v1"
        );
        assert_eq!(
            normalize_client_http_url("http://192.168.1.42:8080/v1"),
            "http://127.0.0.1:8080/v1"
        );
    }

    #[test]
    fn patch_gateway_api_key() {
        let mut file = ConfigFile::default();
        apply_setup_patch(
            &mut file,
            &UpstreamSetupUpdate {
                gateway: Some(GatewayConfigPatch {
                    api_key: Some("token-abcdefghijklmnopqrstuvwxyz012345".into()),
                    ..Default::default()
                }),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(
            file.gateway.api_key.as_deref(),
            Some("token-abcdefghijklmnopqrstuvwxyz012345")
        );
        let view = view_from_config(&file);
        assert!(view.gateway.api_key_set);
        assert_eq!(
            view.gateway.api_key_preview.as_deref(),
            Some("token-abcxxxxxxxxxxxxxxxxxxxxxxxxxx345")
        );
    }

    #[test]
    fn mask_gateway_api_key_preview() {
        assert_eq!(
            mask_gateway_api_key("token-abcdefghijklmnopqrstuvwxyz012345"),
            Some("token-abcxxxxxxxxxxxxxxxxxxxxxxxxxx345".into())
        );
        assert_eq!(mask_gateway_api_key("token-short"), Some("token-xxxxx".into()));
        assert_eq!(mask_gateway_api_key(""), None);
    }

    #[test]
    fn lan_client_http_url_none_when_loopback_only() {
        assert!(lan_client_http_url("127.0.0.1:11080").is_none());
    }

    #[test]
    fn patch_listen_port_and_lan() {
        let mut file = ConfigFile::default();
        apply_setup_patch(
            &mut file,
            &UpstreamSetupUpdate {
                gateway: Some(GatewayConfigPatch {
                    listen_port: Some(12080),
                    listen_lan: Some(true),
                    ..Default::default()
                }),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(file.gateway.listen, "0.0.0.0:12080");
        let view = view_from_config(&file);
        assert_eq!(view.gateway.listen_port, 12080);
        assert!(view.gateway.listen_lan);
    }

    #[test]
    fn patch_auth_enabled() {
        let mut file = ConfigFile::default();
        apply_setup_patch(
            &mut file,
            &UpstreamSetupUpdate {
                gateway: Some(GatewayConfigPatch {
                    auth_enabled: Some(true),
                    ..Default::default()
                }),
                ..Default::default()
            },
        )
        .unwrap();
        assert!(file.gateway.auth_enabled);
        let view = view_from_config(&file);
        assert!(view.gateway.auth_enabled);

        let patch: UpstreamSetupUpdate = serde_json::from_str(
            r#"{"gateway":{"listen_port":11080,"listen_lan":false,"auth_enabled":true}}"#,
        )
        .unwrap();
        apply_setup_patch(&mut file, &patch).unwrap();
        assert!(file.gateway.auth_enabled);
    }

    #[test]
    fn auth_enabled_save_load_roundtrip() {
        let dir = std::env::temp_dir().join(format!(
            "tr-auth-enabled-{}",
            uuid::Uuid::new_v4().simple()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config.toml");

        let mut file = ConfigFile::default();
        file.gateway.auth_enabled = true;
        save(&path, &file).unwrap();

        let raw = std::fs::read_to_string(&path).unwrap();
        assert!(raw.contains("auth_enabled"));

        let (loaded, _) = load_from_path(&path).unwrap();
        assert!(loaded.gateway.auth_enabled);

        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn patch_listen_port_rejects_out_of_range() {
        let mut file = ConfigFile::default();
        assert!(apply_setup_patch(
            &mut file,
            &UpstreamSetupUpdate {
                gateway: Some(GatewayConfigPatch {
                    listen_port: Some(80),
                    ..Default::default()
                }),
                ..Default::default()
            },
        )
        .is_err());
    }

    #[test]
    fn patch_route_and_experience() {
        let mut file = ConfigFile::default();
        apply_setup_patch(
            &mut file,
            &UpstreamSetupUpdate {
                gateway: Some(GatewayConfigPatch {
                    route: Some("edge".into()),
                    experience_enabled: Some(false),
                    work_verify_sample_rate: Some(0.25),
                    ..Default::default()
                }),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(file.gateway.route, "edge");
        assert!(!file.gateway.experience_enabled);
        assert!((file.gateway.work_verify_sample_rate - 0.25).abs() < f32::EPSILON);
    }

    #[test]
    fn patch_rejects_invalid_route() {
        let mut file = ConfigFile::default();
        assert!(apply_setup_patch(
            &mut file,
            &UpstreamSetupUpdate {
                gateway: Some(GatewayConfigPatch {
                    route: Some("invalid".into()),
                    ..Default::default()
                }),
                ..Default::default()
            },
        )
        .is_err());
    }
}
