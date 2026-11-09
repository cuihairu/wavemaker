//! Controller restart recovery (WM-0512) — the snapshot boundary that
//! lets the controller reconstruct its authoritative mesh state after a
//! process restart, without guessing.
//!
//! The registries hold the mesh state; this module composes their
//! snapshots into one file and back:
//!
//! - `save_snapshot` writes atomically (temp file + rename), so a
//!   reader never observes a torn snapshot — the same discipline as
//!   `SwellStore::write_atomic` (WM-0110);
//! - `load_snapshot` reads the file back and `restore_controller`
//!   re-validates every record before adopting ANY of it. A corrupt or
//!   inconsistent snapshot is refused with a named error, never
//!   partially applied, never silently repaired.
//!
//! The honesty contract (each rule structural):
//!
//! - WORKER records carry their generations, lease stamps, and
//!   registration fields verbatim; restore only checks internal
//!   consistency. It does NOT re-run the §5.3 protocol handshake — those
//!   registrations were already accepted, and rejecting them at restart
//!   would fabricate a failure that never happened.
//! - BACKEND records are reconstructed by REPLAYING their recorded D3
//!   transition history through the state machine (WM-0502's promise:
//!   "replay, not guess"). Facts (`observed_at`, `reason_code`,
//!   `detail`, `swell_id`) travel verbatim, never re-stamped.
//! - A restart does NOT force Online workers to LostContact: liveness
//!   after downtime is a lease question, decided honestly by the
//!   recorded last heartbeat against the TTL on the first sweep.

use std::path::Path;

use crate::backend_registry::{BackendRegistry, BackendSnapshot};
use crate::registry::{WorkerRegistry, WorkerSnapshot};

/// The complete controller restart snapshot: both registries.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ControllerSnapshot {
    pub workers: WorkerSnapshot,
    pub backends: BackendSnapshot,
}

#[derive(Debug, thiserror::Error)]
pub enum PersistenceError {
    #[error("snapshot file {0} is corrupt: {1}")]
    Corrupt(String, serde_json::Error),
    #[error("snapshot serialization failed: {0}")]
    Serialize(serde_json::Error),
    #[error("snapshot io error on {0}: {1}")]
    Io(String, std::io::Error),
    #[error("snapshot failed validation: {0}")]
    Restore(String),
}

/// Write the snapshot atomically (temp file + rename, no torn reads).
pub fn save_snapshot(
    path: &Path,
    snapshot: &ControllerSnapshot,
) -> Result<(), PersistenceError> {
    let json = serde_json::to_string_pretty(snapshot).map_err(PersistenceError::Serialize)?;
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, json).map_err(|e| PersistenceError::Io(path.display().to_string(), e))?;
    std::fs::rename(&tmp, path)
        .map_err(|e| PersistenceError::Io(path.display().to_string(), e))
}

/// Read a snapshot back. Deserialization is the first gate — a
/// malformed file is a `Corrupt` error, never silently empty.
pub fn load_snapshot(path: &Path) -> Result<ControllerSnapshot, PersistenceError> {
    let raw = std::fs::read_to_string(path)
        .map_err(|e| PersistenceError::Io(path.display().to_string(), e))?;
    serde_json::from_str(&raw)
        .map_err(|e| PersistenceError::Corrupt(path.display().to_string(), e))
}

/// Reconstruct both registries from a snapshot. Every record is
/// validated before ANY is adopted; a single inconsistent record
/// refuses the whole restore. Lease parameters are caller config (the
/// snapshot carries no timing policy).
pub fn restore_controller(
    snapshot: ControllerSnapshot,
    heartbeat_interval_ms: u32,
    lease_ttl_ms: u64,
) -> Result<(WorkerRegistry, BackendRegistry), PersistenceError> {
    let workers = WorkerRegistry::restore(snapshot.workers, heartbeat_interval_ms, lease_ttl_ms)
        .map_err(|e| PersistenceError::Restore(e.to_string()))?;
    let backends = BackendRegistry::restore(snapshot.backends)
        .map_err(|e| PersistenceError::Restore(e.to_string()))?;
    Ok((workers, backends))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend_registry::{BackendKind, BackendState};
    use crate::registry::WorkerRegistration;
    use wm_protocol::PROTOCOL_VERSION;

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

    fn build_snapshot() -> ControllerSnapshot {
        let workers = WorkerRegistry::new(1000, 5000).expect("lease config");
        workers.register(worker_registration("w-a")).expect("accept");
        workers.register(worker_registration("w-b")).expect("accept");
        workers.mark_lost("w-b");

        let backends = BackendRegistry::new();
        backends
            .register("kb-conf", BackendKind::EngineBot, 1.0)
            .expect("register");
        let generation = backends.backend("kb-conf").expect("record").generation;
        backends
            .report_transition(
                "kb-conf",
                generation,
                BackendState::Handshaking,
                None,
                "go",
                "",
            )
            .expect("step");

        ControllerSnapshot {
            workers: workers.snapshot(),
            backends: backends.snapshot(),
        }
    }

    #[test]
    fn snapshot_file_round_trips_through_disk_and_restore_controller() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("controller-snapshot.json");
        let snapshot = build_snapshot();

        save_snapshot(&path, &snapshot).expect("save");
        let loaded = load_snapshot(&path).expect("load");
        assert_eq!(loaded, snapshot, "the file round-trips exactly");

        let (workers, backends) =
            restore_controller(loaded, 1000, 5000).expect("valid snapshot restores");
        assert_eq!(workers.workers().len(), 2);
        assert_eq!(workers.worker("w-b").expect("record").state, crate::registry::WorkerState::LostContact);
        assert_eq!(backends.backends().len(), 1);
        assert_eq!(
            backends.backend("kb-conf").expect("record").state,
            BackendState::Handshaking
        );
    }

    #[test]
    fn save_replaces_an_existing_file_atomically() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("controller-snapshot.json");
        let first = build_snapshot();
        save_snapshot(&path, &first).expect("first save");
        // A second save over the same path replaces cleanly (the temp
        // file must not linger as a stale sibling).
        save_snapshot(&path, &first).expect("second save");
        assert!(!path.with_extension("json.tmp").exists(), "no temp litter");
        assert_eq!(load_snapshot(&path).expect("load"), first);
    }

    #[test]
    fn load_refuses_a_corrupt_file_and_missing_file_is_named() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("controller-snapshot.json");
        std::fs::write(&path, "{ not json").expect("write garbage");
        assert!(matches!(
            load_snapshot(&path),
            Err(PersistenceError::Corrupt(_, _))
        ));
        assert!(matches!(
            load_snapshot(&dir.path().join("missing.json")),
            Err(PersistenceError::Io(_, _))
        ));
    }

    #[test]
    fn restore_controller_refuses_an_inconsistent_snapshot_whole() {
        // Corrupt the backend history into an illegal jump; the whole
        // restore must fail, naming the reason, and no registry is
        // produced.
        let mut snapshot = build_snapshot();
        snapshot.backends.backends[0]
            .transitions
            .push(crate::backend_registry::BackendTransition {
                backend_id: "kb-conf".into(),
                swell_id: None,
                state: BackendState::Running,
                observed_at_unix_s: 1,
                reason_code: "jump".into(),
                detail: String::new(),
                generation: 3,
            });
        snapshot.backends.backends[0].state = BackendState::Running;
        snapshot.backends.backends[0].generation = 3;
        let outcome = restore_controller(snapshot, 1000, 5000);
        let message = outcome.expect_err("restore must fail").to_string();
        assert!(
            message.contains("kb-conf"),
            "the failure names the offending backend: {message}"
        );
    }
}
