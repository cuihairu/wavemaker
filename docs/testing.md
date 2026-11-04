---
title: Testing & Validation
---

# Testing and Validation Strategy

## Test Levels

### Unit

- Spectrum validation.
- Plan generation.
- ShardPlan freezing.
- Actor state machine.
- Backend state machine.
- Bridge message mapping.
- Metric aggregation.
- Histogram encoding/decoding.

### Contract / Conformance

- Rust Controller ↔ Rust Worker.
- Rust Supervisor ↔ Python Bridge.
- Protobuf codegen consistency across languages.
- Event sequences and repeated-event handling.
- Declared capabilities matching actual behavior.
- HDR V2 blob merging consistency across implementations.

### Integration

- KBEngine Bots startup and Bridge registration.
- Batch creation.
- Login event collection.
- Stop admission.
- Graceful exit.
- Disconnect and reconnect.
- Supervisor / Bridge / bot process failures.

### Load / Soak

- 5k / 20k / 50k Actors.
- 4 timers per Actor.
- Event bursts.
- Mass connection setup and teardown.
- 24h soak.
- Bridge event reporting pressure.
- High-latency monitoring providers.
- Controller restart and Backend re-registration.

## Performance Adjudication Experiments

The ADR's performance arguments for Rust / Go / Python should be validated with real benchmarks, not by comparing data structure complexity alone.

Test at least:

1. Tokio timer insert/cancel/expire.
2. Go timer heap as a comparison.
3. Python heapq / uvloop as comparisons (only for applicable patterns).
4. Scheduling P50/P95/P99 at 5k / 20k / 50k Actors.
5. Memory with 4 timers per Actor.
6. planned_at deviation at creation rates of 1k–5k/s.
7. Bridge IPC throughput, latency, batch size, and queue growth.
8. KBEngine callback scheduling granularity and backlog behavior.
9. Supervisor process monitoring overhead.
10. Generator metrics overhead on the runtime.

## Test Validity

A Swell can be marked Valid only when all of the following hold:

- The plan configuration passed validation.
- The plan digest matches the digest the Backend received.
- Backend states and actual load data are complete.
- The generator did not reach an unacceptable saturated state.
- Required observability data meets coverage requirements, or is explicitly marked Partial.
- The time series has no unexplained large gaps.
- The deviation between planned and actual arrivals is within the user-defined threshold.
- No unhandled critical failures.
