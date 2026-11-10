//! Data quality labels in reports (WM-0415;
//! docs/generator-observability.md "Data Quality Labels").
//!
//! "Reports must be able to show: Complete, Partial, Delayed, Missing,
//! Estimated, Invalid, GeneratorSaturated, ObserverDegraded. Missing data
//! must not default to zero."
//!
//! The vocabulary is the doc's own tokens (serde renames keep the exact
//! spellings on the wire); the derivations are wired to the machinery
//! built in WM-0408..0414 — nothing here invents a signal, it reads the
//! evidence those items already record.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use crate::alignment::TimelineAlignment;
use crate::generator::{GeneratorAggregate, SaturationThresholds};
use crate::quality::QualityReport;

/// The report-facing quality vocabulary, exactly as the doc spells it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum DataQualityLabel {
    Complete,
    Partial,
    Delayed,
    Missing,
    Estimated,
    Invalid,
    #[serde(rename = "GeneratorSaturated")]
    GeneratorSaturated,
    #[serde(rename = "ObserverDegraded")]
    ObserverDegraded,
}

/// The label set a report carries. BTreeSet keeps derivation order-free
/// and the wire form stable.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct QualityLabelSet {
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    labels: BTreeSet<DataQualityLabel>,
}

/// Evidence inputs for label derivation. Every field is the recorded
/// output of an earlier item; `None`/zero values simply do not trigger.
pub struct QualityEvidence<'a> {
    /// WM-0408's detection output.
    pub quality: &'a QualityReport,
    /// WM-0412's generator aggregate + its thresholds.
    pub generator: &'a GeneratorAggregate,
    pub generator_thresholds: SaturationThresholds,
    /// Whether observation degraded mid-run or at baseline (WM-0411/0410).
    pub observer_degraded: bool,
    /// Whether preflight decided ProceedPartialObservability (WM-0409).
    pub partial_observability: bool,
    /// WM-0414's alignment (estimate usage drives `Estimated`).
    pub alignment: &'a TimelineAlignment,
    /// Non-finite samples seen anywhere in the pipeline (server
    /// aggregates count them; WM-0413).
    pub invalid_samples: usize,
}

impl QualityLabelSet {
    pub fn labels(&self) -> impl Iterator<Item = &DataQualityLabel> {
        self.labels.iter()
    }

    pub fn contains(&self, label: DataQualityLabel) -> bool {
        self.labels.contains(&label)
    }

    /// Record one label directly — for callers whose evidence is not
    /// derivable from [`QualityEvidence`] (e.g. the assertion channel's
    /// own §5.3 coupling). `Complete` stays a conclusion: insert it only
    /// when nothing else applies.
    pub fn insert(&mut self, label: DataQualityLabel) {
        self.labels.insert(label);
    }

    /// Derive the report labels from recorded evidence. `Complete` is
    /// added ONLY when nothing else applied — it is a conclusion, not a
    /// default.
    pub fn derive(evidence: &QualityEvidence<'_>) -> Self {
        let mut set = Self::default();
        if evidence.quality.missing_windows() > 0 {
            set.labels.insert(DataQualityLabel::Missing);
        }
        if evidence.quality.delayed_samples() > 0 {
            set.labels.insert(DataQualityLabel::Delayed);
        }
        if !evidence
            .generator
            .saturation_signals(&evidence.generator_thresholds)
            .is_empty()
        {
            set.labels.insert(DataQualityLabel::GeneratorSaturated);
        }
        if evidence.observer_degraded {
            set.labels.insert(DataQualityLabel::ObserverDegraded);
        }
        if evidence.partial_observability {
            set.labels.insert(DataQualityLabel::Partial);
        }
        if evidence
            .alignment
            .clocks
            .values()
            .any(|clock| clock.offset.is_some())
        {
            set.labels.insert(DataQualityLabel::Estimated);
        }
        if evidence.invalid_samples > 0 {
            set.labels.insert(DataQualityLabel::Invalid);
        }
        if set.labels.is_empty() {
            set.labels.insert(DataQualityLabel::Complete);
        }
        set
    }

    /// Empty set and `Complete` mean the same final state, but Complete
    /// is the spelling reports use.
    pub fn is_complete(&self) -> bool {
        self.labels == BTreeSet::from([DataQualityLabel::Complete])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::alignment::SourceClock;
    use crate::generator::WorkerSnapshot;
    use crate::provider::QueryWindow;
    use crate::quality::quality_report;
    use crate::scheduler::{CollectedOutcome as SchedulerOutcome, CollectedQuery};
    use crate::timestamps::ClockOffsetEstimate;
    use crate::Labels;

    fn query_with(outcome: SchedulerOutcome) -> CollectedQuery {
        CollectedQuery {
            entry_index: 0,
            catalog_metric: "login.online".into(),
            service: "loginapp".into(),
            window: QueryWindow {
                start_unix_ms: 0,
                end_unix_ms: 1000,
                step_ms: 1000,
            },
            issued_at_unix_ms: 1000,
            outcome,
        }
    }

    fn clean_batch() -> Vec<CollectedQuery> {
        vec![query_with(SchedulerOutcome::Series(vec![
            crate::provider::ProviderSeries {
                metric: Labels::new(),
                samples: vec![crate::provider::Sample {
                    timestamp_unix_ms: 900,
                    value: 3.0,
                }],
            },
        ]))]
    }

    fn clean_generator() -> GeneratorAggregate {
        crate::generator::aggregate(&[WorkerSnapshot {
            worker_id: "worker-a".into(),
            at_unix_ms: 0,
            cpu_ratio: Some(0.1),
            ..WorkerSnapshot::default()
        }])
    }

    fn plain_alignment() -> TimelineAlignment {
        let mut alignment = TimelineAlignment::new("swell-start", 0);
        alignment.add_clock(SourceClock::trusted("controller"));
        alignment
    }

    fn evidence<'a>(
        quality: &'a QualityReport,
        generator: &'a GeneratorAggregate,
        alignment: &'a TimelineAlignment,
    ) -> QualityEvidence<'a> {
        QualityEvidence {
            quality,
            generator,
            generator_thresholds: SaturationThresholds::default(),
            observer_degraded: false,
            partial_observability: false,
            alignment,
            invalid_samples: 0,
        }
    }

    #[test]
    fn clean_run_is_complete_nothing_else() {
        let quality = quality_report(&clean_batch(), 5000);
        let generator = clean_generator();
        let alignment = plain_alignment();
        let set = QualityLabelSet::derive(&evidence(&quality, &generator, &alignment));
        assert!(set.is_complete());
        assert!(set.contains(DataQualityLabel::Complete));
        assert_eq!(set.labels().count(), 1);
    }

    #[test]
    fn missing_and_delayed_come_from_the_quality_report() {
        let failed = query_with(SchedulerOutcome::Failed("upstream 503".into()));
        let stale = query_with(SchedulerOutcome::Series(vec![
            crate::provider::ProviderSeries {
                metric: Labels::new(),
                samples: vec![crate::provider::Sample {
                    timestamp_unix_ms: 100,
                    value: 1.0,
                }],
            },
        ]));
        let quality = quality_report(&[failed, stale], 100);
        let generator = clean_generator();
        let alignment = plain_alignment();
        let set = QualityLabelSet::derive(&evidence(&quality, &generator, &alignment));
        assert!(set.contains(DataQualityLabel::Missing));
        assert!(set.contains(DataQualityLabel::Delayed));
        assert!(!set.contains(DataQualityLabel::Complete));
    }

    #[test]
    fn generator_saturation_and_observer_degradation_surface() {
        let hot = WorkerSnapshot {
            worker_id: "worker-a".into(),
            cpu_ratio: Some(0.99),
            ..WorkerSnapshot::default()
        };
        let generator = crate::generator::aggregate(&[hot]);
        let quality = quality_report(&clean_batch(), 5000);
        let alignment = plain_alignment();
        let mut inputs = evidence(&quality, &generator, &alignment);
        inputs.observer_degraded = true;
        let set = QualityLabelSet::derive(&inputs);
        assert!(set.contains(DataQualityLabel::GeneratorSaturated));
        assert!(set.contains(DataQualityLabel::ObserverDegraded));
    }

    #[test]
    fn partial_and_estimated_and_invalid_apply() {
        let quality = quality_report(&clean_batch(), 5000);
        let generator = clean_generator();
        let mut alignment = plain_alignment();
        alignment.add_clock(SourceClock::estimated(
            "prometheus",
            ClockOffsetEstimate {
                offset_ms: 30,
                method: "shape-alignment".into(),
            },
        ));
        let mut inputs = evidence(&quality, &generator, &alignment);
        inputs.partial_observability = true;
        inputs.invalid_samples = 2;
        let set = QualityLabelSet::derive(&inputs);
        assert!(set.contains(DataQualityLabel::Partial));
        assert!(set.contains(DataQualityLabel::Estimated));
        assert!(set.contains(DataQualityLabel::Invalid));
        assert!(!set.contains(DataQualityLabel::Complete));
    }

    #[test]
    fn label_tokens_keep_the_doc_spellings() {
        let saturated: DataQualityLabel =
            serde_yaml::from_str("GeneratorSaturated").expect("parse");
        assert_eq!(saturated, DataQualityLabel::GeneratorSaturated);
        let degraded: DataQualityLabel = serde_yaml::from_str("ObserverDegraded").expect("parse");
        assert_eq!(degraded, DataQualityLabel::ObserverDegraded);

        let quality = quality_report(&clean_batch(), 5000);
        let generator = clean_generator();
        let alignment = plain_alignment();
        let set = QualityLabelSet::derive(&evidence(&quality, &generator, &alignment));
        let text = serde_yaml::to_string(&set).expect("serialise");
        assert!(text.contains("Complete"));
        let back: QualityLabelSet = serde_yaml::from_str(&text).expect("reparse");
        assert_eq!(back, set);
    }
}
