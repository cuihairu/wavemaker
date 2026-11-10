//! Multi-Backend mixed Swells (WM-0514) — a swell whose plan spans a
//! MIXED fleet (Native Runtime + Engine Bot backends, ADR-005 D1), plus
//! the honest execution report folded over both kinds.
//!
//! Two concerns, both structural:
//!
//! 1. PLAN CONFORMANCE: the controller mints `actor-*` ids for Native
//!    backends and `bot-*` for Engine Bot backends (WM-0504). The
//!    validator checks EVERY plan entry is kind-consistent with its
//!    backend and that no backend receives more shards than it DECLARED
//!    slots. A violation is a NAMED error — never silently repaired.
//!    An undeclared capacity is an unknown, never treated as zero.
//! 2. THE MIXED REPORT: the Phase 5 deliverable "Mixed Native +
//!    KBEngine execution reports". Per-backend aggregates (WM-0510) are
//!    grouped by kind so both kinds stay visible. Honesty rules:
//!    - COUNTS are additive: per-kind and overall totals SUM the
//!      per-backend counts (arrivals are events).
//!    - GAUGES are listed per backend, never summed.
//!    - PERCENTILES: the per-backend merged histogram (WM-0511) is the
//!      FINEST granularity exposed. Cross-backend or cross-kind merging
//!      is the report-template §11.2 ban — this module exposes no API
//!      that could produce one.
//!    - A backend present in the aggregation but unknown to the fleet
//!      refuses the whole report (cannot be attributed honestly).

use std::collections::BTreeMap;

use wm_core::hdr::Histogram;

use crate::aggregation::{
    AggregationError, AggregationIssue, BackendAggregate, MetricsAggregation,
};
use crate::backend_registry::BackendKind;
use crate::capacity::ResourceCapacity;
use crate::shard_plan::{PlanBackend, ShardPlan};

/// One kind's fleet view inside a mixed swell.
#[derive(Debug, Clone)]
pub struct KindGroupReport {
    pub kind: BackendKind,
    /// This kind's per-backend aggregates, ordered by backend id.
    pub backends: Vec<BackendAggregate>,
    /// Summed per-backend counts — counts are additive within a kind.
    pub arrivals_total: u64,
    pub rejected_total: u64,
    /// Gauges per backend, listed — never summed (gauges taken at
    /// different moments cannot add).
    pub active_connections_per_worker: Vec<(String, u64)>,
    /// Per-backend MERGED histograms (WM-0511), the finest percentile
    /// granularity the report exposes. There is no field or method that
    /// merges across backends — report-template §11.2 is structural.
    pub merged_histograms_per_backend: Vec<(String, Histogram)>,
}

/// The mixed-swell execution report.
#[derive(Debug, Clone)]
pub struct MixedSwellReport {
    pub swell_id: String,
    /// Kind groups, ordered (Native first, then Engine Bot).
    pub groups: Vec<KindGroupReport>,
    /// Summed across every kind — counts are additive.
    pub arrivals_total: u64,
    pub rejected_total: u64,
    /// Data-quality issues carried verbatim from the aggregation.
    pub issues: Vec<AggregationIssue>,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum MixedReportError {
    #[error("swell id must not be empty")]
    EmptySwellId,
    #[error(
        "backend {0} is in the aggregation but not in the fleet; its kind cannot be attributed"
    )]
    UnknownBackend(String),
    #[error("histogram merge failed: {0}")]
    MergeFailed(String),
}

fn merged_or_none(aggregate: &BackendAggregate) -> Result<Option<Histogram>, MixedReportError> {
    match aggregate.merged_histogram() {
        Ok(histogram) => Ok(Some(histogram)),
        // No histograms on this backend is honest absence, not a report
        // of a nonexistent percentile.
        Err(AggregationError::NothingToMerge) => Ok(None),
        Err(other) => Err(MixedReportError::MergeFailed(other.to_string())),
    }
}

/// Fold a swell's per-backend aggregation into a mixed-kind report.
/// `fleet` names every backend available to the swell and its kind —
/// the report attributes each aggregate through it.
pub fn build_mixed_swell_report(
    swell_id: &str,
    fleet: &[PlanBackend],
    aggregation: MetricsAggregation,
) -> Result<MixedSwellReport, MixedReportError> {
    if swell_id.trim().is_empty() {
        return Err(MixedReportError::EmptySwellId);
    }
    let kind_of: BTreeMap<&str, BackendKind> = fleet
        .iter()
        .map(|backend| (backend.backend_id.as_str(), backend.kind))
        .collect();

    let mut groups: BTreeMap<u8, KindGroupReport> = BTreeMap::new();
    for aggregate in aggregation.per_backend {
        let kind = *kind_of
            .get(aggregate.backend_id.as_str())
            .ok_or_else(|| MixedReportError::UnknownBackend(aggregate.backend_id.clone()))?;
        let rank = kind_rank(kind);
        let group = groups.entry(rank).or_insert_with(|| KindGroupReport {
            kind,
            backends: Vec::new(),
            arrivals_total: 0,
            rejected_total: 0,
            active_connections_per_worker: Vec::new(),
            merged_histograms_per_backend: Vec::new(),
        });
        group.arrivals_total += aggregate.arrivals_total;
        group.rejected_total += aggregate.rejected_total;
        group
            .active_connections_per_worker
            .extend(aggregate.active_connections_per_worker.iter().cloned());
        if let Some(histogram) = merged_or_none(&aggregate)? {
            group
                .merged_histograms_per_backend
                .push((aggregate.backend_id.clone(), histogram));
        }
        group.backends.push(aggregate);
    }

    let mut groups: Vec<KindGroupReport> = groups.into_values().collect();
    for group in &mut groups {
        group
            .backends
            .sort_by(|a, b| a.backend_id.cmp(&b.backend_id));
    }
    let arrivals_total = groups.iter().map(|g| g.arrivals_total).sum();
    let rejected_total = groups.iter().map(|g| g.rejected_total).sum();

    Ok(MixedSwellReport {
        swell_id: swell_id.into(),
        groups,
        arrivals_total,
        rejected_total,
        issues: aggregation.issues,
    })
}

fn kind_rank(kind: BackendKind) -> u8 {
    match kind {
        BackendKind::NativeRuntime => 0,
        BackendKind::EngineBot => 1,
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PlanConformanceError {
    #[error("plan entry {actor_or_bot_id} on backend {backend_id} is not {expected_kind:?}-consistent: its id prefix names the wrong kind")]
    IdKindMismatch {
        actor_or_bot_id: String,
        backend_id: String,
        expected_kind: BackendKind,
    },
    #[error(
        "backend {backend_id} owns {shards} shards but declared only {slots} slots for its kind"
    )]
    CapacityExceeded {
        backend_id: String,
        shards: u32,
        slots: u32,
    },
    #[error(
        "plan entry {actor_or_bot_id} references backend {backend_id} which is not in the fleet"
    )]
    UnknownBackend {
        actor_or_bot_id: String,
        backend_id: String,
    },
}

/// Verify a plan over a mixed fleet is kind-consistent and respects
/// declared capacities. `capacities` maps backend id to its declared
/// envelope; `None` capacity (undeclared) is an unknown and never
/// treated as zero. Every violation is named — nothing repaired.
pub fn verify_mixed_plan_conformance(
    plan: &ShardPlan,
    fleet: &[PlanBackend],
    capacities: &BTreeMap<String, ResourceCapacity>,
) -> Result<(), PlanConformanceError> {
    let kind_of: BTreeMap<&str, BackendKind> = fleet
        .iter()
        .map(|backend| (backend.backend_id.as_str(), backend.kind))
        .collect();
    let mut shards_per_backend: BTreeMap<&str, std::collections::BTreeSet<u32>> = BTreeMap::new();
    for entry in &plan.entries {
        let kind = kind_of.get(entry.backend_id.as_str()).ok_or_else(|| {
            PlanConformanceError::UnknownBackend {
                actor_or_bot_id: entry.actor_or_bot_id.clone(),
                backend_id: entry.backend_id.clone(),
            }
        })?;
        let (expected_prefix, actual_prefix) = match kind {
            BackendKind::NativeRuntime => ("actor-", entry.actor_or_bot_id.as_str()),
            BackendKind::EngineBot => ("bot-", entry.actor_or_bot_id.as_str()),
        };
        if !actual_prefix.starts_with(expected_prefix) {
            return Err(PlanConformanceError::IdKindMismatch {
                actor_or_bot_id: entry.actor_or_bot_id.clone(),
                backend_id: entry.backend_id.clone(),
                expected_kind: *kind,
            });
        }
        shards_per_backend
            .entry(entry.backend_id.as_str())
            .or_default()
            .insert(entry.shard_id);
    }
    for (backend_id, shards) in shards_per_backend {
        let kind = kind_of[backend_id];
        if let Some(capacity) = capacities.get(backend_id) {
            if let Some(slots) = capacity.slots_for(kind) {
                let owned = shards.len() as u32;
                if owned > slots {
                    return Err(PlanConformanceError::CapacityExceeded {
                        backend_id: backend_id.to_string(),
                        shards: owned,
                        slots,
                    });
                }
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::aggregation::{aggregate_metrics, WorkerMetrics};
    use crate::shard_plan::{build_shard_plan, PlanEntry, ShardRule};
    use wm_core::hdr::{
        HdrParameters, HDR_HIGHEST_TRACKABLE_VALUE, HDR_LOWEST_DISCERNIBLE_VALUE, HDR_SIG_FIGURES,
    };

    fn native(id: &str, weight: f64) -> PlanBackend {
        PlanBackend {
            backend_id: id.into(),
            kind: BackendKind::NativeRuntime,
            weight,
        }
    }

    fn bot(id: &str, weight: f64) -> PlanBackend {
        PlanBackend {
            backend_id: id.into(),
            kind: BackendKind::EngineBot,
            weight,
        }
    }

    fn histogram(values: &[i64]) -> Histogram {
        let params = HdrParameters {
            lowest_discernible_value: HDR_LOWEST_DISCERNIBLE_VALUE,
            significant_figures: HDR_SIG_FIGURES,
            highest_trackable_value: HDR_HIGHEST_TRACKABLE_VALUE,
        };
        let mut histogram = Histogram::new(params).expect("frozen v1 params");
        for value in values {
            histogram.record_value(*value).expect("in range");
        }
        histogram
    }

    fn metrics(id: &str, backend: &str, arrivals: u64) -> WorkerMetrics {
        WorkerMetrics {
            worker_id: id.into(),
            backend_id: Some(backend.into()),
            window_start_unix_ms: 1_700_000_000_000,
            window_end_unix_ms: 1_700_000_001_000,
            arrivals_total: arrivals,
            rejected_total: 1,
            active_connections: 10,
            latency_histogram: histogram(&[1_000_000, 2_000_000, 3_000_000]),
        }
    }

    #[test]
    fn mixed_plan_is_kind_consistent_and_respects_declared_capacity() {
        let fleet = vec![native("b-native", 1.0), bot("a-bot", 2.0)];
        let arrivals = vec![
            crate::shard_plan::PlannedArrival {
                planned_at_unix_ms: 1,
                lifetime_ms: 5,
                behavior_group: "g".into(),
            };
            12
        ];
        let plan = build_shard_plan("sw-mixed", 6, &fleet, &arrivals).expect("plan");
        // Mixed: both kinds appear.
        assert!(plan
            .entries
            .iter()
            .any(|e| e.actor_or_bot_id.starts_with("actor-")));
        assert!(plan
            .entries
            .iter()
            .any(|e| e.actor_or_bot_id.starts_with("bot-")));

        let capacities = BTreeMap::from([
            (
                "a-bot".to_string(),
                ResourceCapacity {
                    bot_slots: Some(500),
                    ..Default::default()
                },
            ),
            (
                "b-native".to_string(),
                ResourceCapacity {
                    actor_slots: Some(200),
                    ..Default::default()
                },
            ),
        ]);
        assert!(verify_mixed_plan_conformance(&plan, &fleet, &capacities).is_ok());

        // A tight capacity that the plan exceeds is a NAMED violation.
        let tight = BTreeMap::from([(
            "b-native".to_string(),
            ResourceCapacity {
                actor_slots: Some(1),
                ..Default::default()
            },
        )]);
        assert!(matches!(
            verify_mixed_plan_conformance(&plan, &fleet, &tight),
            Err(PlanConformanceError::CapacityExceeded {
                backend_id,
                slots: 1,
                ..
            }) if backend_id == "b-native"
        ));

        // An undeclared capacity is an unknown — never treated as zero.
        let undeclared = BTreeMap::new();
        assert!(verify_mixed_plan_conformance(&plan, &fleet, &undeclared).is_ok());
    }

    #[test]
    fn plan_id_mismatch_and_unknown_backend_are_named() {
        let fleet = vec![native("b-native", 1.0), bot("a-bot", 1.0)];
        // A plan whose entry carries a bot- id on a native backend.
        let plan = ShardPlan {
            swell_id: "sw".into(),
            worker_count: 2,
            rule: ShardRule::Modulo,
            entries: vec![PlanEntry {
                actor_or_bot_id: "bot-0".into(),
                backend_id: "b-native".into(),
                shard_id: 0,
                planned_at_unix_ms: 1,
                lifetime_ms: 5,
                behavior_group: "g".into(),
                seed: 1,
                attributes: BTreeMap::new(),
            }],
            plan_digest: "digest".into(),
        };
        assert!(matches!(
            verify_mixed_plan_conformance(&plan, &fleet, &BTreeMap::new()),
            Err(PlanConformanceError::IdKindMismatch {
                actor_or_bot_id,
                backend_id,
                expected_kind: BackendKind::NativeRuntime,
                ..
            }) if actor_or_bot_id == "bot-0" && backend_id == "b-native"
        ));

        // An entry referencing a backend that is not in the fleet.
        let plan = ShardPlan {
            swell_id: "sw".into(),
            worker_count: 2,
            rule: ShardRule::Modulo,
            entries: vec![PlanEntry {
                actor_or_bot_id: "actor-0".into(),
                backend_id: "ghost".into(),
                shard_id: 0,
                planned_at_unix_ms: 1,
                lifetime_ms: 5,
                behavior_group: "g".into(),
                seed: 1,
                attributes: BTreeMap::new(),
            }],
            plan_digest: "digest".into(),
        };
        assert!(matches!(
            verify_mixed_plan_conformance(&plan, &fleet, &BTreeMap::new()),
            Err(PlanConformanceError::UnknownBackend { backend_id, .. })
                if backend_id == "ghost"
        ));
    }

    #[test]
    fn mixed_report_groups_by_kind_with_honest_sums_and_no_cross_backend_percentile() {
        let fleet = vec![native("b-native", 1.0), bot("a-bot", 1.0)];
        let received = vec![
            metrics("w-a", "a-bot", 100),
            metrics("w-b", "a-bot", 150),
            metrics("w-c", "b-native", 70),
        ];
        let aggregation = aggregate_metrics(&["w-a", "w-b", "w-c"], received).expect("valid");
        let report = build_mixed_swell_report("sw-mixed", &fleet, aggregation).expect("report");

        // Both kinds stay visible; Native group first (stable order).
        let kinds: Vec<BackendKind> = report.groups.iter().map(|g| g.kind).collect();
        assert_eq!(
            kinds,
            vec![BackendKind::NativeRuntime, BackendKind::EngineBot]
        );
        let native_group = &report.groups[0];
        let bot_group = &report.groups[1];
        assert_eq!(native_group.arrivals_total, 70, "counts summed within kind");
        assert_eq!(bot_group.arrivals_total, 250);
        assert_eq!(report.arrivals_total, 320, "counts additive across kinds");
        assert_eq!(report.rejected_total, 3);
        // Gauges listed per contributing Worker, never summed.
        assert_eq!(
            native_group.active_connections_per_worker,
            vec![("w-c".to_string(), 10)]
        );
        // The only percentile source is the per-backend merged histogram;
        // no cross-backend / cross-kind merge field or method exists.
        let bot_histograms = &bot_group.merged_histograms_per_backend;
        assert_eq!(bot_histograms.len(), 1, "one backend, one merged histogram");
        assert_eq!(bot_histograms[0].0, "a-bot");
        let merged_p50 = bot_histograms[0].1.value_at_percentile(50.0);
        assert!(merged_p50 > 0);
    }

    #[test]
    fn single_kind_swell_has_one_group_and_unknown_backend_refuses() {
        let fleet = vec![native("b-native", 1.0), bot("a-bot", 1.0)];
        // Only native workers reported.
        let received = vec![metrics("w-c", "b-native", 70)];
        let aggregation = aggregate_metrics(&["w-c"], received).expect("valid");
        let report = build_mixed_swell_report("sw", &fleet, aggregation).expect("report");
        assert_eq!(report.groups.len(), 1);
        assert_eq!(report.groups[0].kind, BackendKind::NativeRuntime);

        // A backend the fleet does not know cannot be attributed.
        let received = vec![metrics("w-x", "ghost", 5)];
        let aggregation = aggregate_metrics(&["w-x"], received).expect("valid");
        assert!(matches!(
            build_mixed_swell_report("sw", &fleet, aggregation),
            Err(MixedReportError::UnknownBackend(backend)) if backend == "ghost"
        ));

        // Empty swell id is refused.
        let received = vec![metrics("w-c", "b-native", 70)];
        let aggregation = aggregate_metrics(&["w-c"], received).expect("valid");
        assert!(matches!(
            build_mixed_swell_report("  ", &fleet, aggregation),
            Err(MixedReportError::EmptySwellId)
        ));
    }
}
