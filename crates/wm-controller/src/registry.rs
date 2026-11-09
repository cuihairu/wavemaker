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
    /// Seconds since the Unix epoch — when the CURRENT lease
    /// invalidation happened (distributed.md §Leases policy 3: record
    /// the invalidation time). `None` while the lease is valid; set on
    /// Online→LostContact, cleared again when a beat restores Online.
    pub lease_invalidated_at_unix_s: Option<u64>,
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
    #[error("lease ttl {ttl_ms}ms is shorter than the heartbeat interval {interval_ms}ms; a worker would be declared lost before its first beat is due")]
    LeaseTtlTooShort { ttl_ms: u64, interval_ms: u64 },
}

fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// The lease (WM-0506) — explicit TTL over recorded heartbeats.
/// distributed.md §Leases lists the candidate parameters (1 s beat,
/// 5 s TTL, lost after 3 misses); which candidate is live is the
/// CALLER's configuration decision — the registry only demands the
/// TTL be physically achievable (at least one heartbeat interval).
/// Expiry arithmetic stays in unix seconds end to end.
#[derive(Debug)]
pub struct WorkerRegistry {
    inner: Mutex<RegistryInner>,
}

#[derive(Debug)]
struct RegistryInner {
    workers: HashMap<String, WorkerRecord>,
    heartbeat_interval: Duration,
    lease_ttl: Duration,
    next_generation: u64,
}

impl WorkerRegistry {
    /// `lease_ttl_ms` must be at least one heartbeat interval — a
    /// shorter TTL would declare Workers lost before their first beat
    /// can even be due. Exact boundary: a Worker whose silence reaches
    /// the TTL is expired.
    pub fn new(heartbeat_interval_ms: u32, lease_ttl_ms: u64) -> Result<Self, RegistryError> {
        let interval = u64::from(heartbeat_interval_ms);
        if lease_ttl_ms < interval {
            return Err(RegistryError::LeaseTtlTooShort {
                ttl_ms: lease_ttl_ms,
                interval_ms: interval,
            });
        }
        Ok(Self {
            inner: Mutex::new(RegistryInner {
                workers: HashMap::new(),
                heartbeat_interval: Duration::from_millis(interval),
                lease_ttl: Duration::from_millis(lease_ttl_ms),
                next_generation: 0,
            }),
        })
    }

    pub fn heartbeat_interval(&self) -> Duration {
        self.inner.lock().unwrap().heartbeat_interval
    }

    pub fn lease_ttl(&self) -> Duration {
        self.inner.lock().unwrap().lease_ttl
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
                lease_invalidated_at_unix_s: None,
            },
        );
        Ok(RegistrationDecision::Accepted { generation })
    }

    /// Record a heartbeat. `true` also restores LostContact → Online
    /// (a fresh beat is proof of liveness) and clears the recorded
    /// invalidation time — that invalidation window is over. Old
    /// generations never win (policy 7: events from an old generation
    /// never overwrite new state), and Deregistered workers are
    /// refused outright — their generation is dead until a rejoin
    /// mints a new one.
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
                    record.lease_invalidated_at_unix_s = None;
                }
                true
            }
            _ => false,
        }
    }

    /// Workers whose lease has expired AND are still Online — the
    /// caller moves them with `sweep_expired` (or `mark_lost`).
    /// Already-LostContact workers are not re-reported, so loops stay
    /// idempotent. Expiry arithmetic stays entirely in unix seconds —
    /// the same clock the stamps use.
    pub fn expired_workers(&self) -> Vec<String> {
        let ttl_secs = self.lease_ttl().as_secs();
        let inner = self.inner.lock().unwrap();
        let now = unix_now();
        inner
            .workers
            .values()
            .filter(|record| {
                record.state == WorkerState::Online
                    && record
                        .last_heartbeat_unix_s
                        .map(|stamp| stamp.saturating_add(ttl_secs) <= now)
                        .unwrap_or(false)
            })
            .map(|record| record.registration.worker_id.clone())
            .collect()
    }

    /// The lease loop primitive (WM-0506): atomically move every
    /// expired Online worker to LostContact and record the invalidation
    /// time (distributed.md §Leases policy 3), returning the ids newly
    /// lost this sweep. Invalidation is a STATE change only — the
    /// record is kept, nothing disconnects, no sessions are torn down
    /// (policy 2; the exit criterion "lease invalidation does not
    /// create extra disconnection spikes" is structural here). Idempotent.
    pub fn sweep_expired(&self) -> Vec<String> {
        let ttl_secs = self.lease_ttl().as_secs();
        let mut inner = self.inner.lock().unwrap();
        let now = unix_now();
        let mut newly_lost = Vec::new();
        for record in inner.workers.values_mut() {
            let expired = record.state == WorkerState::Online
                && record
                    .last_heartbeat_unix_s
                    .map(|stamp| stamp.saturating_add(ttl_secs) <= now)
                    .unwrap_or(false);
            if expired {
                record.state = WorkerState::LostContact;
                record.lease_invalidated_at_unix_s = Some(now);
                newly_lost.push(record.registration.worker_id.clone());
            }
        }
        newly_lost.sort();
        newly_lost
    }

    /// Online → LostContact, recording the invalidation time. Unknown
    /// workers or wrong states are a no-op reported as `false` (the
    /// caller logs, not panics).
    pub fn mark_lost(&self, worker_id: &str) -> bool {
        let mut inner = self.inner.lock().unwrap();
        match inner.workers.get_mut(worker_id) {
            Some(record) if record.state == WorkerState::Online => {
                record.state = WorkerState::LostContact;
                record.lease_invalidated_at_unix_s = Some(unix_now());
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

    /// New-admission verdicts (WM-0507) for the given workers — the
    /// policy-1 gate: a worker in LostContact freezes NEW load (its
    /// recorded invalidation time names when the window closed) while
    /// existing sessions keep running (policy 2). A beat restoring
    /// Online reopens admission; leaving the mesh (Deregistered) or
    /// never joining it refuses new load outright.
    pub fn admission_verdicts(
        &self,
        worker_ids: &[&str],
    ) -> Vec<(String, AdmissionVerdict)> {
        let inner = self.inner.lock().unwrap();
        let mut verdicts: Vec<(String, AdmissionVerdict)> = worker_ids
            .iter()
            .map(|worker_id| {
                let verdict = match inner.workers.get(*worker_id) {
                    Some(record) => match record.state {
                        WorkerState::Online => AdmissionVerdict::Admit,
                        WorkerState::LostContact => AdmissionVerdict::Frozen {
                            invalidated_at_unix_s: record
                                .lease_invalidated_at_unix_s
                                .expect("LostContact always carries its invalidation time"),
                        },
                        WorkerState::Deregistered => AdmissionVerdict::NotInMesh,
                    },
                    None => AdmissionVerdict::NotInMesh,
                };
                ((*worker_id).to_string(), verdict)
            })
            .collect();
        verdicts.sort_by(|a, b| a.0.cmp(&b.0));
        verdicts
    }

    /// Workers currently admitting new load (exactly the Online set).
    pub fn admitting_workers(&self) -> Vec<String> {
        self.online_workers()
            .into_iter()
            .map(|record| record.registration.worker_id)
            .collect()
    }
}

/// One new-admission verdict (WM-0507).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum AdmissionVerdict {
    /// Admit new load.
    Admit,
    /// New load is frozen — the worker lost contact; the recorded
    /// invalidation time names when the window closed. Existing
    /// sessions are untouched (policy 2), and admission reopens on a
    /// fresh beat.
    Frozen {
        invalidated_at_unix_s: u64,
    },
    /// The worker is not part of the mesh (deregistered or unknown) —
    /// never admit load to a worker the controller does not see.
    NotInMesh,
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
        let registry = WorkerRegistry::new(1000, 1000 * 4).expect("lease config");
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
        let registry = WorkerRegistry::new(1000, 1000 * 4).expect("lease config");
        assert_eq!(
            registry.register(registration("  ")),
            Err(RegistryError::EmptyWorkerId)
        );
    }

    #[test]
    fn registration_records_versions_and_assigns_monotonic_generations() {
        let registry = WorkerRegistry::new(1000, 1000 * 4).expect("lease config");
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
        let registry = WorkerRegistry::new(1000, 1000 * 4).expect("lease config");
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
        let registry = WorkerRegistry::new(1000, 1000 * 4).expect("lease config");
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
        let registry = WorkerRegistry::new(50, 50 * 4).expect("lease config");
        let generation = match registry.register(registration("w1")).expect("accept") {
            RegistrationDecision::Accepted { generation } => generation,
            other => panic!("unexpected {other:?}"),
        };
        registry.heartbeat("w1", generation);
        // Manipulate the recorded beat backwards so the TTL deadline
        // is provably crossed without sleeping.
        {
            let mut inner = registry.inner.lock().unwrap();
            let record = inner.workers.get_mut("w1").expect("record");
            record.last_heartbeat_unix_s =
                Some(record.last_heartbeat_unix_s.unwrap() - 10_000);
        }
        assert_eq!(registry.expired_workers(), vec!["w1".to_string()]);
        assert!(registry.mark_lost("w1"));
        assert_eq!(registry.count_in_state(WorkerState::LostContact), 1);
        // Policy 3: the invalidation time is recorded while lost.
        assert!(registry
            .worker("w1")
            .expect("record")
            .lease_invalidated_at_unix_s
            .is_some());
        // Idempotent loop: no longer reported, and mark_lost on a lost
        // worker is a no-op.
        assert!(registry.expired_workers().is_empty());
        assert!(!registry.mark_lost("w1"));
        // A fresh beat with the current generation restores Online and
        // closes the invalidation window.
        assert!(registry.heartbeat("w1", generation));
        let record = registry.worker("w1").expect("record");
        assert_eq!(record.state, WorkerState::Online);
        assert_eq!(record.lease_invalidated_at_unix_s, None);
    }

    #[test]
    fn workers_without_any_heartbeat_are_never_expired_early() {
        let registry = WorkerRegistry::new(1000, 1000 * 4).expect("lease config");
        registry.register(registration("w1")).expect("accept");
        // last_heartbeat_unix_s is None between registration and the
        // first beat; the lease loop must not expire such a worker
        // (clock-safety beats guessing).
        assert!(registry.expired_workers().is_empty());
    }

    #[test]
    fn ttl_shorter_than_the_heartbeat_interval_is_refused() {
        assert!(matches!(
            WorkerRegistry::new(1000, 999),
            Err(RegistryError::LeaseTtlTooShort {
                ttl_ms: 999,
                interval_ms: 1000
            })
        ));
        // TTL == interval is the tightest legal lease.
        assert!(WorkerRegistry::new(1000, 1000).is_ok());
    }

    #[test]
    fn ttl_boundary_is_exact_silence_reaching_the_ttl_expires() {
        // Candidate parameters from distributed.md: 1 s beat, 5 s TTL.
        let registry = WorkerRegistry::new(1000, 5000).expect("valid lease config");
        let generation = match registry.register(registration("w1")).expect("accept") {
            RegistrationDecision::Accepted { generation } => generation,
            other => panic!("unexpected {other:?}"),
        };
        registry.heartbeat("w1", generation);
        {
            let mut inner = registry.inner.lock().unwrap();
            let record = inner.workers.get_mut("w1").expect("record");
            record.last_heartbeat_unix_s =
                Some(record.last_heartbeat_unix_s.unwrap() - 5);
        }
        // Exactly TTL seconds of silence → expired.
        assert_eq!(registry.expired_workers(), vec!["w1".to_string()]);
    }

    #[test]
    fn sweep_records_invalidation_time_without_disconnecting_anyone() {
        let registry = WorkerRegistry::new(1000, 5000).expect("valid lease config");
        let mut generations = std::collections::BTreeMap::new();
        for id in ["w-b", "w-a"] {
            let generation = match registry.register(registration(id)).expect("accept") {
                RegistrationDecision::Accepted { generation } => generation,
                other => panic!("unexpected {other:?}"),
            };
            generations.insert(id, generation);
            registry.heartbeat(id, generation);
            let mut inner = registry.inner.lock().unwrap();
            let record = inner.workers.get_mut(id).expect("record");
            record.last_heartbeat_unix_s =
                Some(record.last_heartbeat_unix_s.unwrap() - 60);
        }
        let lost = registry.sweep_expired();
        assert_eq!(lost, vec!["w-a".to_string(), "w-b".to_string()]);
        // Policy 2: invalidation is a state change, NOT a disconnection
        // — records stay, sessions would keep running, nothing is
        // removed from the mesh.
        for id in ["w-a", "w-b"] {
            let record = registry.worker(id).expect("record retained");
            assert_eq!(record.state, WorkerState::LostContact);
            assert!(
                record.lease_invalidated_at_unix_s.is_some(),
                "policy 3: invalidation time recorded"
            );
        }
        assert_eq!(registry.workers().len(), 2, "nobody was dropped");
        // Idempotent: a second sweep re-reports nothing.
        assert!(registry.sweep_expired().is_empty());
        // A fresh beat closes the window.
        assert!(registry.heartbeat("w-a", generations["w-a"]));
        let record = registry.worker("w-a").expect("record");
        assert_eq!(record.state, WorkerState::Online);
        assert_eq!(record.lease_invalidated_at_unix_s, None);
    }

    #[test]
    fn deregistration_keeps_the_record_and_refuses_beats() {
        let registry = WorkerRegistry::new(1000, 1000 * 4).expect("lease config");
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
        let registry = WorkerRegistry::new(1000, 1000 * 4).expect("lease config");
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

    #[test]
    fn lost_contact_freezes_new_admission_and_a_beat_reopens_it() {
        let registry = WorkerRegistry::new(1000, 5000).expect("lease config");
        let generation = match registry.register(registration("w1")).expect("accept") {
            RegistrationDecision::Accepted { generation } => generation,
            other => panic!("unexpected {other:?}"),
        };
        registry.heartbeat("w1", generation);
        // Online admits.
        assert_eq!(
            registry.admission_verdicts(&["w1"]),
            vec![(
                "w1".to_string(),
                AdmissionVerdict::Admit
            )]
        );
        assert_eq!(registry.admitting_workers(), vec!["w1".to_string()]);

        // Lease invalidation freezes NEW load for that worker, naming
        // when the window closed.
        {
            let mut inner = registry.inner.lock().unwrap();
            let record = inner.workers.get_mut("w1").expect("record");
            record.last_heartbeat_unix_s =
                Some(record.last_heartbeat_unix_s.unwrap() - 60);
        }
        let invalidated_at = registry.sweep_expired();
        assert_eq!(invalidated_at, vec!["w1".to_string()]);
        let verdict = &registry.admission_verdicts(&["w1"])[0].1;
        let frozen_at = match verdict {
            AdmissionVerdict::Frozen {
                invalidated_at_unix_s,
            } => *invalidated_at_unix_s,
            other => panic!("expected Frozen, got {other:?}"),
        };
        assert!(
            registry
                .worker("w1")
                .expect("record")
                .lease_invalidated_at_unix_s
                .is_some(),
            "the frozen verdict is backed by the recorded invalidation time"
        );
        assert!(frozen_at > 0);
        // Freezing is not a disconnection: the record stays, existing
        // sessions would continue (policy 2).
        assert!(registry.worker("w1").is_some());
        assert!(registry.admitting_workers().is_empty());

        // A fresh beat reopens admission.
        assert!(registry.heartbeat("w1", generation));
        assert_eq!(
            registry.admission_verdicts(&["w1"]),
            vec![("w1".to_string(), AdmissionVerdict::Admit)]
        );
    }

    #[test]
    fn unknown_and_deregistered_workers_never_admit_new_load() {
        let registry = WorkerRegistry::new(1000, 5000).expect("lease config");
        registry.register(registration("left")).expect("accept");
        registry.deregister("left");
        let verdicts = registry.admission_verdicts(&["left", "ghost", "alpha"]);
        assert_eq!(
            verdicts,
            vec![
                ("alpha".to_string(), AdmissionVerdict::NotInMesh),
                ("ghost".to_string(), AdmissionVerdict::NotInMesh),
                ("left".to_string(), AdmissionVerdict::NotInMesh),
            ],
            "sorted output; unknown and deregistered are both NotInMesh"
        );
        assert!(registry.admitting_workers().is_empty());
    }
}
