//! WM-0320: Native Runtime soak test.
//!
//! A soak is a sustained-operation stability check. The production pattern is
//! repeated *swell cycles*: each cycle runs a full actor lifecycle (spawn →
//! start → 4 timers → close) inside its own [`ActorRunner`], which is dropped
//! at cycle end so memory returns to the live-set bound (the cycle's actor
//! count), not the cumulative total.
//!
//! The short test [`soak_short_repeated_cycles`] is CI-safe (bounded cycles).
//! The real long-duration soak [`soak_long_wallclock`] is `#[ignore]`d and
//! runs on demand:
//!
//! ```text
//! WM_SOAK_ACTORS=50000 WM_SOAK_SECONDS=86400 \
//!   cargo test -p wm-runtime --test soak -- --ignored soak_long_wallclock
//! ```
//!
//! Machine capacity ceiling is measured, not assumed: the run reports the
//! sustained cycle rate and RSS at intervals, and the result (cycles completed,
//! total events, elapsed) is the honest per-machine figure for `todo.md`.

use std::time::{Duration, Instant};

use wm_runtime::batch::ActorRunner;
use wm_runtime::behavior::{ActorDecision, Behavior, BehaviorContext, BehaviorEvent};
use wm_runtime::rng::SeededRng;

const TIMERS_PER_ACTOR: u64 = 4;
/// One actor's 4 timers are spaced 1 ms apart.
const TIMER_SPACING_NS: u64 = 1_000_000;
/// Tick advance per scheduler step (100 µs).
const TICK_STEP_NS: u64 = 100_000;
/// Max actors run per tick.
const BATCH_SIZE: usize = 1024;
/// Virtual-time ceiling: a cycle must drain well inside this.
const TIMEOUT_NS: u64 = 10_000_000_000;

/// Behavior that arms 4 timers on start and closes after the last one fires.
#[derive(Default)]
struct FourTimerBehavior {
    fired: u64,
}

impl Behavior for FourTimerBehavior {
    fn on_event(&mut self, ctx: &mut BehaviorContext, event: &BehaviorEvent) -> ActorDecision {
        match event {
            BehaviorEvent::Started => {
                for k in 0..TIMERS_PER_ACTOR {
                    ctx.arm_timer(k, ctx.now_ns() + TIMER_SPACING_NS * (k + 1));
                }
                ActorDecision::Wait
            }
            BehaviorEvent::Timer(_) => {
                self.fired += 1;
                if self.fired >= TIMERS_PER_ACTOR {
                    ActorDecision::Close
                } else {
                    ActorDecision::Wait
                }
            }
            BehaviorEvent::Message(_, _) => ActorDecision::Wait,
        }
    }
}

struct SwellStats {
    created: u64,
    closed: u64,
    events: usize,
    ticks: u64,
}

/// Run one swell cycle: spawn `actors`, tick to completion, and verify every
/// actor closes and every event is processed exactly once. Returns the cycle
/// stats. The `ActorRunner` is dropped by the caller, releasing the cycle's
/// memory.
fn run_swell(actors: usize, seed: u64) -> SwellStats {
    let mut runner = ActorRunner::new(BATCH_SIZE);
    let mut rng = SeededRng::new(seed);
    let mut now = 0u64;
    let mut events = 0usize;
    let mut ticks = 0u64;

    for _ in 0..actors {
        runner.spawn(now, now, Box::new(FourTimerBehavior::default()));
    }
    let target = runner.total_created();
    assert_eq!(target, actors as u64, "every spawn creates one actor");

    while runner.total_closed() < target {
        now += TICK_STEP_NS;
        ticks += 1;
        let out = runner.tick(now, &mut rng);
        events += out.run;
        assert!(
            now < TIMEOUT_NS,
            "swell of {actors} actors did not drain within virtual timeout"
        );
    }

    // Invariants of a clean cycle: no stranded actors, no lost events.
    assert_eq!(runner.total_closed(), target, "every actor closes");
    assert_eq!(runner.runnable_len(), 0, "no runnable actors left behind");
    let expected = actors * (TIMERS_PER_ACTOR as usize + 1);
    assert_eq!(
        events, expected,
        "each actor runs exactly one Started + {TIMERS_PER_ACTOR} Timer events"
    );

    SwellStats {
        created: target,
        closed: runner.total_closed(),
        events,
        ticks,
    }
}

/// Resident set size in KiB (Linux /proc). Returns 0 where unavailable.
fn rss_kb() -> u64 {
    std::fs::read_to_string("/proc/self/statm")
        .ok()
        .and_then(|s| s.split_whitespace().nth(1).and_then(|v| v.parse().ok()))
        .unwrap_or(0)
        * (page_size_kb())
}

fn page_size_kb() -> u64 {
    // statm reports pages; page size is 4 KiB on the supported targets.
    4
}

fn env_usize(name: &str, default: usize) -> usize {
    std::env::var(name)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

/// Report progress every N cycles (default 100; lower for short measurements).
fn env_report_every() -> usize {
    env_usize("WM_SOAK_REPORT_EVERY", 100).max(1)
}

/// Short soak, CI-safe: a bounded number of swell cycles at a modest actor
/// count. Verifies the runtime is stable across repeated full lifecycles.
#[test]
fn soak_short_repeated_cycles() {
    let cycles = 20;
    let actors = 1_000;
    let mut total_events = 0u64;
    for c in 0..cycles {
        let s = run_swell(actors, c as u64 + 1);
        assert_eq!(s.created, actors as u64);
        assert_eq!(s.closed, actors as u64);
        total_events += s.events as u64;
    }
    assert_eq!(
        total_events,
        (cycles * actors * (TIMERS_PER_ACTOR as usize + 1)) as u64
    );
}

/// Long-duration soak (the 24h target), ignored by default.
///
/// Run on demand:
/// `WM_SOAK_ACTORS=50000 WM_SOAK_SECONDS=86400 cargo test -p wm-runtime --test soak -- --ignored soak_long_wallclock`
///
/// The actual wall-clock duration and actor count come from the environment;
/// this machine's sustained rate and memory are reported, not assumed.
#[test]
#[ignore = "long-running soak; run on demand (WM_SOAK_ACTORS / WM_SOAK_SECONDS)"]
fn soak_long_wallclock() {
    let actors = env_usize("WM_SOAK_ACTORS", 50_000);
    let seconds = env_usize("WM_SOAK_SECONDS", 86_400);
    let report_every = env_report_every();
    let start = Instant::now();
    let deadline = start + Duration::from_secs(seconds as u64);

    let mut cycles = 0u64;
    let mut total_events = 0u64;
    let mut total_ticks = 0u64;
    while Instant::now() < deadline {
        let s = run_swell(actors, cycles + 1);
        cycles += 1;
        total_events += s.events as u64;
        total_ticks += s.ticks;
        if cycles.is_multiple_of(report_every as u64) {
            let elapsed = start.elapsed().as_secs_f64();
            let rate = cycles as f64 / elapsed.max(0.001);
            eprintln!(
                "[soak] t={elapsed:.0}s cycles={cycles} actors={actors} \
                 events={total_events} rate={rate:.1} cycles/s rss={}KiB",
                rss_kb()
            );
        }
    }

    let elapsed = start.elapsed();
    let rate = cycles as f64 / elapsed.as_secs_f64().max(0.001);
    eprintln!(
        "[soak] done cycles={cycles} actors={actors} events={total_events} \
         ticks={total_ticks} elapsed={:?} rate={rate:.1} cycles/s rss={}KiB",
        elapsed,
        rss_kb()
    );
    assert!(cycles > 0, "soak must complete at least one cycle");
}
