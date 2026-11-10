//! Preflight (WM-0409; docs/basin.md "Preflight").
//!
//! Checks before a Swell starts, each driven by the primitives built in
//! WM-0401..0408 — no check is invented beyond what those can genuinely
//! answer. If Server Observability is unavailable: the test may continue
//! per configuration (`UnavailablePolicy::MarkPartial`), with the report
//! marked `Partial Observability`, or refuse (`Abort`).

use serde::{Deserialize, Serialize};

use crate::catalog::MetricCatalog;
use crate::mapping::MetricMapping;
use crate::observability::{ObservabilityConfig, UnavailablePolicy};
use crate::provider::{MetricProvider, QueryWindow};
use crate::topology::Topology;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum CheckOutcome {
    Passed,
    Failed {
        reason: String,
    },
    /// The check cannot answer from the available evidence — carried
    /// honestly instead of guessing a pass/fail.
    NotDeterminable {
        reason: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PreflightCheck {
    /// Check name, drawn from docs/basin.md's own checklist wording.
    pub name: String,
    pub outcome: CheckOutcome,
}

impl PreflightCheck {
    fn passed(name: &str) -> Self {
        Self {
            name: name.into(),
            outcome: CheckOutcome::Passed,
        }
    }

    fn failed(name: &str, reason: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            outcome: CheckOutcome::Failed {
                reason: reason.into(),
            },
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PreflightReport {
    pub checks: Vec<PreflightCheck>,
}

impl PreflightReport {
    /// True only when every check passed; a NotDeterminable counts as
    /// not-all-passed.
    pub fn all_passed(&self) -> bool {
        self.checks
            .iter()
            .all(|check| matches!(check.outcome, CheckOutcome::Passed))
    }

    pub fn failures(&self) -> Vec<&PreflightCheck> {
        self.checks
            .iter()
            .filter(|check| matches!(check.outcome, CheckOutcome::Failed { .. }))
            .collect()
    }

    pub fn check(&self, name: &str) -> Option<&PreflightCheck> {
        self.checks.iter().find(|check| check.name == name)
    }
}

/// What a swell may do after preflight, per its configured
/// `on_unavailable` policy.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "decision", rename_all = "snake_case")]
pub enum PreflightDecision {
    /// All checks passed.
    Proceed,
    /// Run, mark the report `Partial Observability` — per the doc, no
    /// complete server-side performance conclusions may be drawn from it.
    ProceedPartialObservability,
    /// Refuse to start the swell.
    Abort,
}

pub fn decision_for(report: &PreflightReport, policy: &UnavailablePolicy) -> PreflightDecision {
    if report.all_passed() {
        return PreflightDecision::Proceed;
    }
    match policy {
        UnavailablePolicy::Abort => PreflightDecision::Abort,
        UnavailablePolicy::MarkPartial => PreflightDecision::ProceedPartialObservability,
    }
}

/// Inputs preflight runs against. Bounds (`max_collection_interval_ms`,
/// `max_series_per_query`) are `None` when the report needs do not set a
/// numeric bar — the check then passes on what it CAN verify.
pub struct Preflight<'a> {
    pub catalog: &'a MetricCatalog,
    pub topology: &'a Topology,
    pub mapping: &'a MetricMapping,
    pub observability: &'a ObservabilityConfig,
    pub provider: &'a dyn MetricProvider,
    pub max_collection_interval_ms: Option<i64>,
    pub max_series_per_query: Option<usize>,
}

impl<'a> Preflight<'a> {
    /// Run all checks. `probe_window` is a small past window used to
    /// prove the query path works (query window availability, data
    /// source permissions, cardinality observation).
    pub fn run(&self, probe_window: &QueryWindow) -> PreflightReport {
        let mut checks = Vec::new();

        // "Providers are reachable."
        checks.push(match self.provider.ready() {
            Ok(()) => PreflightCheck::passed("providers_reachable"),
            Err(err) => PreflightCheck::failed("providers_reachable", err.to_string()),
        });

        // "Required metrics exist." (mapping references resolve in the
        // catalog — units/kinds live there.)
        let mut metric_gaps = Vec::new();
        for entry in self.mapping.entries() {
            if self.catalog.lookup(&entry.catalog_metric).is_none() {
                metric_gaps.push(format!("{} ({})", entry.catalog_metric, entry.service));
            }
        }
        checks.push(if metric_gaps.is_empty() {
            PreflightCheck::passed("required_metrics_exist")
        } else {
            PreflightCheck::failed(
                "required_metrics_exist",
                format!("missing catalog definitions: {}", metric_gaps.join(", ")),
            )
        });

        // "Metric units and type mappings are correct." + structural
        // sanity on both sides of every binding.
        checks.push(match self.mapping.validate(self.catalog, self.topology) {
            Ok(()) => PreflightCheck::passed("mappings_valid"),
            Err(err) => PreflightCheck::failed("mappings_valid", err.to_string()),
        });

        // "The target topology is fully covered."
        let uncovered = self.mapping.uncovered_services(self.topology);
        checks.push(if uncovered.is_empty() {
            PreflightCheck::passed("topology_fully_covered")
        } else {
            PreflightCheck::failed(
                "topology_fully_covered",
                format!(
                    "services without any metric mapping: {}",
                    uncovered.join(", ")
                ),
            )
        });

        // "Sampling intervals satisfy report needs."
        checks.push({
            let interval = self.observability.collection_interval_ms as i64;
            if interval <= 0 {
                PreflightCheck::failed(
                    "sampling_interval_satisfies_report",
                    format!("collection interval must be positive, got {interval}"),
                )
            } else if let Some(max) = self.max_collection_interval_ms {
                if interval > max {
                    PreflightCheck::failed(
                        "sampling_interval_satisfies_report",
                        format!(
                            "collection interval {interval}ms exceeds the report bound {max}ms"
                        ),
                    )
                } else {
                    PreflightCheck::passed("sampling_interval_satisfies_report")
                }
            } else {
                PreflightCheck::passed("sampling_interval_satisfies_report")
            }
        });

        // One probe feeds three checks: "Query windows are available",
        // "Monitoring data source permissions are sufficient", and the
        // "high-cardinality label risks" observation.
        let probe = self.mapping.entries().first().map(|entry| {
            (
                entry.provider_query.clone(),
                self.provider
                    .query_range(&entry.provider_query, probe_window),
            )
        });
        match probe {
            None => {
                for name in [
                    "query_window_available",
                    "data_source_permissions",
                    "no_high_cardinality_surprise",
                ] {
                    checks.push(PreflightCheck {
                        name: name.into(),
                        outcome: CheckOutcome::NotDeterminable {
                            reason: "no metric mapping entries to probe".into(),
                        },
                    });
                }
            }
            Some((_query, Err(err))) => {
                // The reason travels verbatim — permission denials (403
                // etc.) show up in it without us pretending to classify
                // what the provider did not say.
                let reason = err.to_string();
                checks.push(PreflightCheck::failed(
                    "query_window_available",
                    reason.clone(),
                ));
                checks.push(PreflightCheck::failed("data_source_permissions", reason));
                checks.push(PreflightCheck {
                    name: "no_high_cardinality_surprise".into(),
                    outcome: CheckOutcome::NotDeterminable {
                        reason: "probe query failed; cardinality unobserved".into(),
                    },
                });
            }
            Some((_query, Ok(series))) => {
                checks.push(PreflightCheck::passed("query_window_available"));
                // A successful read IS the permission proof available to
                // us; failures surface their reason verbatim above.
                checks.push(PreflightCheck::passed("data_source_permissions"));
                checks.push(match self.max_series_per_query {
                    Some(max) if series.len() > max => PreflightCheck::failed(
                        "no_high_cardinality_surprise",
                        format!(
                            "probe returned {} series, exceeding the bound {max}",
                            series.len()
                        ),
                    ),
                    _ => PreflightCheck::passed("no_high_cardinality_surprise"),
                });
            }
        }

        PreflightReport { checks }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::{Category, MetricDef, MetricKind, Unit};
    use crate::mapping::MappingEntry;
    use crate::provider::ProviderError;
    use crate::provider::ProviderSeries;
    use crate::Labels;

    fn full_catalog() -> MetricCatalog {
        let mut catalog = MetricCatalog::default();
        catalog
            .define(MetricDef {
                name: "login.online".into(),
                kind: MetricKind::Gauge,
                unit: Unit::Count,
                category: Category::GatewayLogin,
                description: "online sessions at the login path".into(),
            })
            .expect("define");
        catalog
            .define(MetricDef {
                name: "game.online".into(),
                kind: MetricKind::Gauge,
                unit: Unit::Count,
                category: Category::Game,
                description: "online players in game services".into(),
            })
            .expect("define");
        catalog
    }

    fn kbengine_topology() -> Topology {
        let mut topology = Topology::default();
        topology
            .add_host(crate::topology::Host {
                id: "host-a".into(),
                labels: Labels::new(),
            })
            .expect("host");
        for (id, kind) in [
            ("loginapp", crate::topology::ServiceKind::Login),
            ("cellapp", crate::topology::ServiceKind::Game),
        ] {
            topology
                .add_service(crate::topology::Service {
                    id: id.into(),
                    kind,
                    host: Some("host-a".into()),
                    labels: Labels::new(),
                })
                .expect("service");
        }
        topology
    }

    fn full_mapping() -> MetricMapping {
        let mut mapping = MetricMapping::default();
        for (metric, service, query) in [
            (
                "login.online",
                "loginapp",
                "kbengine_online{component=\"loginapp\"}",
            ),
            (
                "game.online",
                "cellapp",
                "kbengine_online{component=\"cellapp\"}",
            ),
        ] {
            mapping
                .add_entry(MappingEntry {
                    catalog_metric: metric.into(),
                    service: service.into(),
                    provider_query: query.into(),
                    labels: Labels::new(),
                })
                .expect("entry");
        }
        mapping
    }

    fn observability(policy: UnavailablePolicy) -> ObservabilityConfig {
        ObservabilityConfig {
            required: true,
            provider: crate::observability::ProviderKind::Prometheus,
            collection_interval_ms: 1000,
            on_unavailable: policy,
        }
    }

    fn one_series() -> Vec<ProviderSeries> {
        vec![ProviderSeries {
            metric: Labels::new(),
            samples: vec![crate::provider::Sample {
                timestamp_unix_ms: 0,
                value: 1.0,
            }],
        }]
    }

    /// Healthy provider: ready + one series per query.
    struct HealthyProvider;

    impl MetricProvider for HealthyProvider {
        fn query_range(
            &self,
            _query: &str,
            _window: &QueryWindow,
        ) -> Result<Vec<ProviderSeries>, ProviderError> {
            Ok(one_series())
        }

        fn ready(&self) -> Result<(), ProviderError> {
            Ok(())
        }
    }

    struct UnreadyProvider;

    impl MetricProvider for UnreadyProvider {
        fn query_range(
            &self,
            _query: &str,
            _window: &QueryWindow,
        ) -> Result<Vec<ProviderSeries>, ProviderError> {
            Err(ProviderError::Unreachable("connection refused".into()))
        }

        fn ready(&self) -> Result<(), ProviderError> {
            Err(ProviderError::Unreachable("connection refused".into()))
        }
    }

    struct ForbiddenProvider;

    impl MetricProvider for ForbiddenProvider {
        fn query_range(
            &self,
            _query: &str,
            _window: &QueryWindow,
        ) -> Result<Vec<ProviderSeries>, ProviderError> {
            Err(ProviderError::BadStatus(403))
        }

        fn ready(&self) -> Result<(), ProviderError> {
            Ok(())
        }
    }

    fn probe_window() -> QueryWindow {
        QueryWindow {
            start_unix_ms: 0,
            end_unix_ms: 1000,
            step_ms: 1000,
        }
    }

    fn preflight<'a>(
        provider: &'a dyn MetricProvider,
        catalog: &'a MetricCatalog,
        topology: &'a Topology,
        mapping: &'a MetricMapping,
        obs: &'a ObservabilityConfig,
    ) -> Preflight<'a> {
        Preflight {
            catalog,
            topology,
            mapping,
            observability: obs,
            provider,
            max_collection_interval_ms: Some(5000),
            max_series_per_query: Some(100),
        }
    }

    #[test]
    fn healthy_stack_passes_every_check() {
        let catalog = full_catalog();
        let topology = kbengine_topology();
        let mapping = full_mapping();
        let obs = observability(UnavailablePolicy::Abort);
        let provider = HealthyProvider;
        let report = preflight(&provider, &catalog, &topology, &mapping, &obs).run(&probe_window());
        assert!(report.all_passed(), "failures: {:?}", report.failures());
        assert_eq!(report.checks.len(), 8);
        for name in [
            "providers_reachable",
            "required_metrics_exist",
            "mappings_valid",
            "topology_fully_covered",
            "sampling_interval_satisfies_report",
            "query_window_available",
            "data_source_permissions",
            "no_high_cardinality_surprise",
        ] {
            assert!(report.check(name).is_some(), "missing check {name}");
        }
        assert_eq!(
            decision_for(&report, &obs.on_unavailable),
            PreflightDecision::Proceed
        );
    }

    #[test]
    fn unreachable_provider_fails_and_aborts_per_policy() {
        let catalog = full_catalog();
        let topology = kbengine_topology();
        let mapping = full_mapping();
        let obs = observability(UnavailablePolicy::Abort);
        let provider = UnreadyProvider;
        let report = preflight(&provider, &catalog, &topology, &mapping, &obs).run(&probe_window());
        assert!(!report.all_passed());
        let failed_names: Vec<&str> = report
            .failures()
            .iter()
            .map(|check| check.name.as_str())
            .collect();
        assert!(failed_names.contains(&"providers_reachable"));
        assert!(failed_names.contains(&"query_window_available"));
        assert_eq!(
            decision_for(&report, &obs.on_unavailable),
            PreflightDecision::Abort
        );

        let obs_partial = observability(UnavailablePolicy::MarkPartial);
        assert_eq!(
            decision_for(&report, &obs_partial.on_unavailable),
            PreflightDecision::ProceedPartialObservability
        );
    }

    #[test]
    fn topology_gap_and_missing_metrics_fail_with_names() {
        let catalog = full_catalog();
        let mut topology = kbengine_topology();
        topology
            .add_service(crate::topology::Service {
                id: "dbmgr".into(),
                kind: crate::topology::ServiceKind::Database,
                host: Some("host-a".into()),
                labels: Labels::new(),
            })
            .expect("service");
        let mut mapping = full_mapping();
        mapping
            .add_entry(MappingEntry {
                catalog_metric: "ghost.metric".into(),
                service: "loginapp".into(),
                provider_query: "q".into(),
                labels: Labels::new(),
            })
            .expect("entry");
        let obs = observability(UnavailablePolicy::Abort);
        let provider = HealthyProvider;
        let report = preflight(&provider, &catalog, &topology, &mapping, &obs).run(&probe_window());
        let failed: Vec<&str> = report
            .failures()
            .iter()
            .map(|check| check.name.as_str())
            .collect();
        assert!(failed.contains(&"required_metrics_exist"));
        assert!(failed.contains(&"topology_fully_covered"));
        let coverage = report
            .check("topology_fully_covered")
            .expect("coverage check present");
        match &coverage.outcome {
            CheckOutcome::Failed { reason } => {
                assert!(reason.contains("dbmgr"), "reason: {reason}");
            }
            other => panic!("expected Failed, got {other:?}"),
        }
    }

    #[test]
    fn oversized_sampling_interval_fails_the_report_bound() {
        let catalog = full_catalog();
        let topology = kbengine_topology();
        let mapping = full_mapping();
        let mut obs = observability(UnavailablePolicy::Abort);
        obs.collection_interval_ms = 10_000; // bound is 5000
        let provider = HealthyProvider;
        let report = preflight(&provider, &catalog, &topology, &mapping, &obs).run(&probe_window());
        let check = report
            .check("sampling_interval_satisfies_report")
            .expect("interval check");
        match &check.outcome {
            CheckOutcome::Failed { reason } => {
                assert!(reason.contains("10000"), "reason: {reason}");
            }
            other => panic!("expected Failed, got {other:?}"),
        }
    }

    #[test]
    fn permission_denial_is_named_precisely() {
        let catalog = full_catalog();
        let topology = kbengine_topology();
        let mapping = full_mapping();
        let obs = observability(UnavailablePolicy::Abort);
        let provider = ForbiddenProvider;
        let report = preflight(&provider, &catalog, &topology, &mapping, &obs).run(&probe_window());
        let check = report
            .check("data_source_permissions")
            .expect("permissions check");
        match &check.outcome {
            CheckOutcome::Failed { reason } => {
                assert!(reason.contains("403"), "reason: {reason}");
            }
            other => panic!("expected Failed, got {other:?}"),
        }
        // Reachability passed (ready was ok); the query path is what
        // failed.
        assert_eq!(
            report.check("providers_reachable").expect("ready").outcome,
            CheckOutcome::Passed
        );
    }

    #[test]
    fn cardinality_bound_is_enforced_from_probe_observation() {
        struct ManySeriesProvider;

        impl MetricProvider for ManySeriesProvider {
            fn query_range(
                &self,
                _query: &str,
                _window: &QueryWindow,
            ) -> Result<Vec<ProviderSeries>, ProviderError> {
                Ok((0..5)
                    .map(|_| ProviderSeries {
                        metric: Labels::new(),
                        samples: vec![],
                    })
                    .collect())
            }

            fn ready(&self) -> Result<(), ProviderError> {
                Ok(())
            }
        }

        let catalog = full_catalog();
        let topology = kbengine_topology();
        let mapping = full_mapping();
        let obs = observability(UnavailablePolicy::Abort);
        let provider = ManySeriesProvider;
        let mut pre = preflight(&provider, &catalog, &topology, &mapping, &obs);
        pre.max_series_per_query = Some(2);
        let report = pre.run(&probe_window());
        let check = report
            .check("no_high_cardinality_surprise")
            .expect("cardinality check");
        match &check.outcome {
            CheckOutcome::Failed { reason } => {
                assert!(reason.contains("5 series"), "reason: {reason}");
            }
            other => panic!("expected Failed, got {other:?}"),
        }
    }

    #[test]
    fn empty_mapping_reports_its_checks_as_not_determinable() {
        let catalog = full_catalog();
        let topology = kbengine_topology();
        let mapping = MetricMapping::default();
        let obs = observability(UnavailablePolicy::Abort);
        let provider = HealthyProvider;
        let report = preflight(&provider, &catalog, &topology, &mapping, &obs).run(&probe_window());
        for name in [
            "query_window_available",
            "data_source_permissions",
            "no_high_cardinality_surprise",
        ] {
            match report.check(name).expect(name).outcome {
                CheckOutcome::NotDeterminable { .. } => {}
                ref other => panic!("{name} should be not determinable, got {other:?}"),
            }
        }
        // NotDeterminable still blocks a clean pass.
        assert!(!report.all_passed());
        let text = serde_yaml::to_string(&report).expect("serialise");
        assert!(text.contains("not_determinable"));
        let back: PreflightReport = serde_yaml::from_str(&text).expect("reparse");
        assert_eq!(back, report);
    }
}
