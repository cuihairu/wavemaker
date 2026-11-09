//! Basin — the Wavemaker data model for the environment under test and its
//! observability configuration (WM-0401; docs/basin.md).
//!
//! Basin represents the environment, its service topology, and its
//! observability setup. It is deliberately NOT a monitoring product: no
//! metric storage, no Prometheus re-implementation. The remaining
//! sub-models land in their own roadmap items (catalog WM-0403, providers
//! WM-0404, mapping WM-0405, scheduler WM-0406, timestamps WM-0407,
//! quality WM-0408); this crate owns the root model that ties them
//! together and the parts no later item claims.
//!
//! ```text
//! Basin
//!   ├── Environment
//!   ├── Topology
//!   ├── Observability
//!   └── Correlation Context
//! ```

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

pub mod alignment;
pub mod baseline;
pub mod catalog;
pub mod correlation;
pub mod environment;
pub mod generator;
pub mod labels;
pub mod mapping;
pub mod observability;
pub mod overhead;
pub mod preflight;
pub mod provider;
pub mod quality;
pub mod report;
pub mod scheduler;
pub mod server;
pub mod throughput;
pub mod timestamps;
pub mod topology;

pub use alignment::{
    common_window, shape_alignment_offset, AlignedStamp, ClockTrust,
    SourceClock, TimelineAlignment, TimelineSpan,
};
pub use baseline::{
    degrade_level, evaluate, ADR_PROPOSED_OVERHEAD_THRESHOLD,
    BaselineError, BaselineEvaluation, ObservationLevel, OverheadSample,
    OverheadSamples, BASELINE_WINDOW_MS,
};
pub use catalog::{CatalogError, Category, MetricCatalog, MetricDef, MetricKind, Unit};
pub use correlation::{CorrelationContext, TimeRange};
pub use generator::{
    aggregate, GeneratorAggregate, SaturationSignal, SaturationThresholds,
    WorkerSnapshot,
};
pub use labels::{DataQualityLabel, QualityEvidence, QualityLabelSet};
pub use mapping::{MappingEntry, MappingError, MetricMapping};
pub use provider::{
    HttpTransport, MetricProvider, PrometheusProvider, ProviderError,
    ProviderSeries, QueryWindow, Sample, TcpHttpTransport,
};
pub use scheduler::{
    scheduler_from_interval, entry_of, failed_queries, CollectedOutcome,
    CollectedQuery, QueryScheduler, ScheduledQuery,
};
pub use server::{
    aggregate_collected, aggregate_series, MetricAggregate, SampleSpan,
};
pub use timestamps::{
    compute_missing_intervals, ClockOffsetEstimate, CollectionMetadata,
    MissingInterval, TimestampError, TimestampRecord,
};
pub use throughput::{
    client_rate_series, server_rate_series, ThroughputError, ThroughputKind,
    ThroughputPoint, ThroughputSeries, ThroughputSource, ThroughputSummary,
};
pub use quality::{
    detect_delayed_samples, detect_missing_windows, quality_report,
    QualityFinding, QualityReport,
};
pub use report::{CorrelationInput, CorrelationReport, ReportError};
pub use preflight::{
    decision_for, CheckOutcome, Preflight, PreflightCheck, PreflightDecision,
    PreflightReport,
};
pub use environment::Environment;
pub use observability::{
    ObservabilityConfig, ProviderKind, UnavailablePolicy,
};
pub use overhead::{
    can_enable, grant_l2, observe_overhead, DegradationEvent,
    EnablementDecision, L2Grant, OverheadPolicy,
};
pub use topology::Topology;

/// The root data model (docs/basin.md "Basin's Role").
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Basin {
    /// The environment under test (docs/basin.md); the configuration's
    /// `basin.ref` names it.
    pub environment: Environment,
    /// The service topology of that environment (WM-0402).
    pub topology: Topology,
    /// The observability configuration (providers, catalog, mapping,
    /// scheduling land in WM-0403+; the policy that decides whether a
    /// swell may run without server observability lives here).
    pub observability: ObservabilityConfig,
    /// What later report items use to correlate generator, server, and
    /// experiment data (WM-0414/0416 extend the time-range semantics).
    pub correlation: CorrelationContext,
}

impl Basin {
    /// Structural validation — the invariants no in-place edit should be
    /// able to break.
    pub fn validate(&self) -> Result<(), BasinError> {
        if self.environment.id.trim().is_empty() {
            return Err(BasinError::EmptyEnvironmentId);
        }
        self.observability.validate()?;
        self.correlation.validate()?;
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum BasinError {
    #[error("environment id must not be empty")]
    EmptyEnvironmentId,
    #[error("collection_interval_ms must be positive")]
    ZeroCollectionInterval,
    #[error("time range start {0} is after end {1}")]
    InvertedTimeRange(i64, i64),
}

/// Convenience alias for environment labels (stable order for reports).
pub type Labels = BTreeMap<String, String>;

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Basin {
        Basin {
            environment: Environment {
                id: "staging".into(),
                labels: Labels::new(),
            },
            topology: Topology::default(),
            observability: ObservabilityConfig {
                required: true,
                provider: ProviderKind::Prometheus,
                collection_interval_ms: 1000,
                on_unavailable: UnavailablePolicy::MarkPartial,
            },
            correlation: CorrelationContext {
                swell_id: "sw-1".into(),
                time_range: None,
            },
        }
    }

    #[test]
    fn yaml_roundtrip_matches_the_configuration_shape() {
        // docs/configuration.md `basin:` block: ref names the environment,
        // observability carries the provider and interval.
        let yaml = r#"
environment:
  id: staging
topology: {}
observability:
  required: true
  provider: prometheus
  collection_interval_ms: 1000
  on_unavailable: mark_partial
correlation:
  swell_id: sw-staging-1
"#;
        let basin: Basin = serde_yaml::from_str(yaml).expect("parse");
        assert_eq!(basin.environment.id, "staging");
        assert_eq!(basin.observability.provider, ProviderKind::Prometheus);
        assert_eq!(basin.observability.collection_interval_ms, 1000);
        assert_eq!(basin.correlation.swell_id, "sw-staging-1");

        let text = serde_yaml::to_string(&basin).expect("serialise");
        let reparsed: Basin = serde_yaml::from_str(&text).expect("reparse");
        assert_eq!(reparsed, basin);
    }

    #[test]
    fn valid_basin_passes_validation() {
        sample().validate().expect("sample is valid");
    }

    #[test]
    fn empty_environment_id_is_rejected() {
        let mut basin = sample();
        basin.environment.id = "  ".into();
        assert_eq!(
            basin.validate(),
            Err(BasinError::EmptyEnvironmentId)
        );
    }

    #[test]
    fn zero_collection_interval_is_rejected() {
        let mut basin = sample();
        basin.observability.collection_interval_ms = 0;
        assert_eq!(
            basin.validate(),
            Err(BasinError::ZeroCollectionInterval)
        );
    }

    #[test]
    fn provider_kinds_round_trip_through_yaml() {
        let prometheus: ProviderKind =
            serde_yaml::from_str("prometheus").expect("parse");
        assert_eq!(prometheus, ProviderKind::Prometheus);
        assert_eq!(
            serde_yaml::to_string(&prometheus).expect("serialise").trim(),
            "prometheus"
        );

        let otel: ProviderKind =
            serde_yaml::from_str("opentelemetry").expect("parse");
        assert_eq!(otel, ProviderKind::OpenTelemetry);

        // serde_yaml 0.9 represents newtype variants as tagged scalars.
        let custom: ProviderKind =
            serde_yaml::from_str("!custom inhouse").expect("parse");
        assert_eq!(custom, ProviderKind::Custom("inhouse".into()));
        assert_eq!(
            serde_yaml::to_string(&custom).expect("serialise").trim(),
            "!custom inhouse"
        );
    }

    #[test]
    fn unavailable_policy_round_trips() {
        for (text, policy) in [
            ("abort", UnavailablePolicy::Abort),
            ("mark_partial", UnavailablePolicy::MarkPartial),
        ] {
            let parsed: UnavailablePolicy =
                serde_yaml::from_str(text).expect("parse");
            assert_eq!(parsed, policy);
        }
    }
}
