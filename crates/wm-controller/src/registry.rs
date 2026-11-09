//! The Worker Registry (WM-0501) — the controller's authoritative view
//! of which Workers exist, in which lifecycle state, at which
//! generation.
//!
//! Phase 1's `state.rs` kept a flat worker map; this module is the
//! Phase 5 registry proper. The rules it encodes:
//!
//! - §5.3 handshake rejection: a registration whose `protocol_version`
//!   does not match the controller's is REJECTED with the reason —
//!   never silently degraded into an incompatible mesh. Every accepted
//!   registration records protocol, adapter, engine, and runtime
//!   versions verbatim (§12.1) so the mesh's composition is auditable.
//! - generations are monotonic and sticky: a rejoin always gets a fresh
//!   generation, and a heartbeat carrying an old generation never wins
//!   (the Phase 1 rule, kept intact).
//! - LostContact is a STATE, not a transient list: lease expiry moves a
//!   Worker Online→LostContact (WM-0507's admission freeze consults
//!   this), and a fresh heartbeat with the current generation is
//!   sufficient proof of liveness to move it back Online. WM-0509's
//!   recovery handshake is a different (Backend-side) mechanism.
//! - explicit deregistration keeps the record (state Deregistered) —
//!   WM-0512's controller restart recovery reads records, so history
//!   is not destroyed on leave.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Duration;

use wm_protocol::PROTOCOL_VERSION;

/// Lifecycle of a registered Worker.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkerState {
    /// Registration accepted, heartbeats expected.
    Online,
    /// Lease expired (WM-0506); new-admission freezing consults this
    /// (WM-0507). A fresh heartbeat with the current generation
    /// restores Online.
    LostContact,
    /// Explicitly left; the record is retained for restart recovery
    /// (WM-0512) and audit.
    Deregistered,
}

/// What a Worker declares about itself at registration (§12.1).
// PartialEq only: the declared capacity carries f64 dimensions, so Eq
// is not derivable.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct WorkerRegistration {
    pub worker_id: String,
    pub protocol_version: u32,
    /// Bridge adapter version (§12.1).
    pub adapter_version: String,
    /// Engine version the bridge drives (§12.1).
    pub engine_version: String,
    /// Wavemaker runtime version (§12.1).
    pub runtime_version: String,
    /// Capabilities such as exact_arrival — carried verbatim, the
    /// registry does not interpret them.
    pub capabilities: Vec<String>,
    /// Self-declared resource envelope (distributed.md §Worker/Backend
    /// Registration); `None` = never declared, which stays distinct
    /// from a declaration of zeros.
    #[serde(default)]
    pub resource_capacity: Option<crate::capacity::ResourceCapacity>,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct WorkerRecord {
    pub registration: WorkerRegistration,
    pub state: WorkerState,
    /// Monotonic per-controller; a rejoin always gets a fresh one.
    pub generation: u64,
    /// Seconds since the Unix epoch.
    pub registered_at_unix_s: u64,
    /// Seconds since the Unix epoch; `None` between registration and
    /// the first heartbeat.
    pub last_heartbeat_unix_s: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RegistrationDecision {
    Accepted { generation: u64 },
    /// §5.3: the reason is surfaced to the Worker, not swallowed.
    Rejected { reason: String },
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum RegistryError {
    #[error("worker id must not be empty")]
    EmptyWorkerId,
}

fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Lease window: a Worker is expired once it has missed two consecutive
/// heartbeat intervals (the Phase 1 multiplier, kept).
const EXPIRED_AFTER_INTERVALS: u32 = 2;

#[derive(Debug)]
pub struct WorkerRegistry {
    inner: Mutex<RegistryInner>,
}

#[derive(Debug)]
struct RegistryInner {
    workers: HashMap<String, WorkerRecord>,
    heartbeat_interval: Duration,
    next_generation: u64,
}

impl WorkerRegistry {
    pub fn new(heartbeat_interval_ms: u32) -> Self {
        Self {
            inner: Mutex::new(RegistryInner {
                workers: HashMap::new(),
                heartbeat_interval: Duration::from_millis(u64::from(heartbeat_interval_ms)),
                next_generation: 0,
            }),
        }
    }

    pub fn heartbeat_interval(&self) -> Duration {
        self.inner.lock().unwrap().heartbeat_interval
    }

    /// §5.3 / §12.1: validate the handshake, then accept or reject with
    /// a reason. A rejoin replaces the record with a fresh generation.
    pub fn register(
        &self,
        registration: WorkerRegistration,
    ) -> Result<RegistrationDecision, RegistryError> {
        if registration.worker_id.trim().is_empty() {
            return Err(RegistryError::EmptyWorkerId);
        }
        let mut inner = self.inner.lock().unwrap();
        if registration.protocol_version != PROTOCOL_VERSION {
            return Ok(RegistrationDecision::Rejected {
                reason: format!(
                    "protocol version {} not supported (controller speaks {})",
                    registration.protocol_version, PROTOCOL_VERSION,
                ),
            });
        }
        inner.next_generation += 1;
        let generation = inner.next_generation;
        inner.workers.insert(
            registration.worker_id.clone(),
            WorkerRecord {
                registration,
                state: WorkerState::Online,
                generation,
                registered_at_unix_s: unix_now(),
                last_heartbeat_unix_s: None,
            },
        );
        Ok(RegistrationDecision::Accepted { generation })
    }

    /// Record a heartbeat. `true` also restores LostContact → Online
    /// (a fresh beat is proof of liveness). Old generations never win,
    /// and Deregistered workers are refused outright — their
    /// generation is dead until a rejoin mints a new one.
    pub fn heartbeat(&self, worker_id: &str, generation: u64) -> bool {
        let mut inner = self.inner.lock().unwrap();
        match inner.workers.get_mut(worker_id) {
            Some(record)
                if record.generation <= generation
                    && record.state != WorkerState::Deregistered =>
            {
                record.last_heartbeat_unix_s = Some(unix_now());
                if record.state == WorkerState::LostContact {
                    record.state = WorkerState::Online;
                }
                true
            }
            _ => false,
        }
    }

    /// Workers whose lease has expired AND are still Online — the
    /// caller (WM-0506's lease loop) moves them with `mark_lost`.
    /// Already-LostContact workers are not re-reported, so the caller's
    /// loop stays idempotent. Expiry arithmetic stays entirely in unix
    /// seconds — the same clock the stamps use.
    pub fn expired_workers(&self) -> Vec<String> {
        let interval_secs = self
            .heartbeat_interval()
            .as_secs()
            .max(1)
            * u64::from(EXPIRED_AFTER_INTERVALS);
        let inner = self.inner.lock().unwrap();
        let now = unix_now();
        inner
            .workers
            .values()
            .filter(|record| {
                record.state == WorkerState::Online
                    && record
                        .last_heartbeat_unix_s
                        .map(|stamp| stamp.saturating_add(interval_secs) < now)
                        .unwrap_or(false)
            })
            .map(|record| record.registration.worker_id.clone())
            .collect()
    }

    /// Online → LostContact. Unknown workers or wrong states are a
    /// no-op reported as `false` (the lease loop logs, not panics).
    pub fn mark_lost(&self, worker_id: &str) -> bool {
        let mut inner = self.inner.lock().unwrap();
        match inner.workers.get_mut(worker_id) {
            Some(record) if record.state == WorkerState::Online => {
                record.state = WorkerState::LostContact;
                true
            }
            _ => false,
        }
    }

    /// Explicit leave: the record stays (Deregistered) for WM-0512 and
    /// audit; heartbeats are refused afterwards. `false` = unknown.
    pub fn deregister(&self, worker_id: &str) -> bool {
        let mut inner = self.inner.lock().unwrap();
        match inner.workers.get_mut(worker_id) {
            Some(record) if record.state != WorkerState::Deregistered => {
                record.state = WorkerState::Deregistered;
                true
            }
            _ => false,
        }
    }

    pub fn worker(&self, worker_id: &str) -> Option<WorkerRecord> {
        self.inner.lock().unwrap().workers.get(worker_id).cloned()
    }

    /// All records ordered by worker id (stable API output).
    pub fn workers(&self) -> Vec<WorkerRecord> {
        let inner = self.inner.lock().unwrap();
        let mut records: Vec<WorkerRecord> = inner.workers.values().cloned().collect();
        records.sort_by(|a, b| {
            a.registration
                .worker_id
                .cmp(&b.registration.worker_id)
        });
        records
    }

    pub fn online_workers(&self) -> Vec<WorkerRecord> {
        self.workers()
            .into_iter()
            .filter(|record| record.state == WorkerState::Online)
            .collect()
    }

    pub fn count_in_state(&self, state: WorkerState) -> usize {
        self.workers()
            .into_iter()
            .filter(|record| record.state == state)
            .count()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn registration(id: &str) -> WorkerRegistration {
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

    #[test]
    fn incompatible_protocol_version_is_rejected_with_reason() {
        let registry = WorkerRegistry::new(1000);
        let mut handshake = registration("w1");
        handshake.protocol_version = PROTOCOL_VERSION + 7;
        let decision = registry.register(handshake).expect("valid input");
        assert_eq!(
            decision,
            RegistrationDecision::Rejected {
                reason: format!(
                    "protocol version {} not supported (controller speaks {})",
                    PROTOCOL_VERSION + 7,
                    PROTOCOL_VERSION,
                )
            }
        );
        assert!(registry.worker("w1").is_none(), "rejected workers are not registered");
    }

    #[test]
    fn empty_worker_id_is_a_validation_error() {
        let registry = WorkerRegistry::new(1000);
        assert_eq!(
            registry.register(registration("  ")),
            Err(RegistryError::EmptyWorkerId)
        );
    }

    #[test]
    fn registration_records_versions_and_assigns_monotonic_generations() {
        let registry = WorkerRegistry::new(1000);
        let g1 = registry
            .register(registration("w1"))
            .expect("accept");
        let g2 = registry
            .register(registration("w2"))
            .expect("accept");
        assert!(matches!(g1, RegistrationDecision::Accepted { generation: 1 }));
        assert!(matches!(g2, RegistrationDecision::Accepted { generation: 2 }));
        let record = registry.worker("w1").expect("record");
        assert_eq!(record.registration.adapter_version, "0.3");
        assert_eq!(record.registration.engine_version, "kbengine-1.2");
        assert_eq!(record.registration.runtime_version, "0.1");
        assert_eq!(
            record.registration.capabilities,
            vec!["exact_arrival".to_string()]
        );
        assert_eq!(record.state, WorkerState::Online);
        assert_eq!(record.last_heartbeat_unix_s, None);
        // No declaration in the handshake → capacity stays unknown.
        assert!(record.registration.resource_capacity.is_none());
    }

    #[test]
    fn declared_capacity_is_stored_verbatim_never_interpreted() {
        let registry = WorkerRegistry::new(1000);
        let mut handshake = registration("w1");
        handshake.resource_capacity = Some(crate::capacity::ResourceCapacity {
            bot_slots: Some(500),
            extra: std::collections::BTreeMap::from([("zone".into(), "a".into())]),
            ..Default::default()
        });
        registry.register(handshake).expect("accept");
        let record = registry.worker("w1").expect("record");
        let capacity = record.registration.resource_capacity.expect("declared");
        assert_eq!(capacity.bot_slots, Some(500));
        assert_eq!(capacity.extra.get("zone").map(String::as_str), Some("a"));
        assert!(
            capacity.memory_mib.is_none(),
            "undeclared dimensions stay unknown"
        );
    }

    #[test]
    fn rejoin_bumps_generation_and_stale_beats_never_win() {
        let registry = WorkerRegistry::new(1000);
        let g1 = match registry.register(registration("w1")).expect("accept") {
            RegistrationDecision::Accepted { generation } => generation,
            other => panic!("unexpected {other:?}"),
        };
        let g2 = match registry.register(registration("w1")).expect("rejoin") {
            RegistrationDecision::Accepted { generation } => generation,
            other => panic!("unexpected {other:?}"),
        };
        assert!(g2 > g1);
        assert!(!registry.heartbeat("w1", g1), "old generation must not win");
        assert!(registry.heartbeat("w1", g2));
        assert!(!registry.heartbeat("ghost", 1));
        let record = registry.worker("w1").expect("record");
        assert!(
            record.last_heartbeat_unix_s.is_some(),
            "accepted beat is recorded"
        );
    }

    #[test]
    fn lease_expiry_moves_online_to_lost_contact_and_a_beat_restores_it() {
        let registry = WorkerRegistry::new(50);
        let generation = match registry.register(registration("w1")).expect("accept") {
            RegistrationDecision::Accepted { generation } => generation,
            other => panic!("unexpected {other:?}"),
        };
        registry.heartbeat("w1", generation);
        // Manipulate the recorded beat backwards so the 2-interval
        // deadline is provably crossed without sleeping.
        {
            let mut inner = registry.inner.lock().unwrap();
            let record = inner.workers.get_mut("w1").expect("record");
            record.last_heartbeat_unix_s =
                Some(record.last_heartbeat_unix_s.unwrap() - 10_000);
        }
        assert_eq!(registry.expired_workers(), vec!["w1".to_string()]);
        assert!(registry.mark_lost("w1"));
        assert_eq!(registry.count_in_state(WorkerState::LostContact), 1);
        // Idempotent loop: no longer reported, and mark_lost on a lost
        // worker is a no-op.
        assert!(registry.expired_workers().is_empty());
        assert!(!registry.mark_lost("w1"));
        // A fresh beat with the current generation restores Online.
        assert!(registry.heartbeat("w1", generation));
        assert_eq!(
            registry.worker("w1").expect("record").state,
            WorkerState::Online
        );
    }

    #[test]
    fn workers_without_any_heartbeat_are_never_expired_early() {
        let registry = WorkerRegistry::new(1000);
        registry.register(registration("w1")).expect("accept");
        // last_heartbeat_unix_s is None between registration and the
        // first beat; the lease loop must not expire such a worker
        // (clock-safety beats guessing).
        assert!(registry.expired_workers().is_empty());
    }

    #[test]
    fn deregistration_keeps_the_record_and_refuses_beats() {
        let registry = WorkerRegistry::new(1000);
        let generation = match registry.register(registration("w1")).expect("accept") {
            RegistrationDecision::Accepted { generation } => generation,
            other => panic!("unexpected {other:?}"),
        };
        assert!(registry.deregister("w1"));
        assert!(!registry.deregister("w1"), "second leave is a no-op");
        assert!(!registry.deregister("ghost"));
        assert!(!registry.heartbeat("w1", generation), "no beats after leave");
        let record = registry.worker("w1").expect("record retained");
        assert_eq!(record.state, WorkerState::Deregistered);
        assert_eq!(registry.online_workers().len(), 0);
        // Rejoin brings it back with a fresh generation.
        let rejoined = registry.register(registration("w1")).expect("rejoin");
        assert!(matches!(
            rejoined,
            RegistrationDecision::Accepted { generation: 2 }
        ));
        assert_eq!(registry.count_in_state(WorkerState::Online), 1);
    }

    #[test]
    fn listings_are_stable_and_state_aware() {
        let registry = WorkerRegistry::new(1000);
        registry.register(registration("b")).expect("accept");
        registry.register(registration("a")).expect("accept");
        registry.register(registration("c")).expect("accept");
        registry.deregister("c");
        let all = registry.workers();
        let ids: Vec<&str> = all
            .iter()
            .map(|record| record.registration.worker_id.as_str())
            .collect();
        assert_eq!(ids, vec!["a", "b", "c"], "sorted, Deregistered retained");
        assert_eq!(
            registry
                .online_workers()
                .iter()
                .map(|record| record.registration.worker_id.as_str())
                .collect::<Vec<_>>(),
            vec!["a", "b"]
        );
        assert_eq!(registry.count_in_state(WorkerState::Online), 2);
        assert_eq!(registry.count_in_state(WorkerState::Deregistered), 1);
    }
}
