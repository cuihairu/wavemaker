//! Actor lifecycle management (WM-0307).
//!
//! The [`LifecycleManager`] owns every [`ActorMachine`] and drives the state
//! choreography around the pure schedulers: arrivals arrive on a planned
//! timestamp (WM-0306), start/resume transitions are armed as timers
//! (WM-0304), and the runnable queue holds actors ready to run. Time enters
//! only through method arguments, so a plan plus a seed replays identically.
//!
//! Measurements recorded (docs/native-runtime.md "Scheduler"):
//! - deviation between `planned_at` and `actual_start`
//! - runnable queue length (current and peak)

use std::collections::{HashMap, VecDeque};

use crate::actor::{ActorMachine, ActorState, ActorStateError};
use crate::scheduler::{TimerId, TimerScheduler};

/// Stable identity of one managed actor.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ActorId(pub u64);

#[derive(Debug, thiserror::Error)]
pub enum LifecycleError {
    #[error("unknown actor {0:?}")]
    UnknownActor(ActorId),
    #[error("illegal transition: {0}")]
    Illegal(#[from] ActorStateError),
}

#[derive(Debug)]
enum TimerPayload {
    Start(ActorId),
}

/// Owns and orchestrates all live actors.
///
/// Time is supplied by the caller (typically `clock.now_nanos()`), keeping the
/// manager deterministic and independent of any concrete clock.
#[derive(Debug, Default)]
pub struct LifecycleManager {
    actors: HashMap<ActorId, ActorMachine>,
    planned_at: HashMap<ActorId, u64>,
    runnable: VecDeque<ActorId>,
    timers: TimerScheduler<TimerPayload>,
    next_id: u64,
    total_created: u64,
    total_closed: u64,
    total_failed: u64,
    planned_to_start_sum_ns: u128,
    planned_to_start_count: u64,
    planned_to_start_max_ns: u64,
    peak_runnable: usize,
}

impl LifecycleManager {
    pub fn new() -> Self {
        Self::default()
    }

    /// Create an actor planned to start at `planned_at_ns`, observed at
    /// `now_ns`.
    ///
    /// Transitions `Idle -> Scheduled` (reason "arrival reached") stamped at
    /// `now_ns` — the audit trail records when the spawn was observed, never
    /// a future planned time, so a pre-arrival drain stays monotonic — and
    /// arms a start timer at `planned_at_ns`. A `planned_at_ns` already in
    /// the past starts on the next [`Self::fire_due`]. The planned-to-start
    /// deviation is measured when the actor actually starts, against
    /// `planned_at_ns`.
    pub fn spawn(&mut self, now_ns: u64, planned_at_ns: u64) -> ActorId {
        let id = ActorId(self.next_id);
        self.next_id += 1;
        let mut machine = ActorMachine::new();
        machine
            .transition(ActorState::Scheduled, now_ns, "arrival reached")
            .expect("Idle -> Scheduled is always legal");
        self.actors.insert(id, machine);
        self.planned_at.insert(id, planned_at_ns);
        self.timers.schedule(planned_at_ns, TimerPayload::Start(id));
        self.total_created += 1;
        id
    }

    /// Fire every due start timer, moving affected actors onto the runnable
    /// queue. Returns the number made runnable. Timers whose actor is no
    /// longer `Scheduled` (closed, failed) are skipped.
    pub fn fire_due(&mut self, now_ns: u64) -> usize {
        let mut made_runnable = Vec::new();
        self.timers.expire(now_ns, |_id, payload| match payload {
            TimerPayload::Start(aid) => made_runnable.push(aid),
        });
        let mut count = 0;
        for aid in made_runnable {
            if self.start(aid, now_ns) {
                count += 1;
            }
        }
        count
    }

    /// Transition `Scheduled -> Running` and push onto the runnable queue.
    /// Returns whether it started. The planned-to-start deviation is *not*
    /// recorded here: it is measured when the behavior actually executes
    /// ([`Self::record_first_run`]), so actors waiting behind the batch
    /// budget accrue that wait.
    fn start(&mut self, aid: ActorId, now_ns: u64) -> bool {
        let started = match self.actors.get_mut(&aid) {
            Some(machine) if machine.state() == ActorState::Scheduled => machine
                .transition(ActorState::Running, now_ns, "started")
                .is_ok(),
            _ => false,
        };
        if started {
            self.runnable.push_back(aid);
            self.peak_runnable = self.peak_runnable.max(self.runnable.len());
        }
        started
    }

    /// Record that the actor's behavior actually executed for the first time
    /// at `now_ns` (the batch runner calls this when it delivers the
    /// `Started` event). This is the measurement point of the
    /// planned-to-start deviation against `planned_at_ns`. Unknown or
    /// already-recorded actors are ignored.
    pub fn record_first_run(&mut self, aid: ActorId, now_ns: u64) {
        if let Some(planned) = self.planned_at.remove(&aid) {
            let deviation = now_ns.saturating_sub(planned);
            self.planned_to_start_sum_ns += deviation as u128;
            self.planned_to_start_count += 1;
            self.planned_to_start_max_ns = self.planned_to_start_max_ns.max(deviation);
        }
    }

    /// Pop the next runnable actor (FIFO). The caller owns the actor until it
    /// suspends, closes, or fails.
    pub fn dequeue_runnable(&mut self) -> Option<ActorId> {
        let aid = self.runnable.pop_front()?;
        self.peak_runnable = self.peak_runnable.max(self.runnable.len());
        Some(aid)
    }

    /// Suspend a runnable actor to `Waiting`.
    ///
    /// The suspend does not arm a timer: resumption is driven by the caller
    /// (the batch runner owns behavior timers and calls [`Self::resume`] when
    /// one fires or a message arrives).
    pub fn suspend(
        &mut self,
        aid: ActorId,
        now_ns: u64,
        reason: &str,
    ) -> Result<(), LifecycleError> {
        let machine = self
            .actors
            .get_mut(&aid)
            .ok_or(LifecycleError::UnknownActor(aid))?;
        machine.transition(ActorState::Waiting, now_ns, reason)?;
        Ok(())
    }

    /// Resume a waiting actor to `Running`, pushing it onto the runnable queue.
    pub fn resume(
        &mut self,
        aid: ActorId,
        now_ns: u64,
        reason: &str,
    ) -> Result<(), LifecycleError> {
        let machine = self
            .actors
            .get_mut(&aid)
            .ok_or(LifecycleError::UnknownActor(aid))?;
        machine.transition(ActorState::Running, now_ns, reason)?;
        self.runnable.push_back(aid);
        self.peak_runnable = self.peak_runnable.max(self.runnable.len());
        Ok(())
    }

    /// Re-queue a still-`Running` actor without a state change (used when a
    /// behavior chooses `Continue` with more events queued). Returns whether
    /// the actor was re-queued.
    pub fn requeue(&mut self, aid: ActorId) -> bool {
        if self.actors.get(&aid).map(|m| m.state()) == Some(ActorState::Running) {
            self.runnable.push_back(aid);
            self.peak_runnable = self.peak_runnable.max(self.runnable.len());
            true
        } else {
            false
        }
    }

    /// Close an actor from any non-terminal state.
    pub fn close(&mut self, aid: ActorId, now_ns: u64, reason: &str) -> Result<(), LifecycleError> {
        let machine = self
            .actors
            .get_mut(&aid)
            .ok_or(LifecycleError::UnknownActor(aid))?;
        machine.transition(ActorState::Closed, now_ns, reason)?;
        self.total_closed += 1;
        Ok(())
    }

    /// Fail an actor from any non-terminal state.
    pub fn fail(&mut self, aid: ActorId, now_ns: u64, reason: &str) -> Result<(), LifecycleError> {
        let machine = self
            .actors
            .get_mut(&aid)
            .ok_or(LifecycleError::UnknownActor(aid))?;
        machine.transition(ActorState::Failed, now_ns, reason)?;
        self.total_failed += 1;
        Ok(())
    }

    /// Close every non-terminal actor and clear the runnable queue.
    /// Returns the number closed. Pending timers are left in place; later
    /// [`Self::fire_due`] calls skip their now-terminal actors.
    pub fn drain(&mut self, now_ns: u64) -> usize {
        let ids: Vec<ActorId> = self.actors.keys().copied().collect();
        let mut closed = 0;
        for aid in ids {
            if let Some(machine) = self.actors.get_mut(&aid) {
                if !machine.state().is_terminal()
                    && machine
                        .transition(ActorState::Closed, now_ns, "drained")
                        .is_ok()
                {
                    closed += 1;
                }
            }
        }
        self.runnable.clear();
        // Drained actors never execute, so their planned timestamps are dead.
        self.planned_at.clear();
        self.total_closed += closed as u64;
        closed
    }

    pub fn actor_state(&self, aid: ActorId) -> Option<ActorState> {
        self.actors.get(&aid).map(|m| m.state())
    }

    pub fn actor_history(&self, aid: ActorId) -> Option<&[crate::actor::Transition]> {
        self.actors.get(&aid).map(|m| m.history())
    }

    pub fn len(&self) -> usize {
        self.actors.len()
    }

    pub fn is_empty(&self) -> bool {
        self.actors.is_empty()
    }

    pub fn runnable_len(&self) -> usize {
        self.runnable.len()
    }

    pub fn peak_runnable(&self) -> usize {
        self.peak_runnable
    }

    pub fn total_created(&self) -> u64 {
        self.total_created
    }

    pub fn total_closed(&self) -> u64 {
        self.total_closed
    }

    pub fn total_failed(&self) -> u64 {
        self.total_failed
    }

    /// Mean planned-to-start deviation in ns over all actors started so far;
    /// `None` before any actor starts.
    pub fn mean_planned_to_start_ns(&self) -> Option<f64> {
        if self.planned_to_start_count == 0 {
            None
        } else {
            Some(self.planned_to_start_sum_ns as f64 / self.planned_to_start_count as f64)
        }
    }

    /// Maximum planned-to-start deviation in ns; `None` before any actor starts.
    pub fn max_planned_to_start_ns(&self) -> Option<u64> {
        (self.planned_to_start_count > 0).then_some(self.planned_to_start_max_ns)
    }

    /// Number of start timers still pending (an upper bound on actors not yet
    /// started, including any whose actor has since closed).
    pub fn pending_timers(&self) -> usize {
        self.timers.heap_len()
    }

    pub fn cancel_timer(&mut self, id: TimerId) -> bool {
        self.timers.cancel(id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spawn_schedules_start_and_marks_runnable_at_planned_time() {
        let mut lm = LifecycleManager::new();
        let id = lm.spawn(0, 100);
        assert_eq!(lm.actor_state(id), Some(ActorState::Scheduled));
        assert_eq!(lm.fire_due(99), 0, "not due yet");
        assert_eq!(lm.runnable_len(), 0);
        assert_eq!(lm.fire_due(100), 1);
        assert_eq!(lm.runnable_len(), 1);
        assert_eq!(lm.actor_state(id), Some(ActorState::Running));
        assert_eq!(lm.dequeue_runnable(), Some(id));
        assert_eq!(lm.dequeue_runnable(), None);
    }

    #[test]
    fn past_planned_time_starts_on_next_fire() {
        let mut lm = LifecycleManager::new();
        let id = lm.spawn(0, 100);
        assert_eq!(lm.fire_due(150), 1);
        // The deviation is measured at first execution, not at queue time.
        assert_eq!(lm.mean_planned_to_start_ns(), None);
        lm.record_first_run(id, 150);
        assert_eq!(lm.mean_planned_to_start_ns(), Some(50.0));
        assert_eq!(lm.max_planned_to_start_ns(), Some(50));
        assert_eq!(lm.actor_state(id), Some(ActorState::Running));
    }

    #[test]
    fn on_time_spawn_has_zero_deviation() {
        let mut lm = LifecycleManager::new();
        let id = lm.spawn(0, 100);
        assert_eq!(lm.fire_due(100), 1);
        assert_eq!(lm.mean_planned_to_start_ns(), None, "not executed yet");
        lm.record_first_run(id, 100);
        assert_eq!(lm.mean_planned_to_start_ns(), Some(0.0));
    }

    #[test]
    fn suspend_then_explicit_resume_requeues() {
        let mut lm = LifecycleManager::new();
        let _ = lm.spawn(0, 100);
        lm.fire_due(100);
        let id = lm.dequeue_runnable().unwrap();
        lm.suspend(id, 120, "think timer armed").unwrap();
        assert_eq!(lm.actor_state(id), Some(ActorState::Waiting));
        assert_eq!(lm.runnable_len(), 0);
        lm.resume(id, 500, "timer fired").unwrap();
        assert_eq!(lm.actor_state(id), Some(ActorState::Running));
        assert_eq!(lm.runnable_len(), 1);
        assert_eq!(lm.dequeue_runnable(), Some(id));
    }

    #[test]
    fn resume_after_close_is_rejected() {
        let mut lm = LifecycleManager::new();
        let _ = lm.spawn(0, 100);
        lm.fire_due(100);
        let id = lm.dequeue_runnable().unwrap();
        lm.suspend(id, 120, "block").unwrap();
        lm.close(id, 130, "finished").unwrap();
        assert_eq!(lm.actor_state(id), Some(ActorState::Closed));
        assert!(matches!(
            lm.resume(id, 500, "late"),
            Err(LifecycleError::Illegal(_))
        ));
    }

    #[test]
    fn fail_reachable_from_running() {
        let mut lm = LifecycleManager::new();
        let _ = lm.spawn(0, 10);
        lm.fire_due(10);
        let id = lm.dequeue_runnable().unwrap();
        lm.fail(id, 20, "behavior panicked").unwrap();
        assert_eq!(lm.actor_state(id), Some(ActorState::Failed));
        assert_eq!(lm.total_failed(), 1);
    }

    #[test]
    fn drain_closes_all_nonterminal_and_clears_runnable() {
        let mut lm = LifecycleManager::new();
        let _a = lm.spawn(0, 10);
        lm.fire_due(10);
        lm.dequeue_runnable();
        let _b = lm.spawn(0, 50);
        lm.suspend(_a, 20, "block").unwrap();
        assert_eq!(lm.drain(1000), 2);
        assert_eq!(lm.actor_state(_a), Some(ActorState::Closed));
        assert_eq!(lm.total_closed(), 2);
        assert_eq!(lm.runnable_len(), 0);
        assert_eq!(lm.drain(1000), 0, "second drain closes nothing");
    }

    #[test]
    fn peak_runnable_tracks_queue_high_water_mark() {
        let mut lm = LifecycleManager::new();
        for _ in 0..5u64 {
            lm.spawn(0, 100);
        }
        lm.fire_due(100);
        assert_eq!(lm.runnable_len(), 5);
        assert_eq!(lm.peak_runnable(), 5);
        lm.dequeue_runnable();
        lm.dequeue_runnable();
        assert_eq!(lm.runnable_len(), 3);
        assert_eq!(lm.peak_runnable(), 5, "peak does not shrink");
    }

    #[test]
    fn mean_deviation_reflects_actual_start_times() {
        let mut lm = LifecycleManager::new();
        let a = lm.spawn(0, 100);
        assert_eq!(lm.fire_due(100), 1, "started on time: deviation 0");
        let b = lm.spawn(0, 100);
        assert_eq!(lm.fire_due(120), 1, "started 20ns late");
        let c = lm.spawn(0, 100);
        assert_eq!(lm.fire_due(130), 1, "started 30ns late");
        // Execution (not queue push) is the measurement point.
        assert_eq!(lm.mean_planned_to_start_ns(), None, "nothing executed yet");
        lm.record_first_run(a, 100);
        lm.record_first_run(b, 120);
        lm.record_first_run(c, 130);
        assert_eq!(lm.mean_planned_to_start_ns(), Some(50.0 / 3.0));
        assert_eq!(lm.max_planned_to_start_ns(), Some(30));
    }

    #[test]
    fn record_first_run_is_once_and_ignores_unknown_actors() {
        let mut lm = LifecycleManager::new();
        let id = lm.spawn(0, 100);
        lm.fire_due(100);
        lm.record_first_run(id, 200);
        lm.record_first_run(id, 300); // second call ignored
        assert_eq!(lm.mean_planned_to_start_ns(), Some(100.0));
        assert_eq!(lm.max_planned_to_start_ns(), Some(100));
        lm.record_first_run(ActorId(999), 500); // unknown actor ignored
        assert_eq!(lm.mean_planned_to_start_ns(), Some(100.0));
    }

    #[test]
    fn unknown_actor_is_reported() {
        let mut lm = LifecycleManager::new();
        assert!(matches!(
            lm.suspend(ActorId(999), 0, "x"),
            Err(LifecycleError::UnknownActor(_))
        ));
        assert!(matches!(
            lm.close(ActorId(999), 0, "x"),
            Err(LifecycleError::UnknownActor(_))
        ));
    }

    #[test]
    fn draining_a_not_yet_arrived_actor_keeps_history_monotonic() {
        let mut lm = LifecycleManager::new();
        let id = lm.spawn(0, 1_000); // observed at 0, planned for 1_000
                                     // Drain before the planned arrival: the audit trail must stay
                                     // monotonic instead of panicking on a future-dated stamp.
        assert_eq!(lm.drain(500), 1);
        let stamps: Vec<u64> = lm
            .actor_history(id)
            .unwrap()
            .iter()
            .map(|t| t.at_ns)
            .collect();
        assert_eq!(
            stamps.first(),
            Some(&0),
            "Scheduled stamped at the observation time"
        );
        assert!(
            stamps.windows(2).all(|w| w[0] <= w[1]),
            "timestamps monotonic, got {stamps:?}"
        );
    }

    #[test]
    fn deterministic_replay_same_plan_same_outcome() {
        let run = || {
            let mut lm = LifecycleManager::new();
            for i in 0..10u64 {
                lm.spawn(0, 100 + i * 10);
            }
            let mut started = 0;
            for now in [100u64, 150, 200, 250] {
                started += lm.fire_due(now);
            }
            (started, lm.mean_planned_to_start_ns(), lm.peak_runnable())
        };
        assert_eq!(run(), run());
    }
}
