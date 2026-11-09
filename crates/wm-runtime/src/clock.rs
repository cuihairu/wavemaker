//! Time sources for the runtime.
//!
//! Business behavior must never read the wall clock directly
//! (docs/native-runtime.md): all time flows through the injected [`Clock`].
//! Deterministic implementations report [`Clock::is_deterministic`] `true`
//! and replay identically; `RealClock` is the explicit, marked
//! non-deterministic source for production execution.

/// Monotonic time source in nanoseconds.
///
/// `now_nanos` never goes backwards within one clock instance. The epoch is
/// implementation-defined (`ManualClock` starts at 0; `RealClock` at first
/// construction).
pub trait Clock {
    /// Current monotonic time in nanoseconds.
    fn now_nanos(&self) -> u64;

    /// `true` when the clock is fully controlled and replays identically
    /// across runs (manual/simulated time). Wall-clock sources return
    /// `false` and must not be used in determinism-sensitive tests.
    fn is_deterministic(&self) -> bool {
        false
    }
}

/// Fully controlled clock for tests, simulations and deterministic replay.
///
/// Time starts at 0 and moves only via [`ManualClock::advance`] or
/// [`ManualClock::set`]; nothing else can move it.
#[derive(Debug, Default)]
pub struct ManualClock {
    nanos: u64,
}

impl ManualClock {
    pub fn new() -> Self {
        Self { nanos: 0 }
    }

    /// Start the clock at an explicit non-zero time.
    pub fn starting_at(nanos: u64) -> Self {
        Self { nanos }
    }

    /// Move time forward by `delta` nanoseconds.
    ///
    /// # Panics
    /// Panics if the addition would overflow u64: silently wrapping virtual
    /// time would corrupt every timer scheduled on this clock.
    pub fn advance(&mut self, delta: u64) {
        self.nanos = self
            .nanos
            .checked_add(delta)
            .expect("manual clock overflowed u64 nanoseconds");
    }

    /// Jump to an explicit later time; jumps backwards are refused because
    /// the clock contract guarantees monotonicity.
    ///
    /// # Panics
    /// Panics when `nanos` is earlier than the current time.
    pub fn set(&mut self, nanos: u64) {
        assert!(
            nanos >= self.nanos,
            "manual clock cannot move backwards: {nanos} < {}",
            self.nanos
        );
        self.nanos = nanos;
    }

    /// Current virtual time in nanoseconds (same as [`Clock::now_nanos`]).
    pub fn nanos(&self) -> u64 {
        self.nanos
    }
}

impl Clock for ManualClock {
    fn now_nanos(&self) -> u64 {
        self.nanos
    }

    fn is_deterministic(&self) -> bool {
        true
    }
}

/// Production wall-clock source over a monotonic `std::time::Instant` base.
///
/// Deliberately marked non-deterministic: everything the runtime schedules on
/// it measures real execution, and test assertions must not depend on its
/// exact values.
#[derive(Debug)]
pub struct RealClock {
    start: std::time::Instant,
}

impl RealClock {
    pub fn new() -> Self {
        Self {
            start: std::time::Instant::now(),
        }
    }
}

impl Default for RealClock {
    fn default() -> Self {
        Self::new()
    }
}

impl Clock for RealClock {
    fn now_nanos(&self) -> u64 {
        u64::try_from(self.start.elapsed().as_nanos()).unwrap_or(u64::MAX)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manual_clock_starts_at_zero_and_advances() {
        let mut clock = ManualClock::new();
        assert_eq!(clock.now_nanos(), 0);
        clock.advance(1_000);
        assert_eq!(clock.now_nanos(), 1_000);
        clock.advance(500);
        assert_eq!(clock.now_nanos(), 1_500);
        assert_eq!(clock.nanos(), 1_500);
    }

    #[test]
    fn manual_clock_is_deterministic() {
        assert!(ManualClock::new().is_deterministic());
        assert!(!RealClock::new().is_deterministic());
    }

    #[test]
    fn manual_clock_starting_at_offset() {
        let mut clock = ManualClock::starting_at(7_500);
        assert_eq!(clock.now_nanos(), 7_500);
        clock.set(9_000);
        assert_eq!(clock.now_nanos(), 9_000);
    }

    #[test]
    #[should_panic(expected = "cannot move backwards")]
    fn manual_clock_rejects_backwards_set() {
        let mut clock = ManualClock::starting_at(100);
        clock.set(50);
    }

    #[test]
    fn real_clock_moves_forward_and_is_monotonic() {
        let clock = RealClock::new();
        let first = clock.now_nanos();
        std::thread::sleep(std::time::Duration::from_millis(2));
        let second = clock.now_nanos();
        assert!(second > first, "real clock must move forward");
        assert_ne!(first, 0, "real clock elapsed > 0 after sleep");
    }
}
