use crate::gateway::config::AppConfig;
use crate::gateway::edge_load::EdgeInferenceTracker;
use crate::gateway::error::{AppError, AppResult};
use crate::gateway::routing::RouteTier;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImageTier {
    Edge,
    Cloud,
}

impl ImageTier {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Edge => "edge",
            Self::Cloud => "cloud",
        }
    }
}

/// Resolve edge vs cloud for image generation/edits (no chat `decide()`).
pub fn resolve_image_tier(
    config: &AppConfig,
    edge_load: Option<&EdgeInferenceTracker>,
) -> AppResult<(ImageTier, Vec<String>)> {
    let mut reasons = Vec::new();
    let edge_ok = config.image_edge.is_some();
    let cloud_ok = config.image_cloud.is_some();

    if !edge_ok && !cloud_ok {
        return Err(AppError::Unavailable(
            "no image upstream configured; set [upstream.image.edge] and/or [upstream.image.cloud]"
                .into(),
        ));
    }

    let preferred = match config.image_route {
        Some(RouteTier::Edge) => {
            reasons.push("CONFIG_IMAGE_ROUTE_EDGE".into());
            if !edge_ok {
                return Err(AppError::Unavailable(
                    "gateway.image_route=edge but [upstream.image.edge] is not configured".into(),
                ));
            }
            return Ok((ImageTier::Edge, reasons));
        }
        Some(RouteTier::Cloud) => {
            reasons.push("CONFIG_IMAGE_ROUTE_CLOUD".into());
            if !cloud_ok {
                return Err(AppError::Unavailable(
                    "gateway.image_route=cloud but [upstream.image.cloud] is not configured".into(),
                ));
            }
            return Ok((ImageTier::Cloud, reasons));
        }
        Some(RouteTier::Cascade) | None => {
            reasons.push("CONFIG_IMAGE_ROUTE_AUTO".into());
            if edge_ok && cloud_ok {
                ImageTier::Edge
            } else if edge_ok {
                ImageTier::Edge
            } else {
                ImageTier::Cloud
            }
        }
    };

    // auto + edge busy → cloud when available
    if preferred == ImageTier::Edge
        && cloud_ok
        && edge_load.is_some_and(|t| t.is_busy())
        && matches!(config.image_route, None)
    {
        reasons.push("GATE_EDGE_BUSY".into());
        return Ok((ImageTier::Cloud, reasons));
    }

    Ok((preferred, reasons))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::file::{
        ConfigFile, ImageUpstreamEndpoint, ImageUpstreamSection, UpstreamSection,
    };
    use crate::gateway::edge_load::EdgeInferenceTracker;
    use std::sync::Arc;

    fn cfg_with(image_route: &str, edge: bool, cloud: bool) -> AppConfig {
        let mut file = ConfigFile::default();
        file.gateway.image_route = image_route.into();
        file.upstream = UpstreamSection {
            edge: None,
            cloud: None,
            image: ImageUpstreamSection {
                edge: edge.then(|| ImageUpstreamEndpoint {
                    provider: "comfyui".into(),
                    base_url: "http://127.0.0.1:8188".into(),
                    api_key: None,
                    model: Some("ckpt.safetensors".into()),
                    upstream_model: None,
                    workflow_file: None,
                    workflow_file_i2i: None,
                }),
                cloud: cloud.then(|| ImageUpstreamEndpoint {
                    provider: "openai".into(),
                    base_url: "https://api.openai.com/v1".into(),
                    api_key: Some("sk-test".into()),
                    model: Some("gpt-image-1".into()),
                    upstream_model: None,
                    workflow_file: None,
                    workflow_file_i2i: None,
                }),
            },
            video: Default::default(),
        };
        AppConfig::from_file(file, std::env::temp_dir()).unwrap()
    }

    #[test]
    fn auto_prefers_edge_when_both() {
        let config = cfg_with("auto", true, true);
        let (tier, _) = resolve_image_tier(&config, None).unwrap();
        assert_eq!(tier, ImageTier::Edge);
    }

    #[test]
    fn auto_edge_busy_goes_cloud() {
        let config = cfg_with("auto", true, true);
        let tracker = EdgeInferenceTracker::new();
        let _g = tracker.begin();
        let (tier, reasons) = resolve_image_tier(&config, Some(tracker.as_ref())).unwrap();
        assert_eq!(tier, ImageTier::Cloud);
        assert!(reasons.iter().any(|r| r == "GATE_EDGE_BUSY"));
    }

    #[test]
    fn fixed_edge_ignores_busy() {
        let config = cfg_with("edge", true, true);
        let tracker = EdgeInferenceTracker::new();
        let _g = tracker.begin();
        let (tier, reasons) = resolve_image_tier(&config, Some(tracker.as_ref())).unwrap();
        assert_eq!(tier, ImageTier::Edge);
        assert!(!reasons.iter().any(|r| r == "GATE_EDGE_BUSY"));
    }

    #[test]
    fn auto_cloud_only() {
        let config = cfg_with("auto", false, true);
        let (tier, _) = resolve_image_tier(&config, None).unwrap();
        assert_eq!(tier, ImageTier::Cloud);
    }

    #[test]
    fn none_configured_errors() {
        let config = cfg_with("auto", false, false);
        assert!(resolve_image_tier(&config, None).is_err());
    }
}
