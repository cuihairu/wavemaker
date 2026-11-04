---
title: Risks
---

# Risk Register

| ID | Risk | Impact | Mitigation |
|---|---|---|---|
| R-001 | arrival / concurrency semantics not closed | Wrong plans, invalid load tests | Phase 0 mathematical model and validator |
| R-002 | Insufficient KBEngine callback precision | Arrival plan deviation | Measure and declare scheduling resolution |
| R-003 | KBEngine API lacks precise bot control | False capability claims | Capability Unknown / Unsupported |
| R-004 | Bridge IPC blocks the engine thread | Delayed bot behavior | Dual queues, async IO, callback delivery |
| R-005 | Bridge event loss | Incomplete reports | sequence, cursor, batched retransmission |
| R-006 | Controller / Supervisor lost contact | Inconsistent state | Leases, generation, recovery synchronization |
| R-007 | Generator saturation | Misjudged server capacity | Generator Saturated label |
| R-008 | Inconsistent HDR parameters | Wrong distribution statistics | Controller-issued parameters + conformance tests |
| R-009 | Inconsistent timestamps | Wrong curve correlation | Multi-timestamp model, offset estimation, quality labels |
| R-010 | Containers become a hard dependency | Constrained deployment | Native Process Launcher first |
| R-011 | Multi-language protocol semantics fork | Compatibility problems | Protobuf as the only cross-process contract |
| R-012 | Over-splitting modules | Higher development cost | Start with a small number of crates |
| R-013 | Monitoring collection affects the server | Distorted measurements | observer_baseline and automatic degradation |
| R-014 | Mixed Backend loads not comparable | Misleading conclusions | Per-Backend reporting and semantic calibration |

# Architecture Quality Gates

At the end of every phase, these questions must be answered:

1. Is the plan deterministic and reproducible?
2. Do Backends declare their capabilities truthfully?
3. Is actual load comparable to planned load?
4. Did the generator become the bottleneck?
5. Is server-side observability coverage sufficient?
6. Does the observation behavior affect the system under test?
7. Do lost contact and recovery create additional load?
8. Can data be traced back to a Swell, Backend, Shard, and time window?
9. Can the report clearly state which conclusions are trustworthy and which data is incomplete?
10. Is every new abstraction backed by a real second implementation or an explicit requirement?

No phase may substitute "the interface is already reserved" for a runnable conformance test.
