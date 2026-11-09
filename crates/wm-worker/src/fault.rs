//! Worker fault injection (WM-0513).
//!
//! A deterministic model of how a Worker behaves under the failure
//! modes `docs/distributed.md` classifies. The model is the WORKER's
//! side of the story: it emits the observable control-plane actions
//! (register, heartbeat at a generation, leave, rejoin) that a faulty
//! Worker actually performs, one per heartbeat slot. The controller's
//! reaction — lease expiry into LostContact, admission freeze, the
//! generation guard (policy 7), deregistration — is what a harness
//! asserts by replaying the action stream against the real registry;
//! nothing here simulates the controller.
//!
//! The model is deterministic: the same fault and the same beat count
//! always yield the same action stream, so failure handling can be
//! exercised without sleeps or timing races. It also keeps faults on
//! the Worker side (what the Worker DOES) distinct from the controller
//! semantics (how the controller REACTS) — the injection never
//! fabricates a controller state directly.

use wm_protocol::PROTOCOL_VERSION;

/// The injectable failure mode. Each names WHAT breaks on the Worker,
/// never the controller's response.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkerFault {
    /// After `beats` accepted heartbeats, the Worker goes silent: no
    /// further beats. The controller's lease must expire it into
    /// LostContact (distributed.md policy 1: freeze new load; policy 3:
    /// record the invalidation time).
    SilenceAfter { beats: u32 },
    /// After `beats` beats, the Worker heartbeats with a generation one
    /// behind the one it was granted — an OLD-generation event that
    /// must never overwrite new state (distributed.md policy 7).
    StaleGenerationAfter { beats: u32 },
    /// After `beats` beats, the Worker leaves explicitly.
    DeregisterAfter { beats: u32 },
    /// After `beats` beats, the Worker process dies and rejoins: a
    /// fresh registration whose old generation is dead.
    RestartAfter { beats: u32 },
    /// The Worker speaks a protocol version the controller does not —
    /// the §5.3 handshake must reject it, never admit it.
    ProtocolMismatch,
}

/// One observable control-plane action a (possibly faulty) Worker
/// performs in a heartbeat slot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BeatAction {
    /// A heartbeat at `generation` — the generation the Worker believes
    /// it holds (possibly stale under `StaleGenerationAfter`).
    Heartbeat { generation: u64 },
    /// The Worker sends nothing this slot (silence, or a stopped
    /// process).
    Silent,
    /// The Worker leaves explicitly.
    Leave,
    /// The Worker rejoins after a restart. The harness re-registers and
    /// feeds the freshly granted generation back through
    /// [`FaultyWorker::set_granted_generation`].
    Rejoin,
}

/// A faulty Worker's deterministic action stream.
#[derive(Debug, Clone)]
pub struct FaultyWorker {
    worker_id: String,
    /// The protocol the Worker believes it speaks (correct under every
    /// fault except `ProtocolMismatch`, which presents a different one).
    protocol_version: u32,
    /// The generation the controller granted at (re)registration.
    granted_generation: u64,
    fault: WorkerFault,
    /// Beat slots already consumed (0-based index of the next beat).
    beats_done: u32,
}

impl FaultyWorker {
    pub fn new(
        worker_id: impl Into<String>,
        protocol_version: u32,
        granted_generation: u64,
        fault: WorkerFault,
    ) -> Self {
        Self {
            worker_id: worker_id.into(),
            protocol_version,
            granted_generation,
            fault,
            beats_done: 0,
        }
    }

    pub fn worker_id(&self) -> &str {
        &self.worker_id
    }

    /// The generation the Worker believes the controller granted.
    pub fn granted_generation(&self) -> u64 {
        self.granted_generation
    }

    /// Adopt a freshly granted generation (after a rejoin is accepted).
    pub fn set_granted_generation(&mut self, generation: u64) {
        self.granted_generation = generation;
    }

    /// The protocol version the Worker presents at registration. Under
    /// `ProtocolMismatch` this differs from the controller's, so the
    /// §5.3 handshake is exercised as a real rejection path.
    pub fn register_protocol_version(&self) -> u32 {
        match self.fault {
            WorkerFault::ProtocolMismatch => self.protocol_version.saturating_add(7),
            _ => self.protocol_version,
        }
    }

    /// Whether the registration would present a mismatched protocol.
    pub fn is_protocol_mismatch(&self) -> bool {
        self.register_protocol_version() != PROTOCOL_VERSION
    }

    /// The next heartbeat-slot action, applying the injected fault at
    /// its configured beat. Deterministic: the same state always yields
    /// the same action.
    pub fn next_beat(&mut self) -> BeatAction {
        self.beats_done += 1;
        let index = self.beats_done - 1;
        match self.fault {
            WorkerFault::SilenceAfter { beats } => {
                if index >= beats {
                    BeatAction::Silent
                } else {
                    BeatAction::Heartbeat {
                        generation: self.granted_generation,
                    }
                }
            }
            WorkerFault::StaleGenerationAfter { beats } => {
                let generation = if index >= beats {
                    // One behind the granted generation — the stale event.
                    self.granted_generation.saturating_sub(1)
                } else {
                    self.granted_generation
                };
                BeatAction::Heartbeat { generation }
            }
            WorkerFault::DeregisterAfter { beats } => {
                if index == beats {
                    BeatAction::Leave
                } else if index > beats {
                    BeatAction::Silent
                } else {
                    BeatAction::Heartbeat {
                        generation: self.granted_generation,
                    }
                }
            }
            WorkerFault::RestartAfter { beats } => {
                if index == beats {
                    BeatAction::Rejoin
                } else {
                    BeatAction::Heartbeat {
                        generation: self.granted_generation,
                    }
                }
            }
            WorkerFault::ProtocolMismatch => BeatAction::Silent,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn silence_after_produces_beats_then_silence() {
        let mut worker = FaultyWorker::new("w1", PROTOCOL_VERSION, 3, WorkerFault::SilenceAfter { beats: 2 });
        assert_eq!(worker.next_beat(), BeatAction::Heartbeat { generation: 3 });
        assert_eq!(worker.next_beat(), BeatAction::Heartbeat { generation: 3 });
        assert_eq!(worker.next_beat(), BeatAction::Silent);
        assert_eq!(worker.next_beat(), BeatAction::Silent, "stays silent");
    }

    #[test]
    fn stale_generation_beats_one_behind_after_trigger() {
        let mut worker = FaultyWorker::new("w1", PROTOCOL_VERSION, 3, WorkerFault::StaleGenerationAfter { beats: 1 });
        assert_eq!(worker.next_beat(), BeatAction::Heartbeat { generation: 3 });
        assert_eq!(worker.next_beat(), BeatAction::Heartbeat { generation: 2 });
        assert_eq!(worker.next_beat(), BeatAction::Heartbeat { generation: 2 });
        // A worker at generation 0 has nothing stale to send — the
        // model saturates rather than underflowing.
        let mut zero = FaultyWorker::new("w2", PROTOCOL_VERSION, 0, WorkerFault::StaleGenerationAfter { beats: 0 });
        assert_eq!(zero.next_beat(), BeatAction::Heartbeat { generation: 0 });
    }

    #[test]
    fn deregister_after_leaves_once_then_falls_silent() {
        let mut worker = FaultyWorker::new("w1", PROTOCOL_VERSION, 1, WorkerFault::DeregisterAfter { beats: 1 });
        assert_eq!(worker.next_beat(), BeatAction::Heartbeat { generation: 1 });
        assert_eq!(worker.next_beat(), BeatAction::Leave);
        assert_eq!(worker.next_beat(), BeatAction::Silent, "a departed worker is gone");
    }

    #[test]
    fn restart_after_rejoins_once_then_resumes_at_the_new_generation() {
        let mut worker = FaultyWorker::new("w1", PROTOCOL_VERSION, 1, WorkerFault::RestartAfter { beats: 1 });
        assert_eq!(worker.next_beat(), BeatAction::Heartbeat { generation: 1 });
        assert_eq!(worker.next_beat(), BeatAction::Rejoin);
        // The harness re-registers and grants a fresh generation.
        worker.set_granted_generation(2);
        assert_eq!(worker.next_beat(), BeatAction::Heartbeat { generation: 2 });
        assert_eq!(worker.next_beat(), BeatAction::Heartbeat { generation: 2 });
    }

    #[test]
    fn protocol_mismatch_presents_a_different_version() {
        let worker = FaultyWorker::new("w1", PROTOCOL_VERSION, 0, WorkerFault::ProtocolMismatch);
        assert!(worker.is_protocol_mismatch());
        assert_eq!(worker.register_protocol_version(), PROTOCOL_VERSION + 7);
        let healthy = FaultyWorker::new("w2", PROTOCOL_VERSION, 0, WorkerFault::SilenceAfter { beats: 0 });
        assert!(!healthy.is_protocol_mismatch());
        assert_eq!(healthy.register_protocol_version(), PROTOCOL_VERSION);
    }

    #[test]
    fn action_stream_is_deterministic() {
        let mut a = FaultyWorker::new("w1", PROTOCOL_VERSION, 1, WorkerFault::SilenceAfter { beats: 2 });
        let mut b = FaultyWorker::new("w1", PROTOCOL_VERSION, 1, WorkerFault::SilenceAfter { beats: 2 });
        for _ in 0..6 {
            assert_eq!(a.next_beat(), b.next_beat(), "same fault, same stream");
        }
    }
}
