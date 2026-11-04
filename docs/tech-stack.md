---
title: Tech Stack
---

# Tech Stack Decisions

## Language Choices

| Domain | Technology | Decision |
|---|---|---|
| Controller / Web API | Rust + Axum + Tokio | Primary choice |
| Native Worker | Rust + Tokio | Primary choice |
| Supervisor | Rust + Tokio | Primary choice |
| Core protocol | Protobuf | Unified contract |
| KBEngine Bridge | Python | Required by the engine scripting environment |
| BigWorld Bridge | Depends on the actual SDK / bot environment | To be verified |
| Python SDK | Python | Provided later |
| Lua SDK | Lua | Not implemented in v1 |
| C++ Adapter | Separate process or isolated shim | As needed |
| Go SDK | Go | Provided later, outside the core |
| Report analysis | Rust in the main pipeline; Python may be used as an offline analysis tool | Does not affect the core |

## Rust Ecosystem Suggestions

| Purpose | Suggested Component | Notes |
|---|---|---|
| Async Runtime | Tokio | Core async runtime |
| HTTP | Axum | Controller API |
| RPC | Tonic | gRPC control channel |
| Serialization | prost / serde | Protobuf and configuration |
| Database | SQLx | Metadata storage for the resident Controller |
| CLI | Clap | CLI |
| Logging / Tracing | tracing | Structured logging |
| Metrics | OpenTelemetry Rust + custom aggregation | Metric export |
| Histogram | HDR Histogram compatible implementation | Unified parameters and blob versions |
| Parquet | parquet-rs / Arrow | Version and performance need verification |
| Report Query | DuckDB | Can serve as an external analysis engine; embedding is not required |

Dependency versions should be frozen after Phase 0 establishes a version matrix and a minimal runnable prototype. Do not write unverified crate versions into the plan document ahead of time.

## Is a Database Required?

- CLI-only single Swell runs: the file system, Parquet, JSON, and DuckDB are enough; no resident database is needed.
- Resident Dashboard, multiple users, concurrent management of multiple Swells: persistent Controller metadata is needed; PostgreSQL is recommended.
- Single-machine development mode: SQLite is an option.
- Test detail records and high-frequency time series should not all be written to PostgreSQL; prefer file partitioning and Parquet.

The Controller's metadata storage and experiment data storage must be separated.
