//! WM-0319: Native Runtime end-to-end benchmark (criterion).
//!
//! Mandatory operating point (docs/native-runtime.md §"Scheduler"): 5k / 20k /
//! 50k Actors, each arming 4 timers. This drives the *real* batch scheduler
//! (`wm_runtime::batch::ActorRunner`) end to end — spawn, start, timer fire,
//! resume, close — not a stand-in timer wheel. The timer-scheduler micro-bench
//! lives in wm-core (WM-0011); this one measures the whole pipeline.
//!
//! Numbers are context-sensitive (shared, loaded machine): compare relative
//! trends between scales, not absolute promises. Load context at measurement
//! time is recorded in docs/benchmarks/runtime-benchmark.md.
//!
//! Run: cargo bench -p wm-runtime --bench runtime

use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use wm_runtime::batch::ActorRunner;
use wm_runtime::behavior::{ActorDecision, Behavior, BehaviorContext, BehaviorEvent};
use wm_runtime::rng::SeededRng;

const ACTOR_SCALES: [usize; 3] = [5_000, 20_000, 50_000];
const TIMERS_PER_ACTOR: u64 = 4;
/// One actor's 4 timers are spaced 1 ms apart.
const TIMER_SPACING_NS: u64 = 1_000_000;
/// Tick advance per scheduler step (100 µs).
const TICK_STEP_NS: u64 = 100_000;
/// Virtual-time ceiling: a lifecycle must complete well inside this.
const TIMEOUT_NS: u64 = 10_000_000_000;
/// Max actors run per tick.
const BATCH_SIZE: usize = 1024;

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

/// Start-only behavior: does one unit of work on start, then waits forever.
struct StartOnlyBehavior;

impl Behavior for StartOnlyBehavior {
    fn on_event(&mut self, ctx: &mut BehaviorContext, event: &BehaviorEvent) -> ActorDecision {
        match event {
            BehaviorEvent::Started => {
                ctx.log("started");
                ActorDecision::Wait
            }
            _ => ActorDecision::Wait,
        }
    }
}

/// Drive `actors` actors through a full lifecycle (start + 4 timers + close).
/// Returns the number of behavior events executed (one Started + four Timer
/// per actor).
fn run_lifecycle(actors: usize) -> usize {
    let mut runner = ActorRunner::new(BATCH_SIZE);
    let mut rng = SeededRng::new(42);
    for _ in 0..actors {
        runner.spawn(0, 0, Box::new(FourTimerBehavior::default()));
    }

    let mut now = 0u64;
    let mut events = 0usize;
    loop {
        let out = runner.tick(now, &mut rng);
        events += out.run;
        if runner.total_closed() as usize >= actors {
            break;
        }
        now = now.saturating_add(TICK_STEP_NS);
        assert!(
            now < TIMEOUT_NS,
            "lifecycle did not complete for {actors} actors"
        );
    }

    let expected = actors * (TIMERS_PER_ACTOR as usize + 1);
    assert_eq!(
        runner.total_closed() as usize,
        actors,
        "every actor must close"
    );
    assert_eq!(events, expected, "each actor runs exactly 5 events");
    events
}

/// Spawn and start `actors` actors (no timers), returning the count started.
fn run_start(actors: usize) -> usize {
    let mut runner = ActorRunner::new(BATCH_SIZE);
    let mut rng = SeededRng::new(7);
    for _ in 0..actors {
        runner.spawn(0, 0, Box::new(StartOnlyBehavior));
    }

    let mut started = 0usize;
    let mut now = 0u64;
    while started < actors {
        let out = runner.tick(now, &mut rng);
        started += out.started;
        now = now.saturating_add(TICK_STEP_NS);
        assert!(
            now < TIMEOUT_NS,
            "start did not complete for {actors} actors"
        );
    }
    started
}

fn bench_lifecycle(c: &mut Criterion) {
    let mut group = c.benchmark_group("runtime/lifecycle");
    for &actors in &ACTOR_SCALES {
        let events = actors * (TIMERS_PER_ACTOR as usize + 1);
        group.throughput(Throughput::Elements(events as u64));
        group.bench_with_input(
            BenchmarkId::from_parameter(actors),
            &actors,
            |b, &actors| b.iter(|| black_box(run_lifecycle(actors))),
        );
    }
    group.finish();
}

fn bench_start(c: &mut Criterion) {
    let mut group = c.benchmark_group("runtime/start");
    for &actors in &ACTOR_SCALES {
        group.throughput(Throughput::Elements(actors as u64));
        group.bench_with_input(
            BenchmarkId::from_parameter(actors),
            &actors,
            |b, &actors| b.iter(|| black_box(run_start(actors))),
        );
    }
    group.finish();
}

criterion_group!(benches, bench_lifecycle, bench_start);
criterion_main!(benches);
