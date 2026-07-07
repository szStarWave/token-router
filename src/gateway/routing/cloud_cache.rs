/// Session-scoped cloud prompt-cache warmth: linear boost decays exponentially over time.
pub fn decay_factor(elapsed_secs: f64, half_life_secs: f64) -> f32 {
    if half_life_secs <= 0.0 {
        return 0.0;
    }
    0.5_f64.powf(elapsed_secs / half_life_secs) as f32
}

pub fn cloud_cache_linear_boost(
    anchor_unix: u64,
    peak: f32,
    now: u64,
    half_life_secs: u64,
) -> f32 {
    if peak <= f32::EPSILON {
        return 0.0;
    }
    let elapsed = now.saturating_sub(anchor_unix) as f64;
    peak * decay_factor(elapsed, half_life_secs.max(1) as f64)
}

/// Linear weight for a prior cloud route on the same request context prefix.
pub const REQ_ROUTE_CACHE_CLOUD_LINEAR: f32 = 0.12;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decay_at_half_life_is_half() {
        let f = decay_factor(600.0, 600.0);
        assert!((f - 0.5).abs() < 1e-5);
    }

    #[test]
    fn boost_smoothly_decreases() {
        let peak = 0.18;
        let b0 = cloud_cache_linear_boost(1000, peak, 1000, 600);
        let b1 = cloud_cache_linear_boost(1000, peak, 1600, 600);
        let b2 = cloud_cache_linear_boost(1000, peak, 2200, 600);
        assert!((b0 - 0.18).abs() < 1e-5);
        assert!(b1 > b2);
        assert!(b0 > b1);
    }
}
pub fn cloud_cache_extra_parts(
    anchor_unix: Option<u64>,
    peak_linear: f32,
    route_hint_route: Option<&str>,
    now: u64,
    half_life_secs: u64,
    boost_max: f32,
) -> Vec<(String, f32)> {
    let mut parts = Vec::new();
    let mut session_boost = 0.0f32;
    if let Some(anchor) = anchor_unix {
        session_boost = cloud_cache_linear_boost(anchor, peak_linear, now, half_life_secs);
        if session_boost > f32::EPSILON {
            parts.push(("CLOUD_CACHE_BOOST".to_string(), session_boost));
        }
    }
    let hash_boost = match route_hint_route {
        Some("cloud") => REQ_ROUTE_CACHE_CLOUD_LINEAR,
        _ => 0.0,
    };
    if hash_boost > f32::EPSILON {
        parts.push(("REQ_ROUTE_CACHE_CLOUD".to_string(), hash_boost));
    }
    let total = session_boost + hash_boost;
    if total > boost_max && total > f32::EPSILON {
        let scale = boost_max / total;
        for part in &mut parts {
            part.1 *= scale;
        }
    }
    parts
}