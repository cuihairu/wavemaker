//! Multi-Worker metrics aggregation (WM-0510) — the controller folds
//! per-Worker metric sets into per-Backend aggregates (architecture
//! role: the Controller "aggregates data").
//!
//! Honesty rules, each structural:
//!
//! - COUNTS are additive and are summed (arrivals, rejections).
//!   Effective rates are COMPUTED from summed counts over the union
//!   window — never averaged from per-Worker rates.
//! - GAUGES (active connections) are never summed into one number:
//!   gauges taken at different moments cannot add. They stay listed
//!   per contributing Worker.
//! - PERCENTILES are never aggregated here — report-template §11.2
//!   bans cross-group percentile averaging, so this module has no API
//!   that could produce one. The per-Worker HDR histograms travel
//!   intact; only a counts-level merge (WM-0511) may ever produce an
//!   aggregate percentile, from raw counts.
//! - MISSING workers are NAMED (`MissingWorker` issue). Absence is
//!   never rendered as zeros — zeros would claim "the Worker reported
//!   zero arrivals", which is a different (and false) statement.
//! - UNBOUND workers (no backend binding) stay visible under
//!   `unbound_workers` instead of being silently absorbed into some
//!   group.
//! - WINDOW mismatches inside one Backend are recorded
//!   (`WindowMismatch`) and the union window is used — never silently
//!   assumed equal.

use std::collections::BTreeMap;

use wm_core::hdr::Histogram;

/// One Worker's reported metric set for one collection window.
#[derive(Debug, Clone)]
pub struct WorkerMetrics {
    pub worker_id: String,
    /// Backend the Worker executed for; `None` = unbound (honest
    /// absence — e.g. a Worker that ran nothing for this swell).
    pub backend_id: Option<String>,
    pub window_start_unix_ms: i64,
    pub window_end_unix_ms: i64,
    /// Monotonic arrival count over the window.
    pub arrivals_total: u64,
    /// Monotonic rejection count over the window.
    pub rejected_total: u64,
    /// GAUGE — connections active at the report moment. Never summed.
    pub active_connections: u64,
    /// Latency histogram (frozen v1 HDR parameters, WM-0007).
    pub latency_histogram: Histogram,
}

/// A named aggregation condition. Issues are DATA — they belong in
/// the report's data-quality story, never in a suppressed log line.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum AggregationIssue {
    /// An expected Worker contributed nothing. Named, with no
    /// fabricated zeros.
    MissingWorker { worker_id: String },
    /// Workers of one Backend reported different windows; the union
    /// window is used and the mismatch is visible.
    WindowMismatch {
        backend_id: String,
        workers: Vec<String>,
    },
}

/// Per-Backend aggregate: summed counts, union window, per-Worker
/// gauges and histograms intact for the counts-level merge (WM-0511).
#[derive(Debug, Clone)]
pub struct BackendAggregate {
    pub backend_id: String,
    /// Union of the contributing workers' windows.
    pub window_start_unix_ms: i64,
    pub window_end_unix_ms: i64,
    /// Sum of per-Worker counts — counts are additive.
    pub arrivals_total: u64,
    pub rejected_total: u64,
    /// Workers whose counts went into the sums, sorted by id.
    pub contributing_workers: Vec<String>,
    /// Gauge per Worker, listed — not summed (see module rules).
    pub active_connections_per_worker: Vec<(String, u64)>,
    /// Per-Worker histograms, intact — no percentile API exists here
    /// by construction (report-template §11.2).
    pub histograms_per_worker: Vec<(String, Histogram)>,
}

/// The full aggregation result.
#[derive(Debug, Clone)]
pub struct MetricsAggregation {
    /// Per-Backend aggregates, sorted by backend id.
    pub per_backend: Vec<BackendAggregate>,
    /// Workers without a backend binding, verbatim and visible.
    pub unbound_workers: Vec<WorkerMetrics>,
    pub issues: Vec<AggregationIssue>,
}

impl MetricsAggregation {
    /// Effective request rate per Backend, computed from the SUMMED
    /// arrival counts over the union window — never an average of
    /// per-Worker rates. Returns `None` for an empty or zero-length
    /// window rather than inventing a number.
    pub fn effective_request_rate_per_backend(
        &self,
    ) -> Vec<(String, Option<f64>)> {
        self.per_backend
            .iter()
            .map(|aggregate| {
                let window_ms = aggregate.window_end_unix_ms - aggregate.window_start_unix_ms;
                let rate = if window_ms > 0 {
                    Some(
                        aggregate.arrivals_total as f64 / (window_ms as f64 / 1000.0),
                    )
                } else {
                    None
                };
                (aggregate.backend_id.clone(), rate)
            })
            .collect()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum AggregationError {
    #[error("worker id must not be empty")]
    EmptyWorkerId,
    #[error("worker {0} reported an inverted window (end before start)")]
    InvertedWindow(String),
}

/// Fold received per-Worker metric sets into per-Backend aggregates.
/// `expected_workers` names every Worker that SHOULD have contributed;
/// the difference is recorded, never silently accepted.
pub fn aggregate_metrics(
    expected_workers: &[&str],
    received: Vec<WorkerMetrics>,
) -> Result<MetricsAggregation, AggregationError> {
    let mut issues = Vec::new();
    for metrics in &received {
        if metrics.worker_id.trim().is_empty() {
            return Err(AggregationError::EmptyWorkerId);
        }
        if metrics.window_end_unix_ms < metrics.window_start_unix_ms {
            return Err(AggregationError::InvertedWindow(
                metrics.worker_id.clone(),
            ));
        }
    }
    let received_ids: Vec<&str> = received
        .iter()
        .map(|metrics| metrics.worker_id.as_str())
        .collect();
    for expected in expected_workers {
        if !received_ids.contains(expected) {
            issues.push(AggregationIssue::MissingWorker {
                worker_id: (*expected).to_string(),
            });
        }
    }

    // Group by backend; unbound workers stay visible, never absorbed.
    let mut unbound = Vec::new();
    let mut grouped: BTreeMap<String, Vec<&WorkerMetrics>> = BTreeMap::new();
    for metrics in &received {
        match metrics.backend_id.clone() {
            Some(backend_id) => grouped.entry(backend_id).or_default().push(metrics),
            None => unbound.push(metrics.clone()),
        }
    }
    unbound.sort_by(|a, b| a.worker_id.cmp(&b.worker_id));

    let mut per_backend = Vec::new();
    for (backend_id, members) in grouped {
        let mut sorted: Vec<&WorkerMetrics> = members;
        sorted.sort_by(|a, b| a.worker_id.cmp(&b.worker_id));
        let distinct_windows: Vec<(i64, i64)> = {
            let mut windows: Vec<(i64, i64)> = sorted
                .iter()
                .map(|metrics| (metrics.window_start_unix_ms, metrics.window_end_unix_ms))
                .collect();
            windows.dedup();
            windows
        };
        if distinct_windows.len() > 1 {
            issues.push(AggregationIssue::WindowMismatch {
                backend_id: backend_id.clone(),
                workers: sorted
                    .iter()
                    .map(|metrics| metrics.worker_id.clone())
                    .collect(),
            });
        }
        let window_start_unix_ms = distinct_windows
            .iter()
            .map(|(start, _)| *start)
            .min()
            .expect("at least one member");
        let window_end_unix_ms = distinct_windows
            .iter()
            .map(|(_, end)| *end)
            .max()
            .expect("at least one member");
        per_backend.push(BackendAggregate {
            arrivals_total: sorted.iter().map(|m| m.arrivals_total).sum(),
            rejected_total: sorted.iter().map(|m| m.rejected_total).sum(),
            contributing_workers: sorted
                .iter()
                .map(|metrics| metrics.worker_id.clone())
                .collect(),
            active_connections_per_worker: sorted
                .iter()
                .map(|metrics| (metrics.worker_id.clone(), metrics.active_connections))
                .collect(),
            histograms_per_worker: sorted
                .iter()
                .map(|metrics| (metrics.worker_id.clone(), metrics.latency_histogram.clone()))
                .collect(),
            backend_id,
            window_start_unix_ms,
            window_end_unix_ms,
        });
    }
    Ok(MetricsAggregation {
        per_backend,
        unbound_workers: unbound,
        issues,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use wm_core::hdr::{HdrParameters, HDR_HIGHEST_TRACKABLE_VALUE, HDR_LOWEST_DISCERNIBLE_VALUE, HDR_SIG_FIGURES};

    fn histogram(values: &[i64]) -> Histogram {
        let params = HdrParameters {
            lowest_discernible_value: HDR_LOWEST_DISCERNIBLE_VALUE,
            significant_figures: HDR_SIG_FIGURES,
            highest_trackable_value: HDR_HIGHEST_TRACKABLE_VALUE,
        };
        let mut histogram = Histogram::new(params).expect("frozen v1 params");
        for value in values {
            histogram.record_value(*value).expect("in range");
        }
        histogram
    }

    fn metrics(id: &str, backend: Option<&str>, arrivals: u64) -> WorkerMetrics {
        WorkerMetrics {
            worker_id: id.into(),
            backend_id: backend.map(str::to_string),
            window_start_unix_ms: 1_700_000_000_000,
            window_end_unix_ms: 1_700_000_001_000,
            arrivals_total: arrivals,
            rejected_total: 1,
            active_connections: 10,
            latency_histogram: histogram(&[1_000_000, 2_000_000, 3_000_000]),
        }
    }

    #[test]
    fn counts_sum_rates_are_computed_and_gauges_stay_per_worker() {
        let received = vec![
            metrics("w-a", Some("backend-1"), 100),
            metrics("w-b", Some("backend-1"), 150),
            metrics("w-c", Some("backend-2"), 70),
        ];
        let aggregation =
            aggregate_metrics(&["w-a", "w-b", "w-c"], received).expect("valid input");
        assert_eq!(aggregation.issues, Vec::new());
        assert_eq!(aggregation.per_backend.len(), 2);

        let backend1 = &aggregation.per_backend[0];
        assert_eq!(backend1.backend_id, "backend-1");
        assert_eq!(backend1.arrivals_total, 250, "counts are additive");
        assert_eq!(backend1.rejected_total, 2);
        assert_eq!(backend1.contributing_workers, vec!["w-a", "w-b"]);
        // Gauges are listed per worker — never summed into one number.
        assert_eq!(
            backend1.active_connections_per_worker,
            vec![("w-a".to_string(), 10), ("w-b".to_string(), 10)]
        );

        // Rates computed from summed counts over the union window
        // (1000 ms → 250 arrivals / 1 s = 250/s), not an average of
        // per-worker rates.
        let rates = aggregation.effective_request_rate_per_backend();
        assert_eq!(
            rates,
            vec![
                ("backend-1".to_string(), Some(250.0)),
                ("backend-2".to_string(), Some(70.0)),
            ]
        );
    }

    #[test]
    fn histograms_travel_intact_and_no_aggregate_percentile_api_exists() {
        let received = vec![
            metrics("w-a", Some("backend-1"), 10),
            metrics("w-b", Some("backend-1"), 20),
        ];
        let aggregation =
            aggregate_metrics(&["w-a", "w-b"], received).expect("valid input");
        let histograms = &aggregation.per_backend[0].histograms_per_worker;
        assert_eq!(histograms.len(), 2);
        // Each worker's histogram is intact — the report's §11.2 rule
        // (no cross-group percentile averaging) is structural: this
        // module exposes no percentile function at all; percentiles
        // per worker come only from each worker's own histogram.
        for (worker_id, hist) in histograms {
            assert_eq!(hist.total_count(), 3, "{worker_id} histogram untouched");
            let p99 = hist.value_at_percentile(99.0);
            assert!(p99 > 0);
        }
    }

    #[test]
    fn missing_workers_are_named_never_rendered_as_zeros() {
        let received = vec![metrics("w-a", Some("backend-1"), 100)];
        let aggregation = aggregate_metrics(&["w-a", "w-b", "w-c"], received)
            .expect("valid input");
        assert_eq!(
            aggregation.issues,
            vec![
                AggregationIssue::MissingWorker {
                    worker_id: "w-b".into()
                },
                AggregationIssue::MissingWorker {
                    worker_id: "w-c".into()
                },
            ],
            "absence is named; zeros would claim a report that never came"
        );
        // The backend aggregate sums ONLY what actually arrived.
        assert_eq!(aggregation.per_backend[0].arrivals_total, 100);
        assert_eq!(aggregation.per_backend[0].contributing_workers, vec!["w-a"]);
    }

    #[test]
    fn unbound_workers_stay_visible_and_window_mismatch_is_recorded() {
        let mut off_window = metrics("w-b", Some("backend-1"), 50);
        off_window.window_start_unix_ms = 1_700_000_000_500;
        off_window.window_end_unix_ms = 1_700_000_001_500;
        let received = vec![
            metrics("w-a", Some("backend-1"), 100),
            off_window,
            metrics("w-free", None, 5),
        ];
        let aggregation =
            aggregate_metrics(&["w-a", "w-b", "w-free"], received).expect("valid input");
        // Window mismatch: union window used, mismatch visible.
        assert!(aggregation.issues.contains(&AggregationIssue::WindowMismatch {
            backend_id: "backend-1".into(),
            workers: vec!["w-a".into(), "w-b".into()],
        }));
        let backend1 = &aggregation.per_backend[0];
        assert_eq!(
            (backend1.window_start_unix_ms, backend1.window_end_unix_ms),
            (1_700_000_000_000, 1_700_000_001_500),
            "union window, not an invented common one"
        );
        // Unbound worker: verbatim and visible, not absorbed anywhere.
        assert_eq!(aggregation.unbound_workers.len(), 1);
        assert_eq!(aggregation.unbound_workers[0].worker_id, "w-free");
        assert_eq!(aggregation.unbound_workers[0].arrivals_total, 5);
        assert!(aggregation
            .per_backend
            .iter()
            .all(|aggregate| aggregate.backend_id != "w-free"));
    }

    #[test]
    fn invalid_inputs_are_named() {
        let mut bad = metrics("", Some("backend-1"), 1);
        bad.worker_id = "  ".into();
        assert!(matches!(
            aggregate_metrics(&[], vec![bad]),
            Err(AggregationError::EmptyWorkerId)
        ));
        let mut inverted = metrics("w-a", Some("backend-1"), 1);
        inverted.window_end_unix_ms = 0;
        assert!(matches!(
            aggregate_metrics(&[], vec![inverted]),
            Err(AggregationError::InvertedWindow(_))
        ));
    }
}
