//! WM-0515: distributed soak test over the Phase 5 machinery.
//!
//! A soak is a sustained-operation stability check. Here the unit under
//! soak is the DISTRIBUTED execution stack: the Worker registry
//! (leases / heartbeats / rejoin generations), the Backend registry
//! (D3 transitions), the mixed-fleet shard plan (WM-0504), and the
//! multi-Worker aggregation + mixed-kind report (WM-0510/0511/0514).
//!
//! Each cycle runs one full MIXED swell over a persistent fleet:
//! register workers (a rejoin mints a fresh generation every cycle),
//! heartbeat them, inject a stale-generation fault (policy 7), build
//! the deterministic plan, verify its conformance, aggregate the
//! metrics, build the mixed-kind report, and assert the invariants
//! hold cycle after cycle — nothing drifts, no generation collides, no
//! percentile is ever merged across backends.
//!
//! The short test [`distributed_soak_short_cycles`] is CI-safe (bounded
//! cycles plus one honest lease-expiry window). The real long-duration
//! run [`distributed_soak_long_wallclock`] is `#[ignore]`d and runs on
//! demand:
//!
//! ```text
//! WM_SOAK_SECONDS=86400 cargo test -p wm-controller --test distributed_soak -- --ignored distributed_soak_long_wallclock
//! ```

use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use wm_controller::aggregation::{WorkerMetrics, aggregate_metrics};
use wm_controller::backend_registry::BackendKind;
use wm_controller::capacity::ResourceCapacity;
use wm_controller::mixed_swell::{build_mixed_swell_report, verify_mixed_plan_conformance};
use wm_controller::registry::{
    AdmissionVerdict, RegistrationDecision, WorkerRegistration, WorkerRegistry, WorkerState,
};
use wm_controller::shard_plan::{PlanBackend, PlannedArrival, build_shard_plan};
use wm_core::hdr::{
    HdrParameters, Histogram, HDR_HIGHEST_TRACKABLE_VALUE, HDR_LOWEST_DISCERNIBLE_VALUE,
    HDR_SIG_FIGURES,
};
use wm_protocol::PROTOCOL_VERSION;
use wm_worker::fault::{BeatAction, FaultyWorker, WorkerFault};

/// The persistent mixed fleet: one Native Runtime, one Engine Bot.
fn fleet() -> Vec<PlanBackend> {
    vec![
        PlanBackend {
            backend_id: "b-native".into(),
            kind: BackendKind::NativeRuntime,
            weight: 1.0,
        },
        PlanBackend {
            backend_id: "a-bot".into(),
            kind: BackendKind::EngineBot,
            weight: 2.0,
        },
    ]
}

fn capacities() -> BTreeMap<String, ResourceCapacity> {
    BTreeMap::from([
        (
            "b-native".to_string(),
            ResourceCapacity {
                actor_slots: Some(500),
                ..Default::default()
            },
        ),
        (
            "a-bot".to_string(),
            ResourceCapacity {
                bot_slots: Some(500),
                ..Default::default()
            },
        ),
    ])
}

fn worker_registration(id: &str) -> WorkerRegistration {
    WorkerRegistration {
        worker_id: id.into(),
        protocol_version: PROTOCOL_VERSION,
        adapter_version: "0.3".into(),
        engine_version: "kbengine-1.2".into(),
        runtime_version: "0.1".into(),
        capabilities: vec!["exact_arrival".into()],
        resource_capacity: None,
    }
}

fn register(registry: &WorkerRegistry, id: &str) -> u64 {
    match registry
        .register(worker_registration(id))
        .expect("registration accepted")
    {
        RegistrationDecision::Accepted { generation } => generation,
        other => panic!("registration refused: {other:?}"),
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
        active_connections: 7,
        latency_histogram: histogram(&[1_000_000, 2_000_000, 3_000_000]),
    }
}

/// One mixed swell cycle. Returns the number of workers that rejoined
/// (fresh generations minted), so the caller can assert monotonicity.
///
/// Invariants asserted inside the cycle (so every cycle is a full
/// regression gate):
/// - the plan is deterministic and kind-conformant over the mixed fleet;
/// - a rejoin always mints a fresh generation (never a collision);
/// - a stale-generation heartbeat is refused and leaves zero trace
///   (policy 7);
/// - the aggregation names missing workers and sums counts;
/// - the mixed report groups by kind, sums counts, and exposes no
///   cross-backend percentile merge (report-template §11.2 is
///   structural — the report type has no such field or method).
fn run_cycle(registry: &WorkerRegistry, cycle: u64) -> u64 {
    let swell_id = format!("sw-soak-{cycle}");
    let fleet = fleet();

    // Rejoin all three workers: every cycle mints fresh generations.
    let gen_a = register(registry, "w-a");
    let gen_b = register(registry, "w-b");
    let gen_c = register(registry, "w-c");
    // Heartbeat each at its granted generation.
    assert!(registry.heartbeat("w-a", gen_a));
    assert!(registry.heartbeat("w-b", gen_b));
    assert!(registry.heartbeat("w-c", gen_c));

    // Stale-generation fault: the w-a instance from an OLD process beats
    // at one behind its granted generation — refused, zero trace.
    let mut stale = FaultyWorker::new("w-a", PROTOCOL_VERSION, gen_a, WorkerFault::StaleGenerationAfter { beats: 1 });
    assert_eq!(stale.next_beat(), BeatAction::Heartbeat { generation: gen_a });
    assert_eq!(
        stale.next_beat(),
        BeatAction::Heartbeat { generation: gen_a - 1 }
    );
    assert!(
        !registry.heartbeat("w-a", gen_a - 1),
        "old-generation beats never win (policy 7)"
    );
    let record = registry.worker("w-a").expect("record");
    assert_eq!(record.state, WorkerState::Online, "refused beat leaves no trace");
    assert_eq!(record.generation, gen_a);

    // Deterministic mixed plan (byte-identical for identical inputs).
    let arrivals: Vec<PlannedArrival> = (0..12)
        .map(|i| PlannedArrival {
            planned_at_unix_ms: 1_700_000_000_000 + i as i64 * 5,
            lifetime_ms: 5,
            behavior_group: "soak".into(),
        })
        .collect();
    let plan = build_shard_plan(&swell_id, 6, &fleet, &arrivals).expect("plan");
    assert_eq!(plan.worker_count, 6);
    assert!(plan.entries.iter().any(|e| e.actor_or_bot_id.starts_with("actor-")));
    assert!(plan.entries.iter().any(|e| e.actor_or_bot_id.starts_with("bot-")));
    verify_mixed_plan_conformance(&plan, &fleet, &capacities()).expect("kind-conformant");

    // Aggregate: w-c contributed nothing this cycle (silent), so it is
    // NAMED missing — never rendered as zeros.
    let received = vec![metrics("w-a", "a-bot", 100), metrics("w-b", "b-native", 70)];
    let aggregation =
        aggregate_metrics(&["w-a", "w-b", "w-c"], received).expect("valid aggregation");
    assert!(aggregation
        .issues
        .contains(&wm_controller::aggregation::AggregationIssue::MissingWorker {
            worker_id: "w-c".into()
        }));
    assert_eq!(aggregation.per_backend.len(), 2);

    // The mixed report: both kinds visible, counts summed honestly,
    // percentiles only ever per-backend (no cross-backend merge field).
    let report =
        build_mixed_swell_report(&swell_id, &fleet, aggregation).expect("mixed report");
    assert_eq!(report.groups.len(), 2);
    assert_eq!(report.arrivals_total, 170);
    let bot = report
        .groups
        .iter()
        .find(|g| g.kind == BackendKind::EngineBot)
        .expect("bot group");
    assert_eq!(bot.arrivals_total, 100);
    assert_eq!(bot.merged_histograms_per_backend.len(), 1);
    assert_eq!(bot.merged_histograms_per_backend[0].0, "a-bot");
    assert!(
        bot.merged_histograms_per_backend[0].1.value_at_percentile(50.0) > 0,
        "the only percentile source is the per-backend merged histogram"
    );

    gen_c
}

/// Short soak, CI-safe: bounded mixed swell cycles plus one honest
/// lease-expiry window. Verifies the distributed machinery is stable
/// across repeated full cycles — no drift, no generation collision.
#[test]
fn distributed_soak_short_cycles() {
    let registry = WorkerRegistry::new(1000, 5000).expect("lease config");
    let cycles = 15;
    let mut last_generation = 0u64;
    for cycle in 1..=cycles {
        let generation = run_cycle(&registry, cycle);
        assert!(
            generation > last_generation,
            "generations are strictly monotonic across cycles (cycle {cycle})"
        );
        last_generation = generation;
    }
    // The registry has been soaking: 15 cycles × 3 rejoin registrations,
    // all retained (deregistered records survive for restart recovery).
    assert_eq!(registry.workers().len(), 3);

    // One honest lease-expiry window: w-b goes silent and is expired by
    // the lease loop (policy 1 freeze + policy 3 record), then recovers.
    // A dedicated 1.1s-TTL registry keeps the expiry observable without
    // clock tricks (as_secs() truncation needs a sub-2s window).
    let lease_registry = WorkerRegistry::new(1000, 1100).expect("lease config");
    let generation = register(&lease_registry, "w-b");
    assert!(lease_registry.heartbeat("w-b", generation));
    std::thread::sleep(Duration::from_millis(1100));
    assert_eq!(lease_registry.sweep_expired(), vec!["w-b".to_string()]);
    let record = lease_registry.worker("w-b").expect("record retained");
    assert_eq!(record.state, WorkerState::LostContact);
    assert!(record.lease_invalidated_at_unix_s.is_some());
    assert!(matches!(
        lease_registry.admission_verdicts(&["w-b"])[0].1,
        AdmissionVerdict::Frozen { .. }
    ));
    assert!(
        lease_registry.heartbeat("w-b", generation),
        "a beat recovers it"
    );
    assert_eq!(
        lease_registry.worker("w-b").expect("record").state,
        WorkerState::Online
    );
}

/// Long-duration soak (the 24h target), ignored by default.
///
/// Run on demand:
/// `WM_SOAK_SECONDS=86400 cargo test -p wm-controller --test distributed_soak -- --ignored distributed_soak_long_wallclock`
///
/// The actual wall-clock duration comes from the environment; this
/// machine's sustained cycle rate and memory are reported, not assumed.
#[test]
#[ignore = "long-running soak; run on demand (WM_SOAK_SECONDS)"]
fn distributed_soak_long_wallclock() {
    let seconds = env_usize("WM_SOAK_SECONDS", 86_400);
    let report_every = env_usize("WM_SOAK_REPORT_EVERY", 100).max(1);
    let registry = WorkerRegistry::new(1000, 5000).expect("lease config");
    let start = Instant::now();
    let deadline = start + Duration::from_secs(seconds as u64);

    let mut cycles = 0u64;
    let mut last_generation = 0u64;
    while Instant::now() < deadline {
        let generation = run_cycle(&registry, cycles + 1);
        assert!(
            generation > last_generation,
            "generation regression at cycle {cycles}"
        );
        last_generation = generation;
        cycles += 1;
        if cycles.is_multiple_of(report_every as u64) {
            let elapsed = start.elapsed().as_secs_f64();
            let rate = cycles as f64 / elapsed.max(0.001);
            eprintln!(
                "[distributed-soak] t={elapsed:.0}s cycles={cycles} workers={} \
                 rate={rate:.1} cycles/s rss={}KiB",
                registry.workers().len(),
                rss_kb()
            );
        }
    }

    let elapsed = start.elapsed();
    let rate = cycles as f64 / elapsed.as_secs_f64().max(0.001);
    eprintln!(
        "[distributed-soak] done cycles={cycles} workers={} elapsed={:?} \
         rate={rate:.1} cycles/s rss={}KiB",
        registry.workers().len(),
        elapsed,
        rss_kb()
    );
    assert!(cycles > 0, "soak must complete at least one cycle");
}

/// Resident set size in KiB (Linux /proc). Returns 0 where unavailable.
fn rss_kb() -> u64 {
    std::fs::read_to_string("/proc/self/statm")
        .ok()
        .and_then(|s| s.split_whitespace().nth(1).and_then(|v| v.parse().ok()))
        .unwrap_or(0)
        * 4
}

fn env_usize(name: &str, default: usize) -> usize {
    std::env::var(name)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}
