//! Observability configuration (docs/basin.md "Providers" and "Preflight").
//!
//! The provider registry, metric catalog, mapping, and query scheduler are
//! later items (WM-0403+); this module owns the configuration that decides
//! HOW server observability runs and what happens when it cannot — the
//! Preflight policy half that no later item claims.

use serde::{Deserialize, Serialize};

use crate::BasinError;

/// Which provider kind serves the metric data (docs/basin.md "Providers":
/// Prometheus first, OTel and custom as needed).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderKind {
    Prometheus,
    #[serde(rename = "opentelemetry")]
    OpenTelemetry,
    /// A provider behind the unified custom interface; the payload names
    /// the implementation.
    Custom(String),
}

/// What a swell does when server observability is unavailable
/// (docs/basin.md "Preflight": the test MAY continue per configuration,
/// with the report marked `Partial Observability` and no complete
/// server-side performance conclusions).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UnavailablePolicy {
    /// Refuse to start the swell.
    Abort,
    /// Run, mark the report `Partial Observability`.
    MarkPartial,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ObservabilityConfig {
    /// true = a swell must not run without working observability.
    pub required: bool,
    pub provider: ProviderKind,
    /// Server metric collection cadence in milliseconds (the
    /// configuration's `collection_interval: 1s`). Must be positive —
    /// a zero interval cannot satisfy report needs (Preflight: sampling
    /// intervals must satisfy the report).
    pub collection_interval_ms: u64,
    pub on_unavailable: UnavailablePolicy,
}

impl ObservabilityConfig {
    pub fn validate(&self) -> Result<(), BasinError> {
        if self.collection_interval_ms == 0 {
            return Err(BasinError::ZeroCollectionInterval);
        }
        Ok(())
    }
}
