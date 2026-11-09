//! Behavior trait (WM-0313).
//!
//! A `Behavior` is the business logic of one actor. v1 is an internal Rust
//! trait (docs/native-runtime.md "Behavior": Lua is not in v1). The behavior
//! sees only an injected [`BehaviorContext`] — clock, RNG, send actions,
//! timers, structured logs — and returns an [`ActorDecision`]; it never sees
//! worker internals, sockets, or the global runtime.
//!
//! Effects are collected, not executed: the behavior requests a send or a
//! timer; the batch runner (WM-0315) routes them to the transport and
//! scheduler. This keeps behaviors deterministic and testable.

use crate::rng::SeededRng;

/// One event delivered to a behavior.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BehaviorEvent {
    /// Actor just started (first tick).
    Started,
    /// Incoming message: `(id, payload)`.
    Message(u32, Vec<u8>),
    /// A behavior-armed timer fired: timer id.
    Timer(u64),
}

/// What a behavior wants the runner to do after handling an event.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActorDecision {
    /// Keep the actor runnable; hand it the next queued event immediately.
    Continue,
    /// Suspend the actor until a timer or message arrives.
    Wait,
    /// Close the actor (finished or drained).
    Close,
}

/// One effect a behavior requested, to be routed by the runner.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BehaviorEffect {
    /// Send a message `(id, payload)` on the session transport.
    Send(u32, Vec<u8>),
    /// Arm a one-shot timer `id` firing at absolute `at_ns`.
    ArmTimer { id: u64, at_ns: u64 },
    /// Structured log line.
    Log(String),
}

/// The injected view a behavior is allowed to use.
#[derive(Debug)]
pub struct BehaviorContext<'a> {
    now_ns: u64,
    rng: &'a mut SeededRng,
    effects: Vec<BehaviorEffect>,
}

impl<'a> BehaviorContext<'a> {
    pub fn new(now_ns: u64, rng: &'a mut SeededRng) -> Self {
        Self {
            now_ns,
            rng,
            effects: Vec::new(),
        }
    }

    /// Current clock time in nanoseconds.
    pub fn now_ns(&self) -> u64 {
        self.now_ns
    }

    /// Injected reproducible random source.
    pub fn rng(&mut self) -> &mut SeededRng {
        &mut *self.rng
    }

    /// Request an outbound message `(id, payload)`.
    pub fn send(&mut self, id: u32, payload: &[u8]) {
        self.effects
            .push(BehaviorEffect::Send(id, payload.to_vec()));
    }

    /// Arm a one-shot timer `id` firing at absolute `at_ns`.
    pub fn arm_timer(&mut self, id: u64, at_ns: u64) {
        self.effects.push(BehaviorEffect::ArmTimer { id, at_ns });
    }

    /// Emit a structured log line.
    pub fn log(&mut self, msg: impl Into<String>) {
        self.effects.push(BehaviorEffect::Log(msg.into()));
    }

    /// Drain collected effects (clears the pending list).
    pub fn take_effects(&mut self) -> Vec<BehaviorEffect> {
        std::mem::take(&mut self.effects)
    }
}

/// Business logic of one actor.
///
/// Object-safe: the runner holds `Box<dyn Behavior>`.
pub trait Behavior {
    /// Handle one event. Return how the runner should proceed.
    fn on_event(&mut self, ctx: &mut BehaviorContext, event: &BehaviorEvent) -> ActorDecision;
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Echo behavior: on a message, reply with the same id and a fixed payload.
    struct EchoBehavior;

    impl Behavior for EchoBehavior {
        fn on_event(&mut self, ctx: &mut BehaviorContext, event: &BehaviorEvent) -> ActorDecision {
            match event {
                BehaviorEvent::Message(id, payload) => {
                    ctx.send(*id, payload);
                    ActorDecision::Continue
                }
                BehaviorEvent::Started => {
                    ctx.arm_timer(7, ctx.now_ns() + 1000);
                    ActorDecision::Wait
                }
                BehaviorEvent::Timer(_) => ActorDecision::Close,
            }
        }
    }

    #[test]
    fn context_collects_and_drains_effects() {
        let mut rng = SeededRng::new(1);
        let mut ctx = BehaviorContext::new(42, &mut rng);
        assert_eq!(ctx.now_ns(), 42);
        ctx.send(1, b"a");
        ctx.arm_timer(9, 100);
        ctx.log("hello");
        let effects = ctx.take_effects();
        assert_eq!(
            effects,
            vec![
                BehaviorEffect::Send(1, b"a".to_vec()),
                BehaviorEffect::ArmTimer { id: 9, at_ns: 100 },
                BehaviorEffect::Log("hello".into()),
            ]
        );
        assert!(ctx.take_effects().is_empty(), "drained after take");
    }

    #[test]
    fn context_exposes_injected_rng() {
        // The context hands back the same stream: draws through the context
        // match a direct SeededRng drawing from the same seed.
        let mut rng = SeededRng::new(5);
        let mut ctx = BehaviorContext::new(0, &mut rng);
        let via_ctx = ctx.rng().next_u64();
        let via_ctx_again = ctx.rng().next_u64();
        let mut direct = SeededRng::new(5);
        assert_eq!(via_ctx, direct.next_u64());
        assert_eq!(via_ctx_again, direct.next_u64());
    }

    #[test]
    fn behavior_is_object_safe_and_dispatchable() {
        let mut behavior: Box<dyn Behavior> = Box::new(EchoBehavior);
        let mut rng = SeededRng::new(2);
        let mut ctx = BehaviorContext::new(100, &mut rng);
        let decision = behavior.on_event(&mut ctx, &BehaviorEvent::Message(3, b"ping".to_vec()));
        assert_eq!(decision, ActorDecision::Continue);
        assert_eq!(
            ctx.take_effects(),
            vec![BehaviorEffect::Send(3, b"ping".to_vec())]
        );
    }

    #[test]
    fn started_arms_timer_and_waits() {
        let mut behavior = EchoBehavior;
        let mut rng = SeededRng::new(3);
        let mut ctx = BehaviorContext::new(500, &mut rng);
        let decision = behavior.on_event(&mut ctx, &BehaviorEvent::Started);
        assert_eq!(decision, ActorDecision::Wait);
        assert_eq!(
            ctx.take_effects(),
            vec![BehaviorEffect::ArmTimer { id: 7, at_ns: 1500 }]
        );
    }
}
