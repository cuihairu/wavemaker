//! Metric Mapping (WM-0405; docs/basin.md Observability tree, Preflight
//! "Metric units and type mappings are correct" and "The target topology
//! is fully covered").
//!
//! The mapping binds RAW provider series (a query/selectors as the
//! provider understands them, WM-0404) onto CATALOG metrics (canonical
//! meaning, WM-0403) in the context of a declared topology service
//! (WM-0402). Units and kinds are NOT restated here — the catalog is the
//! single source of semantics; validating that the reference resolves IS
//! the correctness check.

use serde::{Deserialize, Serialize};

use crate::catalog::{CatalogError, MetricCatalog};
use crate::topology::{Topology, TopologyError};
use crate::Labels;

/// One binding: the raw provider query for a catalog metric, scoped to a
/// topology service.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MappingEntry {
    /// Canonical catalog name — the meaning side of the binding.
    pub catalog_metric: String,
    /// The declared topology service this series describes.
    pub service: String,
    /// Raw provider query exactly as the provider understands it
    /// (e.g. PromQL); passed through verbatim, never interpreted here.
    pub provider_query: String,
    /// Static label expectations recorded for the report (which series
    /// instance was meant); not a filter contract — the query carries
    /// matching. Omitted when empty.
    #[serde(default, skip_serializing_if = "Labels::is_empty")]
    pub labels: Labels,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum MappingError {
    #[error("mapping for service {service} references unknown catalog metric {metric}")]
    UnknownCatalogMetric { service: String, metric: String },
    #[error("mapping references undeclared service {0}")]
    UnknownService(String),
    #[error("mapping for {service}/{metric} has an empty provider query")]
    EmptyQuery { service: String, metric: String },
    #[error("duplicate mapping for service {service} metric {metric}")]
    DuplicateMapping { service: String, metric: String },
    #[error("catalog error: {0}")]
    Catalog(#[from] CatalogError),
    #[error("topology error: {0}")]
    Topology(#[from] TopologyError),
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct MetricMapping {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    entries: Vec<MappingEntry>,
}

impl MetricMapping {
    /// Add a binding; duplicates (same service + catalog metric) are
    /// refused — a silent rebind would change a report's meaning without
    /// anyone noticing.
    pub fn add_entry(
        &mut self,
        entry: MappingEntry,
    ) -> Result<(), MappingError> {
        if entry.provider_query.trim().is_empty() {
            return Err(MappingError::EmptyQuery {
                service: entry.service.clone(),
                metric: entry.catalog_metric.clone(),
            });
        }
        if entry.catalog_metric.trim().is_empty() {
            // An empty metric name cannot resolve against any catalog.
            return Err(MappingError::UnknownCatalogMetric {
                service: entry.service.clone(),
                metric: entry.catalog_metric,
            });
        }
        if entry.service.trim().is_empty() {
            return Err(MappingError::UnknownService(entry.service));
        }
        if self.entries.iter().any(|existing| {
            existing.catalog_metric == entry.catalog_metric
                && existing.service == entry.service
        }) {
            return Err(MappingError::DuplicateMapping {
                service: entry.service,
                metric: entry.catalog_metric,
            });
        }
        self.entries.push(entry);
        Ok(())
    }

    /// Validate every binding against the catalog and topology: the
    /// catalog metric must exist (units/kinds live there), the service
    /// must be declared. Also runs the topology's own structural
    /// validation so a broken environment is never reported as a
    /// coverage gap.
    pub fn validate(
        &self,
        catalog: &MetricCatalog,
        topology: &Topology,
    ) -> Result<(), MappingError> {
        topology.validate()?;
        for entry in &self.entries {
            if catalog.lookup(&entry.catalog_metric).is_none() {
                return Err(MappingError::UnknownCatalogMetric {
                    service: entry.service.clone(),
                    metric: entry.catalog_metric.clone(),
                });
            }
            if topology.service(&entry.service).is_none() {
                return Err(MappingError::UnknownService(entry.service.clone()));
            }
        }
        Ok(())
    }

    /// Whether the service has at least one binding — the coverage
    /// predicate Preflight (WM-0411) drives "The target topology is fully
    /// covered" with.
    pub fn covers(&self, service_id: &str) -> bool {
        self.entries
            .iter()
            .any(|entry| entry.service == service_id)
    }

    pub fn covered_services(&self) -> Vec<String> {
        let mut covered: Vec<String> =
            self.entries.iter().map(|entry| entry.service.clone()).collect();
        covered.sort();
        covered.dedup();
        covered
    }

    /// Services of the topology without any binding — Preflight's gap
    /// list, in topology id order.
    pub fn uncovered_services(&self, topology: &Topology) -> Vec<String> {
        topology.uncovered_services(|service| self.covers(&service.id))
    }

    pub fn entries_by_service(
        &self,
        service_id: &str,
    ) -> Vec<&MappingEntry> {
        self.entries
            .iter()
            .filter(|entry| entry.service == service_id)
            .collect()
    }

    pub fn entries(&self) -> &[MappingEntry] {
        &self.entries
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::{Category, MetricDef, MetricKind, Unit};
    use crate::topology::{Host, Service, ServiceKind};

    fn catalog_with(name: &str, category: Category) -> MetricCatalog {
        let mut catalog = MetricCatalog::default();
        catalog
            .define(MetricDef {
                name: name.into(),
                kind: MetricKind::Gauge,
                unit: Unit::Count,
                category,
                description: format!("meaning of {name}"),
            })
            .expect("define");
        catalog
    }

    fn topology_with(services: &[(&str, ServiceKind)]) -> Topology {
        let mut topology = Topology::default();
        topology
            .add_host(Host { id: "host-a".into(), labels: Labels::new() })
            .expect("host");
        for &(id, ref kind) in services {
            topology
                .add_service(Service {
                    id: id.into(),
                    kind: kind.clone(),
                    host: Some("host-a".into()),
                    labels: Labels::new(),
                })
                .expect("service");
        }
        topology
    }

    fn entry(metric: &str, service: &str, query: &str) -> MappingEntry {
        MappingEntry {
            catalog_metric: metric.into(),
            service: service.into(),
            provider_query: query.into(),
            labels: Labels::new(),
        }
    }

    #[test]
    fn binds_raw_queries_to_catalog_meanings() {
        let mut mapping = MetricMapping::default();
        mapping
            .add_entry(entry(
                "login.online",
                "loginapp",
                r#"kbengine_online{component="loginapp"}"#,
            ))
            .expect("bind");
        let bound = &mapping.entries()[0];
        assert_eq!(bound.catalog_metric, "login.online");
        assert_eq!(bound.provider_query, r#"kbengine_online{component="loginapp"}"#);
    }

    #[test]
    fn duplicate_bindings_are_refused() {
        let mut mapping = MetricMapping::default();
        mapping
            .add_entry(entry("login.online", "loginapp", "kbengine_online"))
            .expect("first");
        assert_eq!(
            mapping.add_entry(entry("login.online", "loginapp", "other_query")),
            Err(MappingError::DuplicateMapping {
                service: "loginapp".into(),
                metric: "login.online".into(),
            })
        );
        // A different metric on the same service, or the same metric on
        // another service, is a distinct binding.
        mapping
            .add_entry(entry("login.rps", "loginapp", "rate(login_total)"))
            .expect("second metric on same service");
        mapping
            .add_entry(entry("login.online", "cellapp", "kbengine_online"))
            .expect("same metric on other service");
    }

    #[test]
    fn empty_query_or_references_are_refused() {
        let mut mapping = MetricMapping::default();
        assert!(matches!(
            mapping.add_entry(entry("login.online", "loginapp", "  ")),
            Err(MappingError::EmptyQuery { .. })
        ));
        assert!(matches!(
            mapping.add_entry(entry("", "loginapp", "q")),
            Err(MappingError::UnknownCatalogMetric { .. })
        ));
        assert!(matches!(
            mapping.add_entry(entry("login.online", "  ", "q")),
            Err(MappingError::UnknownService(_))
        ));
    }

    #[test]
    fn validation_checks_both_sides_of_the_binding() {
        let catalog = catalog_with("login.online", Category::GatewayLogin);
        let topology = topology_with(&[("loginapp", ServiceKind::Login)]);
        let mut mapping = MetricMapping::default();
        mapping
            .add_entry(entry("ghost.metric", "loginapp", "q"))
            .expect("add");
        assert_eq!(
            mapping.validate(&catalog, &topology),
            Err(MappingError::UnknownCatalogMetric {
                service: "loginapp".into(),
                metric: "ghost.metric".into(),
            })
        );

        let mut mapping = MetricMapping::default();
        mapping
            .add_entry(entry("login.online", "nosuch", "q"))
            .expect("add");
        assert_eq!(
            mapping.validate(&catalog, &topology),
            Err(MappingError::UnknownService("nosuch".into()))
        );

        let mut mapping = MetricMapping::default();
        mapping
            .add_entry(entry("login.online", "loginapp", "kbengine_online"))
            .expect("add");
        mapping.validate(&catalog, &topology).expect("valid mapping");
    }

    #[test]
    fn coverage_answers_the_topology_gap() {
        let catalog = catalog_with("login.online", Category::GatewayLogin);
        let topology = topology_with(&[
            ("loginapp", ServiceKind::Login),
            ("cellapp", ServiceKind::Game),
            ("dbmgr", ServiceKind::Database),
        ]);
        let mut mapping = MetricMapping::default();
        mapping
            .add_entry(entry("login.online", "loginapp", "q1"))
            .expect("bind loginapp");
        mapping
            .add_entry(entry(
                "login.online",
                "cellapp",
                "kbengine_online{component=\"cellapp\"}",
            ))
            .expect("bind cellapp");

        mapping.validate(&catalog, &topology).expect("valid");
        assert_eq!(
            mapping.uncovered_services(&topology),
            vec!["dbmgr".to_string()]
        );
        assert_eq!(
            mapping.covered_services(),
            vec!["cellapp".to_string(), "loginapp".to_string()]
        );
        assert!(mapping.covers("loginapp"));
        assert!(!mapping.covers("dbmgr"));
        assert_eq!(mapping.entries_by_service("cellapp").len(), 1);
    }

    #[test]
    fn broken_topology_is_reported_before_coverage() {
        let catalog = MetricCatalog::default();
        let mut topology = Topology::default();
        topology
            .add_service(Service {
                id: "dbmgr".into(),
                kind: ServiceKind::Database,
                host: Some("ghost".into()),
                labels: Labels::new(),
            })
            .expect("service");
        let mapping = MetricMapping::default();
        assert!(matches!(
            mapping.validate(&catalog, &topology),
            Err(MappingError::Topology(
                TopologyError::UnknownHostRef(_, _)
            ))
        ));
    }

    #[test]
    fn mapping_serialises_round_trip() {
        let mut mapping = MetricMapping::default();
        let mut labels = Labels::new();
        labels.insert("component".into(), "loginapp".into());
        mapping
            .add_entry(MappingEntry {
                catalog_metric: "login.online".into(),
                service: "loginapp".into(),
                provider_query: "kbengine_online".into(),
                labels,
            })
            .expect("bind");
        let text = serde_yaml::to_string(&mapping).expect("serialise");
        assert!(text.contains("kbengine_online"));
        let back: MetricMapping = serde_yaml::from_str(&text).expect("reparse");
        assert_eq!(back, mapping);
    }
}
