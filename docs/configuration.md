---
title: Configuration
---

# Configuration Design Draft

The following shows configuration layering only; fields are not finally frozen.

The implemented Spectrum schema is `crates/wm-core/src/spectrum.rs` — its
keys are `arrival.ramp.{from,to}_per_second` / `duration_secs`,
`lifetime.{distribution,mean_secs}`, `concurrency.{target,max}` and
`saturation_policy`, and the authoritative user-facing examples are in
[test-plan](./test-plan.md). The draft below intentionally lags that shape.

## Swell

```yaml
api_version: wavemaker/v1

swell:
  name: staging-login-capacity
  seed: 20261104
  duration: 20m

spectrum:
  arrival:
    model: ramp
    from_per_second: 1
    to_per_second: 20
    duration: 10m

  concurrency:
    max: 5000
    saturation_policy: reject_new

  lifetime:
    distribution: fixed
    duration: 15m

  behavior:
    groups:
      - name: normal-player
        ratio: 1.0
        scenario: login-and-idle

execution:
  backends:
    - id: rust-worker-a
      type: native
      replicas: 2

    - id: kbengine-bots
      type: engine_bot
      adapter: kbengine
      replicas: 1

basin:
  ref: staging
  observability:
    required: true
    provider: prometheus
    collection_interval: 1s

report:
  format:
    - html
    - parquet
```

Note: this example only demonstrates the configuration structure. The semantics of `arrival` and `concurrency.max` need to be frozen after BLOCKER-1 is closed.

## Engine Bot Backend

```yaml
backend:
  id: kbengine-bots
  type: engine_bot
  adapter: kbengine

process:
  executable: /opt/kbengine/bin/bots
  working_directory: /opt/kbengine
  restart_policy: on-failure

bridge:
  transport: unix_domain_socket
  endpoint: /run/wavemaker/kbengine-bots.sock
  protocol_version: 1

engine:
  config_path: /opt/kbengine/assets
  bot_script_path: /opt/kbengine/assets/scripts/bots

capability_policy:
  require:
    - process.start
    - process.stop
    - bots.create_batch
    - observability.bot_count
  allow_unknown:
    - bots.destroy_individual
    - scheduling.exact_arrival
```
