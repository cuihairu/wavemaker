//! Timeline alignment (WM-0414; docs/basin.md Timestamp Model, Phase 4
//! goal "bring Generator, Server, and planned load onto a single Swell
//! timeline").
//!
//! "Do not treat absolute NTP synchronization as the only premise. Load
//! curves and event shapes can assist alignment, but shape alignment does
//! not replace real timestamps and must not be used to fabricate time
//! precision."
//!
//! Structural honesty here: every source clock carries its trust; a shape
//! alignment produces a ClockOffsetEstimate with method
//! "shape-alignment" and can never upgrade a source to trusted stamps;
//! a source with no estimate and no trusted stamps passes timestamps
//! through UNSHIFTED with that fact visible — never silently shifted on
//! a guess.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::timestamps::ClockOffsetEstimate;

/// What a source's timestamps are worth on the swell timeline.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "trust", rename_all = "snake_case")]
pub enum ClockTrust {
    /// Source stamps are treated as timeline truth (e.g. a stamped
    /// shared anchor; NOT a blanket NTP assumption — the declaring
    /// configuration carries the evidence).
    Trusted,
    /// Stamps shifted by an ESTIMATE from the named method (e.g.
    /// "shape-alignment"); the estimate travels with every stamp.
    OffsetEstimated,
    /// No usable alignment evidence: stamps pass through unshifted and
    /// the record says so.
    Unverified,
}

/// One data source's alignment onto the swell timeline.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SourceClock {
    pub source: String,
    /// Declared evidence that this source's stamps are timeline truth
    /// as-is (NOT a blanket NTP assumption — the declaring
    /// configuration carries the evidence).
    #[serde(default)]
    pub trusted_stamps: bool,
    /// Offset applied to this source's stamps to land on the timeline
    /// (estimate + provenance). Absent = no shift.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub offset: Option<ClockOffsetEstimate>,
}

impl SourceClock {
    pub fn trusted(source: impl Into<String>) -> Self {
        Self { source: source.into(), trusted_stamps: true, offset: None }
    }

    pub fn estimated(
        source: impl Into<String>,
        offset: ClockOffsetEstimate,
    ) -> Self {
        Self { source: source.into(), trusted_stamps: false, offset: Some(offset) }
    }

    pub fn unverified(source: impl Into<String>) -> Self {
        Self { source: source.into(), trusted_stamps: false, offset: None }
    }

    pub fn trust(&self) -> ClockTrust {
        if self.offset.is_some() {
            ClockTrust::OffsetEstimated
        } else if self.trusted_stamps {
            ClockTrust::Trusted
        } else {
            ClockTrust::Unverified
        }
    }
}

/// A stamp landed on the swell timeline.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct AlignedStamp {
    /// The stamp on the swell timeline (source stamp + offset, or the
    /// source stamp itself when unshifted).
    pub timeline_unix_ms: i64,
    /// The original source stamp.
    pub source_unix_ms: i64,
    /// Whether an offset shift was applied — false means the value IS
    /// the raw source stamp, whatever its clock believed.
    pub shift_applied: bool,
    pub trust: ClockTrust,
}

/// The swell timeline: an anchor and the clocks of the sources aligned
/// onto it (generator, server, planned load).
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct TimelineAlignment {
    /// Name of the anchor event the timeline origin is (e.g. the swell
    /// start command as observed by the controller).
    pub anchor: String,
    pub anchor_origin_unix_ms: i64,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub clocks: BTreeMap<String, SourceClock>,
}

impl TimelineAlignment {
    pub fn new(anchor: impl Into<String>, anchor_origin_unix_ms: i64) -> Self {
        Self {
            anchor: anchor.into(),
            anchor_origin_unix_ms,
            clocks: BTreeMap::new(),
        }
    }

    pub fn add_clock(&mut self, clock: SourceClock) {
        self.clocks.insert(clock.source.clone(), clock);
    }

    /// Land a source stamp on the timeline. Unknown sources pass through
    /// unshifted with `Unverified` — an unregistered clock is never
    /// silently shifted by someone else's offset.
    pub fn to_timeline(&self, source: &str, unix_ms: i64) -> AlignedStamp {
        let offset = self
            .clocks
            .get(source)
            .and_then(|clock| clock.offset.as_ref());
        match offset {
            Some(estimate) => AlignedStamp {
                timeline_unix_ms: unix_ms + estimate.offset_ms,
                source_unix_ms: unix_ms,
                shift_applied: true,
                trust: ClockTrust::OffsetEstimated,
            },
            None => {
                let trust = self
                    .clocks
                    .get(source)
                    .map(|clock| clock.trust())
                    .unwrap_or(ClockTrust::Unverified);
                AlignedStamp {
                    timeline_unix_ms: unix_ms,
                    source_unix_ms: unix_ms,
                    shift_applied: false,
                    trust,
                }
            }
        }
    }
}

/// Interval on the timeline where data exists.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct TimelineSpan {
    pub start_unix_ms: i64,
    pub end_unix_ms: i64,
}

impl TimelineSpan {
    pub fn contains(&self, unix_ms: i64) -> bool {
        unix_ms >= self.start_unix_ms && unix_ms < self.end_unix_ms
    }
}

/// The window where ALL spans overlap — the only range a unified
/// Generator/Server/planned-load timeline can honestly cover.
pub fn common_window(spans: &[TimelineSpan]) -> Option<TimelineSpan> {
    let mut start = i64::MIN;
    let mut end = i64::MAX;
    for span in spans {
        start = start.max(span.start_unix_ms);
        end = end.min(span.end_unix_ms);
    }
    if start < end {
        Some(TimelineSpan { start_unix_ms: start, end_unix_ms: end })
    } else {
        None
    }
}

/// Cross-correlate two evenly sampled curves (same step, same length)
/// and return the lag that best lines them up, as a ClockOffsetEstimate.
/// This is the doc's "load curves and event shapes can assist alignment"
/// — the result is typed as an ESTIMATE with method "shape-alignment"
/// and confers no trusted-stamp status.
///
/// Positive lag means the source curve matches the anchor delayed by
/// `lag` steps (the source's clock stamps events early); the returned
/// offset shifts source stamps onto anchor truth when added. Ties
/// resolve to the smaller |lag| (candidates visited nearest-zero
/// outward, strictly-better scores win — deterministic). Returns None
/// when the inputs are empty, length-mismatched, or the search range is
/// degenerate.
pub fn shape_alignment_offset(
    anchor_curve: &[f64],
    source_curve: &[f64],
    step_ms: i64,
    max_lag_steps: usize,
) -> Option<ClockOffsetEstimate> {
    if anchor_curve.is_empty()
        || source_curve.is_empty()
        || anchor_curve.len() != source_curve.len()
        || step_ms <= 0
        || max_lag_steps == 0
    {
        return None;
    }
    let len = anchor_curve.len();
    let mut candidates: Vec<i64> =
        (-(max_lag_steps as i64)..=(max_lag_steps as i64)).collect();
    candidates.sort_by_key(|lag| lag.unsigned_abs());
    let mut winner = candidates[0];
    let mut winner_score = f64::NEG_INFINITY;
    for lag in candidates {
        let start = lag.unsigned_abs() as usize;
        if start >= len {
            continue;
        }
        let overlap = len - start;
        let mut score = 0.0;
        for index in 0..overlap {
            // Positive lag: source[i] ~ anchor[i + lag].
            // Negative lag -k: source[i] ~ anchor[i - k], i.e. compare
            // anchor[j] with source[j + k].
            let (anchor_value, source_value) = if lag >= 0 {
                (anchor_curve[index + start], source_curve[index])
            } else {
                (anchor_curve[index], source_curve[index + start])
            };
            let difference = anchor_value - source_value;
            score -= difference * difference;
        }
        if score > winner_score {
            winner_score = score;
            winner = lag;
        }
    }
    Some(ClockOffsetEstimate {
        offset_ms: winner * step_ms,
        method: "shape-alignment".into(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trusted_and_estimated_and_unverified_shift_differently() {
        let mut alignment =
            TimelineAlignment::new("swell-start", 1_000_000);
        alignment.add_clock(SourceClock::trusted("controller"));
        alignment.add_clock(SourceClock::estimated(
            "prometheus",
            ClockOffsetEstimate {
                offset_ms: 30,
                method: "probe-roundtrip".into(),
            },
        ));
        alignment.add_clock(SourceClock::unverified("logfile"));

        let trusted = alignment.to_timeline("controller", 1_000_100);
        assert_eq!(trusted.timeline_unix_ms, 1_000_100);
        assert!(!trusted.shift_applied);
        assert_eq!(trusted.trust, ClockTrust::Trusted);

        let estimated = alignment.to_timeline("prometheus", 1_000_100);
        assert_eq!(estimated.timeline_unix_ms, 1_000_130);
        assert!(estimated.shift_applied);
        assert_eq!(estimated.trust, ClockTrust::OffsetEstimated);

        let unverified = alignment.to_timeline("logfile", 1_000_100);
        assert_eq!(unverified.timeline_unix_ms, 1_000_100);
        assert!(!unverified.shift_applied);
        assert_eq!(unverified.trust, ClockTrust::Unverified);

        // Unknown source: passthrough, unverified, no borrowed offset.
        let unknown = alignment.to_timeline("mystery", 1_000_100);
        assert_eq!(unknown.timeline_unix_ms, 1_000_100);
        assert!(!unknown.shift_applied);
        assert_eq!(unknown.trust, ClockTrust::Unverified);
    }

    #[test]
    fn shape_estimate_can_never_look_trusted() {
        let mut alignment = TimelineAlignment::new("swell-start", 0);
        alignment.add_clock(SourceClock::estimated(
            "bots",
            ClockOffsetEstimate {
                offset_ms: 500,
                method: "shape-alignment".into(),
            },
        ));
        let stamp = alignment.to_timeline("bots", 1000);
        assert_eq!(stamp.trust, ClockTrust::OffsetEstimated);
        assert_ne!(stamp.trust, ClockTrust::Trusted);
    }

    #[test]
    fn common_window_is_the_intersection() {
        let spans = vec![
            TimelineSpan { start_unix_ms: 1000, end_unix_ms: 5000 },
            TimelineSpan { start_unix_ms: 2000, end_unix_ms: 4000 },
            TimelineSpan { start_unix_ms: 2500, end_unix_ms: 9000 },
        ];
        let window = common_window(&spans).expect("overlap exists");
        assert_eq!(window.start_unix_ms, 2500);
        assert_eq!(window.end_unix_ms, 4000);
        assert!(window.contains(3000));
        assert!(!window.contains(4000));

        // No overlap -> no honest unified window.
        let disjoint = vec![
            TimelineSpan { start_unix_ms: 0, end_unix_ms: 100 },
            TimelineSpan { start_unix_ms: 500, end_unix_ms: 900 },
        ];
        assert!(common_window(&disjoint).is_none());
        // Touching-but-empty (end == start) is no window.
        let touching = vec![
            TimelineSpan { start_unix_ms: 0, end_unix_ms: 100 },
            TimelineSpan { start_unix_ms: 100, end_unix_ms: 200 },
        ];
        assert!(common_window(&touching).is_none());
    }

    #[test]
    fn shape_alignment_recovers_an_injected_lag() {
        // Anchor: gaussian-ish bump at index 20; source: the same bump
        // shifted 5 steps later (its clock ran 5*step behind).
        let step_ms = 1000;
        let bump = |center: f64| -> Vec<f64> {
            (0..64)
                .map(|index| {
                    let x = (index as f64 - center) / 3.0;
                    (-x * x).exp()
                })
                .collect()
        };
        let anchor = bump(20.0);
        let source = bump(15.0); // source's stamp of the bump is 5 steps early
        let estimate =
            shape_alignment_offset(&anchor, &source, step_ms, 10)
                .expect("estimate");
        assert_eq!(estimate.method, "shape-alignment");
        assert_eq!(estimate.offset_ms, 5 * step_ms);
    }

    #[test]
    fn shape_alignment_is_deterministic_on_flat_curves() {
        let flat = vec![0.0; 16];
        // Two flat curves: no information; whatever comes back must be
        // stable across calls (deterministic tie-break).
        let first =
            shape_alignment_offset(&flat, &flat, 1000, 3).expect("estimate");
        let second =
            shape_alignment_offset(&flat, &flat, 1000, 3).expect("estimate");
        assert_eq!(first, second);
    }

    #[test]
    fn degenerate_shape_inputs_return_none() {
        let a = vec![1.0, 2.0];
        let b = vec![1.0, 2.0, 3.0];
        assert!(shape_alignment_offset(&a, &b, 1000, 2).is_none());
        assert!(shape_alignment_offset(&a, &a, 0, 2).is_none());
        assert!(shape_alignment_offset(&a, &a, 1000, 0).is_none());
        assert!(shape_alignment_offset(&[], &[], 1000, 2).is_none());
    }

    #[test]
    fn alignment_and_stamps_serialise_round_trip() {
        let mut alignment = TimelineAlignment::new("swell-start", 42);
        alignment.add_clock(SourceClock::estimated(
            "prometheus",
            ClockOffsetEstimate {
                offset_ms: -10,
                method: "shape-alignment".into(),
            },
        ));
        let text = serde_yaml::to_string(&alignment).expect("serialise");
        assert!(text.contains("shape-alignment"));
        let back: TimelineAlignment =
            serde_yaml::from_str(&text).expect("reparse");
        assert_eq!(back, alignment);

        let stamp = alignment.to_timeline("prometheus", 1000);
        let text = serde_yaml::to_string(&stamp).expect("serialise");
        let back: AlignedStamp = serde_yaml::from_str(&text).expect("reparse");
        assert_eq!(back, stamp);
    }
}
