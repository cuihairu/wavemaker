//! observer_baseline (WM-0410; docs/basin.md "Observation Overhead").
//!
//! "`observer_baseline` samples 60 seconds with no load; if observation
//! overhead exceeds the target threshold (the ADR currently proposes 3%,
//! measurement method to be defined), automatically degrade and record it
//! in the report."
//!
//! The measurement method is explicitly NOT settled by the docs, so this
//! model does not pick one: the caller supplies overhead samples from
//! whatever probe they measured with, and this module owns the honest
//! parts — sample sanity, the threshold evaluation, the ADR three-level
//! degradation ladder, and the report record. The ADR's 3% is carried as
//! a suggested constant, marked as a proposal, not a law.

use serde::{Deserialize, Serialize};

/// The ADR's three observation levels (docs/basin.md):
/// L0 low-overhead basics, on by default; L1 engine-specific watchers,
/// overhead measured before enabling; L2 high-overhead diagnostics in
/// limited windows only.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ObservationLevel {
    L0,
    L1,
    L2,
}

/// Doc default: the baseline samples 60 seconds with no load.
pub const BASELINE_WINDOW_MS: i64 = 60_000;

/// The ADR's proposal. The docs themselves call the measurement method
/// "to be defined" — treat this as a starting point, not a guarantee.
pub const ADR_PROPOSED_OVERHEAD_THRESHOLD: f64 = 0.03;

/// One overhead observation during the no-load window: the fraction of
/// process capacity consumed by observation itself.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct OverheadSample {
    pub at_unix_ms: i64,
    /// Fraction in [0.0, 1.0]. NaN/Infinity/negative are refused — a
    /// nonsense sample would poison the median silently.
    pub overhead_ratio: f64,
}

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum BaselineError {
    #[error("baseline needs at least one overhead sample")]
    NoSamples,
    #[error("overhead ratio {0} is not a usable fraction")]
    InvalidRatio(f64),
}

/// The no-load sample set, sanity-checked at construction.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OverheadSamples {
    samples: Vec<OverheadSample>,
}

impl OverheadSamples {
    pub fn new(samples: Vec<OverheadSample>) -> Result<Self, BaselineError> {
        if samples.is_empty() {
            return Err(BaselineError::NoSamples);
        }
        for sample in &samples {
            if !sample.overhead_ratio.is_finite() || sample.overhead_ratio < 0.0
                || sample.overhead_ratio > 1.0
            {
                return Err(BaselineError::InvalidRatio(sample.overhead_ratio));
            }
        }
        Ok(Self { samples })
    }

    pub fn samples(&self) -> &[OverheadSample] {
        &self.samples
    }

    pub fn len(&self) -> usize {
        self.samples.len()
    }

    pub fn is_empty(&self) -> bool {
        self.samples.is_empty()
    }

    fn sorted_ratios(&self) -> Vec<f64> {
        let mut ratios: Vec<f64> =
            self.samples.iter().map(|sample| sample.overhead_ratio).collect();
        ratios.sort_by(|a, b| a.partial_cmp(b).expect("ratios are finite"));
        ratios
    }

    pub fn max_ratio(&self) -> f64 {
        self.sorted_ratios().pop().unwrap_or_default()
    }

    /// Median overhead — the statistic the threshold is evaluated
    /// against (robust against one noisy probe tick).
    pub fn median_ratio(&self) -> f64 {
        let ratios = self.sorted_ratios();
        let middle = ratios.len() / 2;
        if ratios.len() % 2 == 1 {
            ratios[middle]
        } else {
            (ratios[middle - 1] + ratios[middle]) / 2.0
        }
    }
}

/// The evaluation record: stats, threshold, verdict — everything the
/// report needs ("automatically degrade and record it in the report").
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BaselineEvaluation {
    pub median_overhead_ratio: f64,
    pub max_overhead_ratio: f64,
    pub threshold_ratio: f64,
    /// True when the median strictly exceeds the threshold ("exceeds the
    /// target threshold").
    pub degraded: bool,
    /// Where the ADR ladder lands after degradation; `None` = no step
    /// taken (accepted, or already at the floor).
    pub degraded_to: Option<ObservationLevel>,
    pub sample_count: usize,
    /// Window length the baseline is supposed to cover (doc: 60s no
    /// load). The caller asserts their samples actually span it; this
    /// model records the nominal window without pretending to verify
    /// wall-clock coverage it cannot see.
    pub nominal_window_ms: i64,
}

/// Evaluate a baseline against a threshold. `current_level` is the
/// observation level whose overhead was measured.
pub fn evaluate(
    samples: &OverheadSamples,
    current_level: ObservationLevel,
    threshold_ratio: f64,
    nominal_window_ms: i64,
) -> BaselineEvaluation {
    let median = samples.median_ratio();
    let degraded = median > threshold_ratio;
    let degraded_to = if degraded {
        degrade_level(current_level)
    } else {
        None
    };
    BaselineEvaluation {
        median_overhead_ratio: median,
        max_overhead_ratio: samples.max_ratio(),
        threshold_ratio,
        degraded,
        degraded_to,
        sample_count: samples.len(),
        nominal_window_ms,
    }
}

/// One step down the ADR ladder: L2 -> L1 -> L0. L0 is the floor —
/// "low-overhead basic process/system metrics, on by default"; there is
/// nothing below it to fall back to, and `None` makes that visible
/// instead of pretending a further step happened.
pub fn degrade_level(level: ObservationLevel) -> Option<ObservationLevel> {
    match level {
        ObservationLevel::L0 => None,
        ObservationLevel::L1 => Some(ObservationLevel::L0),
        ObservationLevel::L2 => Some(ObservationLevel::L1),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(at_ms: i64, ratio: f64) -> OverheadSample {
        OverheadSample { at_unix_ms: at_ms, overhead_ratio: ratio }
    }

    #[test]
    fn window_and_threshold_carry_the_doc_numbers() {
        assert_eq!(BASELINE_WINDOW_MS, 60_000);
        assert_eq!(ADR_PROPOSED_OVERHEAD_THRESHOLD, 0.03);
    }

    #[test]
    fn accepted_baseline_records_stats() {
        let samples = OverheadSamples::new(vec![
            sample(0, 0.010),
            sample(1000, 0.012),
            sample(2000, 0.011),
        ])
        .expect("samples");
        let evaluation =
            evaluate(&samples, ObservationLevel::L1, 0.03, BASELINE_WINDOW_MS);
        assert!(!evaluation.degraded);
        assert_eq!(evaluation.degraded_to, None);
        assert_eq!(evaluation.median_overhead_ratio, 0.011);
        assert_eq!(evaluation.max_overhead_ratio, 0.012);
        assert_eq!(evaluation.sample_count, 3);
        assert_eq!(evaluation.nominal_window_ms, 60_000);
    }

    #[test]
    fn median_over_threshold_degrades() {
        let samples = OverheadSamples::new(vec![
            sample(0, 0.05),
            sample(1000, 0.06),
            sample(2000, 0.20), // max outlier must not become the verdict
        ])
        .expect("samples");
        let evaluation =
            evaluate(&samples, ObservationLevel::L1, 0.03, BASELINE_WINDOW_MS);
        assert!(evaluation.degraded);
        assert_eq!(evaluation.degraded_to, Some(ObservationLevel::L0));
        assert_eq!(evaluation.median_overhead_ratio, 0.06);
        assert_eq!(evaluation.max_overhead_ratio, 0.20);
    }

    #[test]
    fn exactly_at_threshold_does_not_degrade() {
        // "exceeds" is strict.
        let samples = OverheadSamples::new(vec![
            sample(0, 0.03),
            sample(1000, 0.03),
        ])
        .expect("samples");
        let evaluation =
            evaluate(&samples, ObservationLevel::L1, 0.03, BASELINE_WINDOW_MS);
        assert!(!evaluation.degraded);
    }

    #[test]
    fn median_of_even_counts_interpolates() {
        let samples = OverheadSamples::new(vec![
            sample(0, 0.02),
            sample(1000, 0.04),
        ])
        .expect("samples");
        assert_eq!(samples.median_ratio(), 0.03);
    }

    #[test]
    fn nonsense_samples_are_refused() {
        assert_eq!(
            OverheadSamples::new(vec![]),
            Err(BaselineError::NoSamples)
        );
        for bad in [f64::NAN, f64::INFINITY, -0.01, 1.5] {
            assert!(matches!(
                OverheadSamples::new(vec![sample(0, bad)]),
                Err(BaselineError::InvalidRatio(_))
            ));
        }
    }

    #[test]
    fn adr_ladder_steps_down_and_stops_at_the_floor() {
        assert_eq!(degrade_level(ObservationLevel::L2), Some(ObservationLevel::L1));
        assert_eq!(degrade_level(ObservationLevel::L1), Some(ObservationLevel::L0));
        assert_eq!(degrade_level(ObservationLevel::L0), None);

        // At the floor, an over-threshold median still reports degraded —
        // with no further step, so the report cannot hide it.
        let samples = OverheadSamples::new(vec![sample(0, 0.5)])
            .expect("samples");
        let evaluation =
            evaluate(&samples, ObservationLevel::L0, 0.03, BASELINE_WINDOW_MS);
        assert!(evaluation.degraded);
        assert_eq!(evaluation.degraded_to, None);
    }

    #[test]
    fn evaluation_and_levels_serialise_round_trip() {
        let samples = OverheadSamples::new(vec![
            sample(0, 0.01),
            sample(1000, 0.04),
        ])
        .expect("samples");
        let evaluation =
            evaluate(&samples, ObservationLevel::L2, 0.03, BASELINE_WINDOW_MS);
        let text = serde_yaml::to_string(&evaluation).expect("serialise");
        let back: BaselineEvaluation = serde_yaml::from_str(&text).expect("reparse");
        assert_eq!(back, evaluation);

        let level: ObservationLevel =
            serde_yaml::from_str("l0").expect("level");
        assert_eq!(level, ObservationLevel::L0);
    }
}
