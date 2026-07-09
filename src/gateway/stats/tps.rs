//! Per-request TPS samples and trimmed recent averages for global stats.

/// Recent window for global TPS display (matches hourly RPM window).
pub const RECENT_TPS_WINDOW_SECS: u64 = 3600;

#[derive(Debug, Clone, PartialEq)]
pub struct TpsSample {
    pub recorded_at_unix: u64,
    pub tier: String,
    pub tps_x1000: u64,
}

pub fn tps_from_x1000(v: u64) -> f64 {
    v as f64 / 1000.0
}

/// Trimmed mean: compute mean, drop samples at min/max TPS/mean ratio, average the rest.
pub fn trimmed_mean_tps(values: &[f64]) -> Option<f64> {
    if values.is_empty() {
        return None;
    }
    if values.len() == 1 {
        return Some(values[0]);
    }

    let mean = values.iter().sum::<f64>() / values.len() as f64;
    if mean <= f64::EPSILON {
        return Some(*values.last()?);
    }

    if values.len() == 2 {
        return Some(mean);
    }

    let indexed: Vec<(usize, f64, f64)> = values
        .iter()
        .copied()
        .enumerate()
        .map(|(i, tps)| (i, tps, tps / mean))
        .collect();

    let min_ratio_idx = indexed
        .iter()
        .min_by(|a, b| a.2.partial_cmp(&b.2).unwrap_or(std::cmp::Ordering::Equal))
        .map(|(i, _, _)| *i)?;
    let max_ratio_idx = indexed
        .iter()
        .max_by(|a, b| a.2.partial_cmp(&b.2).unwrap_or(std::cmp::Ordering::Equal))
        .map(|(i, _, _)| *i)?;

    let remaining: Vec<f64> = indexed
        .into_iter()
        .filter(|(i, _, _)| *i != min_ratio_idx && *i != max_ratio_idx)
        .map(|(_, tps, _)| tps)
        .collect();

    if remaining.is_empty() {
        return Some(*values.last()?);
    }

    Some(remaining.iter().sum::<f64>() / remaining.len() as f64)
}

/// Recent-window trimmed TPS for a tier (or all tiers when `tier` is None).
/// Falls back to the most recent matching sample when the window is empty.
pub fn recent_tps(samples: &[TpsSample], since_unix: u64, tier: Option<&str>) -> Option<f64> {
    let matches_tier = |s: &TpsSample| tier.is_none_or(|t| s.tier == t);

    let recent: Vec<f64> = samples
        .iter()
        .filter(|s| s.recorded_at_unix >= since_unix && matches_tier(s))
        .map(|s| tps_from_x1000(s.tps_x1000))
        .collect();

    if !recent.is_empty() {
        return trimmed_mean_tps(&recent);
    }

    samples
        .iter()
        .filter(|s| matches_tier(s))
        .max_by_key(|s| s.recorded_at_unix)
        .map(|s| tps_from_x1000(s.tps_x1000))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trimmed_mean_drops_ratio_outliers_and_extremes() {
        // mean = 33.25; 10 and 100 are ratio outliers; then trim 11 and 12 min/max
        let values = [10.0, 100.0, 12.0, 11.0];
        let got = trimmed_mean_tps(&values).unwrap();
        assert!((got - 11.5).abs() < 0.01);
    }

    #[test]
    fn trimmed_mean_single_value() {
        assert_eq!(trimmed_mean_tps(&[42.0]), Some(42.0));
    }

    #[test]
    fn recent_tps_uses_window_then_fallback() {
        let now = 1_700_000_000u64;
        let samples = vec![
            TpsSample {
                recorded_at_unix: now - 7200,
                tier: "edge".into(),
                tps_x1000: 20_000,
            },
            TpsSample {
                recorded_at_unix: now - 100,
                tier: "edge".into(),
                tps_x1000: 50_000,
            },
        ];
        let since = now - 3600;
        assert!((recent_tps(&samples, since, Some("edge")).unwrap() - 50.0).abs() < 0.01);

        let old_only = vec![samples[0].clone()];
        assert!((recent_tps(&old_only, since, Some("edge")).unwrap() - 20.0).abs() < 0.01);
    }
}
