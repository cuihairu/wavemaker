//! Query Scheduler (WM-0406; docs/basin.md Observability tree, Preflight
//! "Sampling intervals satisfy report needs" and "Query windows are
//! available").
//!
//! The scheduler turns the metric mapping (WM-0405) into a deterministic
//! tick timeline and executes the queries against the unified provider
//! interface (WM-0404). Every collection records its query window and the
//! instant it was issued (the timestamps WM-0407 formalizes); per-entry
//! failures are captured as outcomes, NEVER replaced with zeros or
//! interpolations — "Missing data is not treated as zero" is a Phase 4
//! exit criterion and starts here.

use serde::{Deserialize, Serialize};

use crate::mapping::{MappingEntry, MetricMapping};
use crate::provider::{MetricProvider, ProviderError, QueryWindow, ProviderSeries};

/// One planned query: a mapping entry paired with the window it will be
/// asked for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScheduledQuery {
    pub entry_index: usize,
    pub window: QueryWindow,
}

/// What came back for a scheduled query. `Failed` keeps the provider's
/// error verbatim; there is no zero/empty fallback path in this model.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum CollectedOutcome {
    Series(Vec<ProviderSeries>),
    Failed(String),
}

/// The result of one executed query: entry, window, issue instant, and
/// outcome — the collection record downstream items annotate (WM-0407
/// timestamps, WM-0408 gap detection).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CollectedQuery {
    pub entry_index: usize,
    pub catalog_metric: String,
    pub service: String,
    pub window: QueryWindow,
    /// Wall-clock instant the query batch was issued (injected by the
    /// caller — no hidden clock reads in the data model).
    pub issued_at_unix_ms: i64,
    pub outcome: CollectedOutcome,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QueryScheduler {
    step_ms: i64,
    #[serde(default)]
    mapping: MetricMapping,
}

impl QueryScheduler {
    pub fn new(mapping: MetricMapping, step_ms: i64) -> Result<Self, ProviderError> {
        if step_ms <= 0 {
            return Err(ProviderError::InvalidWindow(format!(
                "scheduler step must be positive, got {step_ms}"
            )));
        }
        Ok(Self { step_ms, mapping })
    }

    pub fn step_ms(&self) -> i64 {
        self.step_ms
    }

    pub fn mapping(&self) -> &MetricMapping {
        &self.mapping
    }

    /// Deterministic tick plan over [start, end): one query per mapping
    /// entry per tick, tick windows `[t, t + step)` truncated at `end`.
    /// The last window is short rather than lying about coverage.
    pub fn plan(&self, start_unix_ms: i64, end_unix_ms: i64) -> Vec<ScheduledQuery> {
        let mut plan = Vec::new();
        if start_unix_ms >= end_unix_ms || self.mapping.is_empty() {
            return plan;
        }
        let mut tick = start_unix_ms;
        while tick < end_unix_ms {
            let window_end = (tick + self.step_ms).min(end_unix_ms);
            for entry_index in 0..self.mapping.entries().len() {
                plan.push(ScheduledQuery {
                    entry_index,
                    window: QueryWindow {
                        start_unix_ms: tick,
                        end_unix_ms: window_end,
                        step_ms: self.step_ms,
                    },
                });
            }
            tick += self.step_ms;
        }
        plan
    }

    /// Execute every planned query against the provider. One provider
    /// failure never cancels the batch — each entry/tick keeps its own
    /// outcome. `issued_at_unix_ms` stamps the whole batch.
    pub fn collect(
        &self,
        provider: &dyn MetricProvider,
        start_unix_ms: i64,
        end_unix_ms: i64,
        issued_at_unix_ms: i64,
    ) -> Vec<CollectedQuery> {
        self.plan(start_unix_ms, end_unix_ms)
            .into_iter()
            .map(|scheduled| {
                let entry = &self.mapping.entries()[scheduled.entry_index];
                let outcome =
                    match provider.query_range(&entry.provider_query, &scheduled.window) {
                        Ok(series) => CollectedOutcome::Series(series),
                        Err(err) => CollectedOutcome::Failed(err.to_string()),
                    };
                CollectedQuery {
                    entry_index: scheduled.entry_index,
                    catalog_metric: entry.catalog_metric.clone(),
                    service: entry.service.clone(),
                    window: scheduled.window,
                    issued_at_unix_ms,
                    outcome,
                }
            })
            .collect()
    }
}

/// Convenience view of a collection batch grouped by entry.
pub fn failed_queries(queries: &[CollectedQuery]) -> Vec<&CollectedQuery> {
    queries
        .iter()
        .filter(|query| matches!(query.outcome, CollectedOutcome::Failed(_)))
        .collect()
}

/// Build a scheduler from an observability configuration's cadence
/// (docs/configuration.md `collection_interval`) and a mapping. The step
/// is the collection interval; windows never overlap.
pub fn scheduler_from_interval(
    mapping: MetricMapping,
    collection_interval_ms: i64,
) -> Result<QueryScheduler, ProviderError> {
    QueryScheduler::new(mapping, collection_interval_ms)
}

/// Re-export for callers that want the entry behind a collected record.
pub fn entry_of<'a>(
    mapping: &'a MetricMapping,
    collected: &CollectedQuery,
) -> &'a MappingEntry {
    &mapping.entries()[collected.entry_index]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::{Category, MetricDef, MetricKind, Unit};
    use crate::mapping::MappingEntry;
    use crate::provider::Sample;
    use crate::Labels;

    fn mapping_two_entries() -> MetricMapping {
        let mut mapping = MetricMapping::default();
        mapping
            .add_entry(MappingEntry {
                catalog_metric: "login.online".into(),
                service: "loginapp".into(),
                provider_query: "kbengine_online{component=\"loginapp\"}".into(),
                labels: Labels::new(),
            })
            .expect("entry 1");
        mapping
            .add_entry(MappingEntry {
                catalog_metric: "game.online".into(),
                service: "cellapp".into(),
                provider_query: "kbengine_online{component=\"cellapp\"}".into(),
                labels: Labels::new(),
            })
            .expect("entry 2");
        mapping
    }

    fn series_for(name: &str, tick_ms: i64) -> Vec<ProviderSeries> {
        vec![ProviderSeries {
            metric: {
                let mut labels = Labels::new();
                labels.insert("__name__".into(), name.into());
                labels
            },
            samples: vec![Sample {
                timestamp_unix_ms: tick_ms,
                value: 12.0,
            }],
        }]
    }

    /// Succeeds only for the loginapp query; the cellapp query always
    /// fails — per-entry isolation and the no-zero-fallback rule both
    /// hang on this.
    struct SplitProvider;

    impl MetricProvider for SplitProvider {
        fn query_range(
            &self,
            query: &str,
            _window: &QueryWindow,
        ) -> Result<Vec<ProviderSeries>, ProviderError> {
            if query.contains("loginapp") {
                Ok(series_for("kbengine_online", 0))
            } else {
                Err(ProviderError::BadStatus(503))
            }
        }

        fn ready(&self) -> Result<(), ProviderError> {
            Ok(())
        }
    }

    #[test]
    fn plan_is_deterministic_and_truncates_the_last_window() {
        let scheduler =
            QueryScheduler::new(mapping_two_entries(), 4000).expect("scheduler");
        let plan = scheduler.plan(0, 10_000);
        // 3 ticks (0, 4, 8) x 2 entries; tick windows [0,4),[4,8),[8,10).
        assert_eq!(plan.len(), 6);
        assert_eq!(plan[0].entry_index, 0);
        assert_eq!(plan[0].window, QueryWindow {
            start_unix_ms: 0,
            end_unix_ms: 4000,
            step_ms: 4000,
        });
        assert_eq!(plan[1].entry_index, 1);
        let last = plan.last().expect("non-empty");
        assert_eq!(last.window, QueryWindow {
            start_unix_ms: 8000,
            end_unix_ms: 10_000,
            step_ms: 4000,
        });

        // Same inputs, same plan.
        assert_eq!(plan, scheduler.plan(0, 10_000));
    }

    #[test]
    fn degenerate_ranges_and_empty_mappings_plan_nothing() {
        let scheduler =
            QueryScheduler::new(mapping_two_entries(), 1000).expect("scheduler");
        assert!(scheduler.plan(5000, 5000).is_empty());
        assert!(scheduler.plan(6000, 5000).is_empty());
        let empty =
            QueryScheduler::new(MetricMapping::default(), 1000).expect("scheduler");
        assert!(empty.plan(0, 10_000).is_empty());
    }

    #[test]
    fn non_positive_step_is_rejected() {
        assert!(matches!(
            QueryScheduler::new(mapping_two_entries(), 0),
            Err(ProviderError::InvalidWindow(_))
        ));
        assert!(matches!(
            QueryScheduler::new(mapping_two_entries(), -5),
            Err(ProviderError::InvalidWindow(_))
        ));
    }

    #[test]
    fn collection_records_window_issue_instant_and_outcome() {
        let scheduler =
            QueryScheduler::new(mapping_two_entries(), 4000).expect("scheduler");
        let collected =
            scheduler.collect(&SplitProvider, 0, 8000, 9_999);
        assert_eq!(collected.len(), 4);
        let first = &collected[0];
        assert_eq!(first.catalog_metric, "login.online");
        assert_eq!(first.service, "loginapp");
        assert_eq!(first.issued_at_unix_ms, 9_999);
        assert_eq!(
            first.window,
            QueryWindow { start_unix_ms: 0, end_unix_ms: 4000, step_ms: 4000 }
        );
        match &first.outcome {
            CollectedOutcome::Series(series) => {
                assert_eq!(series.len(), 1);
                assert_eq!(series[0].samples[0].value, 12.0);
            }
            CollectedOutcome::Failed(_) => panic!("expected series"),
        }
    }

    #[test]
    fn provider_failure_is_kept_verbatim_never_zero() {
        let scheduler =
            QueryScheduler::new(mapping_two_entries(), 4000).expect("scheduler");
        let collected = scheduler.collect(&SplitProvider, 0, 4000, 0);
        assert_eq!(collected.len(), 2);
        let second = &collected[1];
        assert_eq!(second.catalog_metric, "game.online");
        assert_eq!(
            second.outcome,
            CollectedOutcome::Failed(
                "metric provider returned http status 503".to_string()
            )
        );
        assert_eq!(failed_queries(&collected).len(), 1);
    }

    #[test]
    fn scheduler_serialises_with_its_mapping() {
        let scheduler =
            QueryScheduler::new(mapping_two_entries(), 1000).expect("scheduler");
        let text = serde_yaml::to_string(&scheduler).expect("serialise");
        assert!(text.contains("step_ms: 1000"));
        let back: QueryScheduler = serde_yaml::from_str(&text).expect("reparse");
        assert_eq!(back, scheduler);
    }

    #[test]
    fn interval_constructor_and_entry_lookup_work() {
        let mapping = mapping_two_entries();
        let scheduler =
            scheduler_from_interval(mapping, 15_000).expect("scheduler");
        assert_eq!(scheduler.step_ms(), 15_000);
        let collected = scheduler.collect(&SplitProvider, 0, 1000, 42);
        let entry = entry_of(scheduler.mapping(), &collected[0]);
        assert_eq!(entry.service, "loginapp");
    }

    #[test]
    fn catalog_validation_still_reaches_through_the_mapping() {
        // The scheduler is honest about unmapped environments: an empty
        // catalog can validate the mapping's references and must fail on
        // unknown metrics even when a scheduler was already built.
        let mut catalog = crate::catalog::MetricCatalog::default();
        catalog
            .define(MetricDef {
                name: "login.online".into(),
                kind: MetricKind::Gauge,
                unit: Unit::Count,
                category: Category::GatewayLogin,
                description: "online sessions at the login path".into(),
            })
            .expect("define");
        let mapping = mapping_two_entries();
        let topology = crate::topology::Topology::default();
        assert!(mapping.validate(&catalog, &topology).is_err());
    }
}
