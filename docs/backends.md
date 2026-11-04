---
title: Execution Backends
---

# Execution Backend Model

## Backend Interface

All execution backends implement the same conceptual interface:

```text
ExecutionBackend
  ├── handshake()
  ├── initialize(config)
  ├── apply_plan(shard_plan)
  ├── start()
  ├── update_load(update)
  ├── stop_admission()
  ├── drain()
  ├── shutdown()
  ├── get_status()
  ├── get_metrics()
  └── subscribe_events()
```

This is a set of domain operations; backends are not required to implement them as the same RPC methods. The protocol must explicitly describe unsupported capabilities.

### Backend States

```text
Created
  -> Handshaking
  -> Ready
  -> Initialized
  -> PlanAccepted
  -> Running
  -> AdmissionStopped
  -> Draining
  -> Stopped

Any running state -> Degraded
Any running state -> Failed
Failed -> Recovering -> Handshaking
```

Every state change must include:

- backend_id
- swell_id
- state
- observed_at
- reason_code
- detail
- generation / epoch

Do not infer that a Backend has completed a state transition merely because the Controller's command was sent successfully.

## Native Runtime Backend

Implemented by Rust Workers:

- Actor lifecycle.
- Session / Connection.
- Transport.
- Framing / Codec.
- Behavior Runtime.
- Virtual Clock.
- Timer Scheduler.
- Arrival Scheduler.
- Local metric collection.
- Batched event reporting.

The Native Runtime Actor API must not be exposed to external engine Bridges.

## Engine Bot Backend

Reuses the game engine's existing bots:

- The Supervisor manages bot processes or containers.
- The Bridge runs in the extension environment provided by the engine.
- The engine itself manages bot connections, protocols, and entity objects.
- The Bridge maps the control capabilities visible to the engine onto the Backend API.
- Operations that cannot be implemented return an explicit Unsupported / CapabilityUnavailable.
- Per-bot control capabilities must not be fabricated.

## Mixed Execution

A Swell can include multiple Backends:

```yaml
execution:
  backends:
    - id: native-east
      type: native
      weight: 0.7
    - id: kbengine-native
      type: engine_bot
      adapter: kbengine
      weight: 0.3
```

The `weight` above is only an allocation sketch. Actual plans must be expanded into deterministic Backend assignments and ShardPlans; allocation must not happen randomly on the Worker side.

Mixed execution must:

- Present Backend results separately in reports.
- Record each Backend's capabilities and scheduling precision.
- Run semantic alignment tests for the same business scenario.
- Not treat results from different client implementations as strictly equivalent by default.
- Provide per-Backend CPU, memory, connection count, and effective request rate comparisons.
