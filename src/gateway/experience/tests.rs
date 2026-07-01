#[cfg(test)]
mod tests {
    use super::super::data;
    use super::super::outcome::RequestOutcome;
    use super::super::store::{ExperienceSettings, ExperienceStore};
    use crate::gateway::multimodal::MultimodalStrategy;
    use crate::gateway::routing::{RouteDecision, RouteTier, RoutingMode, StepKind, WorkStrategy};
    use crate::gateway::routing::Profile;

    fn sample_decision(route: RouteTier, step: StepKind) -> RouteDecision {
        RouteDecision {
            route,
            profile: Profile::Balanced,
            mode: RoutingMode::Cascade,
            step_kind: step,
            difficulty: 0.5,
            reason_codes: vec![],
            tokens_in_estimate: 1000,
            tokens_out_estimate: 50,
            cloud_input_saved_estimate: 0,
            conversation_key: "conv:test".into(),
            assistant_failed_recent: false,
            multimodal_strategy: MultimodalStrategy::None,
            work_strategy: WorkStrategy::None,
            force_cloud_sticky: false,
            edge_ok_probability: None,
            classifier_features: None,
            casual_quality_fallback: false,
        }
    }

    #[test]
    fn edge_trusted_after_verified_samples() {
        let settings = ExperienceSettings::default();
        let store = ExperienceStore::new_in_memory(settings);
        assert!(!store.edge_trusted(StepKind::ToolSelect));
        for _ in 0..3 {
            store.record_outcome(
                StepKind::ToolSelect,
                RequestOutcome {
                    edge_ok: true,
                    cascade_fallback: false,
                    upstream_error: false,
                },
            );
        }
        assert!(store.edge_trusted(StepKind::ToolSelect));
    }

    #[test]
    fn bias_increases_after_repeated_fallbacks() {
        let settings = ExperienceSettings {
            enabled: true,
            learning_rate: 0.5,
            max_bias: 0.12,
            target_fallback: 0.15,
        };
        let store = ExperienceStore::new_in_memory(settings);
        let step = StepKind::ToolResultDigest;
        for _ in 0..10 {
            let d = sample_decision(RouteTier::Cascade, step);
            store.record_outcome(step, RequestOutcome::success(&d, true));
        }
        let bias = store.bias_for(step);
        assert!(bias > 0.05, "expected positive bias, got {bias}");
    }

    #[test]
    fn snapshot_includes_totals_and_trust() {
        let settings = ExperienceSettings::default();
        let store = ExperienceStore::new_in_memory(settings);
        for _ in 0..3 {
            store.record_outcome(
                StepKind::ToolSelect,
                RequestOutcome {
                    edge_ok: true,
                    cascade_fallback: false,
                    upstream_error: false,
                },
            );
        }
        store.record_outcome(
            StepKind::DirectChat,
            RequestOutcome {
                edge_ok: false,
                cascade_fallback: false,
                upstream_error: true,
            },
        );
        let snap = store.snapshot();
        assert_eq!(snap.totals.step_kinds, 2);
        assert_eq!(snap.totals.edge_ok, 3);
        assert_eq!(snap.totals.upstream_error, 1);
        assert_eq!(snap.totals.trusted_steps, 1);
        let tool = snap
            .steps
            .iter()
            .find(|s| s.step_kind == "toolselect")
            .unwrap();
        assert!(tool.edge_trusted);
        assert_eq!(tool.verified_total, 3);
    }

    #[test]
    fn persist_roundtrip() {
        let dir = std::env::temp_dir().join(format!(
            "flowy-experience-test-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("experience.json");
        {
            let store =
                ExperienceStore::open(&dir, ExperienceSettings::default()).unwrap();
            let d = sample_decision(RouteTier::Edge, StepKind::DirectChat);
            store.record_outcome(StepKind::DirectChat, RequestOutcome::success(&d, false));
            store.flush().unwrap();
        }
        let loaded = data::load(&path).unwrap();
        assert_eq!(loaded.by_step.get("directchat").unwrap().edge_ok, 1);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
