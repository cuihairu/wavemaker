---
title: Spectrum & Plans
---

# Spectrum and Deterministic Plans

## Spectrum Structure

Spectrum should express these separately:

- Arrival Model: the bot arrival process.
- Concurrency Constraint: concurrency target or upper bound.
- Lifetime Model: the bot lifetime distribution.
- Behavior Model: the behavior mix.
- Workload Model: request/message and business load characteristics.
- Fault Model: fault injection policy (if enabled).

Do not collapse arrival rate, concurrency count, and bot lifetime into one vague `users` field.

## The arrival / concurrency Blocking Issue

The current ADR contains this example:

- arrival grows exponentially from 1/s to 20/s.
- ramp of 600s.
- lifetime mean of 900s.
- concurrency target of 5000.

The exponential arrival integral is about 3800, which is the total number of creations within 600 seconds; under a fixed 900-second lifetime, those bots have not expired by 600s. If lifetime is exponentially distributed, the active count at 600s would be lower.

Therefore a precise concurrency curve cannot be derived from the lifetime mean alone. In long-run steady state, Little's Law can serve as a reference:

\[
L = \lambda W
\]

but it must not be applied unconditionally to short ramps.

### Suggested v1 Semantics

Distinguish explicitly:

- `arrival.target_rate`: the target arrival rate.
- `concurrency.target`: the concurrency target to reach (optional).
- `concurrency.max`: the hard concurrency cap (optional).
- `lifetime.distribution`: the lifetime probability distribution.
- `saturation_policy`: what to do when `max` is reached.

Suggested saturation policies:

- `reject_new`: stop creating; count as rejected arrivals.
- `queue`: enqueue; actual arrival times deviate from the plan and must be recorded.
- `delay`: postpone creation until capacity is available; record scheduling deviation.

### Validation Rules

- When arrival and target concurrency coexist, do not reject unconditionally.
- Simulate or analytically estimate the active-count curve from the lifetime distribution.
- If the target diverges from the prediction beyond a threshold, emit a configuration warning or ask the user to pick a priority.
- If `max_concurrency` is below the capacity the planned load requires, reject explicitly or apply the saturation policy.
- All validation output must include interpretable numbers and assumptions.

**BLOCKER-1 must be closed in Phase 0, with an executable test.**

## Plan-as-data

The Controller is the only plan generator.

Plan data includes at least:

```text
PlanEntry
  actor_or_bot_id
  backend_id
  shard_id
  planned_at
  lifetime
  behavior_group
  seed
  attributes
```

If an Engine Bot Backend cannot consume per-bot plans, the Controller converts the plan into the batch plan the backend can express, while keeping both the original and converted information.

Workers or Bridges must not recompute the global arrival plan from `(seed, lambda, W, i)`.

## ShardPlan Freezing

- The Worker count W and the ShardPlan are frozen when the Swell is created.
- Once the plan is fixed, record the plan_digest.
- Shard rules must not silently change mid-run because the temporary Worker count changed.
- Failure recovery has explicit reassignment and deduplication policies.
- Arrival sequence sharding must be verified by a global conformance test.
- The ADR's original time-rotation scheme (`i mod W`) remains a decision to be verified; it must not be treated as proven optimal before testing.

## Scope of Determinism

Determinism has three levels:

1. Plan Determinism: the same input yields the same plan.
2. Runtime Determinism: given the plan and event sequence, runtime state changes can be replayed.
3. Behavior Determinism: behavior code produces the same actions for the same input.

Cross-language plugins cannot guarantee the third level by default. Every plugin must declare its determinism level and external side effects.
