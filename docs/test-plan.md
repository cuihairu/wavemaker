---
title: Test Plans
---

# Test Plans

A test plan is a declarative description of one load-verification campaign:
an ordered list of steps (inject load, hold, capture, assert) that a runner
executes end to end, judging every assertion and writing a diffable run
directory with a Markdown report.

Plans are TOML, validated by the same explainable-violation style as
Spectrum configs. The model lives in `crates/wm-core/src/testplan.rs` (pure
model, zero IO); the executor is `wm test run` (`crates/wm-cli/src/runner.rs`).

## Quickstart (stub backend)

```bash
cargo build -p wm-cli
./examples/run_demo.sh                          # all three demo plans
wm test run examples/plans/smoke.toml --time-scale 0   # one plan, instant
```

Without a real game server the default backend is the in-process **stub**
simulator: the load curve comes from the plan's real spectrum file (via
wm-runtime's `ArrivalPlan` — the same arrival math the native runtime
executes), but the metrics are synthetic formulas and every report labels
itself `simulated`. `--time-scale 0` compresses the real sleeps to nothing
while simulated time advances by the full amount; `1.0` is real time.

Three demo plans ship under `examples/plans/`:

| Plan | Shape | Assertions |
|---|---|---|
| `smoke.toml` | 1 bot, 5 min | probe reachable, `error_rate = 0`, p99 < 250 ms |
| `baseline.toml` | staircase 10 → 50 → 100, 2 min per level | worst-level p99 < 250 ms, zero errors |
| `soak.toml` | 5 bots, 30 min | memory growth < 15 MB/h (trend over two snapshots) |

Expected reports (real stub runs) sit next to them under
`examples/plans/expected/`.

## Plan anatomy

```toml
[meta]
name = "smoke"          # required; also names the run directory
game = "demo"           # optional
engine = "stub"         # optional

[[steps]]
id = "load"             # required, unique within the plan
action = "inject_load"
[steps.params]
spectrum = "../spectra/demo-ramp.yaml"
concurrency = 1         # optional override of the spectrum's concurrency
duration_secs = 300     # observation budget (see below)
expect_sha256 = "…"     # optional: pin the spectrum content hash
```

Unknown parameter keys are parse errors; known keys on the wrong action are
`E_PARAM_NOT_APPLICABLE` violations. Relative `spectrum` paths anchor at the
plan file's directory, so a plan runs identically from any working
directory. Top-level keys (`continue_on_assert_fail`) must appear before
`[meta]` — everything after a table header belongs to that table.

### Actions

| Action | Parameters | Effect |
|---|---|---|
| `prepare` | `target`, `timeout_secs`, `warmup_secs` | Probe the target's reachability; aborts the run when unreachable. |
| `inject_load` | `spectrum`, `concurrency`, `swell_id`, `expect_sha256`, `duration_secs` | Start a load from a spectrum file. Stops any previously injected load (single active load per plan). |
| `hold` / `cooldown` | `secs` | Wait (observation window / settling gap). |
| `step_load` | `spectrum`, `levels`, `step_secs`, `capture_per_level` | Staircase: for each level — release, inject at that concurrency, wait, capture (tagged with the level index). |
| `capture` | `label`, `source` | Take one metrics snapshot. |
| `assert` | `metric`, `max` and/or `min`, `source` | Judge the matched snapshots (below). |
| `report` | `stage` | Write an interim `report-<stage>.md` into the run directory. |

### The `inject_load` duration budget

`duration_secs` is not an in-step sleep. It is an observation budget that
the subsequent `hold`/`cooldown` steps consume; when the budget is
exhausted the load is released mid-plan (the event timeline records
`duration budget exhausted`). This keeps the core verification flow —
inject → hold → capture → assert — inside the load's lifetime.

## Assertions

Five metrics:

| Metric | Bound direction | Notes |
|---|---|---|
| `latency_p99_ms` | `max` (cap) | Tail latency. |
| `error_rate` | `max` (cap) | Fraction 0–1. |
| `throughput` | `min` (floor) | Minimum acceptable throughput. |
| `disconnect_recovery_secs` | `max` (cap) | Time from disconnect to recovery. |
| `memory_growth_mb_per_hour` | `max` (cap) | Leak trend: least-squares slope (MB/h) over the matched snapshots' memory series; needs ≥ 2 points. |

An assertion matches snapshots produced by the step named in `source`;
omitted, it matches all snapshots. A single-valued metric fails if *any*
matched snapshot violates the bound (`observed` reports the worst value).

A missing data source is never a PASS: no matched snapshots, or the metric
absent from them, judge **NotApplicable** and the plan carries the gap
openly. The stub, for instance, has no disconnects, so a
`disconnect_recovery_secs` assertion there judges NA.

`continue_on_assert_fail = true` (plan level) lets the runner proceed to
the remaining steps after a FAIL; the default is fail-fast. It affects
sequencing only — verdicts are unaffected, and any FAIL makes the overall
verdict `failed` (CLI exit code 4).

## Backends

| Backend | Flag | Metrics | Notes |
|---|---|---|---|
| `stub` | default | Synthetic, deterministic | In-process simulator; `--time-scale` compresses real sleeps (0–1). Every report carries the simulated banner. |
| `controller` | `--backend controller` | Requires `--basin-url` | Drives a real controller via its REST API (run/stop). Without `--basin-url` the capture step errors naming WM-0412/WM-0413 — the controller carries no metrics yet, and the runner does not fabricate them. |

## Run directory

Each run writes one directory (`wm-runs/<name>-<timestamp>`, or `--out`):

| File | Content |
|---|---|
| `plan.toml` | The plan verbatim. |
| `events.jsonl` | One JSON object per event (step started/finished, load injected/stopped, assert judged, …). |
| `metrics.json` | All captured snapshots. |
| `asserts.json` | Per-assertion verdicts with observed values and reasons. |
| `run.json` | The structured outcome (backend, `time_scale`, plan sha256, spectrum hashes, verdict). |
| `report.md` | The rendered report (plus interim `report-<stage>.md` files). |

The plan content hash and every injected spectrum's sha256 are recorded, so
two runs of the same plan are diffable line by line; `expect_sha256` pins a
spectrum so a plan refuses to run against silently changed load content.

## MCP

The same engine is exposed to AI agents: `run_test_plan` executes a plan
(a `failed` verdict comes back as a success result carrying the reason) and
`get_test_report` reads a finished run back — see
[crates/wm-mcp/README.md](https://github.com/cuihairu/wavemaker/blob/main/crates/wm-mcp/README.md).

## Honest boundaries

- Stub metrics are simulation artifacts: deterministic formulas over the
  simulated load state. They exercise the plan/assert pipeline, not a
  server.
- Live metrics await the engine-side aggregation work (WM-0412 / WM-0413);
  the controller backend captures from an explicit `--basin-url` endpoint
  only, and errors loudly when none is configured.
- KBEngine live load awaits the Bridge execution path
  ([KBEngine Adapter](/kbengine)); today's demos exercise the native
  orchestration face.
