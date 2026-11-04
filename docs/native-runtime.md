---
title: Native Runtime
---

# Native Runtime Design

## Core Modules

```text
runtime/
  actor/
  clock/
  scheduler/
  arrival/
  session/
  transport/
  protocol/
  behavior/
  metrics/
```

The first version avoids splitting into too many crates. Logical modules can live in a small number of crates first.

## Actor Lifecycle

```text
Planned
 -> Scheduled
 -> Creating
 -> Connecting
 -> Authenticating
 -> Active
 -> Draining
 -> Closed

Any stage -> Failed
```

Every state transition must record a timestamp and a reason.

## Virtual Clock

Support at least:

- Manual Clock: for tests and deterministic replay.
- Real Clock: for production execution.
- Timer Queue: timed events.
- Think Time: behavior wait distributions.
- Arrival Scheduler: bot arrival scheduling.
- Seeded RNG: reproducible random sources.

The clock abstraction must not let business behavior read the system wall clock directly. When system time is needed, it goes through an explicit Clock API and is marked as non-deterministic.

## Scheduler

Measurements needed:

- timer insert / cancel / expire.
- expiry queue delay.
- Actor runnable queue length.
- deviation between planned_at and actual_start.
- single tick execution time.
- event loop lag.

Benchmarks are mandatory at 5k / 20k / 50k Actors with 4 timers per Actor. Implementation details of Tokio's timer wheel do not substitute for end-to-end runtime benchmarks.

## Transport / Protocol

Layering:

- Transport: TCP / UDP / WebSocket and other connection types actually needed.
- Framing: message boundaries.
- Schema: message definitions and versions.
- Codec: encoding and decoding.
- Session Adapter: login, heartbeat, reconnection, session state.
- Behavior: business actions.

Schema is not Codec. Transport should not know what game business messages mean.

## Behavior

v1 of the Native Runtime can start with an internal Rust Behavior trait.

The behavior context exposes only:

- the injected clock.
- the injected random source.
- send actions.
- subscribe to messages.
- timers.
- Actor state.
- resource budgets.
- structured logging.

Worker internals, sockets, thread handles, and the global runtime are not exposed.

Lua is not in v1. If introduced later, it must come with an instruction budget, sandboxing, deterministic RNG, time injection, and isolation tests.
