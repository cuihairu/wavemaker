---
title: Introduction
---

## Project Positioning

Wavemaker is a **cross-engine game server load testing framework**. Through a unified load model, plan data, an execution backend interface, and an observability model, it generates, manages, and analyzes the load that large numbers of virtual clients apply to game servers.

It is not a plain bot scripting framework, nor a general-purpose distributed task queue.

Core values:

- Use a unified Spectrum to describe test load.
- Support Wavemaker's own Rust Actor Runtime.
- Reuse the native bots of existing game engines.
- Manage execution backends on a single machine and across multiple machines.
- Observe both the Generator and the server under test.
- Produce auditable reports that include data quality, load deviation, and server-side curves.

## Domain Glossary

| Name | Definition | Should not be confused with |
|---|---|---|
| Wavemaker | The entire load testing framework | A single Worker |
| Swell | One complete load test experiment and its lifecycle | A single task command |
| Spectrum | The configuration describing load shape, arrival process, behavior mix, and lifecycle | A single bot script |
| Basin | The environment under test, its service topology, and its server-side observability configuration | Prometheus itself |
| Controller | The control plane: plan generation, task management, allocation, and report orchestration | An Actor executor |
| Worker | An execution node managed by Wavemaker | Necessarily one Actor |
| Execution Backend | The execution implementation that actually generates load | A process manager |
| Native Runtime Backend | Wavemaker's own Rust execution backend | Native engine bots |
| Engine Bot Backend | An execution backend that reuses the engine's own bots | A Wavemaker Actor |
| Supervisor | The component that manages Backend process/container lifecycle | The engine's internal scheduler |
| Bridge | The control/event/metrics adaptation layer between the engine side and Wavemaker | A mere network proxy |
| Actor | A virtual client in the Native Runtime | A unified object for all engine native bots |
| ShardPlan | An execution plan shard generated and frozen by the Controller | A plan the Worker generates by itself |
| Generator Observability | Observation of the load generator's own resources and runtime state | Basin server-side monitoring |
| Server Observability | Metric collection and time series from the server under test | Generator metrics |
