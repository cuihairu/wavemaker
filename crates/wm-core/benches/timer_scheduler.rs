//! WM-0011: Timer Scheduler benchmark (criterion).
//!
//! The production Timer Scheduler lands in Phase 3 (WM-0304). Until then this
//! benchmark pins a *minimal representative implementation* — a binary min-heap
//! with lazy (tombstone) cancellation, the design intended for WM-0304 — so we
//! get a Phase-0 baseline for insert/cancel/expire throughput and firing
//! lateness at the planned operating points (5k / 20k / 50k actors x 4 timers).
//!
//! Numbers are context-sensitive (shared, loaded machine): compare relative
//! trends between scales, not absolute promises. Load context at measurement
//! time is recorded in docs/benchmarks/timer-benchmark.md.
//!
//! Run: cargo bench -p wm-core --bench timer_scheduler

use std::collections::{BinaryHeap, HashSet};

use criterion::{
    black_box, criterion_group, criterion_main, BenchmarkId, Criterion, BatchSize, Throughput,
};

const ACTOR_SCALES: [usize; 3] = [5_000, 20_000, 50_000];
const TIMERS_PER_ACTOR: usize = 4;

/// One scheduled timer. `seq` keeps FIFO order among equal deadlines and gives
/// every timer a stable identity for cancellation.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Timer {
    deadline_ns: u64,
    seq: u64,
    actor: usize,
    slot: usize,
}

impl Ord for Timer {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        other
            .deadline_ns
            .cmp(&self.deadline_ns)
            .then(other.seq.cmp(&self.seq))
    }
}

impl PartialOrd for Timer {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

/// Minimal binary-heap scheduler: O(log n) insert, O(1) cancel request
/// (tombstone), O(log n) pop with tombstone skipping.
#[derive(Default)]
struct TimerScheduler {
    heap: BinaryHeap<Timer>,
    cancelled: HashSet<u64>,
    next_seq: u64,
    fired: usize,
}

impl TimerScheduler {
    fn insert(&mut self, deadline_ns: u64, actor: usize, slot: usize) {
        let seq = self.next_seq;
        self.next_seq += 1;
        self.heap.push(Timer { deadline_ns, seq, actor, slot });
    }

    fn cancel(&mut self, seq: u64) {
        // lazy cancel: the entry is skipped when it reaches the heap top
        self.cancelled.insert(seq);
    }

    /// Pop and dispatch every timer with `deadline_ns <= now_ns`.
    /// Returns the number of timers actually fired (cancelled ones are skipped
    /// and do not count).
    fn expire(&mut self, now_ns: u64) -> usize {
        self.fired = 0;
        while let Some(top) = self.heap.peek() {
            if top.deadline_ns > now_ns {
                break;
            }
            let timer = self.heap.pop().unwrap();
            if self.cancelled.remove(&timer.seq) {
                continue;
            }
            self.fired += 1;
            black_box((timer.actor, timer.slot));
        }
        self.fired
    }
}

/// Deterministic deadline layout: each actor's 4 timers are spread over the
/// same relative offsets, staggered by actor id so expires drain in a mixed
/// actor order. Cancel targets are every other timer (half the population).
fn build_population(actor_count: usize) -> (TimerScheduler, Vec<u64>) {
    let mut sched = TimerScheduler::default();
    let mut seqs = Vec::with_capacity(actor_count * TIMERS_PER_ACTOR);
    for actor in 0..actor_count {
        for slot in 0..TIMERS_PER_ACTOR {
            let offset_ms = 10 * (slot + 1) as u64 + (actor as u64) % 7;
            seqs.push(sched.next_seq);
            sched.insert(1_000_000 * offset_ms, actor, slot);
        }
    }
    (sched, seqs)
}

fn bench_insert(c: &mut Criterion) {
    let mut group = c.benchmark_group("timer/insert");
    for &actors in &ACTOR_SCALES {
        let total = actors * TIMERS_PER_ACTOR;
        group.throughput(Throughput::Elements(total as u64));
        group.bench_with_input(BenchmarkId::from_parameter(actors), &actors, |b, &actors| {
            b.iter(|| {
                let mut sched = TimerScheduler::default();
                for actor in 0..actors {
                    for slot in 0..TIMERS_PER_ACTOR {
                        let offset_ms = 10 * (slot + 1) as u64 + (actor as u64) % 7;
                        sched.insert(1_000_000 * offset_ms, actor, slot);
                    }
                }
                black_box(sched.next_seq)
            })
        });
    }
    group.finish();
}

fn bench_cancel(c: &mut Criterion) {
    let mut group = c.benchmark_group("timer/cancel");
    for &actors in &ACTOR_SCALES {
        let total = actors * TIMERS_PER_ACTOR;
        group.throughput(Throughput::Elements(total as u64));
        group.bench_with_input(BenchmarkId::from_parameter(actors), &actors, |b, &actors| {
            b.iter_batched(
                || build_population(actors).0,
                |mut sched| {
                    // cancel every other timer, then drain the whole heap so
                    // every cancelled timer hits the tombstone path exactly once
                    for seq in (0..sched.next_seq).step_by(2) {
                        sched.cancel(seq);
                    }
                    let fired = sched.expire(u64::MAX / 2);
                    black_box(fired)
                },
                BatchSize::LargeInput,
            )
        });
    }
    group.finish();
}

fn bench_expire(c: &mut Criterion) {
    let mut group = c.benchmark_group("timer/expire");
    for &actors in &ACTOR_SCALES {
        let total = actors * TIMERS_PER_ACTOR;
        group.throughput(Throughput::Elements(total as u64));
        group.bench_with_input(BenchmarkId::from_parameter(actors), &actors, |b, &actors| {
            // expiry horizon covers the whole population
            let horizon_ns = 1_000_000 * (10 * TIMERS_PER_ACTOR as u64 + 8);
            b.iter_batched(
                || build_population(actors).0,
                |mut sched| {
                    let fired = sched.expire(horizon_ns);
                    black_box(fired)
                },
                BatchSize::LargeInput,
            )
        });
    }
    group.finish();
}

/// Real-clock firing lateness: schedule TIMERS_PER_ACTOR timers per actor at
/// staggered real deadlines, spin-poll the scheduler, and record
/// `fired_at - deadline` for every timer. This measures scheduling accuracy of
/// the design under whatever load the machine happens to be under; treat as
/// context, not a guarantee.
fn measure_lateness_ns(actor_count: usize) -> Vec<u64> {
    let mut sched = TimerScheduler::default();
    let start = std::time::Instant::now();
    let mut deadlines = Vec::with_capacity(actor_count * TIMERS_PER_ACTOR);
    for actor in 0..actor_count {
        for slot in 0..TIMERS_PER_ACTOR {
            let offset_ms = 2 + (actor as u64) % 20 + 5 * slot as u64;
            let dl = (start.elapsed().as_nanos() as u64) + 1_000_000 * offset_ms;
            sched.insert(dl, actor, slot);
            deadlines.push(dl);
        }
    }
    let horizon = *deadlines.iter().max().unwrap() + 500_000_000;
    let mut lateness = Vec::with_capacity(deadlines.len());
    let mut total_fired = 0usize;
    while (start.elapsed().as_nanos() as u64) < horizon {
        let now = start.elapsed().as_nanos() as u64;
        let batch = sched.expire(now);
        for k in 0..batch {
            // heap pops in deadline order, so the k-th timer of the run is
            // deadlines[total_fired + k]; poll granularity charges the whole
            // batch the same observation time
            lateness.push(now.saturating_sub(deadlines[total_fired + k]));
        }
        total_fired += batch;
        std::hint::spin_loop();
    }
    lateness.sort_unstable();
    lateness
}

fn percentile(data: &[u64], p: f64) -> u64 {
    if data.is_empty() {
        return 0;
    }
    let idx = ((data.len() as f64 - 1.0) * p).round() as usize;
    data[idx.min(data.len() - 1)]
}

fn bench_fire_lateness(c: &mut Criterion) {
    let mut group = c.benchmark_group("timer/fire-lateness");
    group.sample_size(10);
    for &actors in &ACTOR_SCALES {
        group.bench_function(BenchmarkId::from_parameter(actors), |b| {
            b.iter_custom(|_iters| {
                let lateness = measure_lateness_ns(actors);
                let n = lateness.len();
                eprintln!(
                    "[fire-lateness actors={actors}] fired={n} p50={}ms p95={}ms p99={}ms max={}ms",
                    percentile(&lateness, 0.50) / 1_000_000,
                    percentile(&lateness, 0.95) / 1_000_000,
                    percentile(&lateness, 0.99) / 1_000_000,
                    lateness.last().copied().unwrap_or(0) / 1_000_000,
                );
                std::time::Duration::from_millis(50)
            })
        });
    }
    group.finish();
}

criterion_group!(
    benches,
    bench_insert,
    bench_cancel,
    bench_expire,
    bench_fire_lateness
);
criterion_main!(benches);
