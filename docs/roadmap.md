---
title: Roadmap
---

# Phased Implementation Plan

## Phase 0: Semantic Closure and Technical Verification

**Goal: remove architecture-level uncertainty before scaling up.**

Tasks:

- [ ] WM-0001: Pin down the precise definitions of Mode A/B/C in the original architecture document.
- [ ] WM-0002: Define the domain model for Swell, Spectrum, Basin, Backend, Worker, Actor.
- [ ] WM-0003: Close the arrival / concurrency / lifetime semantics.
- [ ] WM-0004: Define saturation policies.
- [ ] WM-0005: Verify concurrency curves for exponential arrival under different lifetime distributions.
- [ ] WM-0006: Define plan-as-data and ShardPlan freezing rules.
- [ ] WM-0007: Fix HDR V2 blob parameters and cross-implementation conformance tests.
- [ ] WM-0008: Inspect the target KBEngine version's bot API and callback source code.
- [ ] WM-0009: Verify that the KBEngine Bridge can execute commands safely in the engine scripting context.
- [ ] WM-0010: Measure KBEngine callback scheduling granularity and Bridge queue latency.
- [ ] WM-0011: Build a minimal Rust Worker timer benchmark.
- [ ] WM-0012: Decide whether v1 needs a resident Dashboard and multi-user management.
- [ ] WM-0013: Freeze Protobuf package names, version compatibility policy, and error code conventions.
- [ ] WM-0014: Clarify the persistence boundary for Swell data.

Deliverables:

- ADR-005: execution backend and native bot access decisions.
- ADR-006: Spectrum arrival and concurrency semantics.
- Protobuf v1 first draft.
- KBEngine API verification report.
- Timer / Actor Runtime benchmark report.

Exit criteria:

- BLOCKER-1 closed.
- KBEngine's feasible L0/L1 capability boundaries confirmed.
- Rust Runtime benchmarks with reproducible results.
- v1 control plane deployment shape decided.

## Phase 1: Protocol and Minimal Control Plane

**Goal: first build a Controller, Worker, and Supervisor skeleton that can run independently.**

Tasks:

- [ ] WM-0101: Create the Rust workspace.
- [ ] WM-0102: Implement wm-protocol Protobuf codegen.
- [ ] WM-0103: Implement unified error codes and OperationResult.
- [ ] WM-0104: Implement a minimal Controller Axum API skeleton.
- [ ] WM-0105: Implement Worker gRPC registration and heartbeat.
- [ ] WM-0106: Implement Backend capability handshake.
- [ ] WM-0107: Implement the Supervisor process management abstraction.
- [ ] WM-0108: Implement NativeProcessLauncher.
- [ ] WM-0109: Implement the Backend state machine.
- [ ] WM-0110: Implement Swell metadata and local file storage.
- [ ] WM-0111: Implement CLI validate / run / status / stop.
- [ ] WM-0112: Implement minimal structured logging and tracing.
- [ ] WM-0113: Implement Controller / Worker / Supervisor integration tests.

Deliverables:

- A startable Controller.
- A registrable empty Worker.
- A Supervisor that can start/stop test processes.
- A minimal CLI.
- Protobuf v1 contracts.

Exit criteria:

- The Controller can register, monitor, and stop a Worker.
- The Supervisor can start, detect, and stop a child process.
- Abnormal process exits are recorded correctly.
- Handshake is refused on protocol version incompatibility.

## Phase 2: KBEngine Bridge MVP

**Goal: prove that native bots can be managed, observed, and included in a Swell under a unified model.**

Tasks:

- [ ] WM-0201: Create the KBEngine Bridge Python bootstrap.
- [ ] WM-0202: Implement the Bridge local IPC client.
- [ ] WM-0203: Implement Handshake and capability report.
- [ ] WM-0204: Implement Supervisor startup of KBEngine Bots.
- [ ] WM-0205: Implement Swell / Backend / generation injection.
- [ ] WM-0206: Implement the command queue and engine callback scheduling.
- [ ] WM-0207: Implement ApplyPlan batch plan mapping.
- [ ] WM-0208: Implement bot count collection.
- [ ] WM-0209: Implement connection and login event collection (subject to APIs available in the target version).
- [ ] WM-0210: Implement StopAdmission.
- [ ] WM-0211: Implement Drain / Shutdown.
- [ ] WM-0212: Implement Bridge heartbeat and reconnection.
- [ ] WM-0213: Implement event sequencing, cursor, and duplicate event deduplication.
- [ ] WM-0214: Implement process CPU / RSS / FD collection.
- [ ] WM-0215: Implement Bridge queue length and callback lag metrics.
- [ ] WM-0216: Implement the KBEngine adapter conformance test.
- [ ] WM-0217: Implement an end-to-end login load test sample.
- [ ] WM-0218: Implement Bridge / Supervisor / bot fault injection tests.

Deliverables:

- KBEngine Bridge.
- KBEngine Supervisor Adapter.
- KBEngine Backend capability profile.
- A Swell report containing bot events and generator metrics.

Exit criteria:

- Integration works without modifying the KBEngine C++ network runtime.
- Bots can be started, login states observed, and new admissions stopped.
- Losing the Bridge does not directly wipe existing connections.
- All unknown capabilities are explicitly marked.
- Continuous-run tests show no obvious event loss or resource leaks.

## Phase 3: Native Rust Runtime

**Goal: implement Wavemaker's own high-performance Actor execution backend.**

Tasks:

- [ ] WM-0301: Implement the Actor state machine.
- [ ] WM-0302: Implement the Clock trait and ManualClock.
- [ ] WM-0303: Implement RealClock.
- [ ] WM-0304: Implement the Timer Scheduler.
- [ ] WM-0305: Implement Seeded RNG.
- [ ] WM-0306: Implement the Arrival Scheduler.
- [ ] WM-0307: Implement Actor lifecycle management.
- [ ] WM-0308: Implement the Session state machine.
- [ ] WM-0309: Implement the Transport trait.
- [ ] WM-0310: Implement the first TCP Transport.
- [ ] WM-0311: Implement Framing.
- [ ] WM-0312: Implement Schema / Codec separation.
- [ ] WM-0313: Implement the Behavior trait.
- [ ] WM-0314: Implement message event dispatch.
- [ ] WM-0315: Implement batch Actor scheduling.
- [ ] WM-0316: Implement Worker-local Metrics.
- [ ] WM-0317: Implement HDR histogram and V2 blob.
- [ ] WM-0318: Implement coordinated omission correction.
- [ ] WM-0319: Implement 5k / 20k / 50k benchmarks.
- [ ] WM-0320: Implement the 24h soak test.
- [ ] WM-0321: Implement the Native Backend conformance test.

Deliverables:

- Rust Native Worker.
- Actor Runtime.
- A TCP protocol adaptation example.
- Performance benchmark report.

Exit criteria:

- The same plan + seed reproduces the plan.
- Actor state machine and clock tests pass.
- At target scale, the generator itself is not an obvious bottleneck.
- planned_at vs actual_at deviation meets the project threshold.
- 24h soak passes.

## Phase 4: Basin and Unified Observability

**Goal: bring Generator, Server, and planned load onto a single Swell timeline.**

Tasks:

- [ ] WM-0401: Implement the Basin data model.
- [ ] WM-0402: Implement the service topology model.
- [ ] WM-0403: Implement the Metric Catalog.
- [ ] WM-0404: Implement the Prometheus Provider.
- [ ] WM-0405: Implement Metric Mapping.
- [ ] WM-0406: Implement the Query Scheduler.
- [ ] WM-0407: Implement source_timestamp / observed_at / collected_at.
- [ ] WM-0408: Implement missing-data and delay detection.
- [ ] WM-0409: Implement Preflight.
- [ ] WM-0410: Implement observer_baseline.
- [ ] WM-0411: Implement observation overhead evaluation and degradation.
- [ ] WM-0412: Implement Generator Metrics aggregation.
- [ ] WM-0413: Implement Server Metrics aggregation.
- [ ] WM-0414: Implement timeline alignment.
- [ ] WM-0415: Implement data quality labels in reports.
- [ ] WM-0416: Implement end-to-end correlation reports.

Deliverables:

- Basin configuration.
- Prometheus Provider.
- A unified Generator / Server timeline.
- Reports with data quality.

Exit criteria:

- Coverage of the target service topology can be demonstrated.
- Missing data is not treated as zero.
- Observation overhead is measurable.
- Reports degrade correctly when Server Observability is unavailable.

## Phase 5: Distributed Execution and Recovery

**Goal: scale from single-machine execution to multiple Workers and Backends.**

Tasks:

- [ ] WM-0501: Implement the Worker Registry.
- [ ] WM-0502: Implement the Backend Registry.
- [ ] WM-0503: Implement resource capacity descriptions.
- [ ] WM-0504: Implement ShardPlan.
- [ ] WM-0505: Freeze the Worker count at Swell creation.
- [ ] WM-0506: Implement leases and heartbeats.
- [ ] WM-0507: Implement new-admission freezing on lost contact.
- [ ] WM-0508: Implement Backend generation.
- [ ] WM-0509: Implement recovery handshake and state synchronization.
- [ ] WM-0510: Implement multi-Worker Metrics aggregation.
- [ ] WM-0511: Implement cross-Worker HDR merging.
- [ ] WM-0512: Implement Controller restart recovery.
- [ ] WM-0513: Implement Worker fault injection.
- [ ] WM-0514: Implement multi-Backend mixed Swells.
- [ ] WM-0515: Implement distributed 24h soak.

Deliverables:

- Multi-Worker Swells.
- Failure recovery mechanisms.
- Mixed Native + KBEngine execution reports.

Exit criteria:

- Shard plans do not drift silently.
- Lease invalidation does not create extra disconnection spikes.
- Old generation events do not pollute new state.
- Cross-Worker metric merging passes conformance tests.

## Phase 6: Dashboard and Extension Ecosystem

**Started only when the v1 product form requires it.**

Tasks:

- [ ] WM-0601: Implement Swell list and detail views.
- [ ] WM-0602: Implement Backend / Worker status pages.
- [ ] WM-0603: Implement Spectrum editing and validation display.
- [ ] WM-0604: Implement the load timeline.
- [ ] WM-0605: Implement Generator / Server metric curves.
- [ ] WM-0606: Implement report comparison.
- [ ] WM-0607: Implement users and permissions.
- [ ] WM-0608: Implement audit logs.
- [ ] WM-0609: Evaluate Docker / Podman launchers.
- [ ] WM-0610: Evaluate a Lua Behavior SDK.
- [ ] WM-0611: Evaluate Go / C++ / Python third-party SDK completeness.

# v1 Scope

## v1 Must Have

- Rust Controller.
- Rust Native Worker.
- Rust Supervisor.
- Protobuf contracts.
- Spectrum / Swell / ShardPlan.
- KBEngine Bridge MVP.
- Generator Observability.
- Basic Basin model.
- Prometheus Provider.
- Parquet / JSON report data.
- CLI.
- conformance / integration / soak tests.

## v1 Optional

- Dashboard.
- PostgreSQL.
- Docker / Podman.
- Python general behavior plugin SDK.
- BigWorld Adapter (depending on SDK and test environment).

## v1 Will Not Include

- Kubernetes orchestration.
- Plugin marketplace.
- Lua sandbox.
- General dynamic-library ABI plugins.
- One container per bot.
- A full log and trace storage platform.
- Promises of automatic root-cause diagnosis.
- Unlimited embedding of arbitrary language runtimes.
