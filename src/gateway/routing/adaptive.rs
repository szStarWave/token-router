use serde::Serialize;

use crate::gateway::config::AppConfig;

/// Runtime routing knobs from config only (learning adjusts difficulty, not these thresholds).
#[derive(Debug, Clone, Serialize)]
pub struct EffectiveRouting {
    pub enabled: bool,
    pub work_verify_sample_rate: f32,
    pub theta_edge: f32,
    pub theta_cloud: f32,
    pub base_verify_sample_rate: f32,
    pub base_theta_edge: f32,
    pub base_theta_cloud: f32,
    pub reasons: Vec<String>,
}

impl EffectiveRouting {
    pub fn passthrough(config: &AppConfig) -> Self {
        static_routing(config)
    }
}

pub fn static_routing(config: &AppConfig) -> EffectiveRouting {
    let (theta_edge, theta_cloud) = config.default_profile.thresholds();
    let enabled = config.adaptive_routing.enabled;
    EffectiveRouting {
        enabled,
        work_verify_sample_rate: config.work_verify_sample_rate,
        theta_edge,
        theta_cloud,
        base_verify_sample_rate: config.work_verify_sample_rate,
        base_theta_edge: theta_edge,
        base_theta_cloud: theta_cloud,
        reasons: if enabled {
            vec!["ADAPTIVE_ON".to_string()]
        } else {
            vec!["STATIC_ROUTING".to_string()]
        },
    }
}

/// Returns config-static θ and verify rate. Experience/classifier only adjust difficulty.
pub fn compute_effective_routing(config: &AppConfig) -> EffectiveRouting {
    static_routing(config)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::ConfigFile;
    use crate::gateway::routing::RouteTier;

    fn test_config(verify_rate: f32) -> AppConfig {
        let mut file = ConfigFile::default();
        file.gateway.work_verify_sample_rate = verify_rate;
        file.gateway.route = "auto".to_string();
        file.gateway.routing_mode = "cascade".to_string();
        file.upstream.edge = Some(crate::config::UpstreamEndpoint {
            base_url: "http://127.0.0.1:11434/v1".into(),
            api_key: None,
            model: None,
        });
        file.upstream.cloud = Some(crate::config::UpstreamEndpoint {
            base_url: "https://api.example/v1".into(),
            api_key: None,
            model: None,
        });
        AppConfig::from_file(file, "/tmp/flowy-test".into()).unwrap()
    }

    #[test]
    fn static_routing_matches_config() {
        let config = test_config(0.20);
        let eff = compute_effective_routing(&config);
        assert_eq!(eff.enabled, config.adaptive_routing.enabled);
        assert_eq!(eff.work_verify_sample_rate, 0.20);
        assert_eq!(eff.theta_edge, eff.base_theta_edge);
        assert_eq!(eff.theta_cloud, eff.base_theta_cloud);
        if eff.enabled {
            assert!(eff.reasons.iter().any(|r| r == "ADAPTIVE_ON"));
        } else {
            assert!(eff.reasons.iter().any(|r| r == "STATIC_ROUTING"));
        }
    }

    #[test]
    fn adaptive_enabled_reflects_config_flag() {
        let mut file = ConfigFile::default();
        file.gateway.adaptive_routing_enabled = true;
        file.gateway.work_verify_sample_rate = 0.15;
        file.upstream.edge = Some(crate::config::UpstreamEndpoint {
            base_url: "http://127.0.0.1:11434/v1".into(),
            api_key: None,
            model: None,
        });
        file.upstream.cloud = Some(crate::config::UpstreamEndpoint {
            base_url: "https://api.example/v1".into(),
            api_key: None,
            model: None,
        });
        let config = AppConfig::from_file(file, "/tmp/flowy-test".into()).unwrap();
        let eff = compute_effective_routing(&config);
        assert!(eff.enabled);
        assert!(eff.reasons.iter().any(|r| r == "ADAPTIVE_ON"));
    }

    #[test]
    fn fixed_route_still_static() {
        let mut config = test_config(0.20);
        config.fixed_route = Some(RouteTier::Edge);
        let eff = compute_effective_routing(&config);
        assert_eq!(eff.work_verify_sample_rate, 0.20);
    }
}
