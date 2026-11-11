//! Load timeline command (WM-0604).
//!
//! The planned-load curve reuses wm-runtime's `ArrivalPlan` — the exact
//! NHPP intensity the runtime executes (λ(t) = λ₀·e^{kt} over the ramp,
//! then hold; WM-0306) — so the chart cannot drift from the engine.
//! Active-count estimation is deliberately absent: the engine has no
//! estimator yet, and a naive λ·W curve would fabricate precision the
//! spectrum semantics warn against (§4.4).

use serde::Serialize;
use wm_core::spectrum::SpectrumConfig;
use wm_runtime::arrival::ArrivalPlan;

#[derive(Debug, PartialEq, Serialize)]
pub struct TimelinePoint {
    pub t_secs: f64,
    pub rate_per_second: f64,
    pub expected_arrivals: f64,
}

#[derive(Debug, PartialEq, Serialize)]
pub struct LoadTimeline {
    pub ramp_duration_secs: f64,
    pub horizon_secs: f64,
    pub points: Vec<TimelinePoint>,
}

const SAMPLES: usize = 120;
/// Display window: the ramp plus one ramp-length of hold. The model
/// holds the end rate indefinitely; the chart must pick a window.
const HOLD_MULTIPLE: f64 = 1.0;
/// A constant-rate plan has no ramp; show one hour.
const CONSTANT_HORIZON_SECS: f64 = 3600.0;

#[tauri::command]
pub fn load_timeline(content: String) -> Result<LoadTimeline, String> {
    let cfg: SpectrumConfig =
        serde_yaml::from_str(&content).map_err(|e| format!("cannot parse spectrum: {e}"))?;
    let ramp = &cfg
        .arrival
        .ok_or("cannot build timeline: spectrum has no arrival section")?
        .ramp;
    // Mirrors ArrivalPlan's own constant rule (duration 0 or equal rates).
    let constant =
        ramp.duration_secs <= 0.0 || ramp.from_per_second == ramp.to_per_second;
    let plan = ArrivalPlan::new(
        ramp.from_per_second,
        ramp.to_per_second,
        (ramp.duration_secs * 1e9) as u64,
    )
    .map_err(|e| format!("cannot build timeline: {e}"))?;
    let horizon_secs = if constant {
        CONSTANT_HORIZON_SECS
    } else {
        ramp.duration_secs * (1.0 + HOLD_MULTIPLE)
    };
    let points = (0..=SAMPLES)
        .map(|i| {
            let t_secs = horizon_secs * i as f64 / SAMPLES as f64;
            let t_ns = (t_secs * 1e9) as u64;
            TimelinePoint {
                t_secs,
                rate_per_second: plan.rate_at(t_ns),
                expected_arrivals: plan.cumulative(t_ns),
            }
        })
        .collect();
    Ok(LoadTimeline {
        ramp_duration_secs: ramp.duration_secs,
        horizon_secs,
        points,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reference_yaml() -> String {
        "arrival:\n  ramp:\n    from_per_second: 1\n    to_per_second: 20\n    duration_secs: 600\nlifetime:\n  distribution: fixed\n  mean_secs: 900\n".into()
    }

    #[test]
    fn reference_timeline_shows_ramp_plus_hold() {
        let tl = load_timeline(reference_yaml()).unwrap();
        assert_eq!(tl.points.len(), SAMPLES + 1);
        assert!((tl.horizon_secs - 1200.0).abs() < 1e-9);
        let first = tl.points.first().unwrap();
        let last = tl.points.last().unwrap();
        assert!((first.rate_per_second - 1.0).abs() < 1e-9);
        // 1200s is 600s into the hold: rate pinned at the end rate.
        assert!((last.rate_per_second - 20.0).abs() < 1e-9);
        // Rate at the ramp end equals the end rate.
        let mid = &tl.points[SAMPLES / 2];
        assert!((mid.rate_per_second - 20.0).abs() < 1e-9);
        assert!((mid.t_secs - 600.0).abs() < 1e-9);
        // Cumulative arrivals grow monotonically.
        let mut prev = 0.0;
        for p in &tl.points {
            assert!(p.expected_arrivals >= prev);
            prev = p.expected_arrivals;
        }
    }

    #[test]
    fn constant_rate_uses_the_one_hour_window() {
        let yaml = "arrival:\n  ramp:\n    from_per_second: 5\n    to_per_second: 5\n    duration_secs: 600\n".into();
        let tl = load_timeline(yaml).unwrap();
        assert!((tl.horizon_secs - 3600.0).abs() < 1e-9);
        assert!(tl
            .points
            .iter()
            .all(|p| (p.rate_per_second - 5.0).abs() < 1e-9));
    }

    #[test]
    fn missing_arrival_is_a_named_error() {
        let err =
            load_timeline("lifetime:\n  distribution: fixed\n  mean_secs: 900\n".into())
                .unwrap_err();
        assert!(err.contains("no arrival section"));
    }

    #[test]
    fn parse_failure_is_a_named_error() {
        let err = load_timeline("arrival: [unclosed".into()).unwrap_err();
        assert!(err.starts_with("cannot parse spectrum"));
    }

    #[test]
    fn invalid_rates_do_not_panic() {
        let yaml = "arrival:\n  ramp:\n    from_per_second: -1\n    to_per_second: 20\n    duration_secs: 600\n".into();
        assert!(load_timeline(yaml).unwrap_err().contains("cannot build timeline"));
    }
}
