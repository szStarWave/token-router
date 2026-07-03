use crate::gateway::routing::{RouteDecision, RouteTier, StepKind};

/// Result of a completed chat request (implicit signals for learning).
#[derive(Debug, Clone, Copy, Default)]
pub struct RequestOutcome {
    pub edge_ok: bool,
    pub cascade_fallback: bool,
    pub upstream_error: bool,
}

impl RequestOutcome {
    pub fn success(decision: &RouteDecision, fallback: bool) -> Self {
        match decision.route {
            RouteTier::Edge => {
                if decision.casual_quality_fallback && fallback {
                    Self {
                        edge_ok: false,
                        cascade_fallback: true,
                        upstream_error: false,
                    }
                } else {
                    Self {
                        edge_ok: true,
                        cascade_fallback: false,
                        upstream_error: false,
                    }
                }
            }
            RouteTier::Cloud => Self::default(),
            RouteTier::Cascade => Self {
                edge_ok: !fallback,
                cascade_fallback: fallback,
                upstream_error: false,
            },
        }
    }

    pub fn upstream_error() -> Self {
        Self {
            edge_ok: false,
            cascade_fallback: false,
            upstream_error: true,
        }
    }

    pub fn should_set_cloud_sticky(self, step_kind: StepKind) -> bool {
        self.cascade_fallback
            || (self.upstream_error && step_kind != StepKind::HeartbeatAck)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gateway::routing::{Profile, RoutingMode, StepKind};

    fn edge_decision(casual_quality_fallback: bool) -> RouteDecision {
        RouteDecision {
            route: RouteTier::Edge,
            profile: Profile::Balanced,
            mode: RoutingMode::Cascade,
            step_kind: StepKind::DirectChat,
            difficulty: 0.1,
            reason_codes: vec![],
            tokens_in_estimate: 50,
            tokens_out_estimate: 20,
            cloud_input_saved_estimate: 50,
            conversation_key: "conv:test".into(),
            assistant_failed_recent: false,
            multimodal_strategy: crate::gateway::multimodal::MultimodalStrategy::None,
            work_strategy: crate::gateway::routing::WorkStrategy::None,
            force_cloud_sticky: false,
            edge_ok_probability: None,
            classifier_features: None,
            casual_quality_fallback,
            lexical_learn: Default::default(),
        }
    }

    #[test]
    fn casual_edge_fallback_records_cascade_fallback() {
        let d = edge_decision(true);
        let o = RequestOutcome::success(&d, true);
        assert!(!o.edge_ok);
        assert!(o.cascade_fallback);
    }

    #[test]
    fn casual_edge_ok_without_fallback() {
        let d = edge_decision(true);
        let o = RequestOutcome::success(&d, false);
        assert!(o.edge_ok);
        assert!(!o.cascade_fallback);
    }
}
