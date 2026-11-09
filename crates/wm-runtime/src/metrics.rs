//! Worker-local Metrics (WM-0316).
//!
//! Each worker thread owns one [`WorkerMetrics`]: lock-free atomic counters
//! and duration tracks with no cross-thread contention. A [`MetricsSnapshot`]
//! is a plain, consistent point-in-time read; [`aggregate`] sums totals across
//! workers and takes the max of each max-tracked duration.
//!
//! Covers the runtime's measurement list (docs/native-runtime.md "Scheduler"):
//! timer insert/cancel/expire, expiry-queue delay, runnable queue length,
//! planned-to-start deviation, single-tick execution time, event-loop lag.

use std::sync::atomic::{AtomicU64, Ordering};

/// Lock-free counters for one worker. `&self` methods only: cheap to call from
/// hot paths.
#[derive(Debug, Default)]
pub struct WorkerMetrics {
    timer_insert: AtomicU64,
    timer_cancel: AtomicU64,
    timer_expire: AtomicU64,
    actors_started: AtomicU64,
    actors_closed: AtomicU64,
    actors_failed: AtomicU64,
    messages_sent: AtomicU64,
    messages_recv: AtomicU64,
    sends_bytes: AtomicU64,
    recv_bytes: AtomicU64,
    run_ns_total: AtomicU64,
    run_ns_max: AtomicU64,
    event_loop_lag_ns_max: AtomicU64,
    expiry_delay_ns_total: AtomicU64,
    expiry_delay_ns_max: AtomicU64,
    planned_to_start_ns_total: AtomicU64,
    planned_to_start_ns_max: AtomicU64,
    runnable_queue_len: AtomicU64,
}

/// Point-in-time read of a worker's counters (plain values, safe to share).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct MetricsSnapshot {
    pub timer_insert: u64,
    pub timer_cancel: u64,
    pub timer_expire: u64,
    pub actors_started: u64,
    pub actors_closed: u64,
    pub actors_failed: u64,
    pub messages_sent: u64,
    pub messages_recv: u64,
    pub sends_bytes: u64,
    pub recv_bytes: u64,
    pub run_ns_total: u64,
    pub run_ns_max: u64,
    pub event_loop_lag_ns_max: u64,
    pub expiry_delay_ns_total: u64,
    pub expiry_delay_ns_max: u64,
    pub planned_to_start_ns_total: u64,
    pub planned_to_start_ns_max: u64,
    pub runnable_queue_len: u64,
}

impl WorkerMetrics {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn record_timer_insert(&self) {
        self.timer_insert.fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_timer_cancel(&self) {
        self.timer_cancel.fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_timer_expire(&self) {
        self.timer_expire.fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_actor_started(&self) {
        self.actors_started.fetch_add(1, Ordering::Relaxed);
    }

    /// Record `n` actors started in one go (e.g. a whole arrival wave).
    pub fn record_actors_started(&self, n: u64) {
        self.actors_started.fetch_add(n, Ordering::Relaxed);
    }

    pub fn record_actor_closed(&self) {
        self.actors_closed.fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_actor_failed(&self) {
        self.actors_failed.fetch_add(1, Ordering::Relaxed);
    }

    /// Record an outbound message and its payload bytes.
    pub fn record_message_sent(&self, bytes: u64) {
        self.messages_sent.fetch_add(1, Ordering::Relaxed);
        self.sends_bytes.fetch_add(bytes, Ordering::Relaxed);
    }

    /// Record an inbound message and its payload bytes.
    pub fn record_message_recv(&self, bytes: u64) {
        self.messages_recv.fetch_add(1, Ordering::Relaxed);
        self.recv_bytes.fetch_add(bytes, Ordering::Relaxed);
    }

    /// Record the wall-clock duration of one actor tick.
    pub fn record_run_ns(&self, ns: u64) {
        self.run_ns_total.fetch_add(ns, Ordering::Relaxed);
        self.run_ns_max.fetch_max(ns, Ordering::Relaxed);
    }

    /// Record observed event-loop lag (actual tick time minus scheduled).
    pub fn record_event_loop_lag_ns(&self, ns: u64) {
        self.event_loop_lag_ns_max.fetch_max(ns, Ordering::Relaxed);
    }

    /// Record the delay between a timer's scheduled expiry and its fire.
    pub fn record_expiry_delay_ns(&self, ns: u64) {
        self.expiry_delay_ns_total.fetch_add(ns, Ordering::Relaxed);
        self.expiry_delay_ns_max.fetch_max(ns, Ordering::Relaxed);
    }

    /// Record the planned-to-start deviation of one actor.
    pub fn record_planned_to_start_ns(&self, ns: u64) {
        self.planned_to_start_ns_total
            .fetch_add(ns, Ordering::Relaxed);
        self.planned_to_start_ns_max
            .fetch_max(ns, Ordering::Relaxed);
    }

    /// Observe the runnable queue length (a gauge: last value wins).
    pub fn observe_runnable_queue_len(&self, len: u64) {
        self.runnable_queue_len.store(len, Ordering::Relaxed);
    }

    pub fn snapshot(&self) -> MetricsSnapshot {
        MetricsSnapshot {
            timer_insert: self.timer_insert.load(Ordering::Relaxed),
            timer_cancel: self.timer_cancel.load(Ordering::Relaxed),
            timer_expire: self.timer_expire.load(Ordering::Relaxed),
            actors_started: self.actors_started.load(Ordering::Relaxed),
            actors_closed: self.actors_closed.load(Ordering::Relaxed),
            actors_failed: self.actors_failed.load(Ordering::Relaxed),
            messages_sent: self.messages_sent.load(Ordering::Relaxed),
            messages_recv: self.messages_recv.load(Ordering::Relaxed),
            sends_bytes: self.sends_bytes.load(Ordering::Relaxed),
            recv_bytes: self.recv_bytes.load(Ordering::Relaxed),
            run_ns_total: self.run_ns_total.load(Ordering::Relaxed),
            run_ns_max: self.run_ns_max.load(Ordering::Relaxed),
            event_loop_lag_ns_max: self.event_loop_lag_ns_max.load(Ordering::Relaxed),
            expiry_delay_ns_total: self.expiry_delay_ns_total.load(Ordering::Relaxed),
            expiry_delay_ns_max: self.expiry_delay_ns_max.load(Ordering::Relaxed),
            planned_to_start_ns_total: self.planned_to_start_ns_total.load(Ordering::Relaxed),
            planned_to_start_ns_max: self.planned_to_start_ns_max.load(Ordering::Relaxed),
            runnable_queue_len: self.runnable_queue_len.load(Ordering::Relaxed),
        }
    }
}

/// Sum totals and take the max of each max-tracked duration across workers.
pub fn aggregate(snapshots: &[MetricsSnapshot]) -> MetricsSnapshot {
    let mut out = MetricsSnapshot::default();
    for s in snapshots {
        out.timer_insert += s.timer_insert;
        out.timer_cancel += s.timer_cancel;
        out.timer_expire += s.timer_expire;
        out.actors_started += s.actors_started;
        out.actors_closed += s.actors_closed;
        out.actors_failed += s.actors_failed;
        out.messages_sent += s.messages_sent;
        out.messages_recv += s.messages_recv;
        out.sends_bytes += s.sends_bytes;
        out.recv_bytes += s.recv_bytes;
        out.run_ns_total += s.run_ns_total;
        out.run_ns_max = out.run_ns_max.max(s.run_ns_max);
        out.event_loop_lag_ns_max = out.event_loop_lag_ns_max.max(s.event_loop_lag_ns_max);
        out.expiry_delay_ns_total += s.expiry_delay_ns_total;
        out.expiry_delay_ns_max = out.expiry_delay_ns_max.max(s.expiry_delay_ns_max);
        out.planned_to_start_ns_total += s.planned_to_start_ns_total;
        out.planned_to_start_ns_max = out.planned_to_start_ns_max.max(s.planned_to_start_ns_max);
        out.runnable_queue_len += s.runnable_queue_len;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counters_increment_and_snapshot_reads_them() {
        let m = WorkerMetrics::new();
        m.record_timer_insert();
        m.record_timer_insert();
        m.record_timer_cancel();
        m.record_timer_expire();
        m.record_actor_started();
        m.record_actor_closed();
        m.record_actor_failed();
        m.record_message_sent(10);
        m.record_message_recv(20);
        m.observe_runnable_queue_len(7);

        let s = m.snapshot();
        assert_eq!(s.timer_insert, 2);
        assert_eq!(s.timer_cancel, 1);
        assert_eq!(s.timer_expire, 1);
        assert_eq!(s.actors_started, 1);
        assert_eq!(s.actors_closed, 1);
        assert_eq!(s.actors_failed, 1);
        assert_eq!(s.messages_sent, 1);
        assert_eq!(s.sends_bytes, 10);
        assert_eq!(s.messages_recv, 1);
        assert_eq!(s.recv_bytes, 20);
        assert_eq!(s.runnable_queue_len, 7);
    }

    #[test]
    fn durations_sum_totals_and_keep_max() {
        let m = WorkerMetrics::new();
        m.record_run_ns(100);
        m.record_run_ns(250);
        m.record_run_ns(150);
        m.record_event_loop_lag_ns(40);
        m.record_event_loop_lag_ns(90);
        m.record_expiry_delay_ns(5);
        m.record_expiry_delay_ns(35);
        m.record_planned_to_start_ns(12);
        m.record_planned_to_start_ns(8);

        let s = m.snapshot();
        assert_eq!(s.run_ns_total, 500);
        assert_eq!(s.run_ns_max, 250);
        assert_eq!(s.event_loop_lag_ns_max, 90);
        assert_eq!(s.expiry_delay_ns_total, 40);
        assert_eq!(s.expiry_delay_ns_max, 35);
        assert_eq!(s.planned_to_start_ns_total, 20);
        assert_eq!(s.planned_to_start_ns_max, 12);
    }

    #[test]
    fn aggregate_sums_totals_and_maxes_maxima() {
        let a = {
            let m = WorkerMetrics::new();
            m.record_timer_insert();
            m.record_message_sent(100);
            m.record_run_ns(200);
            m.record_run_ns(50);
            m.record_expiry_delay_ns(10);
            m.observe_runnable_queue_len(3);
            m.snapshot()
        };
        let b = {
            let m = WorkerMetrics::new();
            m.record_timer_insert();
            m.record_timer_insert();
            m.record_message_sent(30);
            m.record_run_ns(400);
            m.record_expiry_delay_ns(25);
            m.observe_runnable_queue_len(5);
            m.snapshot()
        };

        let merged = aggregate(&[a, b]);
        assert_eq!(merged.timer_insert, 3);
        assert_eq!(merged.messages_sent, 2);
        assert_eq!(merged.sends_bytes, 130);
        assert_eq!(merged.run_ns_total, 650);
        assert_eq!(merged.run_ns_max, 400, "max of maxima");
        assert_eq!(merged.expiry_delay_ns_total, 35);
        assert_eq!(merged.expiry_delay_ns_max, 25);
        assert_eq!(merged.runnable_queue_len, 8, "gauge sums across workers");
    }

    #[test]
    fn default_snapshot_is_all_zero() {
        assert_eq!(MetricsSnapshot::default(), aggregate(&[]));
    }
}
