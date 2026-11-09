//! Unified metric catalog (WM-0403; docs/basin.md "Metric Categories" and
//! the closing rule: "Metrics need a unified unit and semantics catalog;
//! meaning must not be guessed from Prometheus metric names alone").
//!
//! The catalog is the SEMANTIC registry: canonical names, units, kinds,
//! and categories. Providers (WM-0404) keep serving their raw series;
//! the mapping item (WM-0405) binds raw provider names onto catalog
//! entries — never the other way round.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// The semantic domains of docs/basin.md, kept as written there
/// ("Gateway / Login", "Queue / RPC", "Database / Cache" are the doc's
/// own groupings).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Category {
    Host,
    Process,
    GatewayLogin,
    Game,
    QueueRpc,
    DatabaseCache,
    Business,
}

/// What the values mean across samples.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MetricKind {
    /// Monotonically increasing total (rates are derived at read time).
    Counter,
    /// Point-in-time value that can go up and down.
    Gauge,
    /// Distribution of observations (latency-style metrics).
    Histogram,
}

/// Units are a closed set so reports can compare and convert honestly;
/// anything else must name itself rather than pretend to a known unit.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Unit {
    Count,
    Seconds,
    Milliseconds,
    Bytes,
    Ratio,
    /// A unit the vocabulary does not cover yet; the payload names it.
    Other(String),
}

/// One catalog entry: a canonical metric with its meaning spelled out.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MetricDef {
    /// Canonical catalog name (dot-separated, stable across providers).
    pub name: String,
    pub kind: MetricKind,
    pub unit: Unit,
    pub category: Category,
    /// The meaning in words — the catalog exists so consumers never have
    /// to guess it, so it is mandatory and must be non-empty.
    pub description: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct MetricCatalog {
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    metrics: BTreeMap<String, MetricDef>,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CatalogError {
    #[error("duplicate metric definition: {0}")]
    DuplicateMetric(String),
    #[error("metric name must not be empty")]
    EmptyName,
    #[error("metric {0} needs a non-empty description")]
    EmptyDescription(String),
}

impl MetricCatalog {
    /// Register a metric definition; duplicates are refused because a
    /// silent redefinition is exactly the "meaning guessed from a name"
    /// failure this catalog exists to prevent.
    pub fn define(&mut self, def: MetricDef) -> Result<(), CatalogError> {
        if def.name.trim().is_empty() {
            return Err(CatalogError::EmptyName);
        }
        if def.description.trim().is_empty() {
            return Err(CatalogError::EmptyDescription(def.name));
        }
        if self.metrics.contains_key(&def.name) {
            return Err(CatalogError::DuplicateMetric(def.name));
        }
        self.metrics.insert(def.name.clone(), def);
        Ok(())
    }

    pub fn lookup(&self, name: &str) -> Option<&MetricDef> {
        self.metrics.get(name)
    }

    pub fn by_category(&self, category: &Category) -> Vec<&MetricDef> {
        self.metrics
            .values()
            .filter(|def| def.category == *category)
            .collect()
    }

    pub fn len(&self) -> usize {
        self.metrics.len()
    }

    pub fn is_empty(&self) -> bool {
        self.metrics.is_empty()
    }

    /// The metrics Wavemaker's own generator side already produces: the
    /// Bridge heartbeat's process block (WM-0214) and bridge metrics
    /// (WM-0215). Seeded so catalog-driven reports can cover the
    /// generator plane from day one; server-side entries arrive with
    /// their providers.
    pub fn with_generator_metrics() -> Self {
        let mut catalog = MetricCatalog::default();
        let defs = [
            (
                "process.cpu_seconds",
                MetricKind::Counter,
                Unit::Seconds,
                "Total CPU time used by the observed process.",
            ),
            (
                "process.rss_bytes",
                MetricKind::Gauge,
                Unit::Bytes,
                "Resident set size of the observed process.",
            ),
            (
                "process.fd_count",
                MetricKind::Gauge,
                Unit::Count,
                "Open file descriptors of the observed process.",
            ),
            (
                "queue.depth",
                MetricKind::Gauge,
                Unit::Count,
                "Pending items in the bridge inbound command queue.",
            ),
            (
                "queue.dropped",
                MetricKind::Counter,
                Unit::Count,
                "Items dropped by bounded inbound queue overflow.",
            ),
            (
                "process.callback_lag_ms",
                MetricKind::Gauge,
                Unit::Milliseconds,
                "How late the last scripting-thread callback fired versus its due time.",
            ),
            (
                "process.callback_lag_max_ms",
                MetricKind::Gauge,
                Unit::Milliseconds,
                "Worst callback lag observed in the current max window.",
            ),
        ];
        let categories = [
            Category::Process,
            Category::Process,
            Category::Process,
            Category::QueueRpc,
            Category::QueueRpc,
            Category::Process,
            Category::Process,
        ];
        for ((name, kind, unit, description), category) in
            defs.into_iter().zip(categories)
        {
            catalog
                .define(MetricDef {
                    name: name.into(),
                    kind,
                    unit,
                    category,
                    description: description.into(),
                })
                .expect("seed definitions are valid");
        }
        catalog
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn def(name: &str, category: Category) -> MetricDef {
        MetricDef {
            name: name.into(),
            kind: MetricKind::Gauge,
            unit: Unit::Count,
            category,
            description: format!("meaning of {name}"),
        }
    }

    #[test]
    fn define_and_lookup_round_trip() {
        let mut catalog = MetricCatalog::default();
        catalog
            .define(def("login.success_total", Category::GatewayLogin))
            .expect("define");
        let found = catalog.lookup("login.success_total").expect("found");
        assert_eq!(found.category, Category::GatewayLogin);
        assert!(catalog.lookup("nope").is_none());
    }

    #[test]
    fn duplicate_definitions_are_refused() {
        let mut catalog = MetricCatalog::default();
        catalog
            .define(def("process.rss_bytes", Category::Process))
            .expect("first");
        assert_eq!(
            catalog.define(def("process.rss_bytes", Category::Process)),
            Err(CatalogError::DuplicateMetric(
                "process.rss_bytes".into()
            ))
        );
    }

    #[test]
    fn empty_name_and_description_are_refused() {
        let mut catalog = MetricCatalog::default();
        assert_eq!(
            catalog.define(def(" ", Category::Game)),
            Err(CatalogError::EmptyName)
        );
        let mut blank = def("game.tick_ms", Category::Game);
        blank.description = "   ".into();
        assert_eq!(
            catalog.define(blank),
            Err(CatalogError::EmptyDescription("game.tick_ms".into()))
        );
    }

    #[test]
    fn by_category_groups_the_vocabulary() {
        let mut catalog = MetricCatalog::default();
        catalog
            .define(def("host.cpu_ratio", Category::Host))
            .expect("host metric");
        catalog
            .define(def("db.query_total", Category::DatabaseCache))
            .expect("db metric");
        assert_eq!(catalog.by_category(&Category::Host).len(), 1);
        assert_eq!(catalog.by_category(&Category::DatabaseCache).len(), 1);
        assert!(catalog.by_category(&Category::Business).is_empty());
    }

    #[test]
    fn catalog_serialises_stably() {
        let catalog = MetricCatalog::with_generator_metrics();
        assert_eq!(catalog.len(), 7);
        let text = serde_yaml::to_string(&catalog).expect("serialise");
        let back: MetricCatalog = serde_yaml::from_str(&text).expect("reparse");
        assert_eq!(back, catalog);
    }

    #[test]
    fn units_and_kinds_keep_their_words() {
        let gauge: MetricKind = serde_yaml::from_str("gauge").expect("kind");
        assert_eq!(gauge, MetricKind::Gauge);
        let bytes: Unit = serde_yaml::from_str("bytes").expect("unit");
        assert_eq!(bytes, Unit::Bytes);
        let other: Unit = serde_yaml::from_str("!other packets").expect("unit");
        assert_eq!(other, Unit::Other("packets".into()));
    }
}
