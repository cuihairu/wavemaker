---
title: Engine Bot Bridge
---

# Engine Bot Bridge Design

## Design Goals

The Bridge's goal is to make engine native bots manageable and observable by Wavemaker, not to turn engine bots into Wavemaker Actors.

The Bridge should support:

- Identity registration.
- Version and capability negotiation.
- Initialization configuration.
- Load plan submission.
- Start and stop of new admissions.
- Graceful exit.
- Status queries.
- Bot lifecycle events.
- Business events.
- Generator metrics.
- Heartbeat and reconnection.
- Data integrity and event sequence checks.

## Two Communication Boundaries

### Controller ↔ Supervisor

Tonic / gRPC + Protobuf is recommended:

- Control commands.
- Status queries.
- Backend registration.
- Leases and heartbeats.
- Batched event/metrics reporting.

### Supervisor ↔ Engine Bridge

Prefer local IPC:

- Linux: Unix Domain Socket.
- Windows: Named Pipe.
- TCP loopback: as a cross-platform fallback or for debugging.
- Stdio RPC: development and debugging only; not the default production channel.

The Bridge and Supervisor may use bidirectional streaming, or request/response plus an event stream. Message ordering, reconnection, and idempotency semantics must be defined first.

## Bridge Protocol Draft

The following is a logical protocol draft; concrete fields are frozen in Phase 0 through Protobuf definitions.

```protobuf
syntax = "proto3";

package wavemaker.engine.v1;

service EngineBridge {
  rpc Handshake(HandshakeRequest) returns (HandshakeResponse);
  rpc Initialize(InitializeRequest) returns (OperationResult);
  rpc ApplyPlan(ApplyPlanRequest) returns (OperationResult);
  rpc Start(StartRequest) returns (OperationResult);
  rpc StopAdmission(StopAdmissionRequest) returns (OperationResult);
  rpc Drain(DrainRequest) returns (OperationResult);
  rpc Shutdown(ShutdownRequest) returns (OperationResult);
  rpc GetStatus(StatusRequest) returns (StatusResponse);
  rpc GetMetrics(MetricsRequest) returns (MetricsResponse);
  rpc SubscribeEvents(EventSubscription) returns (stream EngineEvent);
}
```

### Handshake

Returns:

- engine_name
- engine_version
- adapter_name
- adapter_version
- protocol_version
- supported_capabilities
- unsupported_capabilities
- metric_schema_version
- max_batch_size
- scheduling_resolution
- clock_source
- process_id (when applicable)
- backend_instance_id

When protocol versions are incompatible, startup must be refused; silent degradation is not allowed.

### Capability Model

Capability values must not be a plain true/false. Suggested values:

```text
Supported
Unsupported
Unknown
Degraded
```

Suggested capability catalog:

```yaml
capabilities:
  process:
    start: supported
    stop: supported
    restart: supported
    resource_limits: supported

  bots:
    create_batch: supported
    create_individual: unknown
    destroy_individual: unknown
    pause_individual: unknown
    inspect_individual: unknown

  scheduling:
    exact_arrival: unsupported
    batch_arrival: supported
    resolution_ms: 1000

  observability:
    process_cpu: supported
    process_memory: supported
    bot_count: supported
    connected_count: unknown
    login_success: unknown
    per_bot_events: unknown
    message_latency: unsupported
```

Capability declarations must be generated or verified by adapter conformance tests; a hand-written configuration file alone is not enough.

## Command Semantics

| Command | Semantics | Idempotency Requirement |
|---|---|---|
| Handshake | Protocol and capability negotiation | Yes |
| Initialize | Initialize a Backend instance for one Swell | Idempotent for the same generation |
| ApplyPlan | Receive a frozen plan shard | Idempotent for the same plan_digest |
| Start | Start executing an accepted plan | Idempotent for the same operation_id |
| StopAdmission | Stop new bots without closing existing connections | Idempotent |
| Drain | Wait for or clean up existing bots according to policy | Idempotent per operation_id |
| Shutdown | End the Backend instance | Idempotent |
| GetStatus | Return a snapshot of the current state | Yes |
| GetMetrics | Return a metric snapshot for the given window | Yes |
| SubscribeEvents | Subscribe to the event stream | Resumed via cursor |

Do not interpret `Stop` as both "stop new admissions" and "disconnect existing connections". It must be split into `StopAdmission` and `Drain`.

## Event Model

Events include at least:

- BackendStarted
- BackendReady
- BotCreated
- BotConnected
- BotLoginSucceeded
- BotLoginFailed
- BotStateChanged
- BotDisconnected
- BotDestroyed
- BotError
- AdmissionStopped
- BackendDegraded
- BackendFailed

Every event includes:

- event_id
- backend_id
- swell_id
- shard_id
- bot_id (when obtainable)
- sequence
- source_timestamp
- observed_timestamp
- event_type
- payload
- schema_version

Events should support batch sending, resuming from a cursor after reconnection, and deduplication of repeated events. If the engine cannot provide a stable bot ID, a stable adapter-generated mapping ID must be defined, with its lifecycle scope documented.

## Metrics Model

Distinguish at least:

### Process level

- CPU usage
- RSS / virtual memory
- thread count
- file descriptor / handle count
- process uptime
- event loop / callback lag
- IPC queue depth
- dropped events

### Bot-level aggregates

- total bots
- created bots
- connected bots
- logged-in bots
- failed bots
- disconnected bots
- pending bots
- bots by state

### Business level

- login attempts / success / failure
- message count
- message bytes
- operation success / failure
- operation latency (provided only when it can be measured reliably)
- reconnect count

Every metric must define:

- Name and description.
- Unit.
- Counter / Gauge / Histogram.
- Aggregation semantics.
- Sampling interval.
- Data source.
- Whether it is a native engine metric or a Bridge-derived metric.
- Statistical error and missing-data semantics.

## Bridge Threading Model

Do not call engine objects directly from RPC threads.

Recommended model:

```text
IPC Receiver
    |
    v
Command Queue
    |
    v
Engine Main / Callback Thread
    |
    +--> invoke engine API
    +--> inspect bot state
    +--> collect engine events
    |
    v
Event / Metrics Queue
    |
    v
IPC Sender
```

For KBEngine, after receiving external commands in the Python scripting environment, the Bridge should enqueue operations into the callback scheduling context the engine allows. Concrete thread-safety guarantees need to be verified against the target KBEngine version.
