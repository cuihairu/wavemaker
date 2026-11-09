//! Session state machine (WM-0308).
//!
//! Connection lifecycle of one simulated session, mirroring
//! docs/native-runtime.md: `Planned -> Scheduled -> Creating -> Connecting ->
//! Authenticating -> Active -> Draining -> Closed`, with `Failed` reachable
//! from any non-terminal state. Every transition records a timestamp and a
//! reason (docs/native-runtime.md "Actor Lifecycle" requirement applies to
//! sessions too).
//!
//! Reconnection is a Session Adapter concern (WM-0309+); the v1 machine is a
//! linear chain and a reconnect is modeled as a fresh session machine.

/// Connection states of a session.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SessionState {
    /// Session planned, not yet picked up by the scheduler.
    Planned,
    /// Queued to begin connecting.
    Scheduled,
    /// Underlying connection object being created.
    Creating,
    /// TCP/transport connection established, handshake in progress.
    Connecting,
    /// Login / auth exchange in progress.
    Authenticating,
    /// Authenticated and serving behavior.
    Active,
    /// Graceful shutdown in progress.
    Draining,
    /// Terminal: fully closed.
    Closed,
    /// Terminal: errored or force-closed.
    Failed,
}

impl SessionState {
    pub fn is_terminal(self) -> bool {
        matches!(self, SessionState::Closed | SessionState::Failed)
    }
}

/// One recorded session state change.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionTransition {
    pub from: SessionState,
    pub to: SessionState,
    /// Clock nanoseconds at the transition.
    pub at_ns: u64,
    /// Free-form reason (e.g. "login ok", "auth timeout", "drain requested").
    pub reason: String,
}

#[derive(Debug, thiserror::Error)]
#[error("illegal session transition {from:?} -> {to:?}")]
pub struct SessionStateError {
    pub from: SessionState,
    pub to: SessionState,
}

/// Validated, fully audited state machine for one session.
#[derive(Debug, Clone)]
pub struct SessionMachine {
    state: SessionState,
    history: Vec<SessionTransition>,
}

impl SessionMachine {
    pub fn new() -> Self {
        Self {
            state: SessionState::Planned,
            history: Vec::new(),
        }
    }

    pub fn state(&self) -> SessionState {
        self.state
    }

    pub fn history(&self) -> &[SessionTransition] {
        &self.history
    }

    /// Apply a transition, rejecting illegal ones without mutating anything.
    ///
    /// # Panics
    /// Panics if `at_ns` is earlier than the previous transition's timestamp.
    pub fn transition(
        &mut self,
        to: SessionState,
        at_ns: u64,
        reason: impl Into<String>,
    ) -> Result<(), SessionStateError> {
        if Self::is_legal(self.state, to) {
            if let Some(last) = self.history.last() {
                assert!(
                    at_ns >= last.at_ns,
                    "session transitions must be monotonic in time: {at_ns} < {}",
                    last.at_ns
                );
            }
            let from = self.state;
            self.state = to;
            self.history.push(SessionTransition {
                from,
                to,
                at_ns,
                reason: reason.into(),
            });
            Ok(())
        } else {
            Err(SessionStateError {
                from: self.state,
                to,
            })
        }
    }

    /// Legal transitions from every state (linear chain per the design doc).
    pub fn is_legal(from: SessionState, to: SessionState) -> bool {
        if from == to {
            return false;
        }
        match from {
            SessionState::Planned => matches!(to, SessionState::Scheduled | SessionState::Failed),
            SessionState::Scheduled => matches!(to, SessionState::Creating | SessionState::Failed),
            SessionState::Creating => matches!(to, SessionState::Connecting | SessionState::Failed),
            SessionState::Connecting => {
                matches!(to, SessionState::Authenticating | SessionState::Failed)
            }
            SessionState::Authenticating => {
                matches!(to, SessionState::Active | SessionState::Failed)
            }
            SessionState::Active => matches!(to, SessionState::Draining | SessionState::Failed),
            SessionState::Draining => matches!(to, SessionState::Closed | SessionState::Failed),
            SessionState::Closed | SessionState::Failed => false,
        }
    }
}

impl Default for SessionMachine {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn happy_path_records_timestamps_and_reasons() {
        let mut s = SessionMachine::new();
        assert_eq!(s.state(), SessionState::Planned);
        for (to, at, reason) in [
            (SessionState::Scheduled, 100, "arrival reached"),
            (SessionState::Creating, 110, "start creating"),
            (SessionState::Connecting, 200, "tcp connected"),
            (SessionState::Authenticating, 250, "login started"),
            (SessionState::Active, 400, "login ok"),
            (SessionState::Draining, 900, "drain requested"),
            (SessionState::Closed, 1000, "closed"),
        ] {
            s.transition(to, at, reason).unwrap();
        }
        assert_eq!(s.state(), SessionState::Closed);
        assert!(s.state().is_terminal());
        let history = s.history();
        assert_eq!(history.len(), 7);
        let reasons: Vec<&str> = history.iter().map(|t| t.reason.as_str()).collect();
        assert_eq!(
            reasons,
            vec![
                "arrival reached",
                "start creating",
                "tcp connected",
                "login started",
                "login ok",
                "drain requested",
                "closed"
            ]
        );
        let stamps: Vec<u64> = history.iter().map(|t| t.at_ns).collect();
        assert!(stamps.windows(2).all(|w| w[0] <= w[1]), "monotonic");
        assert_eq!(history[0].from, SessionState::Planned);
        assert_eq!(history[6].to, SessionState::Closed);
    }

    #[test]
    fn failed_is_reachable_from_every_nonterminal_state() {
        for from in [
            SessionState::Planned,
            SessionState::Scheduled,
            SessionState::Creating,
            SessionState::Connecting,
            SessionState::Authenticating,
            SessionState::Active,
            SessionState::Draining,
        ] {
            let mut s = SessionMachine::new();
            // Walk the chain up to `from`.
            let chain = [
                SessionState::Scheduled,
                SessionState::Creating,
                SessionState::Connecting,
                SessionState::Authenticating,
                SessionState::Active,
                SessionState::Draining,
            ];
            for step in chain {
                if step == from {
                    break;
                }
                s.transition(step, 10, "advance").unwrap();
            }
            s.transition(SessionState::Failed, 20, "error")
                .unwrap_or_else(|e| panic!("{from:?}: {e}"));
            assert_eq!(s.state(), SessionState::Failed);
            assert!(s.state().is_terminal());
        }
    }

    #[test]
    fn illegal_transitions_are_rejected_without_mutation() {
        let mut s = SessionMachine::new();
        // Skip-ahead is illegal.
        assert!(s.transition(SessionState::Active, 0, "skip").is_err());
        assert_eq!(s.state(), SessionState::Planned);
        assert!(s.history().is_empty());

        // Drive to Active, then check terminal afterlife and backwards jumps.
        for to in [
            SessionState::Scheduled,
            SessionState::Creating,
            SessionState::Connecting,
            SessionState::Authenticating,
            SessionState::Active,
        ] {
            s.transition(to, 10, "advance").unwrap();
        }
        assert!(s
            .transition(SessionState::Closed, 20, "close early")
            .is_err());
        assert!(s.transition(SessionState::Scheduled, 20, "rewind").is_err());
        // Self-transition is illegal.
        assert!(s.transition(SessionState::Active, 20, "self").is_err());

        s.transition(SessionState::Draining, 30, "drain").unwrap();
        s.transition(SessionState::Closed, 40, "close").unwrap();
        for to in [
            SessionState::Planned,
            SessionState::Active,
            SessionState::Draining,
            SessionState::Closed,
            SessionState::Failed,
        ] {
            assert!(s.transition(to, 50, "afterlife").is_err(), "{to:?}");
        }
    }

    #[test]
    #[should_panic(expected = "monotonic")]
    fn backwards_timestamps_are_refused() {
        let mut s = SessionMachine::new();
        s.transition(SessionState::Scheduled, 100, "s").unwrap();
        s.transition(SessionState::Creating, 50, "time travel")
            .unwrap();
    }
}
