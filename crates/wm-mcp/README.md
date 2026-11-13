# wm-mcp

Wavemaker's MCP server: exposes the controller REST API and the spectrum
engine to AI agents over stdio, built on the official
[rmcp](https://crates.io/crates/rmcp) SDK.

```bash
cargo run -p wm-mcp
```

Point an MCP client at the `wm-mcp` binary over stdio, e.g.:

```json
{ "command": "wm-mcp", "args": [] }
```

## Tools

| Tool | Source | Notes |
|---|---|---|
| `list_runs` | controller REST | Forwards `GET /api/v1/runs/{run_id}`, or lists every run from `GET /api/v1/status` (the controller has no dedicated list endpoint). The controller JSON is forwarded, not re-modeled. |
| `query_status` | controller REST | Forwards `GET /api/v1/status` verbatim. |
| `validate_spectrum` | `wm-cli::commands::validate` | The exact engine behind `wm validate` (`wm-core` `SpectrumConfig::validate`), including the CLI's violation formatting. No second implementation. |
| `load_timeline` | `wm-runtime` `ArrivalPlan` | Same math as the desktop timeline chart (`rate_at` / `cumulative`); the curve cannot drift from what the runtime executes. |
| `run_scenario` | `wm-cli::commands::run` / `stop` | Library call into the CLI's subcommand face — the same code `wm run` / `wm stop` execute, with typed errors instead of stdout parsing. `start` validates the spectrum first. |
| `run_test_plan` | `wm-cli::plan` + `wm-cli::runner` | The `wm test run` engine: executes a declarative test plan TOML end to end (steps, assertions, run directory with report.md). `backend=stub` (default) runs the in-process simulator — metrics are synthetic and the report says so; `backend=controller` drives a real controller. A `failed` verdict is a success result carrying the reason. |
| `get_test_report` | run directory files | Reads a finished plan run back: run.json (structured outcome) plus report.md (rendered view). Without `run_dir`, returns the latest run under `wm-runs`. |

## Honest boundaries

- **No metrics or report tools for live runs.** The engine side has no data
  source for them yet (WM-0412 / WM-0413 / WM-0416 are unimplemented), so
  corresponding tools would fabricate data. They appear here when the data
  does. `run_test_plan` on the controller backend therefore errors without
  `basin_url` instead of inventing numbers; on the stub backend everything
  is a labeled simulation.
- **Controller address**: defaults to `http://127.0.0.1:7600`; override per
  tool call or via the `WM_CONTROLLER_URL` environment variable (the same
  env the CLI uses).
- **Failures are errors**: tool failures return `isError` with the named
  reason — never a silent fake success.
- **stdout discipline**: stdout carries JSON-RPC frames only; diagnostics
  (e.g. violation reports printed by `run_scenario`) go to stderr.
