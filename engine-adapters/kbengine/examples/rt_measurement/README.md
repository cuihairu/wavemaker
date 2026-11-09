# Runtime Measurement Sample (WM-0010, RT1/RT4)

Measures, on a real KBEngine cluster, the two quantities WM-0010 asks for —
with the production Bridge running inside a real bots process:

- **RT1 — callback scheduling granularity**: intervals between consecutive
  pump fires. Each fire is the engine executing a `KBEngine.callback` armed
  for +100 ms (the audit §3 quantum: 1/gameTick). The interval distribution
  is the granularity the engine actually delivers.
- **RT4 — Bridge queue latency**: per-command time from `enqueue` (IPC
  receive thread) to `drain` (scripting-thread pump) — the IPC → scripting
  handoff, bounded by design at one pump quantum.

The measurement wrappers observe the same objects production code uses
(a `MeasuredQueue` injected through the production constructor, the pump
tick wrapped); nothing about Bridge behaviour changes.

## How it runs

`run_rt.sh` reuses the login-load machinery (see examples/login_load): a
DB-backed cluster, the bridge installed into the assets tree, and the
harness installed AS the bots entry module (the engine never reaches the
script's `onFinish` on SIGTERM, verified on this build — so the report is
written deterministically by the pump tick that observes the session
teardown). The driver plays the Supervisor: a 3-bot login swell plus
20 commands/s through the Bridge queue for 15 s, sampling heartbeats.

## Measured reference numbers

Single machine (the build workspace), engine defaults (gameTick 100 ms),
login swell active during the window:

| Metric | Run 1 | Run 2 |
|---|---|---|
| RT1 fire interval median | 99.4 ms | 99.6 ms |
| RT1 fire interval p95 | 102.4 ms | 102.3 ms |
| RT1 fire interval max | 182.0 ms | 105.9 ms |
| RT4 queue latency median | 0.13 ms | 0.09 ms |
| RT4 queue latency p95 | 21.2 ms | 0.61 ms |
| RT4 queue latency max | 101.2 ms | 28.8 ms |
| heartbeat callback_lag_ms max | 0.69 ms | 0.69 ms |

Reading: the pump tracks the 100 ms engine quantum tightly (median ≈ 99.5
ms, p95 ≈ 102 ms) with an honest tail under engine activity (one 182 ms
fire in run 1). Queue handoff is sub-millisecond at the median and bounded
by the pump quantum at the max (a command arriving right after its tick
waits one quantum — 101 ms observed, exactly the designed bound).

## Reproduce

```sh
export WAVEMAKER_KBE_ROOT=/path/to/kbengine-workspace
sh run_rt.sh
```

The JSON report lands in the run directory and is printed at the end.
Numbers in this README are from THIS machine's runs — rerun on your
deployment for your own values; do not cite them as engine constants.
