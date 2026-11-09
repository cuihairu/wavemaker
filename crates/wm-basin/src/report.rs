//! End-to-end correlation reports (WM-0416; docs/basin.md Phase 4 goal
//! "bring Generator, Server, and planned load onto a single Swell
//! timeline", "Reports with data quality", Preflight's partial rule).
//!
//! The report assembles ONLY what the earlier items recorded — timeline
//! alignment (WM-0414), quality detection (WM-0408), generator
//! aggregation (WM-0412), server aggregation (WM-0413), labels derived
//! from that evidence (WM-0415) — under the correlation context's
//! swell_id (WM-0401). Two honesty rules are structural:
//!
//! - the effective window is the INTERSECTION of declared correlation
//!   range and source coverage — never a wider window than the data
//!   covers;
//! - when the report is `Partial Observability`, complete server-side
//!   performance conclusions must not be given — `server_conclusion_allowed`
//!   is the gate a report writer consults, and `Complete` cannot coexist
//!   with `Partial` because labels are derived in one place.

use serde::{Deserialize, Serialize};

use crate::alignment::{common_window, TimelineAlignment, TimelineSpan};
use crate::correlation::CorrelationContext;
use crate::generator::{GeneratorAggregate, SaturationSignal, SaturationThresholds};
use crate::labels::{QualityEvidence, QualityLabelSet};
use crate::quality::QualityReport;
use crate::server::MetricAggregate;

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum ReportError {
    #[error("swell id must not be empty")]
    EmptySwellId,
    #[error("correlation context is invalid: {0}")]
    InvalidCorrelation(String),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CorrelationReport {
    pub swell_id: String,
    /// Name of the anchor the single timeline was built on.
    pub timeline_anchor: String,
    /// The window the report effectively covers: declared correlation
    /// range ∩ source coverage. `None` = no honest window exists.
    pub effective_window: Option<TimelineSpan>,
    /// WM-0414 alignment record (clocks, trusts, offsets).
    pub timeline: TimelineAlignment,
    /// WM-0412 aggregate of the generator plane.
    pub generator: GeneratorAggregate,
    /// Saturation evidence, if any ("Generator Saturated").
    pub saturation: Vec<SaturationSignal>,
    /// WM-0413 per-series server aggregates (failed queries contribute
    /// nothing — their absence is reported through `quality`).
    pub server: Vec<MetricAggregate>,
    /// WM-0408 detection output.
    pub quality: QualityReport,
    /// WM-0415 labels derived from this report's own evidence.
    pub labels: QualityLabelSet,
}

/// Everything the report is assembled from. Aggregates and detection
/// records are supplied by their items; this module computes the window,
/// the saturation list, and the label set so they cannot disagree.
pub struct CorrelationInput<'a> {
    pub correlation: &'a CorrelationContext,
    pub timeline: &'a TimelineAlignment,
    /// Coverage span of each contributing source (already aligned to the
    /// timeline).
    pub source_spans: &'a [TimelineSpan],
    pub quality: &'a QualityReport,
    pub generator: &'a GeneratorAggregate,
    pub generator_thresholds: SaturationThresholds,
    pub server: Vec<MetricAggregate>,
    pub observer_degraded: bool,
    pub partial_observability: bool,
}

impl CorrelationReport {
    pub fn assemble(input: CorrelationInput<'_>) -> Result<Self, ReportError> {
        if input.correlation.swell_id.trim().is_empty() {
            return Err(ReportError::EmptySwellId);
        }
        input
            .correlation
            .validate()
            .map_err(|err| ReportError::InvalidCorrelation(err.to_string()))?;

        // Declared range ∩ source coverage.
        let mut spans: Vec<TimelineSpan> = input.source_spans.to_vec();
        if let Some(range) = &input.correlation.time_range {
            spans.push(TimelineSpan {
                start_unix_ms: range.start_unix_ms,
                end_unix_ms: range.end_unix_ms,
            });
        }
        let effective_window = common_window(&spans);

        let saturation =
            input.generator.saturation_signals(&input.generator_thresholds);

        let evidence = QualityEvidence {
            quality: input.quality,
            generator: input.generator,
            generator_thresholds: input.generator_thresholds,
            observer_degraded: input.observer_degraded,
            partial_observability: input.partial_observability,
            alignment: input.timeline,
            invalid_samples: input.server.iter().fold(0usize, |sum, aggregate| {
                sum + match aggregate {
                    MetricAggregate::Gauge {
                        non_finite_count, ..
                    } => *non_finite_count,
                    _ => 0,
                }
            }),
        };
        let labels = QualityLabelSet::derive(&evidence);

        Ok(Self {
            swell_id: input.correlation.swell_id.clone(),
            timeline_anchor: input.timeline.anchor.clone(),
            effective_window,
            timeline: input.timeline.clone(),
            generator: input.generator.clone(),
            saturation,
            server: input.server,
            quality: input.quality.clone(),
            labels,
        })
    }

    /// Whether complete server-side performance conclusions may be drawn
    /// from this report. Partial observability forbids them (docs/basin.md
    /// Preflight: "Complete server-side performance conclusions must not
    /// be given").
    pub fn server_conclusion_allowed(&self) -> bool {
        !self.labels.contains(crate::labels::DataQualityLabel::Partial)
    }

    /// The "Generator Saturated" rule: with saturation evidence present,
    /// a generator bottleneck must not be misread as a server-side one.
    pub fn generator_saturated(&self) -> bool {
        !self.saturation.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::alignment::SourceClock;
    use crate::catalog::{MetricCatalog, MetricKind, Unit};
    use crate::generator::WorkerSnapshot;
    use crate::labels::DataQualityLabel;
    use crate::provider::{ProviderSeries, QueryWindow, Sample};
    use crate::quality::quality_report;
    use crate::scheduler::{CollectedOutcome, CollectedQuery};
    use crate::server::aggregate_collected;
    use crate::timestamps::ClockOffsetEstimate;
    use crate::Labels;

    fn correlation() -> CorrelationContext {
        CorrelationContext {
            swell_id: "sw-correlated".into(),
            time_range: None,
        }
    }

    fn timeline() -> TimelineAlignment {
        let mut alignment = TimelineAlignment::new("swell-start", 1_000_000);
        alignment.add_clock(SourceClock::trusted("controller"));
        alignment
    }

    fn quality_clean() -> QualityReport {
        quality_report(&[], 5000)
    }

    fn generator_clean() -> GeneratorAggregate {
        crate::generator::aggregate(&[WorkerSnapshot {
            worker_id: "worker-a".into(),
            at_unix_ms: 0,
            cpu_ratio: Some(0.2),
            ..WorkerSnapshot::default()
        }])
    }

    fn server_aggregates() -> Vec<MetricAggregate> {
        let mut catalog = MetricCatalog::default();
        catalog
            .define(crate::catalog::MetricDef {
                name: "login.online".into(),
                kind: MetricKind::Gauge,
                unit: Unit::Count,
                category: crate::catalog::Category::GatewayLogin,
                description: "online sessions".into(),
            })
            .expect("define");
        let series = ProviderSeries {
            metric: Labels::new(),
            samples: vec![Sample { timestamp_unix_ms: 0, value: 7.0 }],
        };
        let query = CollectedQuery {
            entry_index: 0,
            catalog_metric: "login.online".into(),
            service: "loginapp".into(),
            window: QueryWindow {
                start_unix_ms: 0,
                end_unix_ms: 1000,
                step_ms: 1000,
            },
            issued_at_unix_ms: 1000,
            outcome: CollectedOutcome::Series(vec![series]),
        };
        aggregate_collected(&catalog, &[query])
    }

    fn input<'a>(
        correlation: &'a CorrelationContext,
        timeline: &'a TimelineAlignment,
        spans: &'a [TimelineSpan],
        quality: &'a QualityReport,
        generator: &'a GeneratorAggregate,
        server: Vec<MetricAggregate>,
    ) -> CorrelationInput<'a> {
        CorrelationInput {
            correlation,
            timeline,
            source_spans: spans,
            quality,
            generator,
            generator_thresholds: SaturationThresholds::default(),
            server,
            observer_degraded: false,
            partial_observability: false,
        }
    }

    #[test]
    fn assembled_report_carries_all_planes_and_complete_labels() {
        let correlation = correlation();
        let timeline = timeline();
        let spans = vec![TimelineSpan {
            start_unix_ms: 1000,
            end_unix_ms: 9000,
        }];
        let quality = quality_clean();
        let generator = generator_clean();
        let report = CorrelationReport::assemble(input(
            &correlation,
            &timeline,
            &spans,
            &quality,
            &generator,
            server_aggregates(),
        ))
        .expect("assemble");

        assert_eq!(report.swell_id, "sw-correlated");
        assert_eq!(report.timeline_anchor, "swell-start");
        assert_eq!(
            report.effective_window,
            Some(TimelineSpan {
                start_unix_ms: 1000,
                end_unix_ms: 9000
            })
        );
        assert!(report.labels.is_complete());
        assert!(report.server_conclusion_allowed());
        assert!(!report.generator_saturated());
        assert_eq!(report.server.len(), 1);
    }

    #[test]
    fn empty_swell_id_is_refused() {
        let mut correlation = correlation();
        correlation.swell_id = "  ".into();
        let timeline = timeline();
        let spans = vec![];
        let quality = quality_clean();
        let generator = generator_clean();
        assert_eq!(
            CorrelationReport::assemble(input(
                &correlation,
                &timeline,
                &spans,
                &quality,
                &generator,
                vec![]
            )),
            Err(ReportError::EmptySwellId)
        );
    }

    #[test]
    fn declared_range_and_source_coverage_intersect() {
        let mut correlation = correlation();
        correlation.time_range = Some(crate::correlation::TimeRange {
            start_unix_ms: 3000,
            end_unix_ms: 4000,
        });
        let timeline = timeline();
        let spans = vec![TimelineSpan {
            start_unix_ms: 1000,
            end_unix_ms: 9000,
        }];
        let quality = quality_clean();
        let generator = generator_clean();
        let report = CorrelationReport::assemble(input(
            &correlation,
            &timeline,
            &spans,
            &quality,
            &generator,
            vec![],
        ))
        .expect("assemble");
        assert_eq!(
            report.effective_window,
            Some(TimelineSpan {
                start_unix_ms: 3000,
                end_unix_ms: 4000
            })
        );

        // A declared range outside coverage leaves no honest window.
        correlation.time_range = Some(crate::correlation::TimeRange {
            start_unix_ms: 20_000,
            end_unix_ms: 21_000,
        });
        let report = CorrelationReport::assemble(input(
            &correlation,
            &timeline,
            &spans,
            &quality,
            &generator,
            vec![],
        ))
        .expect("assemble");
        assert_eq!(report.effective_window, None);
    }

    #[test]
    fn inverted_correlation_time_range_is_reported_as_invalid() {
        let mut correlation = correlation();
        correlation.time_range = Some(crate::correlation::TimeRange {
            start_unix_ms: 5000,
            end_unix_ms: 4000,
        });
        let timeline = timeline();
        let spans = vec![TimelineSpan {
            start_unix_ms: 1000,
            end_unix_ms: 9000,
        }];
        let quality = quality_clean();
        let generator = generator_clean();
        let assembled =
            CorrelationReport::assemble(input(
                &correlation,
                &timeline,
                &spans,
                &quality,
                &generator,
                vec![],
            ));
        assert!(matches!(
            assembled,
            Err(ReportError::InvalidCorrelation(_))
        ));
    }

    #[test]
    fn partial_observability_forbids_server_conclusions() {
        let correlation = correlation();
        let timeline = timeline();
        let spans = vec![TimelineSpan {
            start_unix_ms: 0,
            end_unix_ms: 1000,
        }];
        let quality = quality_clean();
        let generator = generator_clean();
        let mut assembled_input = input(
            &correlation,
            &timeline,
            &spans,
            &quality,
            &generator,
            vec![],
        );
        assembled_input.partial_observability = true;
        let report = CorrelationReport::assemble(assembled_input)
            .expect("assemble");
        assert!(report
            .labels
            .contains(DataQualityLabel::Partial));
        assert!(!report.labels.is_complete());
        assert!(!report.server_conclusion_allowed());
    }

    #[test]
    fn saturation_evidence_flows_into_the_report() {
        let correlation = correlation();
        let timeline = timeline();
        let spans = vec![TimelineSpan {
            start_unix_ms: 0,
            end_unix_ms: 1000,
        }];
        let quality = quality_clean();
        let hot = WorkerSnapshot {
            worker_id: "worker-a".into(),
            cpu_ratio: Some(0.99),
            ..WorkerSnapshot::default()
        };
        let generator = crate::generator::aggregate(&[hot]);
        let report = CorrelationReport::assemble(input(
            &correlation,
            &timeline,
            &spans,
            &quality,
            &generator,
            vec![],
        ))
        .expect("assemble");
        assert!(report.generator_saturated());
        assert!(report
            .labels
            .contains(DataQualityLabel::GeneratorSaturated));
        assert_eq!(report.saturation.len(), 1);
    }

    #[test]
    fn missing_data_labels_the_report_and_widens_nothing() {
        let correlation = correlation();
        let timeline = timeline();
        let spans = vec![TimelineSpan {
            start_unix_ms: 0,
            end_unix_ms: 1000,
        }];
        let failed = CollectedQuery {
            entry_index: 0,
            catalog_metric: "login.online".into(),
            service: "loginapp".into(),
            window: QueryWindow {
                start_unix_ms: 0,
                end_unix_ms: 1000,
                step_ms: 1000,
            },
            issued_at_unix_ms: 0,
            outcome: CollectedOutcome::Failed("upstream 503".into()),
        };
        let quality = quality_report(&[failed], 5000);
        let generator = generator_clean();
        let report = CorrelationReport::assemble(input(
            &correlation,
            &timeline,
            &spans,
            &quality,
            &generator,
            vec![], // failed query -> no server aggregates
        ))
        .expect("assemble");
        assert!(report.labels.contains(DataQualityLabel::Missing));
        assert!(!report.labels.is_complete());
        assert!(report.server.is_empty());
    }

    #[test]
    fn report_serialises_round_trip() {
        let correlation = correlation();
        let mut timeline = timeline();
        timeline.add_clock(SourceClock::estimated(
            "prometheus",
            ClockOffsetEstimate {
                offset_ms: 30,
                method: "shape-alignment".into(),
            },
        ));
        let spans = vec![TimelineSpan {
            start_unix_ms: 1000,
            end_unix_ms: 9000,
        }];
        let quality = quality_clean();
        let generator = generator_clean();
        let report = CorrelationReport::assemble(input(
            &correlation,
            &timeline,
            &spans,
            &quality,
            &generator,
            server_aggregates(),
        ))
        .expect("assemble");
        assert!(report.labels.contains(DataQualityLabel::Estimated));

        let text = serde_yaml::to_string(&report).expect("serialise");
        assert!(text.contains("sw-correlated"));
        assert!(text.contains("shape-alignment"));
        let back: CorrelationReport = serde_yaml::from_str(&text).expect("reparse");
        assert_eq!(back, report);
    }
}
