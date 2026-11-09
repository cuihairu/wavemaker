//! Coordinated omission correction (WM-0318).
//!
//! When a load generator is paused (GC, scheduler stall, OS preemption) for
//! `T`, the requests that *would have* completed during the pause are omitted
//! from the histogram — yet their latencies were all at least `T`. Failing to
//! account for them understates the tail. Gil Tene's standard correction
//! (HdrHistogram `recordValueWithExpectedInterval`) fills in the omitted
//! intermediate samples: for an observed `value`, also record
//! `value - interval`, `value - 2*interval`, ... down to `interval`.

use crate::histogram::HistogramRecorder;

/// Record `value_ns` plus the samples a pause of `interval_ns` would have
/// omitted. `interval_ns == 0` disables the correction (records once).
///
/// A value no larger than the interval is recorded once (nothing was omitted
/// at that granularity).
pub fn record_with_expected_interval(rec: &mut HistogramRecorder, value_ns: u64, interval_ns: u64) {
    rec.record(value_ns);
    if interval_ns == 0 || value_ns <= interval_ns {
        return;
    }
    let mut missing = value_ns - interval_ns;
    while missing >= interval_ns {
        rec.record(missing);
        missing -= interval_ns;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fills_in_omitted_intermediate_samples() {
        let mut rec = HistogramRecorder::new().unwrap();
        record_with_expected_interval(&mut rec, 1000, 100);
        // 1000, 900, 800, ..., 100 => 10 samples.
        assert_eq!(rec.total_count(), 10);
        let s = rec.summary();
        assert_eq!(s.max_ns, rec.summary().max_ns);
        assert!(s.min_ns <= 100, "min {} should be ~100", s.min_ns);
    }

    #[test]
    fn value_below_interval_is_recorded_once() {
        let mut rec = HistogramRecorder::new().unwrap();
        record_with_expected_interval(&mut rec, 50, 100);
        assert_eq!(rec.total_count(), 1);
    }

    #[test]
    fn zero_interval_disables_correction() {
        let mut rec = HistogramRecorder::new().unwrap();
        record_with_expected_interval(&mut rec, 1000, 0);
        assert_eq!(rec.total_count(), 1);
    }

    #[test]
    fn correction_increases_tail_but_not_max() {
        // A single huge sample with a small interval must not invent a max
        // above the observed value; it only adds lower samples.
        let mut plain = HistogramRecorder::new().unwrap();
        let mut corrected = HistogramRecorder::new().unwrap();
        plain.record(5_000_000);
        record_with_expected_interval(&mut corrected, 5_000_000, 50_000);
        assert!(corrected.total_count() > plain.total_count());
        assert!(corrected.summary().max_ns >= plain.summary().max_ns);
        // The corrected max equals the observed value's highest equivalent.
        assert_eq!(corrected.summary().max_ns, plain.summary().max_ns);
    }

    #[test]
    fn correction_is_bounded_by_value() {
        let mut rec = HistogramRecorder::new().unwrap();
        record_with_expected_interval(&mut rec, 500, 75);
        // 500, 425, 350, 275, 200, 125 (>= 75), next 50 < 75 stops => 6 samples.
        assert_eq!(rec.total_count(), 6);
    }
}
