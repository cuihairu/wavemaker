//! Observation overhead evaluation and degradation (WM-0411;
//! docs/basin.md "Observation Overhead").
//!
//! WM-0410 owns the baseline sampling record; this module owns the POLICY
//! that hangs off it, straight from the doc's sentences:
//!
//! - "L0: low-overhead basic process/system metrics, on by default." —
//!   always allowed.
//! - "L1: watchers or engine-specific monitoring; overhead measured
//!   before enabling." — a clean baseline is an enabling precondition.
//! - "L2: high-overhead diagnostics ... enabled only in limited time
//!   windows." — enabled only with a clean baseline AND a bounded grant.
//! - "if observation overhead exceeds the target threshold, automatically
//!   degrade and record it in the report" — the mid-run watch steps the
//!   level down and produces the reportable event.
//!
//! The KBEngine/BigWorld watcher caveat (call frequencies verified per
//! version, "not as permanent assumptions") is structural: nothing here
//! encodes watcher costs — adapters must supply measured baselines.

use serde::{Deserialize, Serialize};

use crate::baseline::{
    evaluate, BaselineEvaluation, ObservationLevel, OverheadSamples,
    ADR_PROPOSED_OVERHEAD_THRESHOLD,
};

/// The overhead policy: thresholds and the enablement rules the doc
/// states.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OverheadPolicy {
    /// The target threshold (fraction). Defaults to the ADR's proposed
    /// 3% — a proposal, per the docs.
    #[serde(default = "default_threshold")]
    pub threshold_ratio: f64,
    /// "L2 ... enabled only in limited time windows" — the window bound,
    /// in milliseconds.
    #[serde(default = "default_l2_window")]
    pub l2_max_window_ms: i64,
}

fn default_threshold() -> f64 {
    ADR_PROPOSED_OVERHEAD_THRESHOLD
}

fn default_l2_window() -> i64 {
    // No number exists in the docs for the L2 window; the bound must be
    // set by configuration, so the default is deliberately finite but
    // flagged here as a placeholder callers should override.
    30_000
}

impl Default for OverheadPolicy {
    fn default() -> Self {
        Self {
            threshold_ratio: default_threshold(),
            l2_max_window_ms: default_l2_window(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "decision", rename_all = "snake_case")]
pub enum EnablementDecision {
    Allow,
    Deny { reason: String },
}

/// Whether an observation level may be enabled, given the latest baseline
/// evaluation ("overhead measured before enabling"). The CURRENT policy
/// threshold decides — the baseline may have been evaluated under a
/// different threshold value.
pub fn can_enable(
    policy: &OverheadPolicy,
    level: ObservationLevel,
    baseline: Option<&BaselineEvaluation>,
) -> EnablementDecision {
    match level {
        ObservationLevel::L0 => EnablementDecision::Allow,
        ObservationLevel::L1 | ObservationLevel::L2 => match baseline {
            None => EnablementDecision::Deny {
                reason: format!("{level:?} requires a measured overhead baseline before enabling"),
            },
            Some(evaluation) if evaluation.median_overhead_ratio > policy.threshold_ratio => {
                EnablementDecision::Deny {
                    reason: format!(
                        "baseline overhead {:.4} exceeds policy threshold {:.4}",
                        evaluation.median_overhead_ratio, policy.threshold_ratio
                    ),
                }
            }
            Some(_) => EnablementDecision::Allow,
        },
    }
}

/// A bounded L2 enablement: valid from `start_unix_ms` until
/// start + policy window. The expiry is data, checked by the caller's
/// clock.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct L2Grant {
    pub start_unix_ms: i64,
    pub expires_unix_ms: i64,
}

/// Grant an L2 window against the policy bounds.
pub fn grant_l2(policy: &OverheadPolicy, start_unix_ms: i64) -> L2Grant {
    L2Grant {
        start_unix_ms,
        expires_unix_ms: start_unix_ms + policy.l2_max_window_ms,
    }
}

impl L2Grant {
    pub fn covers(&self, at_unix_ms: i64) -> bool {
        at_unix_ms >= self.start_unix_ms && at_unix_ms < self.expires_unix_ms
    }
}

/// A mid-run degradation event ("automatically degrade and record it in
/// the report").
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DegradationEvent {
    pub from_level: ObservationLevel,
    /// The level after the step; `None` = the floor L0 was already in
    /// effect — the over-threshold fact is recorded even though no
    /// further step exists.
    pub to_level: Option<ObservationLevel>,
    pub median_overhead_ratio: f64,
    pub threshold_ratio: f64,
    pub at_unix_ms: i64,
}

/// Feed a mid-run overhead sample set; if its median exceeds the policy
/// threshold, step one level down and return the reportable event.
/// Under-threshold observations produce `None` (no event, no noise).
pub fn observe_overhead(
    policy: &OverheadPolicy,
    current_level: ObservationLevel,
    samples: &OverheadSamples,
    at_unix_ms: i64,
) -> Option<DegradationEvent> {
    let evaluation = evaluate(
        samples,
        current_level,
        policy.threshold_ratio,
        0, // mid-run watch evaluates samples as given; the nominal 60s
           // window belongs to the baseline, not the watch
    );
    if !evaluation.degraded {
        return None;
    }
    Some(DegradationEvent {
        from_level: current_level,
        to_level: evaluation.degraded_to,
        median_overhead_ratio: evaluation.median_overhead_ratio,
        threshold_ratio: policy.threshold_ratio,
        at_unix_ms,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::baseline::OverheadSample;

    fn samples(ratios: &[f64]) -> OverheadSamples {
        OverheadSamples::new(
            ratios
                .iter()
                .enumerate()
                .map(|(index, ratio)| OverheadSample {
                    at_unix_ms: index as i64 * 1000,
                    overhead_ratio: *ratio,
                })
                .collect(),
        )
        .expect("samples")
    }

    fn evaluation(median: f64) -> BaselineEvaluation {
        evaluate(
            &samples(&[median]),
            ObservationLevel::L1,
            ADR_PROPOSED_OVERHEAD_THRESHOLD,
            60_000,
        )
    }

    #[test]
    fn l0_is_on_by_default_no_baseline_needed() {
        let policy = OverheadPolicy::default();
        assert_eq!(
            can_enable(&policy, ObservationLevel::L0, None),
            EnablementDecision::Allow
        );
    }

    #[test]
    fn l1_requires_a_measured_clean_baseline() {
        let policy = OverheadPolicy::default();
        // Nothing measured yet.
        assert!(matches!(
            can_enable(&policy, ObservationLevel::L1, None),
            EnablementDecision::Deny { .. }
        ));
        // Measured but over threshold.
        let degraded = evaluation(0.05);
        assert!(degraded.degraded);
        let reason = match can_enable(&policy, ObservationLevel::L1, Some(&degraded)) {
            EnablementDecision::Deny { reason } => reason,
            other => panic!("expected denial, got {other:?}"),
        };
        assert!(
            reason.contains("exceeds policy threshold"),
            "reason: {reason}"
        );
        // Measured and clean.
        assert_eq!(
            can_enable(&policy, ObservationLevel::L1, Some(&evaluation(0.01))),
            EnablementDecision::Allow
        );
    }

    #[test]
    fn l2_needs_clean_baseline_and_bounded_windows() {
        let policy = OverheadPolicy::default();
        assert!(matches!(
            can_enable(&policy, ObservationLevel::L2, None),
            EnablementDecision::Deny { .. }
        ));
        assert_eq!(
            can_enable(&policy, ObservationLevel::L2, Some(&evaluation(0.02))),
            EnablementDecision::Allow
        );

        let grant = grant_l2(&policy, 100_000);
        assert_eq!(grant.expires_unix_ms, 130_000);
        assert!(grant.covers(100_000));
        assert!(grant.covers(129_999));
        assert!(!grant.covers(130_000));
        assert!(!grant.covers(99_999));
    }

    #[test]
    fn policy_defaults_carry_the_adr_proposal() {
        let policy = OverheadPolicy::default();
        assert_eq!(policy.threshold_ratio, ADR_PROPOSED_OVERHEAD_THRESHOLD);
        assert!(policy.l2_max_window_ms > 0);
    }

    #[test]
    fn mid_run_over_threshold_degrades_one_step_and_records() {
        let policy = OverheadPolicy::default();
        let hot = samples(&[0.04, 0.05, 0.04]);
        let event = observe_overhead(&policy, ObservationLevel::L1, &hot, 500_000)
            .expect("degradation expected");
        assert_eq!(event.from_level, ObservationLevel::L1);
        assert_eq!(event.to_level, Some(ObservationLevel::L0));
        assert_eq!(event.median_overhead_ratio, 0.04);
        assert_eq!(event.threshold_ratio, 0.03);
        assert_eq!(event.at_unix_ms, 500_000);
    }

    #[test]
    fn mid_run_under_threshold_is_silent() {
        let policy = OverheadPolicy::default();
        let cool = samples(&[0.01, 0.02, 0.01]);
        assert_eq!(
            observe_overhead(&policy, ObservationLevel::L2, &cool, 0),
            None
        );
    }

    #[test]
    fn floor_degradation_records_the_fact_without_a_step() {
        let policy = OverheadPolicy::default();
        let hot = samples(&[0.5]);
        let event =
            observe_overhead(&policy, ObservationLevel::L0, &hot, 0).expect("degraded at floor");
        assert_eq!(event.from_level, ObservationLevel::L0);
        assert_eq!(event.to_level, None);
    }

    #[test]
    fn decisions_and_events_serialise_round_trip() {
        let policy = OverheadPolicy::default();
        let text = serde_yaml::to_string(&policy).expect("serialise");
        let back: OverheadPolicy = serde_yaml::from_str(&text).expect("reparse");
        assert_eq!(back, policy);

        let event = DegradationEvent {
            from_level: ObservationLevel::L2,
            to_level: Some(ObservationLevel::L1),
            median_overhead_ratio: 0.04,
            threshold_ratio: 0.03,
            at_unix_ms: 1,
        };
        let text = serde_yaml::to_string(&event).expect("serialise");
        assert!(text.contains("l1"));
        let back: DegradationEvent = serde_yaml::from_str(&text).expect("reparse");
        assert_eq!(back, event);
    }
}
