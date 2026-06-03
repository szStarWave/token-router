use super::file::{ConfigFile, GatewaySection, UpstreamEndpoint, UpstreamSection};
use serde::{Deserialize, Serialize};

pub const CLOUD_MODEL_AUTO: &str = "auto";

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
    pub cloud_sticky_ttl_secs: u64,
    pub session_persist_enabled: bool,
    pub work_verify_sample_rate: f32,
    pub adaptive_routing_enabled: bool,
    pub adaptive_min_verified_samples: u64,
    pub adaptive_verify_rate_floor: f32,
    pub adaptive_verify_rate_ceiling: f32,
    pub adaptive_max_theta_shift: f32,
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
    pub cloud_sticky_ttl_secs: Option<u64>,
    pub session_persist_enabled: Option<bool>,
    pub work_verify_sample_rate: Option<f32>,
    pub adaptive_routing_enabled: Option<bool>,
    pub adaptive_min_verified_samples: Option<u64>,
    pub adaptive_verify_rate_floor: Option<f32>,
    pub adaptive_verify_rate_ceiling: Option<f32>,
    pub adaptive_max_theta_shift: Option<f32>,
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
            && self.cloud_sticky_ttl_secs.is_none()
            && self.session_persist_enabled.is_none()
            && self.work_verify_sample_rate.is_none()
            && self.adaptive_routing_enabled.is_none()
            && self.adaptive_min_verified_samples.is_none()
            && self.adaptive_verify_rate_floor.is_none()
            && self.adaptive_verify_rate_ceiling.is_none()
            && self.adaptive_max_theta_shift.is_none()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpstreamSetupView {
    pub gateway: GatewayConfigView,
    pub edge: Option<UpstreamEndpointView>,
    pub cloud: Option<UpstreamEndpointView>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct UpstreamSetupUpdate {
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
    pub api_key_set: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct UpstreamEndpointPatch {
    /// Set to empty string to clear `base_url`.
    pub base_url: Option<String>,
    /// Omit to keep existing; empty string clears the key.
    pub api_key: Option<String>,
    pub model: Option<String>,
    /// When true, remove this tier entirely (edge only).
    #[serde(default)]
    pub clear: bool,
}

pub fn endpoint_configured(ep: &UpstreamEndpoint) -> bool {
    !ep.base_url.trim().is_empty()
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
        cloud_sticky_ttl_secs: g.cloud_sticky_ttl_secs,
        session_persist_enabled: g.session_persist_enabled,
        work_verify_sample_rate: g.work_verify_sample_rate,
        adaptive_routing_enabled: g.adaptive_routing_enabled,
        adaptive_min_verified_samples: g.adaptive_min_verified_samples,
        adaptive_verify_rate_floor: g.adaptive_verify_rate_floor,
        adaptive_verify_rate_ceiling: g.adaptive_verify_rate_ceiling,
        adaptive_max_theta_shift: g.adaptive_max_theta_shift,
    }
}

pub fn view_from_config(file: &ConfigFile) -> UpstreamSetupView {
    UpstreamSetupView {
        gateway: gateway_view_from_section(&file.gateway),
        edge: file.upstream.edge.as_ref().map(endpoint_view),
        cloud: file.upstream.cloud.as_ref().map(endpoint_view),
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
}

fn endpoint_view(ep: &UpstreamEndpoint) -> UpstreamEndpointView {
    UpstreamEndpointView {
        configured: endpoint_configured(ep),
        base_url: ep.base_url.clone(),
        model: ep.model.clone(),
        api_key_set: ep
            .api_key
            .as_ref()
            .is_some_and(|k| !k.trim().is_empty()),
    }
}

/// Default upstream block: cloud model `auto`, edge unset.
pub fn apply_default_upstream(file: &mut ConfigFile) {
    file.upstream = UpstreamSection {
        cloud: Some(UpstreamEndpoint {
            base_url: String::new(),
            api_key: None,
            model: Some(CLOUD_MODEL_AUTO.to_string()),
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
    if let Some(edge) = &patch.edge {
        apply_tier_patch(&mut file.upstream.edge, edge);
    }
    if let Some(cloud) = &patch.cloud {
        apply_tier_patch(&mut file.upstream.cloud, cloud);
    }
    Ok(())
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
    if let Some(v) = patch.cloud_sticky_ttl_secs {
        g.cloud_sticky_ttl_secs = clamp_range_u64(v, 0, 604_800, "cloud_sticky_ttl_secs")?;
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
    if g.adaptive_verify_rate_floor > g.adaptive_verify_rate_ceiling {
        return Err(
            "adaptive_verify_rate_floor must be <= adaptive_verify_rate_ceiling".into(),
        );
    }
    Ok(())
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
    });

    if let Some(url) = &patch.base_url {
        entry.base_url = url.trim().to_string();
    }
    if let Some(model) = &patch.model {
        let m = model.trim();
        entry.model = if m.is_empty() { None } else { Some(m.to_string()) };
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
