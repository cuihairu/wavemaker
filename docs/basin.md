---
title: Basin
---

# Server Observability: Basin

## Basin's Role

Basin represents the environment under test, its service topology, and its observability configuration. It is not a monitoring product and does not reimplement Prometheus.

```text
Basin
  ├── Environment
  ├── Topology
  ├── Observability
  │     ├── Provider Registry
  │     ├── Metric Catalog
  │     ├── Metric Mapping
  │     ├── Query Scheduler
  │     ├── Time Alignment
  │     └── Data Quality
  └── Correlation Context
```

## Providers

Recommended for v1:

- Prometheus Provider: implement first.
- OpenTelemetry Provider: according to actual data ingestion needs.
- Custom Provider: keep a unified interface.
- Grafana: as the entry point to existing monitoring visualization/data sources, not as a replacement for metric storage semantics.

## Metric Categories

### Host

- CPU, memory, disk IO, network throughput, packet loss.

### Process

- CPU, RSS, thread count, FD, GC (if applicable), event loop latency.

### Gateway / Login

- Connection count, connection establishment rate, message throughput, login success rate and latency.

### Game

- Online users, tick duration, tick overrun, message processing time.

### Queue / RPC

- Queue length, wait time, RPC count, success rate and latency.

### Database / Cache

- Query rate, query latency, connection pool, hit rate and error rate.

### Business

- Matchmaking time, room count, players per scene, key business operation success rates.

Metrics need a unified unit and semantics catalog; meaning must not be guessed from Prometheus metric names alone.

## Timestamp Model

Keep at least:

- source_timestamp: when the source system produced the sample.
- observed_at: when Wavemaker considers the sample observed.
- collected_at: when Wavemaker actually collected the data.

Also record:

- query time window.
- scrape / collection interval.
- data source latency.
- missing intervals.
- clock offset estimates.
- data source instance and labels.

Do not treat absolute NTP synchronization as the only premise. Load curves and event shapes can assist alignment, but shape alignment does not replace real timestamps and must not be used to fabricate time precision.

## Preflight

Checks before a Swell starts:

- Providers are reachable.
- Required metrics exist.
- The target topology is fully covered.
- Metric units and type mappings are correct.
- Sampling intervals satisfy report needs.
- Query windows are available.
- Monitoring data source permissions are sufficient.
- Whether high-cardinality label risks exist.

If Server Observability is unavailable:

- The test may be allowed to continue per configuration.
- The report is marked `Partial Observability`.
- Complete server-side performance conclusions must not be given.

## Observation Overhead

Follow the ADR's three-level observation approach:

- L0: low-overhead basic process/system metrics, on by default.
- L1: watchers or engine-specific monitoring; overhead measured before enabling.
- L2: high-overhead diagnostics such as eBPF, enabled only in limited time windows.

`observer_baseline` samples 60 seconds with no load; if observation overhead exceeds the target threshold (the ADR currently proposes 3%, measurement method to be defined), automatically degrade and record it in the report.

Call frequencies for KBEngine / BigWorld-specific watchers must be verified against the actual implementation. The ADR's conclusions about the 5-second cap on KBEngine's `cluster_controller.py` and BigWorld watcher lock behavior should be written into adapter capabilities only after source review and measurement on the target version — not as permanent assumptions for all versions.
