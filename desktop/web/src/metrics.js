// Engine-defined generator / server metric families (WM-0605).
//
// The field lists and aggregation semantics mirror the authoritative
// engine types in wm-basin — `WorkerSnapshot` / `GeneratorAggregate`
// (crates/wm-basin/src/generator.rs) for the generator side and
// `MetricAggregate` / `MetricKind` (crates/wm-basin/src/server.rs) for
// the server side. Nothing here is invented: these are the metrics the
// engine reports, with the aggregation rule each field's semantics
// demand (gauges -> peak, counters -> sum, latencies -> max of
// per-worker maxima).
//
// The data source for every one of them is currently ABSENT: the
// controller exposes no metrics endpoint (`/api/v1/status` only), no
// metric series is persisted, and the aggregation pipeline (WM-0412 /
// WM-0413) is not a completed deliverable yet. The view therefore
// renders each family with an honest "Not wired" source rather than a
// fabricated curve.

/// Generator-side metric families, in the engine's field order.
export const GENERATOR_METRICS = [
  { metric: 'cpu_ratio', kind: 'gauge', aggregate: 'peak', unit: '0..1' },
  { metric: 'rss_bytes', kind: 'gauge', aggregate: 'peak', unit: 'bytes' },
  { metric: 'thread_count', kind: 'gauge', aggregate: 'peak', unit: 'count' },
  { metric: 'fd_count', kind: 'gauge', aggregate: 'peak', unit: 'count' },
  { metric: 'event_queue_length', kind: 'gauge', aggregate: 'peak', unit: 'count' },
  { metric: 'control_channel_latency_ms', kind: 'latency', aggregate: 'max', unit: 'ms' },
  { metric: 'dropped_event_reports', kind: 'counter', aggregate: 'sum', unit: 'count' },
  { metric: 'metrics_reporting_latency_ms', kind: 'latency', aggregate: 'max', unit: 'ms' },
  { metric: 'planned_actual_deviation_ms', kind: 'latency', aggregate: 'max', unit: 'ms' },
  { metric: 'scheduler_lag_ms', kind: 'latency', aggregate: 'max', unit: 'ms' },
  { metric: 'actor_count', kind: 'gauge', aggregate: 'peak', unit: 'count' },
  { metric: 'active_connections', kind: 'gauge', aggregate: 'peak', unit: 'count' },
  { metric: 'restarts', kind: 'counter', aggregate: 'sum', unit: 'count' },
  { metric: 'lost_contact_events', kind: 'counter', aggregate: 'sum', unit: 'count' },
]

/// Server-side aggregation rules, one per catalog kind (WM-0403). The
/// histogram row is not a number: distribution semantics need merged
/// histograms, so a scalar would fabricate percentiles.
export const SERVER_METRICS = [
  {
    kind: 'gauge',
    aggregate: 'min / mean / max',
    note: 'over finite samples; NaN samples counted, never dropped silently',
  },
  {
    kind: 'counter',
    aggregate: 'first / last / delta',
    note: 'delta = last - first; a negative delta is a reset, recorded as-is',
  },
  {
    kind: 'histogram',
    aggregate: 'not aggregated to scalars',
    note: 'needs merged histograms; a scalar would fabricate percentiles',
  },
]
