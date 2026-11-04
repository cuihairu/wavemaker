//! In-memory controller registry state (Phase 1 minimal control plane).

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// A registered worker's control-plane view.
#[derive(Debug, Clone)]
pub struct WorkerEntry {
    pub worker_id: String,
    pub generation: u64,
    pub capabilities: Vec<String>,
    pub adapter_version: String,
    pub runtime_version: String,
    pub last_heartbeat: Instant,
    pub registered_at: Instant,
}

#[derive(Debug)]
pub struct ControllerState {
    inner: Mutex<HashMap<String, WorkerEntry>>,
    heartbeat_interval: Duration,
    next_generation: Mutex<u64>,
}

impl ControllerState {
    pub fn new(heartbeat_interval_ms: u32) -> Self {
        Self {
            inner: Mutex::new(HashMap::new()),
            heartbeat_interval: Duration::from_millis(u64::from(heartbeat_interval_ms)),
            next_generation: Mutex::new(0),
        }
    }

    pub fn heartbeat_interval(&self) -> Duration {
        self.heartbeat_interval
    }

    /// Register a worker; a re-registration bumps to a fresh generation
    /// (WM-0013 §3.2: duplicate registration keys are deduped by returning
    /// the authoritative entry, not an error).
    pub fn register(
        &self,
        entry_worker_id: &str,
        capabilities: Vec<String>,
        adapter_version: String,
        runtime_version: String,
    ) -> u64 {
        let mut g = self.next_generation.lock().unwrap();
        *g += 1;
        let generation = *g;
        self.inner.lock().unwrap().insert(
            entry_worker_id.to_string(),
            WorkerEntry {
                worker_id: entry_worker_id.to_string(),
                generation,
                capabilities,
                adapter_version,
                runtime_version,
                last_heartbeat: Instant::now(),
                registered_at: Instant::now(),
            },
        );
        generation
    }

    /// Record a heartbeat. Returns false when the worker is unknown or its
    /// generation is stale (WM-0013 §2.4: old generations never win).
    pub fn heartbeat(&self, worker_id: &str, generation: u64) -> bool {
        let mut map = self.inner.lock().unwrap();
        match map.get_mut(worker_id) {
            Some(entry) if entry.generation <= generation => {
                entry.last_heartbeat = Instant::now();
                true
            }
            _ => false,
        }
    }

    /// Workers whose lease (heartbeat interval) has expired.
    pub fn expired_workers(&self) -> Vec<String> {
        let deadline = self.heartbeat_interval * 2;
        let map = self.inner.lock().unwrap();
        map.values()
            .filter(|e| e.last_heartbeat.elapsed() > deadline)
            .map(|e| e.worker_id.clone())
            .collect()
    }

    pub fn workers(&self) -> Vec<WorkerEntry> {
        let map = self.inner.lock().unwrap();
        let mut v: Vec<WorkerEntry> = map.values().cloned().collect();
        v.sort_by(|a, b| a.worker_id.cmp(&b.worker_id));
        v
    }

    pub fn worker(&self, worker_id: &str) -> Option<WorkerEntry> {
        self.inner.lock().unwrap().get(worker_id).cloned()
    }

    pub fn remove(&self, worker_id: &str) -> bool {
        self.inner.lock().unwrap().remove(worker_id).is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn register_assigns_monotonic_generations() {
        let state = ControllerState::new(1000);
        let g1 = state.register("w1", vec![], "0.1".into(), "0.1".into());
        let g2 = state.register("w2", vec![], "0.1".into(), "0.1".into());
        assert_eq!(g1, 1);
        assert_eq!(g2, 2);
    }

    #[test]
    fn re_registration_bumps_generation() {
        let state = ControllerState::new(1000);
        let g1 = state.register("w1", vec![], "0.1".into(), "0.1".into());
        let g2 = state.register("w1", vec!["cap".into()], "0.2".into(), "0.1".into());
        assert!(g2 > g1);
        let entry = state.worker("w1").unwrap();
        assert_eq!(entry.generation, g2);
        assert_eq!(entry.capabilities, vec!["cap".to_string()]);
    }

    #[test]
    fn stale_generation_heartbeat_is_rejected() {
        let state = ControllerState::new(1000);
        let g1 = state.register("w1", vec![], "0.1".into(), "0.1".into());
        let g2 = state.register("w1", vec![], "0.1".into(), "0.1".into());
        assert!(!state.heartbeat("w1", g1), "old generation must not win");
        assert!(state.heartbeat("w1", g2));
        assert!(!state.heartbeat("unknown", 1));
    }

    #[test]
    fn remove_and_list_roundtrip() {
        let state = ControllerState::new(1000);
        state.register("a", vec![], String::new(), String::new());
        state.register("b", vec![], String::new(), String::new());
        assert_eq!(state.workers().len(), 2);
        assert_eq!(state.workers()[0].worker_id, "a");
        assert!(state.remove("a"));
        assert!(!state.remove("a"));
        assert_eq!(state.workers().len(), 1);
    }
}
