//! HDR histogram recorder (WM-0317).
//!
//! Runtime-facing latency recorder over `wm_core::hdr::Histogram`, frozen at
//! the v1 parameters (2 significant figures, ns, 1 ns .. 1 h). Worker-local
//! instances merge into a global via [`HistogramRecorder::merge`] and report
//! both a plain [`HistogramSummary`] and the canonical V2 wire blob
//! (WM-0007 conformance).

use wm_core::hdr::{HdrError, HdrParameters, Histogram};

/// Plain point-in-time summary of one histogram.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct HistogramSummary {
    pub total_count: u64,
    pub min_ns: u64,
    pub max_ns: u64,
    pub mean_ns: f64,
    pub p50_ns: u64,
    pub p95_ns: u64,
    pub p99_ns: u64,
    pub p99_9_ns: u64,
    pub p100_ns: u64,
}

/// Worker-local latency recorder over the frozen-v1 HDR histogram.
#[derive(Debug, Clone)]
pub struct HistogramRecorder {
    hist: Histogram,
    /// Highest trackable value (the frozen 1 h ceiling), used to clamp
    /// out-of-range samples instead of dropping them.
    ceiling_ns: u64,
}

impl HistogramRecorder {
    /// Recorder over `[1, 3.6e12]` ns with 2 significant figures.
    pub fn new() -> Result<Self, HdrError> {
        let params = HdrParameters::frozen_v1();
        let ceiling_ns = params.highest_trackable_value as u64;
        Ok(Self {
            hist: Histogram::new(params)?,
            ceiling_ns,
        })
    }

    /// Record one latency sample (ns). A sample above the trackable ceiling
    /// is clamped to it rather than dropped: counts and the tail stay exact
    /// (a >1 h latency is reported as ≥ the 1 h ceiling, never lost).
    pub fn record(&mut self, value_ns: u64) {
        let clamped = value_ns.min(self.ceiling_ns);
        let _ = self.hist.record_value(clamped as i64);
    }

    pub fn total_count(&self) -> u64 {
        self.hist.total_count().max(0) as u64
    }

    /// Merge another recorder's samples into this one (same frozen geometry).
    pub fn merge(&mut self, other: &HistogramRecorder) {
        let geom = self.hist.geometry();
        for (index, count) in other.hist.counts().iter().enumerate() {
            if *count > 0 {
                let value = geom.value_from_index(index);
                let _ = self.hist.record_value_count(value, *count);
            }
        }
    }

    /// Point-in-time percentile summary. Percentiles follow the Java (canonical)
    /// semantics used by `wm_core::hdr`.
    pub fn summary(&self) -> HistogramSummary {
        let hist = &self.hist;
        HistogramSummary {
            total_count: self.total_count(),
            min_ns: hist.min_value().max(0) as u64,
            max_ns: hist.max_value().max(0) as u64,
            mean_ns: hist.mean(),
            p50_ns: hist.value_at_percentile(50.0).max(0) as u64,
            p95_ns: hist.value_at_percentile(95.0).max(0) as u64,
            p99_ns: hist.value_at_percentile(99.0).max(0) as u64,
            p99_9_ns: hist.value_at_percentile(99.9).max(0) as u64,
            p100_ns: hist.value_at_percentile(100.0).max(0) as u64,
        }
    }

    /// Canonical V2 wire blob (compressed) for reporting/aggregation.
    pub fn encode_v2_blob(&self) -> Result<Vec<u8>, HdrError> {
        self.hist.encode_v2_blob()
    }

    /// Decode a V2 blob produced by [`HistogramRecorder::encode_v2_blob`].
    pub fn from_v2_blob(blob: &[u8]) -> Result<Self, HdrError> {
        let params = HdrParameters::frozen_v1();
        let ceiling_ns = params.highest_trackable_value as u64;
        Ok(Self {
            hist: Histogram::decode_v2_blob(blob)?,
            ceiling_ns,
        })
    }
}

impl Default for HistogramRecorder {
    fn default() -> Self {
        Self::new().expect("frozen-v1 parameters are valid")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn records_and_reports_percentiles() {
        let mut rec = HistogramRecorder::new().unwrap();
        for i in 1..=10_000u64 {
            rec.record(i * 1_000); // 1 ms .. 10 s spread
        }
        let s = rec.summary();
        assert_eq!(s.total_count, 10_000);
        // HDR reports the bucket's equivalent bounds, so min is at/below the
        // smallest recorded value and max at/above the largest.
        assert!(s.min_ns <= 1_000, "min {} should be <= 1_000", s.min_ns);
        assert!(
            s.max_ns >= 10_000_000,
            "max {} should be >= 10_000_000",
            s.max_ns
        );
        assert!(s.p50_ns > 0 && s.p50_ns <= s.max_ns);
        assert!(s.p95_ns >= s.p50_ns);
        assert!(s.p99_ns >= s.p95_ns);
        assert!(s.p99_9_ns >= s.p99_ns);
        assert_eq!(s.p100_ns, s.max_ns);
        assert!(s.mean_ns > 0.0);
    }

    #[test]
    fn merge_combines_counts() {
        let mut a = HistogramRecorder::new().unwrap();
        let mut b = HistogramRecorder::new().unwrap();
        for i in 0..100u64 {
            a.record(i * 1_000);
        }
        for i in 0..50u64 {
            b.record(i * 2_000);
        }
        a.merge(&b);
        assert_eq!(a.total_count(), 150);
        let sa = a.summary();
        let sb = b.summary();
        // Merged max is the max of both (in highest-equivalent terms).
        assert!(sa.max_ns >= 99_000, "merged max {}", sa.max_ns);
        assert!(sa.max_ns >= sb.max_ns, "merged max dominates each part");
    }

    #[test]
    fn v2_blob_round_trips_through_decode() {
        let mut rec = HistogramRecorder::new().unwrap();
        for i in 0..1000u64 {
            rec.record(i * 123);
        }
        let blob = rec.encode_v2_blob().unwrap();
        let decoded = HistogramRecorder::from_v2_blob(&blob).unwrap();
        assert_eq!(decoded.total_count(), rec.total_count());
        let a = rec.summary();
        let b = decoded.summary();
        assert_eq!(a.total_count, b.total_count);
        assert_eq!(a.min_ns, b.min_ns);
        assert_eq!(a.max_ns, b.max_ns);
        assert_eq!(a.p99_ns, b.p99_ns);
    }

    #[test]
    fn empty_histogram_summary_is_zero() {
        let rec = HistogramRecorder::new().unwrap();
        let s = rec.summary();
        assert_eq!(s.total_count, 0);
        assert_eq!(s.min_ns, 0);
        assert_eq!(s.max_ns, 0);
    }

    /// A sample above the 1 h ceiling must be recorded at the ceiling, not
    /// dropped: dropping it would undercount and hide the long tail.
    #[test]
    fn above_ceiling_samples_are_clamped_not_dropped() {
        let mut rec = HistogramRecorder::new().unwrap();
        rec.record(1_000);
        rec.record(4 * 3_600_000_000_000u64); // 4 h, far above the ceiling
        let s = rec.summary();
        assert_eq!(
            s.total_count, 2,
            "out-of-range sample recorded, not silently dropped"
        );
        // HDR reports the top bucket's equivalent bounds, so the clamped
        // sample lands at or above the 1 h ceiling — never below it.
        assert!(
            s.max_ns >= 3_600_000_000_000,
            "clamped to the frozen ceiling, got {}",
            s.max_ns
        );
        assert_eq!(s.p100_ns, s.max_ns);
    }
}
