---
title: Architecture
---

# Architecture

## System Architecture

<ArchitectureDiagram />

## Logical Architecture

```text
                         Web Dashboard / CLI
                                  |
                           HTTP / WebSocket
                                  |
                       +----------------------+
                       | Wavemaker Controller |
                       | Rust + Axum + Tokio  |
                       +----------------------+
                         |        |         |
                  Plan API   Backend API  Report API
                         |        |         |
                  +------+--------+---------+------+
                  |          Control Protocol      |
                  |             Protobuf           |
                  +------+-------------------+------+
                         |                   |
             +-----------+----+      +-------+----------------+
             | Native Worker  |      | Engine Bot Supervisor  |
             | Rust + Tokio   |      | Rust                    |
             +----------------+      +-------------------------+
             | Actor Runtime  |                 |
             | Scheduler      |          Local IPC / RPC
             | Virtual Clock  |                 |
             | Transport      |       +---------+-----------+
             | Protocol       |       | Engine Bot Process  |
             | Metrics        |       | KBEngine Bots       |
             +----------------+       | C++ Runtime         |
                                      | Python Bridge       |
                                      +---------------------+

            +-------------------- Basin ----------------------+
            | Topology / Providers / Metric Mapping / Queries |
            | Prometheus / OTel / Custom Provider             |
            +-------------------------------------------------+
                                  |
                         Swell Report Pipeline
```

## Control Plane and Execution Plane

### Control Plane

The Rust Controller is responsible for:

- Swell lifecycle.
- Spectrum validation.
- Global plan generation.
- ShardPlan generation and freezing.
- Backend registration and capability negotiation.
- Backend allocation and leases.
- Control commands and state synchronization.
- Basin configuration and observability task orchestration.
- Data integrity checks.
- Report generation orchestration.

### Execution Plane

Implemented by different Backends:

- Native Runtime Backend: Rust Workers.
- Engine Bot Backend: Supervisor + engine native bots + Bridge.
- Third-party Backends are allowed later, but must follow the unified protocol and capability model.

### Observability Plane

Data falls into three categories:

1. Generator: Workers, Supervisors, bot processes, and bot states.
2. Server: server-side hosts, processes, services, databases, and business metrics in the Basin.
3. Experiment: planned load, actual arrivals, phases, failure events, deployed versions, and time ranges.

## Three Classes of Execution Entities

These must be strictly distinguished:

1. **Controller**: manages Swells, generates plans, schedules Backends, aggregates data.
2. **Worker / Backend**: the management boundary that actually executes load.
3. **Actor / Engine Bot**: the concrete virtual client.

For the Native Runtime, one Worker can manage thousands of Actors.

For KBEngine, one Engine Bot Backend can manage one or more KBEngine Bots processes, each running multiple Bots managed by KBEngine itself.

The Controller manages Backends; Backends manage processes and the bots visible to them; the engine manages bot internal state.

## Mode A / Mode B / Mode C Definitions

> Status: **frozen for v1** (WM-0001) — this section is the authoritative
> in-repo definition of the three access modes. The original architecture
> document that first introduced the modes is not available locally
> (noted in ADR-005); the definitions below are pinned from the plan
> document §6.6 phrasing plus the audited KBEngine facts in
> `docs/spec/kbengine-api-audit.md`, so every capability claim traces to
> source evidence. If the original document resurfaces and disagrees,
> reconcile through a new ADR.

The modes are not compile options of one Actor type. Each mode is a
combination of execution backend and access capability, distinguished by
three ownership questions: who creates bots, who owns the socket, and at
what granularity Wavemaker can control and observe.

| Question | Mode A — engine-managed bots | Mode B — engine client stack, Wavemaker executor | Mode C — native runtime |
|---|---|---|---|
| Execution Backend | Engine Bot Backend | Engine Bot Backend (capability variant) | Native Runtime Backend |
| Bot creation | Engine creates bots natively | Wavemaker executor creates sessions | Wavemaker Worker creates Actors |
| Socket / connection ownership | Engine C++ runtime | Engine client library, driven by the Wavemaker executor process | Wavemaker Worker |
| Control granularity | Batch level only | Per-session commands | Per-Actor commands |
| Observation | Engine-exposed aggregates and events | Session events surfaced by the client library | Full per-Actor state |
| Behavior code | Engine scripts | Wavemaker-driven via the client library API | Wavemaker Behavior (Rust in v1) |
| v1 availability | First target (KBEngine adapter, Phase 2) | Not in v1; requires a dedicated ADR before any implementation | Phase 3 |

Mode A facts for KBEngine (evidence in `docs/spec/kbengine-api-audit.md`):

- Bot creation is batch-only and additive: `addBots` accumulates a global
  pending-creation counter, returns nothing and has no failure callback
  (`bots.cpp:344-388`); the Bridge must do its own batch budgeting before
  StopAdmission (audit §1, discrepancies D1/D2).
- There is no single-bot creation API: `KBEngine.bots` is a read-only
  mapping keyed by int32 client appID (`pybots.cpp:16-113`); per-bot control
  is not available and per-bot observation must go through each
  `ClientObject` (audit §2).
- Per-bot latency cannot be derived from process-level data, and totals
  must not be presented as online users — capability claims stay per the
  audited APIs ("No Unverified Capability Claims").

Mode B is definitionally frozen here but has no v1 implementation: the
socket is owned by the engine's client library while the executor process
is managed by Wavemaker, which yields per-session control that Mode A
lacks. A future Mode B Backend must declare its capabilities explicitly in
its own ADR and must not claim observation semantics it cannot back.

Mode C is the Native Runtime Backend: the Worker owns connections, protocol
execution, and behavior scheduling (`docs/native-runtime.md`). It is the
only mode where per-Actor arrival precision is fully controlled by
Wavemaker.

Consequences:

- Capability negotiation reports the mode per Backend; a Bridge must return
  Unsupported rather than approximate a finer-grained mode.
- Mixed Swells keep per-mode grouping in reports (ADR-005).
