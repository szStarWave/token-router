#[cfg(test)]
mod tests {
    use super::super::data;
    use super::super::store::SessionStore;
    use crate::gateway::experience::RequestOutcome;
    use crate::gateway::multimodal::MultimodalStrategy;
    use crate::gateway::routing::{RouteDecision, RouteTier, RoutingMode, StepKind, WorkStrategy};
    use crate::gateway::routing::Profile;

    fn sample_decision() -> RouteDecision {
        RouteDecision {
            route: RouteTier::Cascade,
            profile: Profile::Balanced,
            mode: RoutingMode::Cascade,
            step_kind: StepKind::InitialPlan,
            difficulty: 0.6,
            reason_codes: vec![],
            tokens_in_estimate: 500,
            tokens_out_estimate: 50,
            cloud_input_saved_estimate: 0,
            conversation_key: "conv:sticky_test".into(),
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
    fn sticky_persists_to_disk() {
        let dir = std::env::temp_dir().join(format!(
            "flowy-session-test-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let key = "conv:sticky_test";
        {
            let store = SessionStore::open(dir.clone(), true).unwrap();
            let d = sample_decision();
            store.apply_outcome(
                key,
                &d,
                RequestOutcome::success(&d, true),
                600,
                false,
            );
            store.flush().unwrap();
            assert!(store.cloud_sticky_until(key).is_some());
        }
        let path = dir.join("conv_sticky_test.json");
        let loaded = data::load(&path).unwrap();
        assert!(loaded.cloud_sticky_until_unix.is_some());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn edge_success_clears_sticky() {
        let store = SessionStore::new_in_memory();
        let key = "conv:clear_sticky";
        let sticky_decision = sample_decision();
        store.apply_outcome(
            key,
            &sticky_decision,
            RequestOutcome {
                edge_ok: false,
                cascade_fallback: true,
                upstream_error: false,
            },
            600,
            false,
        );
        assert!(store.cloud_sticky_until(key).is_some());

        let edge_decision = RouteDecision {
            route: RouteTier::Edge,
            step_kind: StepKind::DirectChat,
            ..sample_decision()
        };
        store.apply_outcome(
            key,
            &edge_decision,
            RequestOutcome::success(&edge_decision, false),
            600,
            false,
        );
        assert!(store.cloud_sticky_until(key).is_none());
    }
}
