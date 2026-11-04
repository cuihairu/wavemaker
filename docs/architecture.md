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
