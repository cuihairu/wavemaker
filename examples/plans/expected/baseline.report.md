# Test plan report: baseline

- game: demo
- engine: stub
- backend: `stub`
- **simulated**: every metric below is synthesized by the in-process stub (deterministic formulas over wm-runtime's arrival curve); nothing observed a real server.
- time_scale: 0 (real sleeps compressed; simulated durations are full)
- verdict: **passed**

## Loads

| step | run id | spectrum | sha256 | pin | stopped |
|---|---|---|---|---|---|
| staircase | stub-run-1 | examples/plans/../spectra/demo-ramp.yaml | `fc2808d8ada9` | — | yes |
| staircase | stub-run-2 | examples/plans/../spectra/demo-ramp.yaml | `fc2808d8ada9` | — | yes |
| staircase | stub-run-3 | examples/plans/../spectra/demo-ramp.yaml | `fc2808d8ada9` | — | yes |

## Steps

| step | action | duration (ms) | detail |
|---|---|---|---|
| probe | prepare | 0 | target=http://127.0.0.1:8080/healthz |
| staircase | step_load | 360000 | captures=3, levels=[10.0,50.0,100.0], step_secs=120.0 |
| p99 | assert | 0 | metric=latency_p99_ms, observed=100.0, reason=all 3 sample(s) within bounds, verdict=pass |
| no-errors | assert | 0 | metric=error_rate, observed=0.0, reason=all 3 sample(s) within bounds, verdict=pass |
| rest | cooldown | 30000 | secs=30.0 |
| write | report | 0 | file=/tmp/demo/baseline/report-baseline.md, stage=baseline |

## Assertions

| step | metric | bound | observed | verdict | reason |
|---|---|---|---|---|---|
| p99 | latency_p99_ms | max 250 | 100 | PASS | all 3 sample(s) within bounds |
| no-errors | error_rate | max 0 | 0 | PASS | all 3 sample(s) within bounds |

## Snapshots

| step | level | p99 (ms) | error rate | throughput | active | memory (MB) |
|---|---|---|---|---|---|---|
| staircase | 0 | 46 | 0 | 1.8205642030260802 | 10 | 123.54 |
| staircase | 1 | 70 | 0 | 1.8205642030260802 | 50 | 137.54 |
| staircase | 2 | 100 | 0 | 1.8205642030260802 | 100 | 155.04 |

## Honesty notes

- Capture source: the in-process stub simulator. Metrics are synthetic by construction (documented formulas); they exercise the plan/assert pipeline, not a server.
- KBEngine live load awaits the Bridge (docs/kbengine.md is at design-doc stage); this run used the native orchestration face only.

Artifacts: `/tmp/demo/baseline` (plan.toml, events.jsonl, metrics.json, asserts.json, run.json, report.md)
