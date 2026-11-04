---
title: Decisions
---

# Final Technical Decision Summary

| Decision | Result |
|---|---|
| Core language | Rust |
| Controller / Web | Rust + Axum |
| Worker | Rust + Tokio |
| Supervisor | Rust |
| Cross-process contract | Protobuf |
| KBEngine access | Python Bridge + Rust Supervisor |
| Native bot reuse | First-class Execution Backend |
| Containers | Optional deployment method |
| Native Runtime | Wavemaker's own Actor Runtime |
| Behavior in v1 | Internal Rust Behavior |
| Lua | Not implemented in v1 |
| Python | Required for the KBEngine Bridge; general SDK later |
| C++ | Native SDK / separate process / isolated shim |
| Go | Not core; SDK possible in the future |
| Server-side monitoring | Basin |
| Generator monitoring | Generator Observability |
| Plan generation | Controller as the only generator, plan-as-data |
| Distribution | ShardPlan freezing, leases, generation |
| Reports | Planned load, actual load, generator, server, and data quality presented together |

**Final principle: Wavemaker unifies load test management, plans, control protocols, observability, and reporting — it does not force every game engine to use the same bot implementation.**

Engines can keep using their own native bots, network stacks, and scripting systems; Wavemaker brings them into unified experiment management through Backend + Supervisor + Bridge. At the same time, Wavemaker keeps its own high-performance Rust Native Runtime for engines without mature native bots or those needing precise control.

That is the compatibility boundary a cross-engine load testing framework should have.
