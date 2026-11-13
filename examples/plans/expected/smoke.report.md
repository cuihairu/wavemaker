# Test plan report: smoke

- game: demo
- engine: stub
- backend: `stub`
- **simulated**: every metric below is synthesized by the in-process stub (deterministic formulas over wm-runtime's arrival curve); nothing observed a real server.
- time_scale: 0 (real sleeps compressed; simulated durations are full)
- verdict: **passed**

## Loads

| step | run id | spectrum | sha256 | pin | stopped |
|---|---|---|---|---|---|
| load | stub-run-1 | examples/plans/../spectra/demo-ramp.yaml | `fc2808d8ada9` | — | yes |

## Steps

| step | action | duration (ms) | detail |
|---|---|---|---|
| probe | prepare | 0 | target=http://127.0.0.1:8080/healthz |
| load | inject_load | 0 | duration_secs=300.0, run_id=stub-run-1, sha256=fc2808d8ada9d90eb77d57b4b667530f956707b33eb0707f41d0343c7ba17f17, spectrum=examples/plans/../spectra/demo-ramp.yaml |
| settle | hold | 60000 | secs=60.0 |
| steady | capture | 0 | captured_at_unix_ms=1794591578442, label=steady |
| no-errors | assert | 0 | metric=error_rate, observed=0.0, reason=all 1 sample(s) within bounds, verdict=pass |
| responsive | assert | 0 | metric=latency_p99_ms, observed=40.6, reason=all 1 sample(s) within bounds, verdict=pass |
| rest | cooldown | 30000 | secs=30.0 |
| write | report | 0 | file=/tmp/demo/smoke/report-smoke.md, stage=smoke |

## Assertions

| step | metric | bound | observed | verdict | reason |
|---|---|---|---|---|---|
| no-errors | error_rate | max 0 | 0 | PASS | all 1 sample(s) within bounds |
| responsive | latency_p99_ms | max 250 | 40.6 | PASS | all 1 sample(s) within bounds |

## Snapshots

| step | level | p99 (ms) | error rate | throughput | active | memory (MB) |
|---|---|---|---|---|---|---|
| steady | — | 40.6 | 0 | 1.3492828476735632 | 1 | 120.36999999999999 |

## Honesty notes

- Capture source: the in-process stub simulator. Metrics are synthetic by construction (documented formulas); they exercise the plan/assert pipeline, not a server.
- KBEngine live load awaits the Bridge (docs/kbengine.md is at design-doc stage); this run used the native orchestration face only.

Artifacts: `/tmp/demo/smoke` (plan.toml, events.jsonl, metrics.json, asserts.json, run.json, report.md)
