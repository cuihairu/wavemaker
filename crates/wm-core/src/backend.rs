//! Execution Backend state machine (WM-0109).
//!
//! State table and observability fields follow `docs/backends.md` ("Backend
//! States"): a linear lifecycle Created → Handshaking → Ready → Initialized →
//! PlanAccepted → Running → AdmissionStopped → Draining → Stopped, with
//! Degraded / Failed reachable from any running state and recovery through
//! Recovering → Handshaking. Every transition records backend_id, state,
//! observed_at, reason_code, detail and generation (docs/backends.md
//! "Every state change must include").
//!
//! Generation guard (docs/supervisor.md §"A Backend after restart"): events
//! stamped with an older generation never overwrite newer state; a restart
//! bumps the generation and the machine re-enters via Handshaking.

use crate::{BackendId, Generation};
use std::time::{Duration, Instant};
use thiserror::Error;

/// Lifecycle states of an execution backend (docs/backends.md).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BackendState {
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
}

impl BackendState {
    /// States from which Degraded / Failed may be entered ("any running
    /// state" in docs/backends.md).
    pub fn is_running(self) -> bool {
        matches!(
            self,
            BackendState::Ready
                | BackendState::Initialized
                | BackendState::PlanAccepted
                | BackendState::Running
                | BackendState::AdmissionStopped
                | BackendState::Draining
        )
    }
}

/// One recorded state change with the observability fields required by
/// docs/backends.md.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TransitionRecord {
    pub backend_id: BackendId,
    pub from: BackendState,
    pub to: BackendState,
    pub reason_code: String,
    pub detail: String,
    pub generation: Generation,
    /// Monotonic age of the machine when the transition happened (tests
    /// inject a virtual clock via `StateMachineOptions`; production uses
    /// process uptime).
    pub elapsed: Duration,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum StateMachineError {
    #[error("invalid transition {from:?} -> {to:?}")]
    InvalidTransition {
        from: BackendState,
        to: BackendState,
    },
    #[error("stale generation {got} rejected: current is {current} (late event must never overwrite newer state)")]
    StaleGeneration { got: u64, current: u64 },
}

/// Options controlling clock and history depth; kept as a struct so tests can
/// pin behavior without global state.
#[derive(Debug, Clone, Copy, Default)]
pub struct StateMachineOptions {
    /// Maximum number of retained transition records; the oldest are dropped
    /// first. `None` retains everything (Phase 1 default).
    pub max_history: Option<usize>,
}

/// Domain state machine for one execution backend.
#[derive(Debug)]
pub struct BackendStateMachine {
    backend_id: BackendId,
    state: BackendState,
    generation: Generation,
    started: Instant,
    history: Vec<TransitionRecord>,
    options: StateMachineOptions,
}

impl BackendStateMachine {
    /// A backend starts in `Created` at generation 1 (the initial generation
    /// assigned by the controller on first registration).
    pub fn new(backend_id: impl Into<BackendId>) -> Self {
        Self::with_options(backend_id, StateMachineOptions::default())
    }

    pub fn with_options(backend_id: impl Into<BackendId>, options: StateMachineOptions) -> Self {
        Self {
            backend_id: backend_id.into(),
            state: BackendState::Created,
            generation: Generation::INITIAL,
            started: Instant::now(),
            history: Vec::new(),
            options,
        }
    }

    pub fn backend_id(&self) -> &BackendId {
        &self.backend_id
    }

    pub fn state(&self) -> BackendState {
        self.state
    }

    pub fn generation(&self) -> Generation {
        self.generation
    }

    pub fn history(&self) -> &[TransitionRecord] {
        &self.history
    }

    /// Restart bookkeeping: bumps the generation. Subsequent transitions
    /// stamped with the old generation are rejected.
    pub fn bump_generation(&mut self) -> Generation {
        self.generation = self.generation.next();
        self.generation
    }

    /// Apply a state transition stamped with `generation`.
    ///
    /// Late events from an older generation are rejected without mutating
    /// any state (docs/supervisor.md: "Late events from an old generation
    /// must not overwrite new state").
    pub fn transition(
        &mut self,
        generation: Generation,
        to: BackendState,
        reason_code: impl Into<String>,
        detail: impl Into<String>,
    ) -> Result<&TransitionRecord, StateMachineError> {
        if generation < self.generation {
            return Err(StateMachineError::StaleGeneration {
                got: generation.0,
                current: self.generation.0,
            });
        }

        if !is_allowed(self.state, to) {
            return Err(StateMachineError::InvalidTransition {
                from: self.state,
                to,
            });
        }

        let record = TransitionRecord {
            backend_id: self.backend_id.clone(),
            from: self.state,
            to,
            reason_code: reason_code.into(),
            detail: detail.into(),
            generation: self.generation,
            elapsed: self.started.elapsed(),
        };
        self.state = to;

        if let Some(max) = self.options.max_history {
            while self.history.len() >= max {
                self.history.remove(0);
            }
        }
        self.history.push(record);
        Ok(self.history.last().expect("just pushed"))
    }

    /// Degradation entry point (any running state → Degraded).
    pub fn degrade(
        &mut self,
        reason_code: impl Into<String>,
        detail: impl Into<String>,
    ) -> Result<&TransitionRecord, StateMachineError> {
        let gen = self.generation;
        self.transition(gen, BackendState::Degraded, reason_code, detail)
    }

    /// Failure entry point (any running state → Failed).
    pub fn fail(
        &mut self,
        reason_code: impl Into<String>,
        detail: impl Into<String>,
    ) -> Result<&TransitionRecord, StateMachineError> {
        let gen = self.generation;
        self.transition(gen, BackendState::Failed, reason_code, detail)
    }
}

/// The transition table from docs/backends.md.
fn is_allowed(from: BackendState, to: BackendState) -> bool {
    use BackendState::*;
    match (from, to) {
        // Linear lifecycle.
        (Created, Handshaking)
        | (Handshaking, Ready)
        | (Ready, Initialized)
        | (Initialized, PlanAccepted)
        | (PlanAccepted, Running)
        | (Running, AdmissionStopped)
        | (AdmissionStopped, Draining)
        | (Draining, Stopped) => true,
        // Resume admission is a real control action.
        (AdmissionStopped, Running) => true,
        // Failure paths: any running state may degrade or fail; a failed
        // handshake also lands in Failed.
        (_, Degraded) | (_, Failed) if from.is_running() || from == Handshaking => true,
        // Recovery: Failed (and Degraded) → Recovering → Handshaking;
        // Degraded may also re-handshake directly.
        (Failed, Recovering) | (Degraded, Recovering) | (Recovering, Handshaking) => true,
        (Degraded, Handshaking) => true,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn machine() -> BackendStateMachine {
        BackendStateMachine::new("backend-1")
    }

    /// Walk the linear lifecycle with the given machine.
    fn walk_lifecycle(m: &mut BackendStateMachine) {
        let g = m.generation();
        for to in [
            BackendState::Handshaking,
            BackendState::Ready,
            BackendState::Initialized,
            BackendState::PlanAccepted,
            BackendState::Running,
            BackendState::AdmissionStopped,
            BackendState::Draining,
            BackendState::Stopped,
        ] {
            m.transition(g, to, "ok", "").expect("legal step");
        }
    }

    #[test]
    fn linear_lifecycle_is_accepted_in_order() {
        let mut m = machine();
        assert_eq!(m.state(), BackendState::Created);
        assert_eq!(m.generation(), Generation::INITIAL);
        walk_lifecycle(&mut m);
        assert_eq!(m.state(), BackendState::Stopped);
        assert_eq!(m.history().len(), 8);
    }

    #[test]
    fn illegal_jump_is_rejected_and_state_untouched() {
        let mut m = machine();
        let err = m
            .transition(Generation::INITIAL, BackendState::Running, "skip", "")
            .unwrap_err();
        assert_eq!(
            err,
            StateMachineError::InvalidTransition {
                from: BackendState::Created,
                to: BackendState::Running
            }
        );
        assert_eq!(m.state(), BackendState::Created);
        assert!(m.history().is_empty());
    }

    #[test]
    fn stopped_is_terminal() {
        let mut m = machine();
        walk_lifecycle(&mut m);
        let err = m
            .transition(m.generation(), BackendState::Ready, "restart", "")
            .unwrap_err();
        assert!(matches!(err, StateMachineError::InvalidTransition { .. }));
    }

    /// Drive a machine through the linear lifecycle up to `start` and apply
    /// `action`. `start` must be on the Handshaking→…→Draining chain.
    fn drive_to(
        start: BackendState,
        action: impl FnOnce(&mut BackendStateMachine),
    ) -> BackendStateMachine {
        let chain = [
            BackendState::Handshaking,
            BackendState::Ready,
            BackendState::Initialized,
            BackendState::PlanAccepted,
            BackendState::Running,
            BackendState::AdmissionStopped,
            BackendState::Draining,
        ];
        let mut m = machine();
        let g = m.generation();
        for &step in &chain {
            m.transition(g, step, "ok", "").unwrap();
            if step == start {
                break;
            }
        }
        assert_eq!(
            m.state(),
            start,
            "drive_to must land on the requested state"
        );
        action(&mut m);
        m
    }

    #[test]
    fn any_running_state_can_degrade_and_fail() {
        for start in [
            BackendState::Ready,
            BackendState::Initialized,
            BackendState::PlanAccepted,
            BackendState::Running,
            BackendState::AdmissionStopped,
            BackendState::Draining,
        ] {
            let m = drive_to(start, |m| {
                m.degrade("overload", "rss above budget").unwrap();
            });
            assert_eq!(m.state(), BackendState::Degraded);
        }
    }

    #[test]
    fn any_running_state_can_fail() {
        for start in [
            BackendState::Ready,
            BackendState::Running,
            BackendState::Draining,
        ] {
            let m = drive_to(start, |m| {
                m.fail("crash", "process exited abnormally").unwrap();
            });
            assert_eq!(m.state(), BackendState::Failed);
        }
    }

    #[test]
    fn created_and_stopped_are_not_running_states() {
        assert!(!BackendState::Created.is_running());
        assert!(!BackendState::Handshaking.is_running());
        assert!(!BackendState::Stopped.is_running());
        assert!(BackendState::Running.is_running());
    }

    #[test]
    fn failed_recovers_through_recovering_into_handshaking() {
        let mut m = machine();
        let g = m.generation();
        m.transition(g, BackendState::Handshaking, "ok", "")
            .unwrap();
        m.transition(g, BackendState::Ready, "ok", "").unwrap();
        m.fail("crash", "lost contact").unwrap();

        m.transition(g, BackendState::Recovering, "recover", "")
            .unwrap();
        m.transition(g, BackendState::Handshaking, "re-handshake", "")
            .unwrap();
        assert_eq!(m.state(), BackendState::Handshaking);
    }

    #[test]
    fn degraded_may_recover_or_rehandshake_directly() {
        let mut m = machine();
        let g = m.generation();
        m.transition(g, BackendState::Handshaking, "ok", "")
            .unwrap();
        m.transition(g, BackendState::Ready, "ok", "").unwrap();
        m.degrade("overload", "").unwrap();
        m.transition(g, BackendState::Recovering, "recover", "")
            .unwrap();
        assert_eq!(m.state(), BackendState::Recovering);

        let mut m2 = machine();
        let g2 = m2.generation();
        m2.transition(g2, BackendState::Handshaking, "ok", "")
            .unwrap();
        m2.transition(g2, BackendState::Ready, "ok", "").unwrap();
        m2.degrade("overload", "").unwrap();
        m2.transition(g2, BackendState::Handshaking, "re-handshake", "")
            .unwrap();
        assert_eq!(m2.state(), BackendState::Handshaking);
    }

    #[test]
    fn stale_generation_never_overwrites_newer_state() {
        let mut m = machine();
        let old = m.generation();
        let new = m.bump_generation();
        assert_eq!(new, Generation(2));

        // Late event stamped with the pre-restart generation.
        let err = m
            .transition(old, BackendState::Handshaking, "late", "")
            .unwrap_err();
        assert_eq!(
            err,
            StateMachineError::StaleGeneration { got: 1, current: 2 }
        );
        assert_eq!(m.state(), BackendState::Created);

        // Same-generation transitions still work after the bump.
        m.transition(new, BackendState::Handshaking, "ok", "")
            .unwrap();
        assert_eq!(m.state(), BackendState::Handshaking);
    }

    #[test]
    fn records_carry_required_observability_fields() {
        let mut m = machine();
        let g = m.generation();
        let rec = m
            .transition(g, BackendState::Handshaking, "handshake_requested", "v1")
            .unwrap();
        assert_eq!(rec.backend_id, BackendId("backend-1".into()));
        assert_eq!(rec.from, BackendState::Created);
        assert_eq!(rec.to, BackendState::Handshaking);
        assert_eq!(rec.reason_code, "handshake_requested");
        assert_eq!(rec.detail, "v1");
        assert_eq!(rec.generation, Generation::INITIAL);
    }

    #[test]
    fn max_history_bounds_retention() {
        let mut m = BackendStateMachine::with_options(
            "backend-1",
            StateMachineOptions {
                max_history: Some(2),
            },
        );
        let g = m.generation();
        m.transition(g, BackendState::Handshaking, "a", "").unwrap();
        m.transition(g, BackendState::Ready, "b", "").unwrap();
        m.transition(g, BackendState::Degraded, "c", "").unwrap();
        assert_eq!(m.history().len(), 2);
        assert_eq!(m.history()[0].reason_code, "b");
        assert_eq!(m.history()[1].reason_code, "c");
    }

    #[test]
    fn admission_can_resume_before_draining() {
        let mut m = machine();
        let g = m.generation();
        for to in [
            BackendState::Handshaking,
            BackendState::Ready,
            BackendState::Initialized,
            BackendState::PlanAccepted,
            BackendState::Running,
            BackendState::AdmissionStopped,
        ] {
            m.transition(g, to, "ok", "").unwrap();
        }
        m.transition(g, BackendState::Running, "resume_admission", "")
            .unwrap();
        assert_eq!(m.state(), BackendState::Running);
    }
}
