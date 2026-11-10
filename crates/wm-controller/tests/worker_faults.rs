//! Worker fault injection (WM-0513): replay the `wm-worker` fault model
//! against the real Worker registry and assert the controller's
//! documented reactions (`docs/distributed.md` §Failure Classification
//! and §Leases policy).
//!
//! Each test injects a fault through the Worker's action stream (what a
//! faulty Worker actually DOES) and asserts the controller-side outcome:
//! detection (which state it lands in), state, recovery action, report
//! impact (what a report would show), and whether automatic retry is
//! allowed. The injection never fabricates a controller state directly —
//! only the Worker's observable actions drive it.

use std::time::Duration;

use wm_controller::registry::{
    AdmissionVerdict, RegistrationDecision, WorkerRegistration, WorkerRegistry, WorkerState,
};
use wm_protocol::PROTOCOL_VERSION;
use wm_worker::fault::{BeatAction, FaultyWorker, WorkerFault};

/// Register the worker through its (possibly mismatched) presented
/// protocol; returns the granted generation.
fn register(registry: &WorkerRegistry, worker: &FaultyWorker) -> u64 {
    let registration = WorkerRegistration {
        worker_id: worker.worker_id().into(),
        protocol_version: worker.register_protocol_version(),
        adapter_version: "0.3".into(),
        engine_version: "kbengine-1.2".into(),
        runtime_version: "0.1".into(),
        capabilities: vec!["exact_arrival".into()],
        resource_capacity: None,
    };
    match registry.register(registration).expect("valid registration") {
        RegistrationDecision::Accepted { generation } => generation,
        other => panic!("registration was not accepted: {other:?}"),
    }
}

/// Replay `slots` heartbeat slots of the faulty Worker's action stream
/// against the registry. Returns the heartbeat outcomes (true =
/// accepted, false = refused); silence / leave / rejoin slots produce
/// no heartbeat and are applied for their side effects.
fn replay(registry: &WorkerRegistry, worker: &mut FaultyWorker, slots: u32) -> Vec<bool> {
    let mut outcomes = Vec::new();
    for _ in 0..slots {
        match worker.next_beat() {
            BeatAction::Heartbeat { generation } => {
                outcomes.push(registry.heartbeat(worker.worker_id(), generation));
            }
            BeatAction::Silent => {}
            BeatAction::Leave => {
                registry.deregister(worker.worker_id());
            }
            BeatAction::Rejoin => {
                let granted = register(registry, worker);
                worker.set_granted_generation(granted);
            }
        }
    }
    outcomes
}

#[test]
fn silence_after_leases_the_worker_into_lost_contact_and_freezes_admission() {
    // 1 s beat, 1.1 s TTL — long enough for `as_secs()` truncation to be
    // honest, short enough to observe the expiry without clock tricks.
    let registry = WorkerRegistry::new(1000, 1100).expect("lease config");
    let mut worker = FaultyWorker::new(
        "w1",
        PROTOCOL_VERSION,
        1,
        WorkerFault::SilenceAfter { beats: 2 },
    );
    let granted = register(&registry, &worker);

    // Exactly two accepted beats, then the fault fires: silence.
    assert_eq!(replay(&registry, &mut worker, 2), vec![true, true]);
    assert_eq!(worker.next_beat(), BeatAction::Silent);

    // The worker stops heartbeating; the lease honestly expires it.
    std::thread::sleep(Duration::from_millis(1100));
    assert_eq!(registry.sweep_expired(), vec!["w1".to_string()]);

    // Detection + state + report impact (distributed.md policy 1 & 3):
    // LostContact, invalidation time recorded, admission frozen.
    let record = registry.worker("w1").expect("record retained");
    assert_eq!(record.state, WorkerState::LostContact);
    assert!(record.lease_invalidated_at_unix_s.is_some());
    assert_eq!(
        registry.admission_verdicts(&["w1"]),
        vec![(
            "w1".to_string(),
            AdmissionVerdict::Frozen {
                invalidated_at_unix_s: record.lease_invalidated_at_unix_s.unwrap(),
            }
        )],
        "policy 1: new load is frozen while the worker is gone"
    );
    assert!(registry.admitting_workers().is_empty());

    // Recovery (automatic retry is allowed): a fresh beat restores
    // Online and reopens admission.
    assert!(registry.heartbeat("w1", granted));
    assert_eq!(
        registry.worker("w1").expect("record").state,
        WorkerState::Online
    );
    assert_eq!(
        registry.admission_verdicts(&["w1"]),
        vec![("w1".to_string(), AdmissionVerdict::Admit)]
    );
}

#[test]
fn stale_generation_beats_never_overwrite_new_state() {
    let registry = WorkerRegistry::new(1000, 5000).expect("lease config");
    let mut worker = FaultyWorker::new(
        "w1",
        PROTOCOL_VERSION,
        1,
        WorkerFault::StaleGenerationAfter { beats: 1 },
    );
    let granted = register(&registry, &worker);

    // Slot 0: a healthy beat at the granted generation. Slots 1+ : the
    // fault fires — the Worker beats at generation 0, one behind.
    assert_eq!(
        replay(&registry, &mut worker, 3),
        vec![true, false, false],
        "the stale-generation beats are refused (policy 7)"
    );
    // Refused old-generation events leave zero trace: the record is
    // still Online at the granted generation, nothing overwritten.
    let record = registry.worker("w1").expect("record");
    assert_eq!(record.state, WorkerState::Online);
    assert_eq!(record.generation, granted);
    assert!(record.lease_invalidated_at_unix_s.is_none());
}

#[test]
fn deregister_after_leaves_the_record_and_refuses_further_beats() {
    let registry = WorkerRegistry::new(1000, 5000).expect("lease config");
    let mut worker = FaultyWorker::new(
        "w1",
        PROTOCOL_VERSION,
        1,
        WorkerFault::DeregisterAfter { beats: 1 },
    );
    let granted = register(&registry, &worker);

    // Slot 0: healthy beat. Slot 1: the worker leaves explicitly.
    assert_eq!(replay(&registry, &mut worker, 1), vec![true]);
    assert_eq!(
        registry.worker("w1").expect("record").state,
        WorkerState::Online
    );
    assert_eq!(worker.next_beat(), BeatAction::Leave);
    assert!(registry.deregister("w1"), "the leave is applied");

    // Leave is recorded (Deregistered), the record is retained for
    // restart recovery and audit, and later beats are refused.
    assert_eq!(
        registry.worker("w1").expect("record").state,
        WorkerState::Deregistered
    );
    assert!(!registry.heartbeat("w1", granted), "no beats after leave");
    assert!(registry.worker("w1").is_some(), "record retained");
    assert_eq!(
        registry.admission_verdicts(&["w1"]),
        vec![("w1".to_string(), AdmissionVerdict::NotInMesh)],
        "a departed worker never admits new load"
    );
}

#[test]
fn restart_after_rejoins_with_a_fresh_generation_and_old_events_die() {
    let registry = WorkerRegistry::new(1000, 5000).expect("lease config");
    let mut worker = FaultyWorker::new(
        "w1",
        PROTOCOL_VERSION,
        1,
        WorkerFault::RestartAfter { beats: 1 },
    );
    let first_generation = register(&registry, &worker);

    // Slot 0: healthy beat. Slot 1: the process dies and rejoins — a
    // fresh registration grants a NEW generation.
    assert_eq!(replay(&registry, &mut worker, 1), vec![true]);
    assert_eq!(worker.next_beat(), BeatAction::Rejoin);
    let fresh_generation = register(&registry, &worker);
    worker.set_granted_generation(fresh_generation);
    assert!(
        fresh_generation > first_generation,
        "a rejoin always mints a fresh generation"
    );

    // The rejoined worker beats at the fresh generation and is accepted.
    assert_eq!(replay(&registry, &mut worker, 1), vec![true]);
    assert_eq!(
        registry.worker("w1").expect("record").state,
        WorkerState::Online
    );

    // An OLD-generation event from the previous process is refused and
    // leaves the new state untouched (policy 7).
    assert!(
        !registry.heartbeat("w1", first_generation),
        "the dead process's beats never win"
    );
    let record = registry.worker("w1").expect("record");
    assert_eq!(record.generation, fresh_generation);
    assert_eq!(record.state, WorkerState::Online);
}

#[test]
fn protocol_mismatch_is_rejected_at_handshake_and_never_admitted() {
    let registry = WorkerRegistry::new(1000, 5000).expect("lease config");
    let worker = FaultyWorker::new("w1", PROTOCOL_VERSION, 0, WorkerFault::ProtocolMismatch);
    assert!(worker.is_protocol_mismatch());

    // §5.3: the handshake rejects the mismatched worker with a reason —
    // it is never silently degraded into an incompatible mesh.
    let registration = WorkerRegistration {
        worker_id: worker.worker_id().into(),
        protocol_version: worker.register_protocol_version(),
        adapter_version: "0.3".into(),
        engine_version: "kbengine-1.2".into(),
        runtime_version: "0.1".into(),
        capabilities: vec!["exact_arrival".into()],
        resource_capacity: None,
    };
    let decision = registry.register(registration).expect("valid input");
    match decision {
        RegistrationDecision::Rejected { reason } => {
            assert!(
                reason.contains("protocol version"),
                "reason names the mismatch"
            );
        }
        RegistrationDecision::Accepted { .. } => {
            panic!("a mismatched worker must be rejected, never admitted")
        }
    }
    assert!(
        registry.worker("w1").is_none(),
        "rejected workers are not registered"
    );
    assert!(registry.admitting_workers().is_empty());
}
