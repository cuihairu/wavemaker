//! Report throughput metrics: QPS / TPS (WM-0417; docs/spec/report-template.md
//! sections 5 `client_performance` and 8 `server_performance`).
//!
//! qps = requests per second; tps = transactions or messages per second.
//! Server-side rates derive from catalog COUNTER series (delta between
//! consecutive samples over the elapsed time, stamped at the midpoint);
//! client-side rates are the per-backend effective_request_rate samples
//! taken as-is. Every series exports a time curve, peak, and p50/p95/p99
//! OF THE RATE VALUES (nearest-rank, no interpolation), plus a magnitude
//! check against the planned load so a report can demonstrate its qps/tps
//! matches what was actually driven.

use serde::{Deserialize, Serialize};

use crate::catalog::{MetricCatalog, MetricKind};
use crate::provider::ProviderSeries;

/// What the rate counts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ThroughputKind {
    /// Requests per second.
    Qps,
    /// Transactions or messages per second.
    Tps,
}

/// Which plane a throughput series came from (the report sections keep
/// the planes separate — no cross-plane blending).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "source", rename_all = "snake_case")]
pub enum ThroughputSource {
    /// Server section 8: a catalog counter under a declared service.
    Server {
        service: String,
        catalog_metric: String,
    },
    /// Client section 5: a backend's effective request rate.
    Client { backend: String },
}

/// One rate sample on the swell timeline.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct ThroughputPoint {
    pub timeline_unix_ms: i64,
    /// Rate in units per second.
    pub per_second: f64,
}

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum ThroughputError {
    #[error("metric {0} is not a catalog counter; rate semantics would be guessed")]
    NotACounter(String),
    #[error("metric {0} has fewer than two usable samples")]
    NotEnoughSamples(String),
}

/// A full throughput series with its provenance.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ThroughputSeries {
    pub kind: ThroughputKind,
    pub source: ThroughputSource,
    pub points: Vec<ThroughputPoint>,
    /// Counter resets observed between consecutive samples; the reset
    /// pair is excluded from the rate curve (a reset is not negative
    /// throughput) but never hidden.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub resets_observed: usize,
}

fn is_zero(value: &usize) -> bool {
    *value == 0
}

/// Server-side throughput: catalog COUNTER -> per-interval rate series.
/// Rates stamp at the interval midpoint; a negative delta (counter
/// reset) is excluded and counted in `resets_observed`.
pub fn server_rate_series(
    catalog: &MetricCatalog,
    kind: ThroughputKind,
    service: &str,
    catalog_metric: &str,
    series: &ProviderSeries,
) -> Result<ThroughputSeries, ThroughputError> {
    match catalog.lookup(catalog_metric).map(|def| &def.kind) {
        Some(MetricKind::Counter) => {}
        _ => {
            return Err(ThroughputError::NotACounter(catalog_metric.into()));
        }
    }
    let mut points = Vec::new();
    let mut resets = 0usize;
    let mut finite_samples = series
        .samples
        .iter()
        .filter(|sample| sample.value.is_finite())
        .collect::<Vec<_>>();
    finite_samples.sort_by_key(|sample| sample.timestamp_unix_ms);
    for pair in finite_samples.windows(2) {
        let (first, second) = (pair[0], pair[1]);
        let elapsed_s = (second.timestamp_unix_ms - first.timestamp_unix_ms) as f64 / 1000.0;
        if elapsed_s <= 0.0 {
            continue;
        }
        let delta = second.value - first.value;
        if delta < 0.0 {
            resets += 1;
            continue;
        }
        points.push(ThroughputPoint {
            timeline_unix_ms: (first.timestamp_unix_ms + second.timestamp_unix_ms) / 2,
            per_second: delta / elapsed_s,
        });
    }
    if points.is_empty() {
        return Err(ThroughputError::NotEnoughSamples(catalog_metric.into()));
    }
    Ok(ThroughputSeries {
        kind,
        source: ThroughputSource::Server {
            service: service.into(),
            catalog_metric: catalog_metric.into(),
        },
        points,
        resets_observed: resets,
    })
}

/// Client-side throughput: the backend's effective request rate samples
/// as recorded by the generator plane, taken as-is (they are already
/// rates, not counters).
pub fn client_rate_series(
    kind: ThroughputKind,
    backend: &str,
    samples: &[(i64, f64)],
) -> Result<ThroughputSeries, ThroughputError> {
    let mut points: Vec<ThroughputPoint> = samples
        .iter()
        .map(|(ts, rate)| ThroughputPoint {
            timeline_unix_ms: *ts,
            per_second: *rate,
        })
        .collect();
    points.sort_by_key(|point| point.timeline_unix_ms);
    if points.is_empty() {
        return Err(ThroughputError::NotEnoughSamples(backend.into()));
    }
    Ok(ThroughputSeries {
        kind,
        source: ThroughputSource::Client {
            backend: backend.into(),
        },
        points,
        resets_observed: 0,
    })
}

/// Curve + peak + p50/p95/p99 of a series' rate values. Percentiles use
/// the nearest-rank method on sorted values — a real observed sample,
/// never an interpolated invention.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ThroughputSummary {
    pub series: ThroughputSeries,
    pub peak_per_second: f64,
    pub p50_per_second: f64,
    pub p95_per_second: f64,
    pub p99_per_second: f64,
    pub point_count: usize,
}

fn nearest_rank(sorted: &[f64], percentile: f64) -> f64 {
    let rank = ((percentile / 100.0) * sorted.len() as f64).ceil() as usize;
    let index = rank.saturating_sub(1).min(sorted.len() - 1);
    sorted[index]
}

impl ThroughputSeries {
    pub fn summarize(&self) -> Result<ThroughputSummary, ThroughputError> {
        if self.points.is_empty() {
            return Err(ThroughputError::NotEnoughSamples("series".into()));
        }
        let mut rates: Vec<f64> = self.points.iter().map(|point| point.per_second).collect();
        rates.sort_by(|a, b| a.partial_cmp(b).expect("rates are finite"));
        Ok(ThroughputSummary {
            peak_per_second: *rates.last().expect("non-empty"),
            p50_per_second: nearest_rank(&rates, 50.0),
            p95_per_second: nearest_rank(&rates, 95.0),
            p99_per_second: nearest_rank(&rates, 99.0),
            point_count: self.points.len(),
            series: self.clone(),
        })
    }

    /// Magnitude check for report acceptance ("与实际压测量级一致"):
    /// the series' median rate is within `tolerance_ratio` (relative) of
    /// the expected per-second magnitude.
    pub fn magnitude_matches(
        &self,
        expected_per_second: f64,
        tolerance_ratio: f64,
    ) -> Result<bool, ThroughputError> {
        let summary = self.summarize()?;
        if expected_per_second <= 0.0 {
            return Ok(false);
        }
        let deviation =
            ((summary.p50_per_second - expected_per_second) / expected_per_second).abs();
        Ok(deviation <= tolerance_ratio)
    }
}

impl ThroughputSummary {
    /// Deterministic Markdown fragment for the report export ("导出的
    /// JSON/Markdown 含 qps/tps").
    pub fn markdown(&self) -> String {
        let unit = match self.series.kind {
            ThroughputKind::Qps => "req/s",
            ThroughputKind::Tps => "tx/s",
        };
        let source = match &self.series.source {
            ThroughputSource::Server {
                service,
                catalog_metric,
            } => format!("server:{service}/{catalog_metric}"),
            ThroughputSource::Client { backend } => format!("client:{backend}"),
        };
        let mut out = String::new();
        out.push_str("| throughput | source | unit | peak | p50 | p95 | p99 | points |\n");
        out.push_str("|---|---|---|---|---|---|---|---|\n");
        out.push_str(&format!(
            "| {kind} | {source} | {unit} | {peak:.3} | {p50:.3} | {p95:.3} | {p99:.3} | {points} |\n",
            kind = match self.series.kind {
                ThroughputKind::Qps => "qps",
                ThroughputKind::Tps => "tps",
            },
            peak = self.peak_per_second,
            p50 = self.p50_per_second,
            p95 = self.p95_per_second,
            p99 = self.p99_per_second,
            points = self.point_count,
        ));
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::{Category, MetricDef};
    use crate::Labels;

    fn catalog_with_counter(name: &str) -> MetricCatalog {
        let mut catalog = MetricCatalog::default();
        catalog
            .define(MetricDef {
                name: name.into(),
                kind: MetricKind::Counter,
                unit: Unit::Count,
                category: Category::GatewayLogin,
                description: format!("meaning of {name}"),
            })
            .expect("define");
        catalog
    }

    fn counter_series(values: &[(i64, f64)]) -> ProviderSeries {
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

    use crate::catalog::Unit;
    use crate::provider::Sample;

    #[test]
    fn counter_becomes_per_second_rates_at_midpoints() {
        let catalog = catalog_with_counter("login.request_total");
        // 10 req in 2s, then 15 in 1s.
        let series = counter_series(&[(0, 100.0), (2000, 110.0), (3000, 125.0)]);
        let throughput = server_rate_series(
            &catalog,
            ThroughputKind::Qps,
            "loginapp",
            "login.request_total",
            &series,
        )
        .expect("series");
        assert_eq!(throughput.points.len(), 2);
        assert_eq!(throughput.points[0].timeline_unix_ms, 1000);
        assert_eq!(throughput.points[0].per_second, 5.0);
        assert_eq!(throughput.points[1].timeline_unix_ms, 2500);
        assert_eq!(throughput.points[1].per_second, 15.0);
        assert_eq!(throughput.resets_observed, 0);

        let summary = throughput.summarize().expect("summary");
        assert_eq!(summary.peak_per_second, 15.0);
        assert_eq!(summary.p50_per_second, 5.0);
        assert_eq!(summary.point_count, 2);
    }

    #[test]
    fn counter_reset_is_excluded_and_counted() {
        let catalog = catalog_with_counter("login.request_total");
        let series = counter_series(&[(0, 100.0), (1000, 2.0), (2000, 12.0)]);
        let throughput = server_rate_series(
            &catalog,
            ThroughputKind::Qps,
            "loginapp",
            "login.request_total",
            &series,
        )
        .expect("series");
        // Reset pair (100 -> 2) excluded; (2 -> 12) over 1s is 10/s.
        assert_eq!(throughput.resets_observed, 1);
        assert_eq!(throughput.points.len(), 1);
        assert_eq!(throughput.points[0].per_second, 10.0);
    }

    #[test]
    fn non_counter_catalog_metrics_are_refused() {
        let mut catalog = MetricCatalog::default();
        catalog
            .define(MetricDef {
                name: "game.online".into(),
                kind: MetricKind::Gauge,
                unit: Unit::Count,
                category: Category::Game,
                description: "online".into(),
            })
            .expect("define");
        let series = counter_series(&[(0, 1.0), (1000, 2.0)]);
        assert!(matches!(
            server_rate_series(
                &catalog,
                ThroughputKind::Qps,
                "cellapp",
                "game.online",
                &series
            ),
            Err(ThroughputError::NotACounter(_))
        ));
        assert!(matches!(
            server_rate_series(
                &catalog,
                ThroughputKind::Qps,
                "cellapp",
                "ghost.metric",
                &series
            ),
            Err(ThroughputError::NotACounter(_))
        ));
    }

    #[test]
    fn client_rates_pass_through_and_empty_input_is_refused() {
        let throughput = client_rate_series(
            ThroughputKind::Tps,
            "native-backend",
            &[(0, 12.5), (1000, 14.0)],
        )
        .expect("series");
        match &throughput.source {
            ThroughputSource::Client { backend } => {
                assert_eq!(backend, "native-backend");
            }
            other => panic!("expected client source, got {other:?}"),
        }
        assert_eq!(throughput.points[0].per_second, 12.5);
        assert!(matches!(
            client_rate_series(ThroughputKind::Tps, "native-backend", &[]),
            Err(ThroughputError::NotEnoughSamples(_))
        ));
    }

    #[test]
    fn percentiles_are_nearest_rank_observed_values() {
        // 10 points 1..10: p50 -> rank ceil(5)=5 -> 5; p95 -> ceil(9.5)=10 -> 10;
        // p99 -> ceil(9.9)=10 -> 10.
        let rates: Vec<(i64, f64)> = (1..=10)
            .map(|index| (index as i64 * 1000, index as f64))
            .collect();
        let throughput = client_rate_series(ThroughputKind::Qps, "b", &rates).expect("series");
        let summary = throughput.summarize().expect("summary");
        assert_eq!(summary.p50_per_second, 5.0);
        assert_eq!(summary.p95_per_second, 10.0);
        assert_eq!(summary.p99_per_second, 10.0);
        assert_eq!(summary.peak_per_second, 10.0);
    }

    #[test]
    fn magnitude_check_compares_median_to_expected_load() {
        let rates: Vec<(i64, f64)> = (1..=10)
            .map(|index| (index as i64 * 1000, 20.0 + index as f64 * 0.1))
            .collect();
        let throughput = client_rate_series(ThroughputKind::Qps, "b", &rates).expect("series");
        // Median ~20.55: within 10% of 20, not within 1%.
        assert!(throughput.magnitude_matches(20.0, 0.10).expect("check"));
        assert!(!throughput.magnitude_matches(20.0, 0.01).expect("check"));
        assert!(!throughput.magnitude_matches(0.0, 1.0).expect("check"));
    }

    #[test]
    fn markdown_export_names_qps_tps_with_numbers() {
        let throughput = client_rate_series(
            ThroughputKind::Qps,
            "native-backend",
            &[(0, 100.0), (1000, 200.0)],
        )
        .expect("series");
        let summary = throughput.summarize().expect("summary");
        let text = summary.markdown();
        assert!(text.contains("qps"));
        assert!(text.contains("req/s"));
        assert!(text.contains("client:native-backend"));
        assert!(text.contains("200.000"));

        let tps = client_rate_series(ThroughputKind::Tps, "b", &[(0, 50.0)])
            .expect("series")
            .summarize()
            .expect("summary");
        assert!(tps.markdown().contains("tps"));
        assert!(tps.markdown().contains("tx/s"));
    }

    #[test]
    fn series_and_summary_serialise_json_round_trip() {
        let throughput = client_rate_series(
            ThroughputKind::Qps,
            "native-backend",
            &[(0, 100.0), (1000, 200.0)],
        )
        .expect("series");
        let summary = throughput.summarize().expect("summary");
        let json = serde_json::to_string(&summary).expect("json");
        assert!(json.contains("\"qps\""));
        let back: ThroughputSummary = serde_json::from_str(&json).expect("reparse");
        assert_eq!(back, summary);
    }
}
