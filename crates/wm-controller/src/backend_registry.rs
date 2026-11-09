//! The Backend Registry (WM-0502) — the controller's authoritative view
//! of execution backends (ADR-005).
//!
//! D1: v1 has two backend kinds — Native Runtime and Engine Bot —
//! behind one concept. A `weight` may be declared but is stored
//! VERBATIM: it is illustrative for allocation (the real allocation is
//! a deterministic expansion into a ShardPlan, WM-0504), so the
//! registry interprets nothing.
//!
//! D3: the backend state machine is
//! `Created -> Handshaking -> Ready -> Initialized -> PlanAccepted ->
//!  Running -> AdmissionStopped -> Draining -> Stopped`
//! with `Degraded`/`Failed` reachable from any running state and
//! `Failed -> Recovering -> Handshaking` the rejoin path. Two honesty
//! rules are structural here:
//!
//! - states are only ever entered through REPORTED transitions, never
//!   because a command "sent successfully" — the caller reports what
//!   the backend observed, the registry validates legality;
//! - every transition record carries the mandatory D3 fields
//!   (backend_id, swell_id, state, observed_at, reason_code, detail,
//!   generation) so reconstruction after a controller restart (WM-0512)
//!   can replay history instead of guessing it.

use std::collections::HashMap;
use std::sync::Mutex;

/// Which of the two v1 backend kinds this is (ADR-005 D1/D2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BackendKind {
    /// Rust Worker executing actors itself (Mode C).
    NativeRuntime,
    /// Supervisor + engine-native bots + Bridge (Mode A; KBEngine
    /// adapter in v1).
    EngineBot,
}

/// D3 backend state machine states.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
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
    /// The D3 machine as an explicit table. Degraded/Failed are
    /// reachable from every running state ("任意运行态"); `Created` is
    /// pre-execution — a backend that never handshakes simply stays
    /// Created, degrading nothing. `Stopped` is terminal.
    fn legal_successors(self) -> &'static [BackendState] {
        use BackendState::*;
        match self {
            Created => &[Handshaking],
            Handshaking => &[Ready, Degraded, Failed],
            Ready => &[Initialized, Degraded, Failed],
            Initialized => &[PlanAccepted, Degraded, Failed],
            PlanAccepted => &[Running, Degraded, Failed],
            Running => &[AdmissionStopped, Degraded, Failed],
            AdmissionStopped => &[Draining, Degraded, Failed],
            Draining => &[Stopped, Degraded, Failed],
            // Degraded may stack (another reason) and can only exit
            // through the failure/recovery paths D3 defines — never
            // silently back into a running state.
            Degraded => &[Degraded, Failed, Recovering],
            Failed => &[Recovering],
            Recovering => &[Handshaking, Degraded, Failed],
            Stopped => &[],
        }
    }
}

/// One D3 transition record — every field the ADR makes mandatory.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct BackendTransition {
    pub backend_id: String,
    /// Swell the backend is bound to; `None` before any binding exists
    /// (e.g. the initial registration). Never fabricated.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub swell_id: Option<String>,
    pub state: BackendState,
    /// Seconds since the Unix epoch — when the controller OBSERVED the
    /// transition (reported by the backend, not inferred).
    pub observed_at_unix_s: u64,
    /// Machine-readable reason (e.g. `handshake-ok`, `lost-contact`).
    pub reason_code: String,
    /// Free-form detail, stored verbatim.
    pub detail: String,
    /// Per-backend epoch: bumped on every accepted transition so
    /// consumers can tell record vintages apart (WM-0508 defines the
    /// resync semantics over these).
    pub generation: u64,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct BackendRecord {
    pub backend_id: String,
    pub kind: BackendKind,
    /// Allocation hint, stored verbatim — D1: illustrative only; the
    /// deterministic plan expansion (WM-0504) is the real allocator.
    pub weight: f64,
    /// Latest self-declared resource envelope (WM-0503); `None` until
    /// the backend declares one.
    #[serde(default)]
    pub capacity: Option<crate::capacity::ResourceCapacity>,
    pub state: BackendState,
    pub generation: u64,
    /// Full D3 history, in order, first entry = registration.
    pub transitions: Vec<BackendTransition>,
}

impl BackendRecord {
    pub fn current(&self) -> &BackendTransition {
        self.transitions
            .last()
            .expect("records always carry the registration entry")
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum BackendRegistryError {
    #[error("backend id must not be empty")]
    EmptyBackendId,
    #[error("backend {0} is already registered")]
    DuplicateBackend(String),
    #[error("backend {0} is not registered")]
    UnknownBackend(String),
    #[error("reason_code must not be empty: a state change without a reason is unauditable")]
    EmptyReasonCode,
    #[error("stale report for {backend}: reported generation {reported} is behind current {current}; old-generation events never overwrite new state")]
    StaleGeneration {
        backend: String,
        reported: u64,
        current: u64,
    },
    #[error("future report for {backend}: reported generation {reported} is ahead of current {current}; a gap needs a resync, not a blind accept")]
    FutureGeneration {
        backend: String,
        reported: u64,
        current: u64,
    },
    #[error("illegal transition for {backend}: {from:?} -> {to:?} (legal: {legal:?})")]
    IllegalTransition {
        backend: String,
        from: BackendState,
        to: BackendState,
        legal: Vec<BackendState>,
    },
}

fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[derive(Debug)]
pub struct BackendRegistry {
    inner: Mutex<HashMap<String, BackendRecord>>,
}

impl BackendRegistry {
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(HashMap::new()),
        }
    }

    /// Register a backend; the initial state is `Created` and the
    /// registration itself is the first transition record.
    pub fn register(
        &self,
        backend_id: &str,
        kind: BackendKind,
        weight: f64,
    ) -> Result<BackendRecord, BackendRegistryError> {
        if backend_id.trim().is_empty() {
            return Err(BackendRegistryError::EmptyBackendId);
        }
        let mut inner = self.inner.lock().unwrap();
        if inner.contains_key(backend_id) {
            return Err(BackendRegistryError::DuplicateBackend(backend_id.into()));
        }
        let record = BackendRecord {
            backend_id: backend_id.into(),
            kind,
            weight,
            capacity: None,
            state: BackendState::Created,
            generation: 1,
            transitions: vec![BackendTransition {
                backend_id: backend_id.into(),
                swell_id: None,
                state: BackendState::Created,
                observed_at_unix_s: unix_now(),
                reason_code: "registered".into(),
                detail: String::new(),
                generation: 1,
            }],
        };
        inner.insert(backend_id.into(), record.clone());
        Ok(record)
    }

    /// Record a REPORTED transition, stamped with the generation the
    /// REPORTER observed (WM-0508). Legality is the D3 machine; the
    /// registry's epoch bumps on every accepted change. Generation
    /// guard (distributed.md policy 7): a report from an OLD epoch is
    /// refused — dropped or archived by the caller, never applied; a
    /// report claiming a FUTURE epoch is refused too (a gap is a resync
    /// problem, not a blind accept). Only reports observing the current
    /// epoch are applied. The controller never moves a backend's state
    /// as a side effect of sending a command.
    pub fn report_transition(
        &self,
        backend_id: &str,
        observed_generation: u64,
        to: BackendState,
        swell_id: Option<String>,
        reason_code: &str,
        detail: &str,
    ) -> Result<BackendTransition, BackendRegistryError> {
        if reason_code.trim().is_empty() {
            return Err(BackendRegistryError::EmptyReasonCode);
        }
        let mut inner = self.inner.lock().unwrap();
        let record = inner
            .get_mut(backend_id)
            .ok_or_else(|| BackendRegistryError::UnknownBackend(backend_id.into()))?;
        let current = record.generation;
        if observed_generation < current {
            return Err(BackendRegistryError::StaleGeneration {
                backend: backend_id.into(),
                reported: observed_generation,
                current,
            });
        }
        if observed_generation > current {
            return Err(BackendRegistryError::FutureGeneration {
                backend: backend_id.into(),
                reported: observed_generation,
                current,
            });
        }
        let from = record.state;
        if !record.state.legal_successors().contains(&to) {
            return Err(BackendRegistryError::IllegalTransition {
                backend: backend_id.into(),
                from,
                to,
                legal: record.state.legal_successors().to_vec(),
            });
        }
        record.generation += 1;
        let transition = BackendTransition {
            backend_id: backend_id.into(),
            swell_id,
            state: to,
            observed_at_unix_s: unix_now(),
            reason_code: reason_code.into(),
            detail: detail.into(),
            generation: record.generation,
        };
        record.state = to;
        record.transitions.push(transition.clone());
        Ok(transition)
    }

    /// Store a (re-)declared resource envelope, verbatim. This is a
    /// declaration update, NOT a D3 state change — the generation does
    /// not move. `false` = unknown backend.
    pub fn declare_capacity(
        &self,
        backend_id: &str,
        capacity: crate::capacity::ResourceCapacity,
    ) -> bool {
        let mut inner = self.inner.lock().unwrap();
        match inner.get_mut(backend_id) {
            Some(record) => {
                record.capacity = Some(capacity);
                true
            }
            None => false,
        }
    }

    pub fn backend(&self, backend_id: &str) -> Option<BackendRecord> {
        self.inner.lock().unwrap().get(backend_id).cloned()
    }

    /// All records ordered by backend id (stable API output).
    pub fn backends(&self) -> Vec<BackendRecord> {
        let inner = self.inner.lock().unwrap();
        let mut records: Vec<BackendRecord> = inner.values().cloned().collect();
        records.sort_by(|a, b| a.backend_id.cmp(&b.backend_id));
        records
    }

    pub fn by_kind(&self, kind: BackendKind) -> Vec<BackendRecord> {
        self.backends()
            .into_iter()
            .filter(|record| record.kind == kind)
            .collect()
    }

    pub fn by_state(&self, state: BackendState) -> Vec<BackendRecord> {
        self.backends()
            .into_iter()
            .filter(|record| record.state == state)
            .collect()
    }
}

impl Default for BackendRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn happy_path(registry: &BackendRegistry, id: &str) {
        let steps = [
            (BackendState::Handshaking, "handshake-ok", "engine bridge online"),
            (BackendState::Ready, "handshake-complete", ""),
            (BackendState::Initialized, "init-ok", ""),
            (BackendState::PlanAccepted, "plan-applied", "shard-plan v3"),
            (BackendState::Running, "start-ok", ""),
            (BackendState::AdmissionStopped, "stop-admission", "fault injection"),
            (BackendState::Draining, "drain", ""),
            (BackendState::Stopped, "drained", ""),
        ];
        for (state, reason, detail) in steps {
            report(registry, id, state, reason, detail).expect("legal step");
        }
    }

    /// Report observing the CURRENT epoch — the well-behaved reporter.
    fn report(
        registry: &BackendRegistry,
        id: &str,
        to: BackendState,
        reason: &str,
        detail: &str,
    ) -> Result<BackendTransition, BackendRegistryError> {
        let generation = registry.backend(id).expect("record").generation;
        registry.report_transition(
            id,
            generation,
            to,
            Some("sw-1".into()),
            reason,
            detail,
        )
    }

    #[test]
    fn registration_creates_the_record_with_the_first_transition() {
        let registry = BackendRegistry::new();
        let record = registry
            .register("kb-conf", BackendKind::EngineBot, 1.0)
            .expect("register");
        assert_eq!(record.kind, BackendKind::EngineBot);
        assert_eq!(record.weight, 1.0);
        assert_eq!(record.state, BackendState::Created);
        assert_eq!(record.generation, 1);
        let first = record.current();
        assert_eq!(first.state, BackendState::Created);
        assert_eq!(first.reason_code, "registered");
        assert_eq!(first.swell_id, None);
        assert_eq!(record.transitions.len(), 1);
    }

    #[test]
    fn weight_is_stored_verbatim_never_interpreted() {
        let registry = BackendRegistry::new();
        let record = registry
            .register("odd", BackendKind::NativeRuntime, 0.0)
            .expect("register");
        assert_eq!(record.weight, 0.0, "D1: the registry does not judge weights");
    }

    #[test]
    fn registration_refuses_empty_and_duplicate_ids() {
        let registry = BackendRegistry::new();
        assert_eq!(
            registry.register("  ", BackendKind::NativeRuntime, 1.0),
            Err(BackendRegistryError::EmptyBackendId)
        );
        registry
            .register("kb-conf", BackendKind::EngineBot, 1.0)
            .expect("first");
        assert_eq!(
            registry.register("kb-conf", BackendKind::NativeRuntime, 1.0),
            Err(BackendRegistryError::DuplicateBackend("kb-conf".into()))
        );
    }

    #[test]
    fn full_lifecycle_walks_the_d3_machine_with_bumping_generations() {
        let registry = BackendRegistry::new();
        registry
            .register("kb-conf", BackendKind::EngineBot, 1.0)
            .expect("register");
        happy_path(&registry, "kb-conf");
        let record = registry.backend("kb-conf").expect("record");
        assert_eq!(record.state, BackendState::Stopped);
        assert_eq!(record.generation, 9); // registration + 8 steps
        assert_eq!(record.transitions.len(), 9);
        // Every record carries the mandatory fields.
        for transition in &record.transitions {
            assert_eq!(transition.backend_id, "kb-conf");
            assert!(!transition.reason_code.is_empty());
            assert!(transition.observed_at_unix_s > 0);
        }
        // The swell binding travels on the transitions that carried it.
        assert_eq!(record.transitions[1].swell_id.as_deref(), Some("sw-1"));
    }

    #[test]
    fn illegal_transitions_are_named_not_guessed() {
        let registry = BackendRegistry::new();
        registry
            .register("kb-conf", BackendKind::EngineBot, 1.0)
            .expect("register");
        let outcome = report(&registry, "kb-conf", BackendState::Running, "jump-ahead", "");
        assert!(matches!(
            outcome,
            Err(BackendRegistryError::IllegalTransition {
                from: BackendState::Created,
                to: BackendState::Running,
                ..
            })
        ));
        // And the failed attempt left no trace.
        let record = registry.backend("kb-conf").expect("record");
        assert_eq!(record.state, BackendState::Created);
        assert_eq!(record.generation, 1);
    }

    #[test]
    fn unknown_backend_and_empty_reason_are_refused() {
        let registry = BackendRegistry::new();
        assert!(matches!(
            registry.report_transition(
                "ghost",
                1,
                BackendState::Handshaking,
                None,
                "x",
                ""
            ),
            Err(BackendRegistryError::UnknownBackend(_))
        ));
        registry
            .register("kb-conf", BackendKind::EngineBot, 1.0)
            .expect("register");
        assert_eq!(
            registry.report_transition("kb-conf", 1, BackendState::Handshaking, None, "  ", ""),
            Err(BackendRegistryError::EmptyReasonCode)
        );
    }

    #[test]
    fn any_running_state_can_degrade_and_recovery_reenters_handshake() {
        let registry = BackendRegistry::new();
        registry
            .register("kb-conf", BackendKind::EngineBot, 1.0)
            .expect("register");
        report(&registry, "kb-conf", BackendState::Handshaking, "handshake-start", "")
            .expect("step");
        report(
            &registry,
            "kb-conf",
            BackendState::Degraded,
            "slow-handshake",
            "bridge echo latency 4s",
        )
        .expect("degrade from handshaking");
        // Degraded stacks another reason without changing state shape.
        report(&registry, "kb-conf", BackendState::Degraded, "events-lagging", "")
            .expect("degrade again");
        report(&registry, "kb-conf", BackendState::Failed, "handshake-timeout", "")
            .expect("worsen");
        report(&registry, "kb-conf", BackendState::Recovering, "retry", "")
            .expect("recover");
        let record = registry.backend("kb-conf").expect("record");
        assert_eq!(record.state, BackendState::Recovering);
        assert!(record
            .state
            .legal_successors()
            .contains(&BackendState::Handshaking));
    }

    #[test]
    fn stopped_is_terminal() {
        let registry = BackendRegistry::new();
        registry
            .register("kb-conf", BackendKind::EngineBot, 1.0)
            .expect("register");
        happy_path(&registry, "kb-conf");
        let outcome = report(&registry, "kb-conf", BackendState::Running, "zombie", "");
        assert!(matches!(
            outcome,
            Err(BackendRegistryError::IllegalTransition { from: BackendState::Stopped, .. })
        ));
    }

    #[test]
    fn capacity_declaration_is_verbatim_and_never_a_state_change() {
        let registry = BackendRegistry::new();
        registry
            .register("kb-conf", BackendKind::EngineBot, 1.0)
            .expect("register");
        assert!(
            registry.backend("kb-conf").expect("record").capacity.is_none(),
            "before any declaration, capacity is unknown — not zero"
        );
        let declared = crate::capacity::ResourceCapacity {
            bot_slots: Some(500),
            ..Default::default()
        };
        assert!(registry.declare_capacity("kb-conf", declared.clone()));
        let record = registry.backend("kb-conf").expect("record");
        assert_eq!(record.capacity, Some(declared));
        // A declaration is not a D3 transition: state and generation
        // stay put, and the transition history is untouched.
        assert_eq!(record.state, BackendState::Created);
        assert_eq!(record.generation, 1);
        assert_eq!(record.transitions.len(), 1);
        // Re-declaration replaces (latest wins), still verbatim.
        let tighter = crate::capacity::ResourceCapacity {
            bot_slots: Some(0),
            ..Default::default()
        };
        assert!(registry.declare_capacity("kb-conf", tighter));
        assert_eq!(
            registry.backend("kb-conf").expect("record").capacity,
            Some(crate::capacity::ResourceCapacity {
                bot_slots: Some(0),
                ..Default::default()
            }),
            "explicit zero is a real claim and must not be dropped"
        );
        assert!(!registry.declare_capacity("ghost", Default::default()));
    }

    #[test]
    fn stale_reports_never_overwrite_new_state_and_future_reports_need_a_resync() {
        let registry = BackendRegistry::new();
        registry
            .register("kb-conf", BackendKind::EngineBot, 1.0)
            .expect("register");
        // Current epoch is 1; a report claiming epoch 0 is STALE.
        let outcome = registry.report_transition(
            "kb-conf",
            0,
            BackendState::Handshaking,
            None,
            "late-report",
            "from an observer that missed the registration",
        );
        assert_eq!(
            outcome,
            Err(BackendRegistryError::StaleGeneration {
                backend: "kb-conf".into(),
                reported: 0,
                current: 1,
            })
        );
        // Zero trace: state, epoch and history unchanged.
        let record = registry.backend("kb-conf").expect("record");
        assert_eq!(record.state, BackendState::Created);
        assert_eq!(record.generation, 1);
        assert_eq!(record.transitions.len(), 1);

        // A report claiming a FUTURE epoch is refused too — a gap
        // means the reporter must resync, not push through.
        assert!(matches!(
            registry.report_transition(
                "kb-conf",
                5,
                BackendState::Handshaking,
                None,
                "from-the-future",
                ""
            ),
            Err(BackendRegistryError::FutureGeneration {
                reported: 5,
                current: 1,
                ..
            })
        ));
        assert_eq!(registry.backend("kb-conf").expect("record").generation, 1);

        // The well-behaved report at the current epoch applies and
        // bumps the epoch the reporter must observe next.
        let applied = report(&registry, "kb-conf", BackendState::Handshaking, "go", "")
            .expect("current-epoch report applies");
        assert_eq!(applied.generation, 2);
        assert_eq!(registry.backend("kb-conf").expect("record").generation, 2);
    }

    #[test]
    fn epochs_are_strictly_monotonic_across_the_recovery_path() {
        let registry = BackendRegistry::new();
        registry
            .register("kb-conf", BackendKind::EngineBot, 1.0)
            .expect("register");
        for (state, reason) in [
            (BackendState::Handshaking, "go"),
            (BackendState::Degraded, "slow"),
            (BackendState::Failed, "timeout"),
            (BackendState::Recovering, "retry"),
            (BackendState::Handshaking, "rejoined"),
        ] {
            report(&registry, "kb-conf", state, reason, "").expect("step");
        }
        let record = registry.backend("kb-conf").expect("record");
        // Recovery reenters Handshaking with the SAME record — the
        // epoch never resets, so vintages stay comparable forever.
        let epochs: Vec<u64> = record.transitions.iter().map(|t| t.generation).collect();
        assert_eq!(epochs, vec![1, 2, 3, 4, 5, 6]);
        let mut sorted = epochs.clone();
        sorted.dedup();
        assert_eq!(epochs, sorted, "strictly increasing, never reset");
    }

    #[test]
    fn queries_filter_by_kind_and_state_stably() {
        let registry = BackendRegistry::new();
        registry
            .register("b-native", BackendKind::NativeRuntime, 1.0)
            .expect("register");
        registry
            .register("a-bot", BackendKind::EngineBot, 1.0)
            .expect("register");
        registry
            .register("c-bot", BackendKind::EngineBot, 1.0)
            .expect("register");
        registry
            .report_transition("a-bot", 1, BackendState::Handshaking, None, "go", "")
            .expect("step");
        let ids: Vec<String> = registry
            .by_kind(BackendKind::EngineBot)
            .into_iter()
            .map(|record| record.backend_id)
            .collect();
        assert_eq!(ids, vec!["a-bot", "c-bot"]);
        let ids: Vec<String> = registry
            .by_state(BackendState::Handshaking)
            .into_iter()
            .map(|record| record.backend_id)
            .collect();
        assert_eq!(ids, vec!["a-bot"]);
    }
}
