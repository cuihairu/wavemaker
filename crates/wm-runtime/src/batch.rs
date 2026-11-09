//! Batch actor scheduling (WM-0315).
//!
//! The [`ActorRunner`] is the event loop: it owns the lifecycle manager, the
//! event dispatcher, and the per-actor behaviors, and drives them with a
//! bounded batch per [`tick`](ActorRunner::tick). Each tick:
//!
//! 1. starts actors whose arrival time was reached,
//! 2. fires behavior timers (delivering `Timer` events and resuming waiters),
//! 3. runs up to `batch_size` runnable actors, routing their effects.
//!
//! The runner owns behavior timers (so a resume is always attributable to a
//! specific timer or message) and never exposes sockets or worker internals to
//! a behavior.

use std::collections::{HashMap, HashSet};

use crate::actor::ActorState;
use crate::behavior::{ActorDecision, Behavior, BehaviorContext, BehaviorEffect, BehaviorEvent};
use crate::dispatch::EventDispatcher;
use crate::lifecycle::{ActorId, LifecycleManager};
use crate::metrics::WorkerMetrics;
use crate::rng::SeededRng;
use crate::scheduler::TimerScheduler;

/// Everything one [`ActorRunner::tick`] produced.
#[derive(Debug, Default)]
pub struct TickOutcome {
    /// Outbound messages the tick's behaviors requested:
    /// `(source actor, message id, payload)`. The source is what lets a
    /// multi-session caller route each send to that session's transport.
    pub sends: Vec<(ActorId, u32, Vec<u8>)>,
    /// Structured log lines the tick's behaviors emitted.
    pub logs: Vec<String>,
    /// Actors that started this tick (arrival reached).
    pub started: usize,
    /// Actors resumed from a timer or message this tick.
    pub resumed: usize,
    /// Behavior events actually executed this tick.
    pub run: usize,
    /// Actors closed by their behavior this tick.
    pub closed: usize,
}

#[derive(Debug, Clone, Copy)]
struct BehaviorTimer {
    actor: ActorId,
    timer_id: u64,
}

/// Drives actors in bounded batches per tick.
pub struct ActorRunner {
    lifecycle: LifecycleManager,
    dispatcher: EventDispatcher,
    behaviors: HashMap<ActorId, Box<dyn Behavior>>,
    started: HashSet<ActorId>,
    timers: TimerScheduler<BehaviorTimer>,
    batch_size: usize,
    /// Scheduler/behavior-path counters fed by this runner: timer arm /
    /// fire / cancel, actor start / close, message send / recv with bytes,
    /// planned-to-start deviation, runnable-queue gauge. The wall-clock
    /// tracks (`run_ns`, event-loop lag) are *not* recorded here — a
    /// deterministic runner has no real-time event loop to lag; the
    /// real-time worker layer owns those (`actors_failed` likewise, which
    /// belongs to behavior panic handling at the worker).
    metrics: WorkerMetrics,
}

impl ActorRunner {
    pub fn new(batch_size: usize) -> Self {
        assert!(batch_size > 0, "batch size must be positive");
        Self {
            lifecycle: LifecycleManager::new(),
            dispatcher: EventDispatcher::new(),
            behaviors: HashMap::new(),
            started: HashSet::new(),
            timers: TimerScheduler::new(),
            batch_size,
            metrics: WorkerMetrics::new(),
        }
    }

    /// Counter view of this runner's scheduler/behavior paths.
    pub fn metrics(&self) -> &WorkerMetrics {
        &self.metrics
    }

    /// Spawn an actor with its behavior, observed at `now_ns` and planned to
    /// start at `planned_at_ns`.
    pub fn spawn(
        &mut self,
        now_ns: u64,
        planned_at_ns: u64,
        behavior: Box<dyn Behavior>,
    ) -> ActorId {
        let id = self.lifecycle.spawn(now_ns, planned_at_ns);
        self.behaviors.insert(id, behavior);
        id
    }

    /// Subscribe an actor to a message id.
    pub fn subscribe(&mut self, actor: ActorId, message_id: u32) {
        self.dispatcher.subscribe(actor, message_id);
    }

    /// Route one incoming message, waking any suspended subscriber.
    pub fn route_message(&mut self, now_ns: u64, message_id: u32, payload: &[u8]) -> usize {
        self.metrics.record_message_recv(payload.len() as u64);
        let actors = self.dispatcher.route_message(message_id, payload);
        let mut woken = 0;
        for actor in &actors {
            if self.lifecycle.actor_state(*actor) == Some(ActorState::Waiting)
                && self
                    .lifecycle
                    .resume(*actor, now_ns, "message arrived")
                    .is_ok()
            {
                woken += 1;
            }
        }
        woken
    }

    /// Advance to `now_ns`, running up to `batch_size` actors.
    pub fn tick(&mut self, now_ns: u64, rng: &mut SeededRng) -> TickOutcome {
        let started = self.lifecycle.fire_due(now_ns);
        self.metrics.record_actors_started(started as u64);
        let mut out = TickOutcome {
            started,
            ..TickOutcome::default()
        };
        self.fire_behavior_timers(now_ns, &mut out);

        let mut budget = self.batch_size;
        while budget > 0 {
            let Some(actor) = self.lifecycle.dequeue_runnable() else {
                break;
            };
            budget -= 1;
            self.run_one(actor, now_ns, rng, &mut out);
        }
        self.metrics
            .observe_runnable_queue_len(self.lifecycle.runnable_len() as u64);
        out
    }

    fn fire_behavior_timers(&mut self, now_ns: u64, out: &mut TickOutcome) {
        let mut due = Vec::new();
        let Self {
            timers, metrics, ..
        } = self;
        timers.expire_with_deadline(now_ns, |_id, deadline_ns, timer| {
            metrics.record_timer_expire();
            metrics.record_expiry_delay_ns(now_ns.saturating_sub(deadline_ns));
            due.push(timer);
        });
        for timer in due {
            self.dispatcher.deliver_timer(timer.actor, timer.timer_id);
            if self.lifecycle.actor_state(timer.actor) == Some(ActorState::Waiting)
                && self
                    .lifecycle
                    .resume(timer.actor, now_ns, "timer fired")
                    .is_ok()
            {
                out.resumed += 1;
            }
        }
    }

    fn run_one(&mut self, actor: ActorId, now_ns: u64, rng: &mut SeededRng, out: &mut TickOutcome) {
        let first_run = !self.started.contains(&actor);
        let Some(event) = self.next_event_for(actor) else {
            // Runnable but with nothing to do: suspend rather than spin.
            let _ = self.lifecycle.suspend(actor, now_ns, "no event pending");
            return;
        };
        if first_run {
            // The behavior is about to execute for the first time: this is
            // the honest measurement point for planned-to-start, after any
            // wait in the runnable queue behind the batch budget.
            if let Some(deviation) = self.lifecycle.record_first_run(actor, now_ns) {
                self.metrics.record_planned_to_start_ns(deviation);
            }
        }

        let (decision, effects) = {
            let Some(behavior) = self.behaviors.get_mut(&actor) else {
                // No behavior: nothing to run; close the actor defensively.
                let _ = self.lifecycle.close(actor, now_ns, "missing behavior");
                self.metrics.record_actor_closed();
                out.closed += 1;
                return;
            };
            let mut ctx = BehaviorContext::new(now_ns, rng);
            let decision = behavior.on_event(&mut ctx, &event);
            (decision, ctx.take_effects())
        };
        out.run += 1;

        for effect in effects {
            self.route_effect(actor, effect, &mut out.sends, &mut out.logs);
        }

        match decision {
            ActorDecision::Continue => {
                if self.dispatcher.pending(actor) > 0 {
                    self.lifecycle.requeue(actor);
                } else {
                    let _ = self.lifecycle.suspend(actor, now_ns, "waiting for event");
                }
            }
            ActorDecision::Wait => {
                // A Wait decision with events already queued must not strand
                // them: if further timers/messages arrived while the actor was
                // runnable, keep it runnable so they are processed. Suspend
                // only when the actor is genuinely done for now.
                if self.dispatcher.pending(actor) > 0 {
                    self.lifecycle.requeue(actor);
                } else {
                    let _ = self.lifecycle.suspend(actor, now_ns, "waiting");
                }
            }
            ActorDecision::Close => {
                let _ = self.lifecycle.close(actor, now_ns, "behavior closed");
                // A closed actor can never run again: drop its message
                // subscriptions and cancel its armed timers so nothing keeps
                // queueing on it afterwards.
                let cancelled = self.timers.cancel_where(|t| t.actor == actor);
                for _ in 0..cancelled {
                    self.metrics.record_timer_cancel();
                }
                self.dispatcher.unsubscribe_all(actor);
                self.dispatcher.clear(actor);
                self.metrics.record_actor_closed();
                out.closed += 1;
            }
        }
    }

    fn route_effect(
        &mut self,
        actor: ActorId,
        effect: BehaviorEffect,
        sends: &mut Vec<(ActorId, u32, Vec<u8>)>,
        logs: &mut Vec<String>,
    ) {
        match effect {
            BehaviorEffect::Send(id, payload) => {
                self.metrics.record_message_sent(payload.len() as u64);
                sends.push((actor, id, payload));
            }
            BehaviorEffect::ArmTimer { id, at_ns } => {
                self.metrics.record_timer_insert();
                self.timers.schedule(
                    at_ns,
                    BehaviorTimer {
                        actor,
                        timer_id: id,
                    },
                );
            }
            BehaviorEffect::Log(msg) => logs.push(msg),
        }
    }

    fn next_event_for(&mut self, actor: ActorId) -> Option<BehaviorEvent> {
        if self.started.insert(actor) {
            Some(BehaviorEvent::Started)
        } else {
            self.dispatcher.next_event(actor)
        }
    }

    // Read-only introspection for tests and reporting.
    pub fn actor_state(&self, actor: ActorId) -> Option<ActorState> {
        self.lifecycle.actor_state(actor)
    }

    pub fn runnable_len(&self) -> usize {
        self.lifecycle.runnable_len()
    }

    pub fn total_created(&self) -> u64 {
        self.lifecycle.total_created()
    }

    pub fn total_closed(&self) -> u64 {
        self.lifecycle.total_closed()
    }

    pub fn mean_planned_to_start_ns(&self) -> Option<f64> {
        self.lifecycle.mean_planned_to_start_ns()
    }

    /// Behavior timers currently armed (introspection).
    pub fn behavior_timers_len(&self) -> usize {
        self.timers.heap_len()
    }

    /// Events queued for `actor` (introspection).
    pub fn pending_events(&self, actor: ActorId) -> usize {
        self.dispatcher.pending(actor)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Behavior that logs the events it sees and echoes every message.
    struct EchoBehavior;

    impl Behavior for EchoBehavior {
        fn on_event(&mut self, ctx: &mut BehaviorContext, event: &BehaviorEvent) -> ActorDecision {
            match event {
                BehaviorEvent::Started => {
                    ctx.log("started");
                    ActorDecision::Continue
                }
                BehaviorEvent::Message(id, payload) => {
                    ctx.send(*id, payload);
                    ActorDecision::Continue
                }
                BehaviorEvent::Timer(_) => ActorDecision::Close,
            }
        }
    }

    /// Behavior that arms a fixed timer on start, then closes on its fire.
    struct TimedBehavior;

    impl Behavior for TimedBehavior {
        fn on_event(&mut self, ctx: &mut BehaviorContext, event: &BehaviorEvent) -> ActorDecision {
            match event {
                BehaviorEvent::Started => {
                    ctx.arm_timer(1, ctx.now_ns() + 1000);
                    ctx.log("armed");
                    ActorDecision::Wait
                }
                BehaviorEvent::Timer(id) => {
                    assert_eq!(*id, 1);
                    ctx.log("timer-fired");
                    ActorDecision::Close
                }
                BehaviorEvent::Message(_, _) => ActorDecision::Wait,
            }
        }
    }

    #[test]
    fn started_event_then_echo_round_trip() {
        let mut runner = ActorRunner::new(64);
        let mut rng = SeededRng::new(1);
        let actor = runner.spawn(0, 0, Box::new(EchoBehavior));
        runner.subscribe(actor, 5);

        let t0 = runner.tick(0, &mut rng);
        assert_eq!(t0.started, 1);
        assert_eq!(t0.run, 1);
        assert!(t0.logs.contains(&"started".to_string()));
        assert_eq!(runner.actor_state(actor), Some(ActorState::Waiting));

        // A message resumes the actor and it echoes.
        let woken = runner.route_message(10, 5, b"ping");
        assert_eq!(woken, 1);
        let t1 = runner.tick(10, &mut rng);
        assert_eq!(t1.run, 1);
        assert_eq!(t1.sends, vec![(actor, 5, b"ping".to_vec())]);
    }

    /// Two sessions (actors) sending on the same message id must produce
    /// distinguishable sends: each entry names its source actor so the caller
    /// can route it to that session's transport.
    #[test]
    fn sends_carry_the_source_actor() {
        let mut runner = ActorRunner::new(64);
        let mut rng = SeededRng::new(21);
        let a = runner.spawn(0, 0, Box::new(EchoBehavior));
        let b = runner.spawn(0, 0, Box::new(EchoBehavior));
        runner.subscribe(a, 9);
        runner.subscribe(b, 9);

        runner.tick(0, &mut rng);
        runner.route_message(10, 9, b"from-a");
        let out = runner.tick(10, &mut rng);
        let mut sends = out.sends;
        sends.sort();
        assert_eq!(
            sends,
            vec![(a, 9, b"from-a".to_vec()), (b, 9, b"from-a".to_vec()),],
            "each send names its source actor"
        );
    }

    #[test]
    fn timer_wakes_waiting_actor_and_delivers_timer_event() {
        let mut runner = ActorRunner::new(64);
        let mut rng = SeededRng::new(2);
        let actor = runner.spawn(0, 0, Box::new(TimedBehavior));
        let t0 = runner.tick(0, &mut rng);
        assert_eq!(t0.started, 1);
        assert_eq!(t0.logs, vec!["armed".to_string()]);
        assert_eq!(runner.actor_state(actor), Some(ActorState::Waiting));

        let t1 = runner.tick(1000, &mut rng);
        assert_eq!(t1.resumed, 1);
        assert_eq!(t1.run, 1);
        assert_eq!(t1.closed, 1);
        assert!(t1.logs.contains(&"timer-fired".to_string()));
        assert_eq!(runner.actor_state(actor), Some(ActorState::Closed));
    }

    #[test]
    fn batch_size_bounds_actors_run_per_tick() {
        let mut runner = ActorRunner::new(2);
        let mut rng = SeededRng::new(3);
        let _a = runner.spawn(0, 0, Box::new(EchoBehavior));
        let _b = runner.spawn(0, 0, Box::new(EchoBehavior));
        let _c = runner.spawn(0, 0, Box::new(EchoBehavior));
        let t0 = runner.tick(0, &mut rng);
        assert_eq!(t0.started, 3);
        assert_eq!(t0.run, 2, "batch size 2 caps the run");
        // One actor remains runnable; the next tick drains it.
        let t1 = runner.tick(0, &mut rng);
        assert_eq!(t1.run, 1);
        let t2 = runner.tick(0, &mut rng);
        assert_eq!(t2.run, 0);
    }

    #[test]
    fn close_decision_closes_actor_and_clears_events() {
        let mut runner = ActorRunner::new(64);
        let mut rng = SeededRng::new(4);
        let _actor = runner.spawn(0, 0, Box::new(TimedBehavior));
        runner.tick(0, &mut rng);
        // Timer at 1000 closes it; also deliver a stray message first.
        runner.route_message(10, 9, b"x");
        let t1 = runner.tick(1000, &mut rng);
        assert_eq!(t1.closed, 1);
        assert_eq!(runner.total_closed(), 1);
    }

    #[test]
    fn deterministic_replay_same_seed_same_outcome() {
        let run = || {
            let mut runner = ActorRunner::new(16);
            let mut rng = SeededRng::new(7);
            let mut actors = Vec::new();
            for _i in 0..8u64 {
                let a = runner.spawn(0, 0, Box::new(TimedBehavior));
                runner.subscribe(a, 100);
                actors.push(a);
            }
            let mut outcomes = Vec::new();
            for now in [0u64, 500, 1000, 1500] {
                let o = runner.tick(now, &mut rng);
                outcomes.push((o.started, o.resumed, o.run, o.closed, o.sends.len()));
            }
            outcomes
        };
        assert_eq!(run(), run());
    }

    /// A behavior returning `Wait` while further events are already queued must
    /// not strand them: the runner requeues the actor so the queued events are
    /// processed instead of leaving it suspended forever.
    ///
    /// Two timers armed at the same deadline fire in one tick; the actor is
    /// resumed once and must still observe *both* `Timer` events before it can
    /// close.
    #[test]
    fn wait_decision_requeues_when_events_are_pending() {
        struct TwoTimerBehavior {
            fired: u8,
        }
        impl Behavior for TwoTimerBehavior {
            fn on_event(
                &mut self,
                ctx: &mut BehaviorContext,
                event: &BehaviorEvent,
            ) -> ActorDecision {
                match event {
                    BehaviorEvent::Started => {
                        ctx.arm_timer(1, ctx.now_ns() + 1000);
                        ctx.arm_timer(2, ctx.now_ns() + 1000);
                        ActorDecision::Wait
                    }
                    BehaviorEvent::Timer(_) => {
                        self.fired += 1;
                        if self.fired >= 2 {
                            ActorDecision::Close
                        } else {
                            ActorDecision::Wait
                        }
                    }
                    BehaviorEvent::Message(_, _) => ActorDecision::Wait,
                }
            }
        }

        let mut runner = ActorRunner::new(64);
        let mut rng = SeededRng::new(11);
        let actor = runner.spawn(0, 0, Box::new(TwoTimerBehavior { fired: 0 }));
        let t0 = runner.tick(0, &mut rng);
        assert_eq!(t0.started, 1);
        assert_eq!(runner.actor_state(actor), Some(ActorState::Waiting));

        // Both timers fire in this tick. The actor is resumed once (second
        // resume is skipped because it is already runnable) but both Timer
        // events are queued; the actor must not strand the second one.
        let t1 = runner.tick(1000, &mut rng);
        assert_eq!(t1.resumed, 1);
        assert!(
            t1.closed >= 1,
            "actor must close after processing both timers"
        );
        assert_eq!(runner.actor_state(actor), Some(ActorState::Closed));
    }

    /// A closed actor must stop collecting events: its message subscriptions
    /// are dropped and its still-armed behavior timers are cancelled at
    /// close, or every later message and timer fire queues up forever on an
    /// actor that can never run again.
    #[test]
    fn closed_actor_drops_subscriptions_and_pending_timers() {
        struct CloseOnFirstTimer;
        impl Behavior for CloseOnFirstTimer {
            fn on_event(
                &mut self,
                ctx: &mut BehaviorContext,
                event: &BehaviorEvent,
            ) -> ActorDecision {
                match event {
                    BehaviorEvent::Started => {
                        ctx.arm_timer(1, ctx.now_ns() + 1_000);
                        ctx.arm_timer(2, ctx.now_ns() + 60_000_000_000);
                        ActorDecision::Wait
                    }
                    BehaviorEvent::Timer(1) => ActorDecision::Close,
                    _ => ActorDecision::Wait,
                }
            }
        }

        let mut runner = ActorRunner::new(64);
        let mut rng = SeededRng::new(13);
        let actor = runner.spawn(0, 0, Box::new(CloseOnFirstTimer));
        runner.subscribe(actor, 42);

        runner.tick(0, &mut rng); // start: arms both timers
        let t1 = runner.tick(1_000, &mut rng); // timer 1 fires -> close
        assert_eq!(t1.closed, 1);
        assert_eq!(runner.actor_state(actor), Some(ActorState::Closed));
        assert_eq!(
            runner.behavior_timers_len(),
            0,
            "armed timers cancelled at close"
        );

        // Later traffic must not queue on the closed actor.
        runner.route_message(2_000, 42, b"late");
        assert_eq!(
            runner.pending_events(actor),
            0,
            "subscription dropped at close"
        );
        runner.tick(60_000_000_000 + 1, &mut rng);
        assert_eq!(
            runner.pending_events(actor),
            0,
            "far timer cancelled at close"
        );
    }

    /// WorkerMetrics is wired into the runner's real paths: timer arm /
    /// fire / cancel, actor start / close, inbound and outbound messages
    /// with byte counts, the planned-to-start deviation, and the
    /// runnable-queue gauge. Wall-clock tracks (run_ns, event-loop lag)
    /// belong to the real-time worker layer, not this deterministic runner.
    #[test]
    fn metrics_track_the_tick_paths() {
        struct ArmTwoCloseOnFirst;
        impl Behavior for ArmTwoCloseOnFirst {
            fn on_event(
                &mut self,
                ctx: &mut BehaviorContext,
                event: &BehaviorEvent,
            ) -> ActorDecision {
                match event {
                    BehaviorEvent::Started => {
                        ctx.arm_timer(1, ctx.now_ns() + 1_000);
                        ctx.arm_timer(2, ctx.now_ns() + 60_000_000_000);
                        ActorDecision::Wait
                    }
                    BehaviorEvent::Timer(1) => ActorDecision::Close,
                    _ => ActorDecision::Wait,
                }
            }
        }

        // Runner A: message traffic, a timed close, and a deviation.
        let mut runner = ActorRunner::new(64);
        let mut rng = SeededRng::new(23);
        let timed = runner.spawn(0, 50, Box::new(TimedBehavior));
        runner.subscribe(timed, 42);
        let echo = runner.spawn(0, 0, Box::new(EchoBehavior));
        runner.subscribe(echo, 7);

        runner.route_message(10, 7, b"echo-me"); // wakes the echo actor
        runner.tick(50, &mut rng); // echo runs; timed actor starts (deviation 0)
        runner.route_message(60, 42, b"hello"); // queues on the waiting actor
        runner.tick(1_100, &mut rng); // timer (deadline 1050) fires; actor closes

        let s = runner.metrics().snapshot();
        assert_eq!(s.actors_started, 2);
        assert_eq!(s.actors_closed, 1);
        assert_eq!(s.timer_insert, 1);
        assert_eq!(s.timer_expire, 1);
        assert_eq!(s.messages_recv, 2);
        assert_eq!(s.recv_bytes, 12, "7 + 5 payload bytes");
        assert_eq!(s.messages_sent, 1);
        assert_eq!(s.sends_bytes, 7);
        assert_eq!(
            s.expiry_delay_ns_total, 50,
            "fired 50 ns past the 1050 deadline"
        );
        assert_eq!(
            s.planned_to_start_ns_total, 50,
            "echo actor planned at 0 ran at 50; timed actor on time"
        );
        assert_eq!(s.runnable_queue_len, 0);

        // Runner B: a close with a still-armed timer exercises the cancel path.
        let mut runner = ActorRunner::new(64);
        let mut rng = SeededRng::new(29);
        let _ = runner.spawn(0, 0, Box::new(ArmTwoCloseOnFirst));
        runner.tick(0, &mut rng); // arms both timers
        runner.tick(1_000, &mut rng); // timer 1 fires; close cancels timer 2

        let s = runner.metrics().snapshot();
        assert_eq!(s.timer_insert, 2);
        assert_eq!(s.timer_expire, 1);
        assert_eq!(s.timer_cancel, 1, "armed timer cancelled at close");
        assert_eq!(s.actors_closed, 1);
    }

    /// The planned-to-start deviation is measured when the behavior actually
    /// executes, not when the actor is pushed onto the runnable queue:
    /// actors waiting behind the batch budget accrue that wait.
    #[test]
    fn planned_to_start_deviation_measures_actual_execution() {
        struct WaitForeverBehavior;
        impl Behavior for WaitForeverBehavior {
            fn on_event(
                &mut self,
                _ctx: &mut BehaviorContext,
                _event: &BehaviorEvent,
            ) -> ActorDecision {
                ActorDecision::Wait
            }
        }

        let mut runner = ActorRunner::new(1); // one behavior per tick
        let mut rng = SeededRng::new(17);
        for _ in 0..3 {
            runner.spawn(0, 100, Box::new(WaitForeverBehavior));
        }

        runner.tick(100, &mut rng); // queue all three, execute only the first
        runner.tick(200, &mut rng);
        assert_eq!(
            runner.mean_planned_to_start_ns(),
            Some(50.0),
            "(0 + 100) / 2 — the third actor is still queued"
        );
        runner.tick(300, &mut rng);
        assert_eq!(runner.mean_planned_to_start_ns(), Some(100.0));
    }
}
