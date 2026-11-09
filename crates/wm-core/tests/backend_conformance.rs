//! Native Backend conformance test (WM-0321).
//!
//! Validates the wm-core backend state machine against the contract in
//! `docs/backends.md` ("Backend States" + "Every state change must include"):
//!
//! 1. the allowed transition graph is exactly the documented one (every
//!    documented edge accepted, no undocumented edge accepted) — checked
//!    exhaustively over all state pairs;
//! 2. every accepted state change records all required observability fields
//!    (backend_id, swell_id, state, observed_at, reason_code, detail,
//!    generation) with a monotonic observed_at;
//! 3. the failure / recovery paths and the stale-generation guard behave per
//!    the documented contract.

use wm_core::backend::{BackendState, BackendStateMachine};
use wm_core::{BackendId, SwellId};

use BackendState::*;

/// Every allowed edge per docs/backends.md, including the documented-consistent
/// extensions the machine admits (reversible admission stop, a failed
/// handshake landing in a terminal state, degraded recovery). Kept in the test
/// so a change to the machine's table fails here rather than silently drifting
/// from the documented contract.
const DOCUMENTED_EDGES: &[(BackendState, BackendState)] = &[
    // Linear lifecycle (docs/backends.md "Backend States").
    (Created, Handshaking),
    (Handshaking, Ready),
    (Ready, Initialized),
    (Initialized, PlanAccepted),
    (PlanAccepted, Running),
    (Running, AdmissionStopped),
    (AdmissionStopped, Draining),
    (Draining, Stopped),
    // stop_admission is a reversible control action (resume admission).
    (AdmissionStopped, Running),
    // A failed handshake terminates the attempt (Handshaking is the only
    // pre-running state that can reach a terminal state).
    (Handshaking, Degraded),
    (Handshaking, Failed),
    // Any running state may degrade or fail.
    (Ready, Degraded),
    (Ready, Failed),
    (Initialized, Degraded),
    (Initialized, Failed),
    (PlanAccepted, Degraded),
    (PlanAccepted, Failed),
    (Running, Degraded),
    (Running, Failed),
    (AdmissionStopped, Degraded),
    (AdmissionStopped, Failed),
    (Draining, Degraded),
    (Draining, Failed),
    // Recovery: failed / degraded → Recovering → Handshaking; degraded may
    // also re-handshake directly.
    (Failed, Recovering),
    (Degraded, Recovering),
    (Recovering, Handshaking),
    (Degraded, Handshaking),
];

const ALL_STATES: &[BackendState] = &[
    Created,
    Handshaking,
    Ready,
    Initialized,
    PlanAccepted,
    Running,
    AdmissionStopped,
    Draining,
    Stopped,
    Degraded,
    Failed,
    Recovering,
];

/// Drive the machine along the linear lifecycle to `target` (inclusive),
/// returning the machine. `target` must lie on the Created→Stopped chain.
fn drive_linear(target: BackendState) -> BackendStateMachine {
    let mut m = BackendStateMachine::new("backend-1");
    let g = m.generation();
    for &step in &[
        Handshaking,
        Ready,
        Initialized,
        PlanAccepted,
        Running,
        AdmissionStopped,
        Draining,
        Stopped,
    ] {
        m.transition(g, step, "ok", "").unwrap();
        if step == target {
            break;
        }
    }
    m
}

#[test]
fn transition_graph_matches_documented_contract_exactly() {
    for &from in ALL_STATES {
        for &to in ALL_STATES {
            let expected = DOCUMENTED_EDGES.contains(&(from, to));
            assert_eq!(
                from.can_transition_to(to),
                expected,
                "edge {from:?} -> {to:?} diverges from docs/backends.md"
            );
        }
    }
}

#[test]
fn every_transition_records_required_observability_fields() {
    let mut m = BackendStateMachine::new("backend-1");
    let g = m.generation();
    m.bind_swell("swell-001");

    let chain = [
        Handshaking,
        Ready,
        Initialized,
        PlanAccepted,
        Running,
        AdmissionStopped,
        Draining,
        Stopped,
    ];
    let mut prev_from = Created;
    let mut prev_elapsed = std::time::Duration::ZERO;
    for (i, &to) in chain.iter().enumerate() {
        let rec = m.transition(g, to, "ok", format!("step-{i}")).unwrap();
        assert_eq!(rec.backend_id, BackendId("backend-1".into()));
        assert_eq!(rec.swell_id, SwellId("swell-001".into()));
        assert_eq!(rec.from, prev_from, "records chain state correctly");
        assert_eq!(rec.to, to);
        assert_eq!(rec.reason_code, "ok");
        assert_eq!(rec.detail, format!("step-{i}"));
        assert_eq!(rec.generation, g);
        assert!(rec.elapsed >= prev_elapsed, "observed_at must be monotonic");
        prev_elapsed = rec.elapsed;
        prev_from = to;
    }
    assert_eq!(m.history().len(), chain.len());
}

#[test]
fn swell_binding_is_recorded_only_after_bind() {
    let mut m = BackendStateMachine::new("backend-1");
    let g = m.generation();
    // Pre-bind transitions carry an empty (unbound) swell_id.
    m.transition(g, Handshaking, "handshake_requested", "")
        .unwrap();
    m.transition(g, Ready, "handshake_completed", "").unwrap();
    for rec in m.history() {
        assert_eq!(rec.swell_id, SwellId(String::new()));
    }

    m.bind_swell("swell-002");
    m.transition(g, Initialized, "backend_initialized", "")
        .unwrap();
    m.transition(g, PlanAccepted, "plan_accepted", "").unwrap();
    for rec in m.history().iter().skip(2) {
        assert_eq!(rec.swell_id, SwellId("swell-002".into()));
    }
    assert_eq!(m.swell_id(), &SwellId("swell-002".into()));
}

#[test]
fn failure_and_recovery_paths_record_fields_and_generation() {
    let mut m = drive_linear(Running);
    let g = m.generation();

    m.fail("crash", "process exited abnormally").unwrap();
    let rec = m.history().last().unwrap();
    assert_eq!(rec.from, Running);
    assert_eq!(rec.to, Failed);
    assert_eq!(rec.reason_code, "crash");
    assert_eq!(rec.detail, "process exited abnormally");
    assert_eq!(rec.generation, g);

    m.transition(g, Recovering, "recover", "").unwrap();
    m.transition(g, Handshaking, "re-handshake", "").unwrap();
    assert_eq!(m.state(), Handshaking);
    // The recovery path re-enters through Handshaking at the same generation.
    let rec = m.history().last().unwrap();
    assert_eq!(rec.from, Recovering);
    assert_eq!(rec.to, Handshaking);
    assert_eq!(rec.generation, g);
}

#[test]
fn stale_generation_is_rejected_without_recording_or_mutating() {
    let mut m = drive_linear(Ready);
    let old = m.generation();
    let new = m.bump_generation();
    m.bind_swell("swell-003");

    let before = m.history().len();
    let err = m.transition(old, Handshaking, "late", "").unwrap_err();
    assert_eq!(
        err,
        wm_core::backend::StateMachineError::StaleGeneration {
            got: old.0,
            current: new.0,
        }
    );
    assert_eq!(m.history().len(), before, "rejected event records nothing");
    assert_eq!(m.state(), Ready, "rejected event never overwrites state");
    assert_eq!(m.swell_id(), &SwellId("swell-003".into()));
}

#[test]
fn illegal_transition_records_nothing_and_keeps_state() {
    let mut m = drive_linear(PlanAccepted);
    let before = m.history().len();
    let err = m
        .transition(m.generation(), Stopped, "skip", "")
        .unwrap_err();
    assert!(matches!(
        err,
        wm_core::backend::StateMachineError::InvalidTransition { .. }
    ));
    assert_eq!(m.history().len(), before);
    assert_eq!(m.state(), PlanAccepted);
}

#[test]
fn stopped_is_terminal_and_rejects_every_outgoing_edge() {
    let m = drive_linear(Stopped);
    for &to in ALL_STATES {
        assert!(
            !m.state().can_transition_to(to),
            "Stopped must not transition to {to:?}"
        );
    }
}
