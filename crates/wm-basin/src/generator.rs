//! Generator Metrics aggregation (WM-0412; docs/generator-observability.md
//! "The Generator Itself Must Be Monitored" and the saturation rule).
//!
//! Every Worker / Supervisor / Engine Bot Process records the doc's list;
//! the aggregate across workers follows each field's semantics honestly:
//! point-in-time gauges aggregate as PEAKS, monotone counters SUM, and a
//! metric no worker could measure stays `None` — missing data must not
//! default to zero. Distribution-level latencies are NOT folded in here
//! (averaging P99 across Workers is not allowed; mergeable histograms are
//! the HDR item's territory) — this aggregate carries maxima of per-worker
//! maxima and says so.
//!
//! Saturation: "When the generator hits a resource bottleneck, the report
//! must be marked Generator Saturated; a generator bottleneck must not be
//! misread as a server-side bottleneck." The detection here produces the
//! evidence list the report label hangs on.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

/// One worker's generator-side readings at one instant. Every metric is
/// optional: a worker that cannot measure something reports absence
/// rather than a fake value.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct WorkerSnapshot {
    pub worker_id: String,
    pub at_unix_ms: i64,
    /// Fraction 0..=1.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cpu_ratio: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rss_bytes: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thread_count: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fd_count: Option<u64>,
    /// Event queue length.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub event_queue_length: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub control_channel_latency_ms: Option<f64>,
    /// Monotone counter.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dropped_event_reports: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metrics_reporting_latency_ms: Option<f64>,
    /// Deviation between planned_at and actual_at.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub planned_actual_deviation_ms: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scheduler_lag_ms: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub actor_count: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active_connections: Option<u64>,
    /// Monotone counters: restarts and lost-contact events.
    #[serde(default)]
    pub restarts: u64,
    #[serde(default)]
    pub lost_contact_events: u64,
}

/// Aggregated across workers. Field semantics:
/// - gauges -> peak across every worker snapshot,
/// - counters -> sum,
/// - latencies -> max of per-worker maxima (deliberately NOT an average;
///   distribution semantics need merged histograms, see HDR rules).
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct GeneratorAggregate {
    /// Distinct workers that reported at least one snapshot.
    pub workers_reported: usize,
    pub peak_cpu_ratio: Option<f64>,
    pub peak_rss_bytes: Option<u64>,
    pub peak_thread_count: Option<u64>,
    pub peak_fd_count: Option<u64>,
    pub peak_event_queue_length: Option<u64>,
    pub max_control_channel_latency_ms: Option<f64>,
    pub total_dropped_event_reports: Option<u64>,
    pub max_metrics_reporting_latency_ms: Option<f64>,
    pub max_planned_actual_deviation_ms: Option<f64>,
    pub max_scheduler_lag_ms: Option<f64>,
    pub peak_actor_count: Option<u64>,
    pub peak_active_connections: Option<u64>,
    pub total_restarts: u64,
    pub total_lost_contact_events: u64,
}

fn fold_gauge_u64(values: impl Iterator<Item = Option<u64>>) -> Option<u64> {
    values.flatten().max()
}

fn fold_gauge_f64(values: impl Iterator<Item = Option<f64>>) -> Option<f64> {
    values
        .flatten()
        .fold(None, |acc: Option<f64>, value| {
            // NaN readings are refused at the fold: a NaN peak would lie
            // about saturation either direction.
            if !value.is_finite() {
                return acc;
            }
            Some(match acc {
                Some(current) if current >= value => current,
                _ => value,
            })
        })
}

fn fold_counter(values: impl Iterator<Item = Option<u64>>) -> Option<u64> {
    // Sum over the workers that reported; when NOBODY reported, the sum
    // stays None — a partially-reported counter is the sum of what is
    // known, a fully-unreported one is not zero.
    let mut sum: Option<u64> = None;
    for value in values {
        match (sum, value) {
            (Some(current), Some(value)) => sum = Some(current + value),
            (Some(current), None) => sum = Some(current),
            (None, Some(value)) => sum = Some(value),
            (None, None) => {}
        }
    }
    sum
}

/// Aggregate worker snapshots. Counters are monotone, so feeding several
/// snapshots from the SAME worker double-counts them; pass the final
/// snapshot of each worker for a swell-level aggregate (per-tick
/// aggregation happens upstream on those inputs).
pub fn aggregate(snapshots: &[WorkerSnapshot]) -> GeneratorAggregate {
    let mut workers: BTreeSet<&str> = BTreeSet::new();
    for snapshot in snapshots {
        workers.insert(snapshot.worker_id.as_str());
    }
    let total_restarts = snapshots.iter().map(|s| s.restarts).sum();
    let total_lost_contact = snapshots.iter().map(|s| s.lost_contact_events).sum();
    GeneratorAggregate {
        workers_reported: workers.len(),
        peak_cpu_ratio: fold_gauge_f64(snapshots.iter().map(|s| s.cpu_ratio)),
        peak_rss_bytes: fold_gauge_u64(snapshots.iter().map(|s| s.rss_bytes)),
        peak_thread_count: fold_gauge_u64(
            snapshots.iter().map(|s| s.thread_count),
        ),
        peak_fd_count: fold_gauge_u64(snapshots.iter().map(|s| s.fd_count)),
        peak_event_queue_length: fold_gauge_u64(
            snapshots.iter().map(|s| s.event_queue_length),
        ),
        max_control_channel_latency_ms: fold_gauge_f64(
            snapshots.iter().map(|s| s.control_channel_latency_ms),
        ),
        total_dropped_event_reports: fold_counter(
            snapshots.iter().map(|s| s.dropped_event_reports),
        ),
        max_metrics_reporting_latency_ms: fold_gauge_f64(
            snapshots.iter().map(|s| s.metrics_reporting_latency_ms),
        ),
        max_planned_actual_deviation_ms: fold_gauge_f64(
            snapshots.iter().map(|s| s.planned_actual_deviation_ms),
        ),
        max_scheduler_lag_ms: fold_gauge_f64(
            snapshots.iter().map(|s| s.scheduler_lag_ms),
        ),
        peak_actor_count: fold_gauge_u64(
            snapshots.iter().map(|s| s.actor_count),
        ),
        peak_active_connections: fold_gauge_u64(
            snapshots.iter().map(|s| s.active_connections),
        ),
        total_restarts,
        total_lost_contact_events: total_lost_contact,
    }
}

/// One saturation signal: which generator resource crossed which bound.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SaturationSignal {
    pub metric: String,
    pub observed: f64,
    pub threshold: f64,
}

/// Bounds past which the GENERATOR — not the server — is the bottleneck
/// ("the report must be marked Generator Saturated").
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct SaturationThresholds {
    pub max_cpu_ratio: f64,
    pub max_event_queue_length: u64,
    pub max_scheduler_lag_ms: f64,
    pub max_planned_actual_deviation_ms: f64,
}

impl Default for SaturationThresholds {
    fn default() -> Self {
        // No authoritative numbers exist in the docs; these are
        // deliberately conservative defaults callers are expected to
        // override per environment.
        Self {
            max_cpu_ratio: 0.9,
            max_event_queue_length: 10_000,
            max_scheduler_lag_ms: 1000.0,
            max_planned_actual_deviation_ms: 500.0,
        }
    }
}

impl GeneratorAggregate {
    /// Evidence of generator saturation. Empty = no evidence; the report
    /// label (WM-0415's vocabulary) is derived from this, so a missing
    /// metric can never trigger saturation (missing != zero).
    pub fn saturation_signals(
        &self,
        thresholds: &SaturationThresholds,
    ) -> Vec<SaturationSignal> {
        let mut signals = Vec::new();
        if let Some(cpu) = self.peak_cpu_ratio {
            if cpu > thresholds.max_cpu_ratio {
                signals.push(SaturationSignal {
                    metric: "peak_cpu_ratio".into(),
                    observed: cpu,
                    threshold: thresholds.max_cpu_ratio,
                });
            }
        }
        if let Some(queue) = self.peak_event_queue_length {
            if queue > thresholds.max_event_queue_length {
                signals.push(SaturationSignal {
                    metric: "peak_event_queue_length".into(),
                    observed: queue as f64,
                    threshold: thresholds.max_event_queue_length as f64,
                });
            }
        }
        if let Some(lag) = self.max_scheduler_lag_ms {
            if lag > thresholds.max_scheduler_lag_ms {
                signals.push(SaturationSignal {
                    metric: "max_scheduler_lag_ms".into(),
                    observed: lag,
                    threshold: thresholds.max_scheduler_lag_ms,
                });
            }
        }
        if let Some(deviation) = self.max_planned_actual_deviation_ms {
            if deviation > thresholds.max_planned_actual_deviation_ms {
                signals.push(SaturationSignal {
                    metric: "max_planned_actual_deviation_ms".into(),
                    observed: deviation,
                    threshold: thresholds.max_planned_actual_deviation_ms,
                });
            }
        }
        signals
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn worker_a() -> WorkerSnapshot {
        WorkerSnapshot {
            worker_id: "worker-a".into(),
            at_unix_ms: 1000,
            cpu_ratio: Some(0.42),
            rss_bytes: Some(1_000_000),
            thread_count: Some(12),
            fd_count: Some(40),
            event_queue_length: Some(3),
            control_channel_latency_ms: Some(1.5),
            dropped_event_reports: Some(7),
            metrics_reporting_latency_ms: Some(2.0),
            planned_actual_deviation_ms: Some(12.0),
            scheduler_lag_ms: Some(3.0),
            actor_count: Some(100),
            active_connections: Some(60),
            restarts: 1,
            lost_contact_events: 2,
        }
    }

    fn worker_b() -> WorkerSnapshot {
        WorkerSnapshot {
            worker_id: "worker-b".into(),
            at_unix_ms: 1000,
            cpu_ratio: Some(0.61),
            rss_bytes: Some(1_200_000),
            // worker-b cannot measure threads/fd: absent stays absent.
            thread_count: None,
            fd_count: None,
            event_queue_length: Some(9),
            control_channel_latency_ms: Some(4.5),
            dropped_event_reports: Some(5),
            metrics_reporting_latency_ms: None,
            planned_actual_deviation_ms: Some(30.0),
            scheduler_lag_ms: None,
            actor_count: None,
            active_connections: Some(80),
            restarts: 0,
            lost_contact_events: 1,
        }
    }

    #[test]
    fn gauges_peak_counters_sum_absence_stays_absent() {
        let aggregate = aggregate(&[worker_a(), worker_b()]);
        assert_eq!(aggregate.workers_reported, 2);
        // Gauges: peaks.
        assert_eq!(aggregate.peak_cpu_ratio, Some(0.61));
        assert_eq!(aggregate.peak_rss_bytes, Some(1_200_000));
        assert_eq!(aggregate.peak_event_queue_length, Some(9));
        assert_eq!(aggregate.max_control_channel_latency_ms, Some(4.5));
        assert_eq!(aggregate.max_planned_actual_deviation_ms, Some(30.0));
        assert_eq!(aggregate.peak_active_connections, Some(80));
        // Counters: sums.
        assert_eq!(aggregate.total_dropped_event_reports, Some(12));
        assert_eq!(aggregate.total_restarts, 1);
        assert_eq!(aggregate.total_lost_contact_events, 3);
        // Absence: thread/fd measured by nobody-but-a? worker_a has them,
        // so the peak exists; scheduler lag only worker_a reported.
        assert_eq!(aggregate.max_scheduler_lag_ms, Some(3.0));
        assert_eq!(aggregate.peak_actor_count, Some(100));
    }

    #[test]
    fn metric_nobody_reported_is_none_not_zero() {
        let mut lone = worker_a();
        lone.scheduler_lag_ms = None;
        lone.dropped_event_reports = None;
        let aggregate = aggregate(&[lone]);
        assert_eq!(aggregate.max_scheduler_lag_ms, None);
        assert_eq!(aggregate.total_dropped_event_reports, None);
    }

    #[test]
    fn empty_input_aggregates_to_nothing() {
        let aggregate = aggregate(&[]);
        assert_eq!(aggregate.workers_reported, 0);
        assert_eq!(aggregate.peak_cpu_ratio, None);
        assert_eq!(aggregate.total_dropped_event_reports, None);
        assert_eq!(aggregate.total_restarts, 0);
    }

    #[test]
    fn nan_readings_do_not_poison_peaks() {
        let mut broken = worker_a();
        broken.cpu_ratio = Some(f64::NAN);
        let aggregate = aggregate(&[worker_b(), broken]);
        assert_eq!(aggregate.peak_cpu_ratio, Some(0.61));
    }

    #[test]
    fn saturation_signals_name_metric_observed_and_bound() {
        let mut hot = worker_a();
        hot.cpu_ratio = Some(0.97);
        hot.event_queue_length = Some(20_000);
        hot.scheduler_lag_ms = Some(1500.0);
        let aggregate = aggregate(&[hot]);
        let signals = aggregate.saturation_signals(&SaturationThresholds::default());
        assert_eq!(signals.len(), 3);
        assert_eq!(signals[0].metric, "peak_cpu_ratio");
        assert_eq!(signals[0].observed, 0.97);
        assert_eq!(signals[0].threshold, 0.9);
        assert_eq!(signals[1].metric, "peak_event_queue_length");
        assert_eq!(signals[2].metric, "max_scheduler_lag_ms");
    }

    #[test]
    fn clean_generator_yields_no_saturation_signals() {
        let aggregate = aggregate(&[worker_a(), worker_b()]);
        assert!(aggregate
            .saturation_signals(&SaturationThresholds::default())
            .is_empty());
    }

    #[test]
    fn absent_metric_never_triggers_saturation() {
        // worker_b cannot measure scheduler lag; with worker_a absent the
        // aggregate has None and must not saturate on it.
        let aggregate = aggregate(&[worker_b()]);
        let mut thresholds = SaturationThresholds::default();
        thresholds.max_scheduler_lag_ms = 0.0;
        assert!(aggregate.saturation_signals(&thresholds).is_empty());
    }

    #[test]
    fn snapshots_and_aggregates_serialise_round_trip() {
        let snapshots = vec![worker_a(), worker_b()];
        let text = serde_yaml::to_string(&snapshots).expect("serialise");
        assert!(text.contains("worker-b"));
        let back: Vec<WorkerSnapshot> =
            serde_yaml::from_str(&text).expect("reparse");
        assert_eq!(back, snapshots);

        let aggregate = aggregate(&snapshots);
        let text = serde_yaml::to_string(&aggregate).expect("serialise");
        let back: GeneratorAggregate = serde_yaml::from_str(&text).expect("reparse");
        assert_eq!(back, aggregate);
    }
}
