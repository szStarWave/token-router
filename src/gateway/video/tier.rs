use crate::gateway::config::AppConfig;
use crate::gateway::edge_load::EdgeInferenceTracker;
use crate::gateway::error::{AppError, AppResult};
use crate::gateway::routing::RouteTier;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VideoTier {
    Edge,
    Cloud,
}

impl VideoTier {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Edge => "edge",
            Self::Cloud => "cloud",
        }
    }
}

/// Resolve edge vs cloud for video generation (mirrors image tier logic).
pub fn resolve_video_tier(
    config: &AppConfig,
    edge_load: Option<&EdgeInferenceTracker>,
) -> AppResult<(VideoTier, Vec<String>)> {
    let mut reasons = Vec::new();
    let edge_ok = config.video_edge.is_some();
    let cloud_ok = config.video_cloud.is_some();

    if !edge_ok && !cloud_ok {
        return Err(AppError::Unavailable(
            "no video upstream configured; set [upstream.video.edge] and/or [upstream.video.cloud]"
                .into(),
        ));
    }

    let preferred = match config.video_route {
        Some(RouteTier::Edge) => {
            reasons.push("CONFIG_VIDEO_ROUTE_EDGE".into());
            if !edge_ok {
                return Err(AppError::Unavailable(
                    "gateway.video_route=edge but [upstream.video.edge] is not configured".into(),
                ));
            }
            return Ok((VideoTier::Edge, reasons));
        }
        Some(RouteTier::Cloud) => {
            reasons.push("CONFIG_VIDEO_ROUTE_CLOUD".into());
            if !cloud_ok {
                return Err(AppError::Unavailable(
                    "gateway.video_route=cloud but [upstream.video.cloud] is not configured".into(),
                ));
            }
            return Ok((VideoTier::Cloud, reasons));
        }
        Some(RouteTier::Cascade) | None => {
            reasons.push("CONFIG_VIDEO_ROUTE_AUTO".into());
            if edge_ok {
                VideoTier::Edge
            } else {
                VideoTier::Cloud
            }
        }
    };

    if preferred == VideoTier::Edge
        && cloud_ok
        && edge_load.is_some_and(|t| t.is_busy())
        && matches!(config.video_route, None)
    {
        reasons.push("GATE_EDGE_BUSY".into());
        return Ok((VideoTier::Cloud, reasons));
    }

    Ok((preferred, reasons))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::file::{
        ConfigFile, UpstreamSection, VideoUpstreamEndpoint, VideoUpstreamSection,
    };
    use crate::gateway::edge_load::EdgeInferenceTracker;

    fn cfg_with(video_route: &str, edge: bool, cloud: bool) -> AppConfig {
        let mut file = ConfigFile::default();
        file.gateway.video_route = video_route.into();
        file.upstream = UpstreamSection {
            edge: None,
            cloud: None,
            image: Default::default(),
            video: VideoUpstreamSection {
                edge: edge.then(|| VideoUpstreamEndpoint {
                    provider: "comfyui".into(),
                    base_url: "http://127.0.0.1:8188".into(),
                    api_key: None,
                    model: Some("ckpt.safetensors".into()),
                    upstream_model: None,
                    workflow_file: None,
                    workflow_file_i2v: None,
                }),
                cloud: cloud.then(|| VideoUpstreamEndpoint {
                    provider: "openai".into(),
                    base_url: "https://api.openai.com/v1".into(),
                    api_key: Some("sk-test".into()),
                    model: Some("sora-2".into()),
                    upstream_model: None,
                    workflow_file: None,
                    workflow_file_i2v: None,
                }),
            },
        };
        AppConfig::from_file(file, std::env::temp_dir()).unwrap()
    }

    #[test]
    fn auto_prefers_edge_when_both() {
        let config = cfg_with("auto", true, true);
        let (tier, _) = resolve_video_tier(&config, None).unwrap();
        assert_eq!(tier, VideoTier::Edge);
    }

    #[test]
    fn auto_edge_busy_goes_cloud() {
        let config = cfg_with("auto", true, true);
        let tracker = EdgeInferenceTracker::new();
        let _g = tracker.begin();
        let (tier, reasons) = resolve_video_tier(&config, Some(tracker.as_ref())).unwrap();
        assert_eq!(tier, VideoTier::Cloud);
        assert!(reasons.iter().any(|r| r == "GATE_EDGE_BUSY"));
    }

    #[test]
    fn fixed_edge_ignores_busy() {
        let config = cfg_with("edge", true, true);
        let tracker = EdgeInferenceTracker::new();
        let _g = tracker.begin();
        let (tier, reasons) = resolve_video_tier(&config, Some(tracker.as_ref())).unwrap();
        assert_eq!(tier, VideoTier::Edge);
        assert!(!reasons.iter().any(|r| r == "GATE_EDGE_BUSY"));
    }

    #[test]
    fn auto_cloud_only() {
        let config = cfg_with("auto", false, true);
        let (tier, _) = resolve_video_tier(&config, None).unwrap();
        assert_eq!(tier, VideoTier::Cloud);
    }

    #[test]
    fn none_configured_errors() {
        let config = cfg_with("auto", false, false);
        assert!(resolve_video_tier(&config, None).is_err());
    }
}
