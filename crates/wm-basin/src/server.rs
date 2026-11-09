//! Server Metrics aggregation (WM-0413; docs/basin.md "Metric Categories",
//! HDR rules, Phase 4 exit criteria).
//!
//! Aggregation follows the CATALOG kind (WM-0403) per provider series
//! (WM-0404):
//! - Gauge -> min / mean / max over the finite samples,
//! - Counter -> first / last / delta over the window,
//! - Histogram -> NOT aggregated here: distribution semantics need
//!   merged histograms ("Averaging P99 values across Workers is not
//!   allowed"); folding raw histogram buckets into scalars would
//!   fabricate percentiles.
//!
//! Series identity is per label map — metrics from different instances
//! are NEVER silently merged into one number (that would fabricate both
//! gauges and counters). NaN samples are excluded from stats but COUNTED,
//! never dropped invisibly.

use serde::{Deserialize, Serialize};

use crate::catalog::{MetricCatalog, MetricKind};
use crate::provider::{ProviderSeries, Sample};
use crate::scheduler::{CollectedOutcome, CollectedQuery};

/// Where the samples fed to one aggregate physically spanned. Reported,
/// not assumed: the span comes from the source timestamps.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct SampleSpan {
    pub first_timestamp_unix_ms: i64,
    pub last_timestamp_unix_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "aggregate", rename_all = "snake_case")]
pub enum MetricAggregate {
    Gauge {
        service: String,
        catalog_metric: String,
        sample_count: usize,
        /// NaN/infinite samples seen but excluded from the stats.
        non_finite_count: usize,
        min: f64,
        mean: f64,
        max: f64,
        span: SampleSpan,
    },
    Counter {
        service: String,
        catalog_metric: String,
        sample_count: usize,
        first_value: f64,
        last_value: f64,
        /// last - first. `None` with fewer than two samples. A NEGATIVE
        /// delta means the underlying counter reset between scrapes —
        /// recorded as-is; reset handling is downstream, not a silent
        /// clamp here.
        delta: Option<f64>,
        span: SampleSpan,
    },
    /// The catalog kind cannot be aggregated to scalars honestly.
    NotAggregated {
        service: String,
        catalog_metric: String,
        reason: String,
    },
}

impl MetricAggregate {
    pub fn service(&self) -> &str {
        match self {
            MetricAggregate::Gauge { service, .. }
            | MetricAggregate::Counter { service, .. }
            | MetricAggregate::NotAggregated { service, .. } => service,
        }
    }

    pub fn catalog_metric(&self) -> &str {
        match self {
            MetricAggregate::Gauge { catalog_metric, .. }
            | MetricAggregate::Counter { catalog_metric, .. }
            | MetricAggregate::NotAggregated { catalog_metric, .. } => {
                catalog_metric
            }
        }
    }
}

fn span_of(samples: &[Sample]) -> Option<SampleSpan> {
    let first = samples.first()?;
    let last = samples.last()?;
    Some(SampleSpan {
        first_timestamp_unix_ms: first.timestamp_unix_ms,
        last_timestamp_unix_ms: last.timestamp_unix_ms,
    })
}

/// Aggregate ONE provider series according to its catalog kind. Unknown
/// catalog metrics produce `NotAggregated` (the mapping/preflight is
/// where that gets caught; aggregation refuses to guess semantics).
pub fn aggregate_series(
    catalog: &MetricCatalog,
    service: &str,
    catalog_metric: &str,
    series: &ProviderSeries,
) -> MetricAggregate {
    let span = match span_of(&series.samples) {
        Some(span) => span,
        None => {
            return MetricAggregate::NotAggregated {
                service: service.into(),
                catalog_metric: catalog_metric.into(),
                reason: "no samples in series".into(),
            }
        }
    };
    let kind = match catalog.lookup(catalog_metric) {
        Some(def) => def.kind.clone(),
        None => {
            return MetricAggregate::NotAggregated {
                service: service.into(),
                catalog_metric: catalog_metric.into(),
                reason: "metric not defined in catalog".into(),
            }
        }
    };
    match kind {
        MetricKind::Gauge => {
            let mut min = f64::INFINITY;
            let mut max = f64::NEG_INFINITY;
            let mut sum = 0.0;
            let mut count = 0usize;
            let mut non_finite = 0usize;
            for sample in &series.samples {
                if sample.value.is_finite() {
                    min = min.min(sample.value);
                    max = max.max(sample.value);
                    sum += sample.value;
                    count += 1;
                } else {
                    non_finite += 1;
                }
            }
            if count == 0 {
                return MetricAggregate::NotAggregated {
                    service: service.into(),
                    catalog_metric: catalog_metric.into(),
                    reason: "all samples non-finite".into(),
                };
            }
            MetricAggregate::Gauge {
                service: service.into(),
                catalog_metric: catalog_metric.into(),
                sample_count: count,
                non_finite_count: non_finite,
                min,
                mean: sum / count as f64,
                max,
                span,
            }
        }
        MetricKind::Counter => {
            let mut first: Option<f64> = None;
            let mut last: Option<f64> = None;
            let mut count = 0usize;
            for sample in &series.samples {
                if !sample.value.is_finite() {
                    continue;
                }
                if first.is_none() {
                    first = Some(sample.value);
                }
                last = Some(sample.value);
                count += 1;
            }
            match (first, last) {
                (Some(first_value), Some(last_value)) => {
                    let delta = if count >= 2 {
                        Some(last_value - first_value)
                    } else {
                        None
                    };
                    MetricAggregate::Counter {
                        service: service.into(),
                        catalog_metric: catalog_metric.into(),
                        sample_count: count,
                        first_value,
                        last_value,
                        delta,
                        span,
                    }
                }
                _ => MetricAggregate::NotAggregated {
                    service: service.into(),
                    catalog_metric: catalog_metric.into(),
                    reason: "no finite counter samples".into(),
                },
            }
        }
        MetricKind::Histogram => MetricAggregate::NotAggregated {
            service: service.into(),
            catalog_metric: catalog_metric.into(),
            reason:
                "histogram needs merged histograms; scalars would fabricate percentiles"
                    .into(),
        },
    }
}

/// Aggregate every raw series inside a collection batch (WM-0406 records).
/// Failed queries contribute nothing here — the quality detector (WM-0408)
/// owns reporting them; aggregation never substitutes their absence.
pub fn aggregate_collected(
    catalog: &MetricCatalog,
    queries: &[CollectedQuery],
) -> Vec<MetricAggregate> {
    let mut aggregates = Vec::new();
    for query in queries {
        let CollectedOutcome::Series(series_list) = &query.outcome else {
            continue;
        };
        for series in series_list {
            aggregates.push(aggregate_series(
                catalog,
                &query.service,
                &query.catalog_metric,
                series,
            ));
        }
    }
    aggregates
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::{Category, MetricDef, Unit};
    use crate::scheduler::CollectedOutcome;
    use crate::Labels;

    fn catalog() -> MetricCatalog {
        let mut catalog = MetricCatalog::default();
        for (name, kind, unit) in [
            ("game.online", MetricKind::Gauge, Unit::Count),
            ("login.success_total", MetricKind::Counter, Unit::Count),
            ("login.latency", MetricKind::Histogram, Unit::Milliseconds),
        ] {
            catalog
                .define(MetricDef {
                    name: name.into(),
                    kind,
                    unit,
                    category: Category::Game,
                    description: format!("meaning of {name}"),
                })
                .expect("define");
        }
        catalog
    }

    fn series(values: &[(i64, f64)]) -> ProviderSeries {
        ProviderSeries {
            metric: Labels::new(),
            samples: values
                .iter()
                .map(|(ts, value)| Sample {
                    timestamp_unix_ms: *ts,
                    value: *value,
                })
                .collect(),
        }
    }

    #[test]
    fn gauge_stats_cover_min_mean_max_and_span() {
        let aggregate = aggregate_series(
            &catalog(),
            "cellapp",
            "game.online",
            &series(&[(0, 4.0), (1000, 10.0), (2000, 6.0)]),
        );
        match aggregate {
            MetricAggregate::Gauge {
                sample_count,
                non_finite_count,
                min,
                mean,
                max,
                span,
                ..
            } => {
                assert_eq!(sample_count, 3);
                assert_eq!(non_finite_count, 0);
                assert_eq!(min, 4.0);
                assert_eq!(mean, 20.0 / 3.0);
                assert_eq!(max, 10.0);
                assert_eq!(span.first_timestamp_unix_ms, 0);
                assert_eq!(span.last_timestamp_unix_ms, 2000);
            }
            other => panic!("expected gauge aggregate, got {other:?}"),
        }
    }

    #[test]
    fn non_finite_gauge_samples_are_counted_not_included() {
        let aggregate = aggregate_series(
            &catalog(),
            "cellapp",
            "game.online",
            &series(&[(0, 5.0), (1000, f64::NAN), (2000, 7.0)]),
        );
        match aggregate {
            MetricAggregate::Gauge {
                sample_count,
                non_finite_count,
                mean,
                ..
            } => {
                assert_eq!(sample_count, 2);
                assert_eq!(non_finite_count, 1);
                assert_eq!(mean, 6.0);
            }
            other => panic!("expected gauge aggregate, got {other:?}"),
        }
    }

    #[test]
    fn counter_delta_computes_and_single_sample_has_none() {
        let counter =
            aggregate_series(&catalog(), "loginapp", "login.success_total",
                &series(&[(0, 10.0), (1000, 15.0), (2000, 18.0)]));
        match counter {
            MetricAggregate::Counter { delta, first_value, last_value, .. } => {
                assert_eq!(first_value, 10.0);
                assert_eq!(last_value, 18.0);
                assert_eq!(delta, Some(8.0));
            }
            other => panic!("expected counter aggregate, got {other:?}"),
        }

        let single = aggregate_series(&catalog(), "loginapp",
            "login.success_total", &series(&[(0, 3.0)]));
        match single {
            MetricAggregate::Counter { delta, .. } => assert_eq!(delta, None),
            other => panic!("expected counter aggregate, got {other:?}"),
        }
    }

    #[test]
    fn counter_reset_is_recorded_not_clamped() {
        // A scrape across a counter reset shows a negative delta here;
        // clamping it silently would fabricate a larger increase.
        let reset = aggregate_series(&catalog(), "loginapp",
            "login.success_total", &series(&[(0, 100.0), (1000, 2.0)]));
        match reset {
            MetricAggregate::Counter { delta, .. } => {
                assert_eq!(delta, Some(-98.0));
            }
            other => panic!("expected counter aggregate, got {other:?}"),
        }
    }

    #[test]
    fn histograms_refuse_scalar_aggregation() {
        let aggregate = aggregate_series(
            &catalog(),
            "loginapp",
            "login.latency",
            &series(&[(0, 1.0), (1000, 2.0)]),
        );
        match aggregate {
            MetricAggregate::NotAggregated { reason, .. } => {
                assert!(reason.contains("merged histograms"), "{reason}");
            }
            other => panic!("expected NotAggregated, got {other:?}"),
        }
    }

    #[test]
    fn unknown_catalog_metric_and_empty_series_refuse_to_guess() {
        let unknown = aggregate_series(&catalog(), "loginapp", "ghost.metric",
            &series(&[(0, 1.0)]));
        match unknown {
            MetricAggregate::NotAggregated { reason, .. } => {
                assert!(reason.contains("catalog"), "{reason}");
            }
            other => panic!("expected NotAggregated, got {other:?}"),
        }
        let empty = aggregate_series(&catalog(), "loginapp", "game.online",
            &ProviderSeries { metric: Labels::new(), samples: vec![] });
        assert!(matches!(empty, MetricAggregate::NotAggregated { .. }));
    }

    #[test]
    fn batch_aggregation_skips_failed_queries_and_keeps_series_identity() {
        let catalog = catalog();
        let query = CollectedQuery {
            entry_index: 0,
            catalog_metric: "game.online".into(),
            service: "cellapp".into(),
            window: crate::provider::QueryWindow {
                start_unix_ms: 0,
                end_unix_ms: 2000,
                step_ms: 1000,
            },
            issued_at_unix_ms: 3000,
            outcome: CollectedOutcome::Series(vec![
                // Two distinct instances: one aggregate each, no merge.
                {
                    let mut labels = Labels::new();
                    labels.insert("instance".into(), "a".into());
                    ProviderSeries {
                        metric: labels,
                        samples: vec![Sample {
                            timestamp_unix_ms: 0,
                            value: 5.0,
                        }],
                    }
                },
                {
                    let mut labels = Labels::new();
                    labels.insert("instance".into(), "b".into());
                    ProviderSeries {
                        metric: labels,
                        samples: vec![Sample {
                            timestamp_unix_ms: 0,
                            value: 50.0,
                        }],
                    }
                },
            ]),
        };
        let failed = CollectedQuery {
            outcome: CollectedOutcome::Failed("upstream timeout".into()),
            ..query.clone()
        };
        let aggregates = aggregate_collected(&catalog, &[query, failed]);
        assert_eq!(aggregates.len(), 2);
        let values: Vec<f64> = aggregates
            .iter()
            .map(|aggregate| match aggregate {
                MetricAggregate::Gauge { mean, .. } => *mean,
                other => panic!("expected gauge, got {other:?}"),
            })
            .collect();
        assert_eq!(values, vec![5.0, 50.0]);
    }

    #[test]
    fn aggregates_serialise_tagged_round_trip() {
        let aggregate = aggregate_series(
            &catalog(),
            "loginapp",
            "login.success_total",
            &series(&[(0, 1.0), (1000, 4.0)]),
        );
        let text = serde_yaml::to_string(&aggregate).expect("serialise");
        assert!(text.contains("aggregate: counter"));
        let back: MetricAggregate = serde_yaml::from_str(&text).expect("reparse");
        assert_eq!(back, aggregate);
        assert_eq!(back.service(), "loginapp");
        assert_eq!(back.catalog_metric(), "login.success_total");
    }
}
