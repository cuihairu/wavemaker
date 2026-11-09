//! Native load-generation runtime (Phase 3).
//!
//! Deterministic, event-driven core: every source of nondeterminism enters
//! through an injected abstraction (clock, RNG, transport), so the same plan
//! plus the same seed replays identically in tests and simulations. Async /
//! real-IO adapters live behind the same traits.
//!
//! Module map mirrors docs/native-runtime.md: `clock`, `rng`, `actor`,
//! `scheduler` now; arrival, session, transport, protocol, behavior and
//! metrics land with their WM tasks.

pub mod actor;
pub mod clock;
pub mod rng;
pub mod scheduler;
