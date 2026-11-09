//! Missing-data and delay detection (WM-0408; docs/basin.md Phase 4 exit
//! criterion "Missing data is not treated as zero", Timestamp Model).
//!
//! The detector scans a collection batch (WM-0406 `CollectedQuery` records)
//! and produces explicit findings: windows that produced nothing (failed
//! query or EMPTY result — an empty result is a gap, never a zero) and
//! samples whose raw source age exceeds a threshold. Delay thresholds
//! operate on `source_age_ms` (WM-0407) — a raw age by our clock that
//! conflates latency with unknown clock offset; findings carry the raw
//! number and say so, they do not dress it up as true latency.

use serde::{Deserialize, Serialize};

use crate::provider::QueryWindow;
use crate::scheduler::{CollectedOutcome, CollectedQuery};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "finding", rename_all = "snake_case")]
pub enum QualityFinding {
    /// A window that should have data produced none. `reason` keeps the
    /// provider error verbatim or names the empty-result case.
    MissingWindow {
        catalog_metric: String,
        service: String,
        window: QueryWindow,
        reason: String,
    },
    /// A sample whose raw source age exceeded the threshold. The age is
    /// by OUR clock (offset unknown — see WM-0407).
    DelayedSample {
        catalog_metric: String,
        service: String,
        source_age_ms: i64,
        threshold_ms: i64,
        sample_timestamp_unix_ms: i64,
        collected_at_unix_ms: i64,
    },
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct QualityReport {
    pub findings: Vec<QualityFinding>,
    /// Number of collection records inspected.
    pub inspected: usize,
}

impl QualityReport {
    pub fn missing_windows(&self) -> usize {
        self.findings
            .iter()
            .filter(|finding| {
                matches!(finding, QualityFinding::MissingWindow { .. })
            })
            .count()
    }

    pub fn delayed_samples(&self) -> usize {
        self.findings
            .iter()
            .filter(|finding| matches!(finding, QualityFinding::DelayedSample { .. }))
            .count()
    }

    /// Clean only means "nothing detected", not "data guaranteed good".
    pub fn is_clean(&self) -> bool {
        self.findings.is_empty()
    }
}

/// Windows with no usable data: failed queries keep the provider error
/// verbatim; success with zero series or series without samples is a gap
/// named as such. No zero substitution happens anywhere on this path.
pub fn detect_missing_windows(queries: &[CollectedQuery]) -> Vec<QualityFinding> {
    let mut findings = Vec::new();
    for query in queries {
        match &query.outcome {
            CollectedOutcome::Failed(error) => findings.push(
                QualityFinding::MissingWindow {
                    catalog_metric: query.catalog_metric.clone(),
                    service: query.service.clone(),
                    window: query.window,
                    reason: error.clone(),
                },
            ),
            CollectedOutcome::Series(series) => {
                let empty = series.is_empty()
                    || series.iter().all(|entry| entry.samples.is_empty());
                if empty {
                    findings.push(QualityFinding::MissingWindow {
                        catalog_metric: query.catalog_metric.clone(),
                        service: query.service.clone(),
                        window: query.window,
                        reason: "empty result set".to_string(),
                    });
                }
            }
        }
    }
    findings
}

/// Samples whose raw source age (issued_at - source stamp) exceeds the
/// threshold. A missing source stamp cannot be checked for delay — it is
/// reported as a missing window instead by `detect_missing_windows` only
/// if the whole window is empty; otherwise the timestamp model keeps its
/// absence visible downstream.
pub fn detect_delayed_samples(
    queries: &[CollectedQuery],
    threshold_ms: i64,
) -> Vec<QualityFinding> {
    let mut findings = Vec::new();
    if threshold_ms <= 0 {
        return findings;
    }
    for query in queries {
        let CollectedOutcome::Series(series) = &query.outcome else {
            continue;
        };
        for entry in series {
            for sample in &entry.samples {
                let age = query.issued_at_unix_ms - sample.timestamp_unix_ms;
                if age > threshold_ms {
                    findings.push(QualityFinding::DelayedSample {
                        catalog_metric: query.catalog_metric.clone(),
                        service: query.service.clone(),
                        source_age_ms: age,
                        threshold_ms,
                        sample_timestamp_unix_ms: sample.timestamp_unix_ms,
                        collected_at_unix_ms: query.issued_at_unix_ms,
                    });
                }
            }
        }
    }
    findings
}

/// Full pass over a batch: missing windows first, then delayed samples.
pub fn quality_report(
    queries: &[CollectedQuery],
    delay_threshold_ms: i64,
) -> QualityReport {
    let mut findings = detect_missing_windows(queries);
    findings.extend(detect_delayed_samples(queries, delay_threshold_ms));
    QualityReport { findings, inspected: queries.len() }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provider::{ProviderSeries, Sample};
    use crate::Labels;

    fn series(name: &str, samples: Vec<Sample>) -> Vec<ProviderSeries> {
        vec![ProviderSeries {
            metric: {
                let mut labels = Labels::new();
                labels.insert("__name__".into(), name.into());
                labels
            },
            samples,
        }]
    }

    fn ok_query(samples: Vec<Sample>, issued_at: i64) -> CollectedQuery {
        CollectedQuery {
            entry_index: 0,
            catalog_metric: "login.online".into(),
            service: "loginapp".into(),
            window: QueryWindow {
                start_unix_ms: 0,
                end_unix_ms: 1000,
                step_ms: 1000,
            },
            issued_at_unix_ms: issued_at,
            outcome: CollectedOutcome::Series(series("kbengine_online", samples)),
        }
    }

    #[test]
    fn failed_query_is_a_missing_window_with_verbatim_reason() {
        let query = CollectedQuery {
            outcome: CollectedOutcome::Failed(
                "metric provider returned http status 503".into(),
            ),
            ..ok_query(vec![], 0)
        };
        let findings = detect_missing_windows(&[query]);
        assert_eq!(findings.len(), 1);
        match &findings[0] {
            QualityFinding::MissingWindow {
                catalog_metric,
                service,
                reason,
                ..
            } => {
                assert_eq!(catalog_metric, "login.online");
                assert_eq!(service, "loginapp");
                assert_eq!(
                    reason,
                    "metric provider returned http status 503"
                );
            }
            other => panic!("expected MissingWindow, got {other:?}"),
        }
    }

    #[test]
    fn empty_result_is_a_gap_not_a_zero() {
        let no_series = ok_query(vec![], 0);
        let series_without_samples = ok_query(vec![], 0);
        // (both shapes of emptiness)
        let findings = detect_missing_windows(&[no_series, series_without_samples]);
        assert_eq!(findings.len(), 2);
        for finding in &findings {
            match finding {
                QualityFinding::MissingWindow { reason, .. } => {
                    assert_eq!(reason, "empty result set");
                }
                other => panic!("expected MissingWindow, got {other:?}"),
            }
        }
    }

    #[test]
    fn samples_within_threshold_pass() {
        // issued 1500, source stamp 1200: age 300, threshold 500.
        let query = ok_query(
            vec![Sample {
                timestamp_unix_ms: 1200,
                value: 7.0,
            }],
            1500,
        );
        assert!(detect_delayed_samples(&[query], 500).is_empty());
    }

    #[test]
    fn stale_sample_is_flagged_with_raw_age() {
        // issued 1500, source stamp 200: age 1300 > 500.
        let query = ok_query(
            vec![Sample {
                timestamp_unix_ms: 200,
                value: 7.0,
            }],
            1500,
        );
        let findings = detect_delayed_samples(&[query], 500);
        assert_eq!(findings.len(), 1);
        match &findings[0] {
            QualityFinding::DelayedSample {
                source_age_ms,
                threshold_ms,
                sample_timestamp_unix_ms,
                collected_at_unix_ms,
                ..
            } => {
                assert_eq!(*source_age_ms, 1300);
                assert_eq!(*threshold_ms, 500);
                assert_eq!(*sample_timestamp_unix_ms, 200);
                assert_eq!(*collected_at_unix_ms, 1500);
            }
            other => panic!("expected DelayedSample, got {other:?}"),
        }
    }

    #[test]
    fn non_positive_threshold_disables_delay_detection() {
        let query = ok_query(
            vec![Sample {
                timestamp_unix_ms: 0,
                value: 1.0,
            }],
            100_000,
        );
        assert!(detect_delayed_samples(std::slice::from_ref(&query), 0).is_empty());
        assert!(detect_delayed_samples(&[query], -1).is_empty());
    }

    #[test]
    fn clean_batch_reports_clean_and_counts_consistently() {
        let good = ok_query(
            vec![Sample {
                timestamp_unix_ms: 900,
                value: 3.0,
            }],
            1000,
        );
        let failed = CollectedQuery {
            outcome: CollectedOutcome::Failed("upstream timeout".into()),
            ..ok_query(vec![], 1000)
        };
        let report = quality_report(&[good.clone(), failed], 500);
        assert_eq!(report.inspected, 2);
        assert_eq!(report.missing_windows(), 1);
        assert_eq!(report.delayed_samples(), 0);
        assert!(!report.is_clean());

        let clean = quality_report(std::slice::from_ref(&good), 500);
        assert!(clean.is_clean());
        assert_eq!(clean.inspected, 1);
    }

    #[test]
    fn report_serialises_tagged_findings_round_trip() {
        let report = quality_report(
            &[
                ok_query(
                    vec![Sample {
                        timestamp_unix_ms: 100,
                        value: 1.0,
                    }],
                    10_000,
                ),
                CollectedQuery {
                    outcome: CollectedOutcome::Failed("boom".into()),
                    ..ok_query(vec![], 0)
                },
            ],
            1000,
        );
        let text = serde_yaml::to_string(&report).expect("serialise");
        assert!(text.contains("finding: delayed_sample"));
        assert!(text.contains("finding: missing_window"));
        let back: QualityReport = serde_yaml::from_str(&text).expect("reparse");
        assert_eq!(back, report);
    }
}
