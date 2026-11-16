---
title: MCP Server
---

# MCP Server

Wavemaker ships an MCP server (`wm-mcp`, built on the official
[rmcp](https://crates.io/crates/rmcp) SDK) that exposes the controller REST
surface, the spectrum engine, and the test-plan engine to AI agents over
stdio — the same chain an MCP proxy or IDE uses.

```bash
cargo run -p wm-mcp
```

Point an MCP client at the binary:

```json
{ "command": "wm-mcp", "args": [] }
```

## Tools

Every tool delegates to an existing engine face — the controller REST API,
the `wm` CLI's commands, or the runtime's own math. Nothing is re-modeled,
so the tools cannot drift from what the engine executes.

| Tool | Source | Notes |
|---|---|---|
| `list_runs` | controller REST | Forwards `GET /api/v1/runs/{run_id}`, or lists every run from `GET /api/v1/status` (the controller has no dedicated list endpoint). The controller JSON is forwarded, not re-modeled. |
| `query_status` | controller REST | Forwards `GET /api/v1/status` verbatim. |
| `validate_spectrum` | `wm-cli::commands::validate` | The exact engine behind `wm validate` (`wm-core` `SpectrumConfig::validate`), including the CLI's violation formatting. Error-severity issues come back as a tool-level error carrying the report; warnings still validate. |
| `load_timeline` | `wm-runtime` `ArrivalPlan` | Same math as the desktop timeline chart (`rate_at` / `cumulative`) — the curve cannot drift from what the runtime executes. |
| `run_scenario` | `wm-cli::commands::run` / `stop` | Library call into the CLI's subcommand face — the same code `wm run` / `wm stop` execute, with typed errors instead of stdout parsing. `start` validates the spectrum first. |
| `run_test_plan` | `wm-cli::plan` + `wm-cli::runner` | The `wm test run` engine: executes a declarative test-plan TOML end to end (steps, assertions, run directory with `report.md`). A `failed` verdict is a success result carrying the reason. |
| `get_test_report` | run directory files | Reads a finished plan run back: `run.json` (structured outcome) plus `report.md` (rendered view). Without `run_dir`, returns the latest run under `wm-runs`. |

The stdio e2e test asserts `tools/list` returns **exactly these seven** —
no phantom tools (see honest boundaries below).

## Resources

Tools carry state, not knowledge. Configuration and glossary questions —
"what does `arrival.ramp` mean", "how do I configure a soak test" — are
served through MCP **resources** so an agent reads the authoritative page
instead of answering from memory:

| URI | Page |
|---|---|
| `wm://docs/configuration` / `wm://docs/configuration.zh` | configuration reference (en / zh-CN) |
| `wm://docs/glossary` | domain glossary (bilingual by its own convention) |
| `wm://docs/spectrum` / `wm://docs/spectrum.zh` | Spectrum & deterministic plans (en / zh-CN) |
| `wm://docs/test-plan` / `wm://docs/test-plan.zh` | test-plan structure, actions, assertions (en / zh-CN) |

All resources are `text/markdown`. Pages are embedded at **build time**
(`include_str!`): a binary carries the docs snapshot of the commit it was
built from, never a live read of the repository.

## Configuration

- **Controller address**: defaults to `http://127.0.0.1:7600`; override per
  tool call or via the `WM_CONTROLLER_URL` environment variable — the same
  env var and default the `wm` CLI uses.
- **Test-plan runs**: `backend=stub` (default) executes the in-process
  simulator; `backend=controller` drives a real controller and requires
  `basin_url` for captures.

## Honest boundaries

- **No metrics or report tools for live runs.** The engine side has no data
  source for them yet (WM-0412 / WM-0413 / WM-0416 are unimplemented), so
  corresponding tools would fabricate data. They appear here when the data
  does. `run_test_plan` on the controller backend therefore errors without
  `basin_url` instead of inventing numbers; on the stub backend everything
  is a labeled simulation.
- **Failures are errors**: tool failures return `isError` with the named
  reason — never a silent fake success. Unknown resource URIs return the
  MCP `RESOURCE_NOT_FOUND` error, never fabricated content.
- **stdout discipline**: stdout carries JSON-RPC frames only; diagnostics
  (e.g. violation reports printed by `run_scenario`) go to stderr.
