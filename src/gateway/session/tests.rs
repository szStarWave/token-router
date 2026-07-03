#[cfg(test)]
mod tests {
    use std::path::Path;
    use std::time::{Duration, SystemTime, UNIX_EPOCH};

    use super::super::data::{self, SessionData};
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
            lexical_learn: Default::default(),
        }
    }

    fn temp_sessions_dir(label: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "flowy-session-test-{label}-{}",
            std::process::id()
        ))
    }

    fn touch_old(path: &Path, days_ago: u64) {
        let ts = SystemTime::now()
            .checked_sub(Duration::from_secs(days_ago * 86_400))
            .unwrap_or(UNIX_EPOCH);
        filetime::set_file_mtime(path, filetime::FileTime::from_system_time(ts)).unwrap();
    }

    #[test]
    fn sticky_persists_to_disk() {
        let dir = temp_sessions_dir("sticky");
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
        assert!(loaded.last_updated_unix > 0);
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

    #[test]
    fn cleanup_removes_invalid_json_and_tmp() {
        let dir = temp_sessions_dir("invalid");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("broken.json"), "{not json").unwrap();
        std::fs::write(dir.join("orphan.json.tmp"), "{}").unwrap();

        let store = SessionStore::open(dir.clone(), true).unwrap();
        let stats = store.cleanup(7).unwrap();
        assert_eq!(stats.removed_invalid, 2);
        assert_eq!(stats.removed_expired, 0);
        assert!(!dir.join("broken.json").exists());
        assert!(!dir.join("orphan.json.tmp").exists());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn cleanup_keeps_active_sticky_even_when_old() {
        let dir = temp_sessions_dir("sticky-keep");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("conv_old_sticky.json");
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let data = SessionData {
            version: data::SESSION_VERSION,
            last_tok_in: 100,
            cloud_sticky_until_unix: Some(now + 3600),
            last_updated_unix: now.saturating_sub(30 * 86_400),
            ..SessionData::default()
        };
        data::save(&path, &data).unwrap();
        touch_old(&path, 30);

        let store = SessionStore::open(dir.clone(), true).unwrap();
        let stats = store.cleanup(7).unwrap();
        assert_eq!(stats.removed_expired, 0);
        assert!(path.exists());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn cleanup_removes_expired_session_by_last_updated() {
        let dir = temp_sessions_dir("expired-updated");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("conv_expired.json");
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let data = SessionData {
            version: data::SESSION_VERSION,
            last_tok_in: 100,
            last_updated_unix: now.saturating_sub(10 * 86_400),
            ..SessionData::default()
        };
        data::save(&path, &data).unwrap();

        let store = SessionStore::open(dir.clone(), true).unwrap();
        let stats = store.cleanup(7).unwrap();
        assert_eq!(stats.removed_expired, 1);
        assert!(!path.exists());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn cleanup_removes_legacy_session_by_mtime() {
        let dir = temp_sessions_dir("expired-mtime");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("conv_legacy.json");
        let data = SessionData {
            version: data::SESSION_VERSION,
            last_tok_in: 50,
            ..SessionData::default()
        };
        data::save(&path, &data).unwrap();
        touch_old(&path, 10);

        let store = SessionStore::open(dir.clone(), true).unwrap();
        let stats = store.cleanup(7).unwrap();
        assert_eq!(stats.removed_expired, 1);
        assert!(!path.exists());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn cleanup_retention_zero_skips_expired_but_removes_invalid() {
        let dir = temp_sessions_dir("retention-zero");
        std::fs::create_dir_all(&dir).unwrap();
        let expired = dir.join("conv_expired.json");
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();
        data::save(
            &expired,
            &SessionData {
                last_updated_unix: now.saturating_sub(30 * 86_400),
                ..SessionData::default()
            },
        )
        .unwrap();
        std::fs::write(dir.join("broken.json"), "[]]").unwrap();

        let store = SessionStore::open(dir.clone(), true).unwrap();
        let stats = store.cleanup(0).unwrap();
        assert_eq!(stats.removed_invalid, 1);
        assert_eq!(stats.removed_expired, 0);
        assert!(expired.exists());
        assert!(!dir.join("broken.json").exists());

        let _ = std::fs::remove_dir_all(&dir);
    }
}
