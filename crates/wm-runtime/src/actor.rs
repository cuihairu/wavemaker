//! Actor state machine (WM-0301).
//!
//! Execution states of one simulated actor: `Idle -> Scheduled -> Running ->
//! Waiting -> Closed` with `Running <-> Waiting` loops and `Failed` reachable
//! from any non-terminal state. Every transition carries a timestamp and a
//! reason (docs/native-runtime.md: "Every state transition must record a
//! timestamp and a reason").

/// Execution states of an actor.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ActorState {
    /// Created, not yet picked up by the scheduler.
    Idle,
    /// Arrival time reached, queued to start.
    Scheduled,
    /// Behavior is executing.
    Running,
    /// Behavior is blocked on a timer or message.
    Waiting,
    /// Terminal: finished normally (completed or drained).
    Closed,
    /// Terminal: errored or force-stopped.
    Failed,
}

impl ActorState {
    pub fn is_terminal(self) -> bool {
        matches!(self, ActorState::Closed | ActorState::Failed)
    }
}

/// One recorded state change.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Transition {
    pub from: ActorState,
    pub to: ActorState,
    /// Clock nanoseconds at the transition.
    pub at_ns: u64,
    /// Free-form reason (e.g. "arrival reached", "timer fired", "drain").
    pub reason: String,
}

#[derive(Debug, thiserror::Error)]
#[error("illegal actor transition {from:?} -> {to:?}")]
pub struct ActorStateError {
    pub from: ActorState,
    pub to: ActorState,
}

/// Validated, fully audited state machine for one actor.
#[derive(Debug, Clone)]
pub struct ActorMachine {
    state: ActorState,
    history: Vec<Transition>,
}

impl ActorMachine {
    pub fn new() -> Self {
        Self {
            state: ActorState::Idle,
            history: Vec::new(),
        }
    }

    pub fn state(&self) -> ActorState {
        self.state
    }

    pub fn history(&self) -> &[Transition] {
        &self.history
    }

    /// Apply a transition, rejecting illegal ones without mutating anything.
    ///
    /// # Panics
    /// Panics if `at_ns` is earlier than the previous transition's timestamp:
    /// transitions must be monotonic in clock time.
    pub fn transition(
        &mut self,
        to: ActorState,
        at_ns: u64,
        reason: impl Into<String>,
    ) -> Result<(), ActorStateError> {
        if Self::is_legal(self.state, to) {
            if let Some(last) = self.history.last() {
                assert!(
                    at_ns >= last.at_ns,
                    "actor transitions must be monotonic in time: {at_ns} < {}",
                    last.at_ns
                );
            }
            let from = self.state;
            self.state = to;
            self.history.push(Transition {
                from,
                to,
                at_ns,
                reason: reason.into(),
            });
            Ok(())
        } else {
            Err(ActorStateError {
                from: self.state,
                to,
            })
        }
    }

    /// Legal transitions from every state.
    ///
    /// - `Idle -> {Scheduled, Failed}`
    /// - `Scheduled -> {Running, Closed, Failed}`
    /// - `Running -> {Waiting, Closed, Failed}`
    /// - `Waiting -> {Running, Closed, Failed}`
    /// - terminal states transition nowhere.
    pub fn is_legal(from: ActorState, to: ActorState) -> bool {
        if from == to {
            return false;
        }
        match from {
            ActorState::Idle => matches!(to, ActorState::Scheduled | ActorState::Failed),
            ActorState::Scheduled | ActorState::Running | ActorState::Waiting => {
                matches!(
                    to,
                    ActorState::Running
                        | ActorState::Waiting
                        | ActorState::Closed
                        | ActorState::Failed
                )
            }
            ActorState::Closed | ActorState::Failed => false,
        }
    }
}

impl Default for ActorMachine {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn happy_path_records_timestamps_and_reasons() {
        let mut actor = ActorMachine::new();
        assert_eq!(actor.state(), ActorState::Idle);
        actor
            .transition(ActorState::Scheduled, 100, "arrival reached")
            .unwrap();
        actor
            .transition(ActorState::Running, 110, "tick started")
            .unwrap();
        actor
            .transition(ActorState::Waiting, 200, "think timer armed")
            .unwrap();
        actor
            .transition(ActorState::Running, 300, "timer fired")
            .unwrap();
        actor
            .transition(ActorState::Closed, 400, "behavior finished")
            .unwrap();
        assert_eq!(actor.state(), ActorState::Closed);
        assert!(actor.state().is_terminal());

        let history = actor.history();
        assert_eq!(history.len(), 5);
        let reasons: Vec<&str> = history.iter().map(|t| t.reason.as_str()).collect();
        assert_eq!(
            reasons,
            vec![
                "arrival reached",
                "tick started",
                "think timer armed",
                "timer fired",
                "behavior finished"
            ]
        );
        let stamps: Vec<u64> = history.iter().map(|t| t.at_ns).collect();
        assert!(
            stamps.windows(2).all(|w| w[0] <= w[1]),
            "timestamps monotonic"
        );
        assert_eq!(history[0].from, ActorState::Idle);
        assert_eq!(history[4].to, ActorState::Closed);
    }

    #[test]
    fn waiting_running_loop_is_legal() {
        let mut actor = ActorMachine::new();
        actor.transition(ActorState::Scheduled, 0, "s").unwrap();
        for i in 0..5u64 {
            actor
                .transition(ActorState::Waiting, i * 10, "wait")
                .unwrap();
            actor
                .transition(ActorState::Running, i * 10 + 5, "resume")
                .unwrap();
        }
        assert_eq!(actor.state(), ActorState::Running);
        assert_eq!(actor.history().len(), 11);
    }

    #[test]
    fn drain_paths_reach_closed() {
        // Never started: Scheduled -> Closed.
        let mut never_started = ActorMachine::new();
        never_started
            .transition(ActorState::Scheduled, 5, "arrival")
            .unwrap();
        never_started
            .transition(ActorState::Closed, 10, "drained before start")
            .unwrap();
        // Waiting actors are drained too.
        let mut waiting = ActorMachine::new();
        waiting
            .transition(ActorState::Scheduled, 0, "arrival")
            .unwrap();
        waiting.transition(ActorState::Running, 1, "start").unwrap();
        waiting
            .transition(ActorState::Waiting, 2, "blocked")
            .unwrap();
        waiting.transition(ActorState::Closed, 3, "drain").unwrap();
    }

    #[test]
    fn failed_is_reachable_from_every_nonterminal_state() {
        for from in [
            ActorState::Idle,
            ActorState::Scheduled,
            ActorState::Running,
            ActorState::Waiting,
        ] {
            let mut actor = ActorMachine::new();
            if from != ActorState::Idle {
                actor
                    .transition(ActorState::Scheduled, 0, "arrival")
                    .unwrap();
                if from == ActorState::Running || from == ActorState::Waiting {
                    actor.transition(ActorState::Running, 1, "start").unwrap();
                }
                if from == ActorState::Waiting {
                    actor.transition(ActorState::Waiting, 2, "block").unwrap();
                }
            }
            actor
                .transition(ActorState::Failed, 10, "behavior panicked")
                .unwrap_or_else(|e| panic!("{from:?}: {e}"));
            assert!(actor.transition(ActorState::Idle, 20, "restart").is_err());
        }
    }

    #[test]
    fn illegal_transitions_are_rejected_without_mutation() {
        let mut actor = ActorMachine::new();
        assert!(actor
            .transition(ActorState::Running, 0, "skip scheduling")
            .is_err());
        assert_eq!(
            actor.state(),
            ActorState::Idle,
            "failed transition left no trace"
        );
        assert!(actor.history().is_empty());

        actor
            .transition(ActorState::Scheduled, 0, "arrival")
            .unwrap();
        actor.transition(ActorState::Running, 1, "start").unwrap();
        actor.transition(ActorState::Closed, 2, "done").unwrap();
        for to in [
            ActorState::Idle,
            ActorState::Scheduled,
            ActorState::Running,
            ActorState::Waiting,
            ActorState::Closed,
            ActorState::Failed,
        ] {
            assert!(actor.transition(to, 3, "afterlife").is_err(), "{to:?}");
        }
        // Self-transition is illegal everywhere.
        let mut idle = ActorMachine::new();
        assert!(idle.transition(ActorState::Idle, 0, "self").is_err());
    }

    #[test]
    #[should_panic(expected = "monotonic")]
    fn backwards_timestamps_are_refused() {
        let mut actor = ActorMachine::new();
        actor
            .transition(ActorState::Scheduled, 100, "arrival")
            .unwrap();
        actor
            .transition(ActorState::Running, 50, "time travel")
            .unwrap();
    }
}
