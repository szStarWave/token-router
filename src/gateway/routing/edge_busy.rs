use crate::gateway::edge_load::EdgeInferenceTracker;
use crate::gateway::multimodal::MultimodalStrategy;

use super::decision::RouteTier;
use super::step_kind::StepKind;
use super::upstream_availability::{cloud_configured, edge_configured};
use super::work::WorkStrategy;
use crate::gateway::config::AppConfig;

fn would_use_edge(route: RouteTier, work: WorkStrategy, multimodal: MultimodalStrategy) -> bool {
    matches!(route, RouteTier::Edge | RouteTier::Cascade)
        || matches!(work, WorkStrategy::Verify)
        || matches!(
            multimodal,
            MultimodalStrategy::CachedEdge
                | MultimodalStrategy::CachedEdgeFallback
                | MultimodalStrategy::Probe
        )
}

/// Returns true when edge is busy and a non-casual step would have used edge.
pub fn edge_busy_applies(
    route: RouteTier,
    work: WorkStrategy,
    multimodal: MultimodalStrategy,
    step_kind: StepKind,
    config: &AppConfig,
    edge_load: Option<&EdgeInferenceTracker>,
) -> bool {
    let Some(tracker) = edge_load else {
        return false;
    };
    if matches!(step_kind, StepKind::DirectChat | StepKind::HeartbeatAck) {
        return false;
    }
    if !tracker.is_busy() || !cloud_configured(config) || !would_use_edge(route, work, multimodal) {
        return false;
    }
    !strict_single_tier(config)
}

/// When edge is mid-inference and cloud is available, skip edge and route cloud directly.
/// Used only for fixed-route config paths that bypass difficulty scoring.
pub fn apply_edge_busy_fallback(
    route: RouteTier,
    work: WorkStrategy,
    multimodal: MultimodalStrategy,
    step_kind: StepKind,
    config: &AppConfig,
    edge_load: Option<&EdgeInferenceTracker>,
    reason_codes: &mut Vec<String>,
) -> (RouteTier, WorkStrategy, MultimodalStrategy) {
    if !edge_busy_applies(route, work, multimodal, step_kind, config, edge_load) {
        return (route, work, multimodal);
    }

    if !reason_codes.iter().any(|c| c == "GATE_EDGE_BUSY") {
        reason_codes.push("GATE_EDGE_BUSY".to_string());
    }

    let route = match route {
        RouteTier::Edge | RouteTier::Cascade => RouteTier::Cloud,
        RouteTier::Cloud => RouteTier::Cloud,
    };
    let work = match work {
        WorkStrategy::Verify => WorkStrategy::None,
        other => other,
    };
    let multimodal = match multimodal {
        MultimodalStrategy::CachedEdge
        | MultimodalStrategy::CachedEdgeFallback
        | MultimodalStrategy::Probe => MultimodalStrategy::None,
        other => other,
    };

    (route, work, multimodal)
}

/// `route=edge` / `route=cloud`, or only one upstream configured — no cross-tier redirects.
fn strict_single_tier(config: &AppConfig) -> bool {
    matches!(
        config.fixed_route,
        Some(RouteTier::Edge) | Some(RouteTier::Cloud)
    ) || matches!(
        (edge_configured(config), cloud_configured(config)),
        (true, false) | (false, true)
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{ConfigFile, UpstreamEndpoint};
    use crate::gateway::config::AppConfig;

    fn dual_config() -> AppConfig {
        let mut file = ConfigFile::default();
        file.upstream.edge = Some(UpstreamEndpoint {
            base_url: "http://127.0.0.1:11434/v1".into(),
            api_key: None,
            model: None,
        });
        file.upstream.cloud = Some(UpstreamEndpoint {
            base_url: "https://api.example.com/v1".into(),
            api_key: None,
            model: None,
        });
        AppConfig::from_file(file, "/tmp/flowy-edge-busy".into()).unwrap()
    }

    #[test]
    fn idle_does_not_apply_busy_bias() {
        let config = dual_config();
        let tracker = EdgeInferenceTracker::new();
        assert!(!edge_busy_applies(
            RouteTier::Edge,
            WorkStrategy::None,
            MultimodalStrategy::None,
            StepKind::ToolSelect,
            &config,
            Some(tracker.as_ref()),
        ));
    }

    #[test]
    fn busy_applies_for_work_step() {
        let config = dual_config();
        let tracker = EdgeInferenceTracker::new();
        let _g = tracker.begin();
        assert!(edge_busy_applies(
            RouteTier::Edge,
            WorkStrategy::None,
            MultimodalStrategy::None,
            StepKind::ToolSelect,
            &config,
            Some(tracker.as_ref()),
        ));
    }

    #[test]
    fn busy_keeps_edge_when_route_fixed_edge() {
        let mut file = ConfigFile::default();
        file.gateway.route = "edge".into();
        file.upstream.edge = Some(UpstreamEndpoint {
            base_url: "http://127.0.0.1:11434/v1".into(),
            api_key: None,
            model: None,
        });
        file.upstream.cloud = Some(UpstreamEndpoint {
            base_url: "https://api.example.com/v1".into(),
            api_key: None,
            model: None,
        });
        let config = AppConfig::from_file(file, "/tmp/flowy-edge-busy-fixed".into()).unwrap();
        let tracker = EdgeInferenceTracker::new();
        let _g = tracker.begin();
        assert!(!edge_busy_applies(
            RouteTier::Edge,
            WorkStrategy::None,
            MultimodalStrategy::None,
            StepKind::ToolSelect,
            &config,
            Some(tracker.as_ref()),
        ));
    }

    #[test]
    fn busy_forces_cloud_on_fixed_route_fallback() {
        let config = dual_config();
        let tracker = EdgeInferenceTracker::new();
        let _g = tracker.begin();
        let mut codes = Vec::new();
        let (route, work, mm) = apply_edge_busy_fallback(
            RouteTier::Cascade,
            WorkStrategy::Verify,
            MultimodalStrategy::None,
            StepKind::ToolSelect,
            &config,
            Some(tracker.as_ref()),
            &mut codes,
        );
        assert_eq!(route, RouteTier::Cloud);
        assert_eq!(work, WorkStrategy::None);
        assert_eq!(mm, MultimodalStrategy::None);
        assert!(codes.iter().any(|c| c == "GATE_EDGE_BUSY"));
    }

    #[test]
    fn busy_does_not_apply_to_direct_chat() {
        let config = dual_config();
        let tracker = EdgeInferenceTracker::new();
        let _g = tracker.begin();
        assert!(!edge_busy_applies(
            RouteTier::Edge,
            WorkStrategy::None,
            MultimodalStrategy::None,
            StepKind::DirectChat,
            &config,
            Some(tracker.as_ref()),
        ));
    }
}
