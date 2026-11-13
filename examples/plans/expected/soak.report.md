# Test plan report: soak

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
| load | inject_load | 0 | duration_secs=1800.0, run_id=stub-run-1, sha256=fc2808d8ada9d90eb77d57b4b667530f956707b33eb0707f41d0343c7ba17f17, spectrum=examples/plans/../spectra/demo-ramp.yaml |
| warmup | hold | 60000 | secs=60.0 |
| early | capture | 0 | captured_at_unix_ms=1794591578469, label=early |
| dwell | hold | 1580000 | secs=1580.0 |
| late | capture | 0 | captured_at_unix_ms=1794593158469, label=late |
| no-leak | assert | 0 | metric=memory_growth_mb_per_hour, observed=1.2000000000000135, reason=memory growth 1.20 MB/h within bounds, verdict=pass |
| rest | cooldown | 30000 | secs=30.0 |
| write | report | 0 | file=/tmp/demo/soak/report-soak.md, stage=soak |

## Assertions

| step | metric | bound | observed | verdict | reason |
|---|---|---|---|---|---|
| no-leak | memory_growth_mb_per_hour | max 15 | 1.2000000000000135 | PASS | memory growth 1.20 MB/h within bounds |

## Snapshots

| step | level | p99 (ms) | error rate | throughput | active | memory (MB) |
|---|---|---|---|---|---|---|
| early | — | 43 | 0 | 1.3492828476735632 | 5 | 121.77 |
| late | — | 43 | 0 | 20 | 5 | 122.29666666666667 |

## Honesty notes

- Capture source: the in-process stub simulator. Metrics are synthetic by construction (documented formulas); they exercise the plan/assert pipeline, not a server.
- KBEngine live load awaits the Bridge (docs/kbengine.md is at design-doc stage); this run used the native orchestration face only.

Artifacts: `/tmp/demo/soak` (plan.toml, events.jsonl, metrics.json, asserts.json, run.json, report.md)
