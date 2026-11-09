//! Timer Scheduler (WM-0304).
//!
//! Binary min-heap with lazy (tombstone) cancellation — the design baselined
//! by `wm-core`'s `timer_scheduler` benchmark (WM-0011): O(log n) insert,
//! O(1) cancel request, O(log n) amortized pop with tombstone skipping.
//! Timers are one-shot; periodic behavior re-schedules on fire.

use std::cmp::Ordering;
use std::collections::{BinaryHeap, HashSet};

/// Stable identity of one scheduled timer, returned by
/// [`TimerScheduler::schedule`] and accepted by [`TimerScheduler::cancel`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TimerId(pub u64);

#[derive(Debug)]
struct Entry<T> {
    deadline_ns: u64,
    seq: u64,
    payload: T,
}

impl<T> PartialEq for Entry<T> {
    fn eq(&self, other: &Self) -> bool {
        self.deadline_ns == other.deadline_ns && self.seq == other.seq
    }
}

impl<T> Eq for Entry<T> {}

impl<T> Ord for Entry<T> {
    fn cmp(&self, other: &Self) -> Ordering {
        // BinaryHeap is a max-heap; invert to get a min-heap keyed by deadline
        // with FIFO order among equal deadlines.
        other
            .deadline_ns
            .cmp(&self.deadline_ns)
            .then(other.seq.cmp(&self.seq))
    }
}

impl<T> PartialOrd for Entry<T> {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

/// One-shot timer queue over externally supplied time.
///
/// The scheduler is a pure data structure: time comes in through `expire`
/// calls (typically `clock.now_nanos()`), which keeps it deterministic and
/// independent of any particular [`crate::clock::Clock`] wiring.
#[derive(Debug)]
pub struct TimerScheduler<T> {
    heap: BinaryHeap<Entry<T>>,
    cancelled: HashSet<u64>,
    /// Seq ids currently in the heap. Mirrors `heap` so `cancel` can tell a
    /// live timer from an already-fired or never-allocated one in O(1): only
    /// live ids may be tombstoned, which keeps `cancelled` bounded by the
    /// live population instead of growing forever on cancels of dead ids.
    live_seqs: HashSet<u64>,
    next_seq: u64,
}

impl<T> Default for TimerScheduler<T> {
    fn default() -> Self {
        Self {
            heap: BinaryHeap::new(),
            cancelled: HashSet::new(),
            live_seqs: HashSet::new(),
            next_seq: 0,
        }
    }
}

impl<T> TimerScheduler<T> {
    pub fn new() -> Self {
        Self::default()
    }

    /// Schedule a one-shot timer firing at absolute `deadline_ns`.
    ///
    /// A deadline already in the past fires on the next `expire` call with
    /// `now >= deadline`.
    pub fn schedule(&mut self, deadline_ns: u64, payload: T) -> TimerId {
        let seq = self.next_seq;
        self.next_seq += 1;
        self.heap.push(Entry {
            deadline_ns,
            seq,
            payload,
        });
        self.live_seqs.insert(seq);
        TimerId(seq)
    }

    /// Cancel a timer: later `expire` calls will never fire it.
    ///
    /// Returns `false` when the id is not a cancellable live timer — never
    /// allocated, already fired, or already cancelled — in which case no
    /// tombstone is left behind. The return value cannot report whether a
    /// timer already fired: handle cancellation inherently races the fire, so
    /// callers doing cancel accounting must keep their own bookkeeping.
    /// Cancelling a never-allocated id must not silence a later timer that
    /// reuses the sequence, and cancelling a fired id must not accumulate
    /// tombstones forever.
    pub fn cancel(&mut self, id: TimerId) -> bool {
        if id.0 >= self.next_seq {
            return false;
        }
        if !self.live_seqs.contains(&id.0) {
            return false;
        }
        self.cancelled.insert(id.0)
    }

    /// Cancel every live timer whose payload matches `pred`, removing the
    /// entries from the heap immediately. Returns how many were removed.
    /// Used by owners tearing one actor down: its timers vanish instead of
    /// firing into a queue nobody will drain.
    pub fn cancel_where<F>(&mut self, mut pred: F) -> usize
    where
        F: FnMut(&T) -> bool,
    {
        let mut removed: Vec<u64> = Vec::new();
        self.heap.retain(|e| {
            if pred(&e.payload) {
                removed.push(e.seq);
                false
            } else {
                true
            }
        });
        let count = removed.len();
        for seq in removed {
            self.cancelled.remove(&seq);
            self.live_seqs.remove(&seq);
        }
        count
    }

    /// Number of entries in the heap, including cancelled ones not yet
    /// popped (an honest upper bound on live timers).
    pub fn heap_len(&self) -> usize {
        self.heap.len()
    }

    /// Number of cancellation tombstones currently pending. Bounded by the
    /// live-timer population: tombstones are only ever created for ids still
    /// in the heap and are removed when their entry pops.
    pub fn cancelled_len(&self) -> usize {
        self.cancelled.len()
    }

    /// Deadline of the next timer that would fire, skipping cancelled
    /// entries; `None` when nothing live is queued.
    pub fn next_deadline(&mut self) -> Option<u64> {
        while let Some(top) = self.heap.peek() {
            if self.cancelled.contains(&top.seq) {
                let seq = top.seq;
                let entry = self.heap.pop();
                debug_assert!(entry.is_some());
                self.cancelled.remove(&seq);
                self.live_seqs.remove(&seq);
                continue;
            }
            return Some(top.deadline_ns);
        }
        None
    }

    /// Fire every timer with `deadline_ns <= now_ns` in deadline order
    /// (FIFO among equal deadlines), passing `(id, payload)` to `fire`.
    /// Cancelled timers are skipped silently. Returns the number fired.
    pub fn expire<F>(&mut self, now_ns: u64, mut fire: F) -> usize
    where
        F: FnMut(TimerId, T),
    {
        let mut fired = 0;
        while let Some(top) = self.heap.peek() {
            if top.deadline_ns > now_ns {
                break;
            }
            let entry = self.heap.pop().expect("peeked entry exists");
            self.live_seqs.remove(&entry.seq);
            if self.cancelled.remove(&entry.seq) {
                continue;
            }
            fired += 1;
            fire(TimerId(entry.seq), entry.payload);
        }
        fired
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fires_in_deadline_order_fifo_among_equal_deadlines() {
        let mut sched: TimerScheduler<&'static str> = TimerScheduler::new();
        sched.schedule(300, "late");
        sched.schedule(100, "first");
        sched.schedule(100, "second");
        sched.schedule(200, "middle");

        let mut fired = Vec::new();
        sched.expire(1_000, |_, name| fired.push(name));
        assert_eq!(fired, vec!["first", "second", "middle", "late"]);
        assert_eq!(sched.heap_len(), 0);
    }

    #[test]
    fn expire_only_fires_due_timers() {
        let mut sched: TimerScheduler<u64> = TimerScheduler::new();
        let _early = sched.schedule(100, 1);
        let _later = sched.schedule(900, 2);

        let mut fired = Vec::new();
        assert_eq!(sched.expire(150, |_, p| fired.push(p)), 1);
        assert_eq!(fired, vec![1]);
        assert_eq!(sched.expire(150, |_, p| fired.push(p)), 0, "one-shot");
        assert_eq!(sched.expire(900, |_, p| fired.push(p)), 1);
        assert_eq!(fired, vec![1, 2]);
    }

    #[test]
    fn cancel_skips_firing_without_disturbing_others() {
        let mut sched: TimerScheduler<char> = TimerScheduler::new();
        let _a = sched.schedule(100, 'a');
        let b = sched.schedule(110, 'b');
        let _c = sched.schedule(120, 'c');

        assert!(sched.cancel(b));
        assert!(!sched.cancel(b), "second cancel is a no-op");
        // Unknown ids are harmless: cancelling something never scheduled
        // neither panics nor affects the pending population.
        sched.cancel(TimerId(99_999));

        let mut fired = Vec::new();
        sched.expire(1_000, |_, ch| fired.push(ch));
        assert_eq!(fired, vec!['a', 'c']);
    }

    #[test]
    fn cancel_after_fire_is_harmless() {
        let mut sched: TimerScheduler<u8> = TimerScheduler::new();
        let id = sched.schedule(10, 7);
        let mut fired = 0;
        sched.expire(20, |_, _| fired += 1);
        assert_eq!(fired, 1);
        // Handle cancellation raced the fire: no panic, and the fired count
        // never double-counts on later expires.
        sched.cancel(id);
        sched.expire(100, |_, _| fired += 1);
        assert_eq!(fired, 1);
    }

    #[test]
    fn cancelling_an_unallocated_id_does_not_kill_a_later_timer() {
        let mut sched: TimerScheduler<u8> = TimerScheduler::new();
        assert!(
            !sched.cancel(TimerId(2)),
            "a never-allocated id is not cancellable"
        );
        sched.schedule(10, 1); // seq 0
        sched.schedule(20, 2); // seq 1
        sched.schedule(30, 3); // seq 2 — the id the stray cancel targeted
        let mut fired = Vec::new();
        sched.expire(100, |_, p| fired.push(p));
        assert_eq!(
            fired,
            vec![1, 2, 3],
            "seq 2 must fire: the stale cancel must not inherit a later timer"
        );
    }

    #[test]
    fn cancelling_fired_and_unknown_ids_keeps_tombstone_set_empty() {
        let mut sched: TimerScheduler<u8> = TimerScheduler::new();
        let id = sched.schedule(10, 7);
        sched.expire(20, |_, _| {});
        sched.cancel(id); // already fired
        sched.cancel(TimerId(999)); // never allocated
        assert_eq!(
            sched.cancelled_len(),
            0,
            "no permanent tombstone accumulation"
        );
    }

    #[test]
    fn cancel_where_removes_matching_live_timers_only() {
        let mut sched: TimerScheduler<(u64, u8)> = TimerScheduler::new();
        sched.schedule(100, (1, 0));
        sched.schedule(150, (2, 7));
        sched.schedule(200, (3, 0));
        assert_eq!(sched.cancel_where(|(_, kind)| *kind == 0), 2);
        let mut fired = Vec::new();
        sched.expire(1_000, |_, p| fired.push(p));
        assert_eq!(fired, vec![(2, 7)], "only non-matching timers survive");
        assert_eq!(sched.heap_len(), 0);
        assert_eq!(sched.next_deadline(), None);
    }

    #[test]
    fn past_deadlines_fire_on_next_expire() {
        let mut sched: TimerScheduler<u8> = TimerScheduler::new();
        sched.schedule(50, 1);
        let mut fired = 0;
        sched.expire(10, |_, _| fired += 1);
        assert_eq!(fired, 0);
        sched.expire(50, |_, _| fired += 1);
        assert_eq!(fired, 1);
    }

    #[test]
    fn next_deadline_skips_cancelled_top_entries() {
        let mut sched: TimerScheduler<u8> = TimerScheduler::new();
        let first = sched.schedule(100, 1);
        let _second = sched.schedule(200, 2);
        sched.cancel(first);
        assert_eq!(sched.next_deadline(), Some(200));
        let second = sched.schedule(300, 3);
        sched.cancel(second);
        assert_eq!(sched.next_deadline(), Some(200));
    }

    #[test]
    fn next_deadline_empty_and_fully_cancelled() {
        let mut sched: TimerScheduler<u8> = TimerScheduler::new();
        assert_eq!(sched.next_deadline(), None);
        let id = sched.schedule(5, 1);
        sched.cancel(id);
        assert_eq!(sched.next_deadline(), None);
        assert_eq!(sched.heap_len(), 0, "tombstones drained from the top");
    }

    #[test]
    fn integrates_with_manual_clock_steps() {
        use crate::clock::{Clock, ManualClock};

        let mut clock = ManualClock::new();
        let mut sched: TimerScheduler<&'static str> = TimerScheduler::new();
        let now = clock.now_nanos();
        let a = sched.schedule(now + 1_000, "think");
        let b = sched.schedule(now + 2_000, "probe");
        sched.cancel(a);

        let mut log = Vec::new();
        clock.advance(1_000);
        let fired = sched.expire(clock.now_nanos(), |_, tag| log.push(tag));
        assert_eq!((fired, clock.now_nanos() - now), (0, 1_000));

        clock.advance(1_000);
        let fired = sched.expire(clock.now_nanos(), |id, tag| {
            log.push(tag);
            assert_eq!(id, b);
        });
        assert_eq!(fired, 1);
        assert_eq!(log, vec!["probe"]);
    }

    #[test]
    fn scales_to_heavy_population_correctly() {
        // Correctness at the benchmark operating point (50k actors x 4 timers,
        // half cancelled) — throughput lives in the criterion bench.
        let mut sched: TimerScheduler<(usize, usize)> = TimerScheduler::new();
        let mut ids = Vec::with_capacity(200_000);
        for actor in 0..50_000 {
            for slot in 0..4 {
                let deadline = 1_000_000 * (10 * (slot + 1) as u64 + (actor as u64) % 7);
                ids.push(sched.schedule(deadline, (actor, slot)));
            }
        }
        for (index, id) in ids.iter().enumerate() {
            if index % 2 == 0 {
                sched.cancel(*id);
            }
        }
        let mut fired = 0usize;
        sched.expire(u64::MAX / 2, |_, _| fired += 1);
        assert_eq!(fired, 100_000);
    }
}
