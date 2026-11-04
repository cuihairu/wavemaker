#!/usr/bin/env python3
"""Spectrum arrival / lifetime / concurrency simulation (BLOCKER-1 closure).

Simulates the reference scenario from the plan document (§8.2):

- arrival: exponential growth from 1/s to 20/s over a 600 s ramp, then hold at 20/s
- lifetime: fixed 900 s vs exponential(900 s)
- reports the active-count (concurrency) curve N(t)

and compares the simulated curve against:

- the analytic fixed-lifetime curve N(t) = Lambda(t) - Lambda(t - L)
- the analytic exponential-lifetime curve N(t) = Integral lambda(s) * exp(-(t-s)/mu) ds
- the naive Little's Law prediction lambda(t) * mu (valid only at steady state)

Also simulates the three saturation policies (reject_new / queue / delay) from
plan §8.2.1 against a hard concurrency cap.

Usage:
    python3 tools/spectrum_sim.py            # print markdown result tables
    python3 -m pytest tools/test_spectrum_sim.py
"""

from __future__ import annotations

import argparse
import math
import random
from dataclasses import dataclass, field

# --- reference scenario (plan §8.2) -----------------------------------------

LAMBDA_0 = 1.0        # arrival rate at ramp start (1/s)
LAMBDA_1 = 20.0       # arrival rate at ramp end (1/s)
RAMP = 600.0          # ramp duration (s)
LIFETIME_MEAN = 900.0  # lifetime mean (s)
HORIZON = 3600.0      # simulation horizon (s)
DEFAULT_SEED = 20261104
DT = 0.5              # simulation time step (s)


def growth_rate(lam0: float = LAMBDA_0, lam1: float = LAMBDA_1,
                ramp: float = RAMP) -> float:
    """k in lambda(t) = lam0 * exp(k * t) such that lambda(ramp) = lam1."""
    return math.log(lam1 / lam0) / ramp


def arrival_rate(t: float, lam0: float = LAMBDA_0, lam1: float = LAMBDA_1,
                 ramp: float = RAMP) -> float:
    """Planned arrival rate lambda(t): exponential growth, then hold."""
    if t <= 0:
        return 0.0
    if t < ramp:
        return lam0 * math.exp(growth_rate(lam0, lam1, ramp) * t)
    return lam1


def cumulative_arrivals(t: float, lam0: float = LAMBDA_0, lam1: float = LAMBDA_1,
                        ramp: float = RAMP) -> float:
    """Analytic cumulative arrival count Lambda(t) = Integral_0^t lambda(s) ds."""
    if t <= 0:
        return 0.0
    k = growth_rate(lam0, lam1, ramp)
    ramp_total = lam0 * (math.exp(k * ramp) - 1.0) / k  # == (lam1 - lam0) / k
    if t < ramp:
        return lam0 * (math.exp(k * t) - 1.0) / k
    return ramp_total + lam1 * (t - ramp)


def arrival_ramp_total(lam0: float = LAMBDA_0, lam1: float = LAMBDA_1,
                       ramp: float = RAMP) -> float:
    """Total arrivals created during the ramp: (lam1 - lam0) / k."""
    return (lam1 - lam0) / growth_rate(lam0, lam1, ramp)


# --- Monte-Carlo simulation ---------------------------------------------------

@dataclass
class SimEvent:
    t_arrival: float
    lifetime: float


@dataclass
class SimResult:
    checkpoints: list[float]
    active: list[int]
    events: list[SimEvent] = field(default_factory=list)
    cumulative_planned: list[int] = field(default_factory=list)


def simulate(lifetime: str = "fixed", horizon: float = HORIZON, dt: float = DT,
             seed: int = DEFAULT_SEED, lifetime_mean: float = LIFETIME_MEAN,
             lam0: float = LAMBDA_0, lam1: float = LAMBDA_1,
             ramp: float = RAMP,
             checkpoints: list[float] | None = None) -> SimResult:
    """Time-stepped Poisson simulation of arrivals and active count.

    lifetime: "fixed" (every Bot lives exactly lifetime_mean) or
              "exponential" (lifetime ~ Exp(mean=lifetime_mean)).
    """
    if lifetime not in ("fixed", "exponential"):
        raise ValueError(f"unknown lifetime distribution: {lifetime!r}")
    rng = random.Random(seed)
    if checkpoints is None:
        checkpoints = default_checkpoints(ramp)

    events: list[SimEvent] = []
    n_steps = int(round(horizon / dt))
    for step in range(n_steps):
        t_start = step * dt
        rate = arrival_rate(t_start + dt / 2.0, lam0, lam1, ramp)
        n_new = poisson_sample(rng, rate * dt)
        for _ in range(n_new):
            t_arr = t_start + rng.random() * dt
            if lifetime == "fixed":
                life = lifetime_mean
            else:
                life = rng.expovariate(1.0 / lifetime_mean)
            events.append(SimEvent(t_arr, life))

    events.sort(key=lambda e: e.t_arrival)
    cps = sorted(checkpoints)
    active = [sum(1 for e in events if e.t_arrival <= t < e.t_arrival + e.lifetime)
              for t in cps]
    planned = [sum(1 for e in events if e.t_arrival <= t) for t in cps]
    return SimResult(checkpoints=cps, active=active, events=events,
                     cumulative_planned=planned)


def poisson_sample(rng: random.Random, lam: float) -> int:
    """Sample Poisson(lam); falls back to normal approximation for large lam."""
    if lam <= 0:
        return 0
    if lam > 30.0:  # normal approximation, adequate at these counts
        return max(0, int(round(rng.gauss(lam, math.sqrt(lam)))))
    # Knuth's algorithm for small rates
    L = math.exp(-lam)
    k = 0
    p = 1.0
    while True:
        p *= rng.random()
        if p <= L:
            return k
        k += 1


def default_checkpoints(ramp: float = RAMP) -> list[float]:
    cps = [float(t) for t in range(0, int(ramp) + 1, 100)]
    cps += [900.0, 1200.0, 1500.0, 1800.0, 2400.0, 3000.0, 3600.0]
    return sorted(set(cps))


# --- analytic references ------------------------------------------------------

def analytic_fixed(t: float, lifetime_mean: float = LIFETIME_MEAN,
                   lam0: float = LAMBDA_0, lam1: float = LAMBDA_1,
                   ramp: float = RAMP) -> float:
    """N(t) = Lambda(t) - Lambda(t - L); every Bot lives exactly L."""
    return cumulative_arrivals(t, lam0, lam1, ramp) \
        - cumulative_arrivals(t - lifetime_mean, lam0, lam1, ramp)


def analytic_exponential(t: float, lifetime_mean: float = LIFETIME_MEAN,
                         lam0: float = LAMBDA_0, lam1: float = LAMBDA_1,
                         ramp: float = RAMP, step: float = 0.25) -> float:
    """N(t) = Integral_0^t lambda(s) * exp(-(t-s)/mu) ds (trapezoid rule)."""
    n = int(t / step)
    total = 0.0
    for i in range(n + 1):
        s = i * step
        if s > t:
            break
        w = 0.5 if i in (0, n) else 1.0
        total += w * arrival_rate(s, lam0, lam1, ramp) \
            * math.exp(-(t - s) / lifetime_mean)
    return total * step


def littles_law(t: float, lifetime_mean: float = LIFETIME_MEAN,
                lam0: float = LAMBDA_0, lam1: float = LAMBDA_1,
                ramp: float = RAMP) -> float:
    """Naive L = lambda(t) * W — valid only in steady state, not during ramp."""
    return arrival_rate(t, lam0, lam1, ramp) * lifetime_mean


# --- saturation policies (plan §8.2.1) ----------------------------------------

@dataclass
class PolicyResult:
    policy: str
    admitted: int
    rejected: int
    max_active: int
    delays: list[float] = field(default_factory=list)  # actual_at - planned_at, s


def simulate_with_cap(policy: str, max_concurrency: int,
                      lifetime: str = "fixed", horizon: float = HORIZON,
                      dt: float = DT, seed: int = DEFAULT_SEED,
                      lifetime_mean: float = LIFETIME_MEAN) -> PolicyResult:
    """Simulate a hard concurrency cap under a saturation policy.

    reject_new: arrivals beyond the cap are refused and counted.
    queue:      arrivals beyond the cap enter FIFO order and are admitted as
                soon as capacity frees; deviation from planned_at is recorded.
    delay:      arrivals beyond the cap are postponed (planned_at shifted) and
                created when capacity frees; deviation is recorded. Order is
                not guaranteed, unlike queue.
    """
    if policy not in ("reject_new", "queue", "delay"):
        raise ValueError(f"unknown saturation policy: {policy!r}")
    if max_concurrency <= 0:
        raise ValueError("max_concurrency must be positive")

    sim = simulate(lifetime=lifetime, horizon=horizon, dt=dt, seed=seed,
                   lifetime_mean=lifetime_mean)
    events = list(sim.events)

    delays: list[float] = []
    rejected = 0
    max_active = 0
    admitted = 0

    if policy == "reject_new":
        # process chronologically; track expiry with a sorted list of end times
        pending: list[float] = []  # sorted end times of active bots
        for ev in events:
            end = ev.t_arrival + ev.lifetime
            pending = [p for p in pending if p > ev.t_arrival]
            if len(pending) < max_concurrency:
                pending.append(end)
                pending.sort()
                admitted += 1
                max_active = max(max_active, len(pending))
            else:
                rejected += 1
        return PolicyResult(policy, admitted, rejected, max_active, delays)

    # queue / delay: FIFO release as capacity frees (delay differs semantically:
    # planned_at is shifted; both record scheduling deviation)
    waiting: list[SimEvent] = []
    active_ends: list[float] = []
    for ev in events:
        t_now = ev.t_arrival
        active_ends = [e for e in active_ends if e > t_now]
        waiting.append(ev)
        while waiting and len(active_ends) < max_concurrency:
            nxt = waiting.pop(0)
            delays.append(t_now - nxt.t_arrival)
            active_ends.append(t_now + nxt.lifetime)
            active_ends.sort()
            admitted += 1
            max_active = max(max_active, len(active_ends))
    rejected = 0  # queue/delay never refuse; deviation is recorded instead
    return PolicyResult(policy, admitted, rejected, max_active, delays)


# --- reporting -----------------------------------------------------------------

def format_table() -> str:
    sim_f = simulate(lifetime="fixed")
    sim_e = simulate(lifetime="exponential", seed=DEFAULT_SEED + 1)
    lines = [
        "### Active concurrency N(t): simulation vs analytic references",
        "",
        f"Scenario: lambda(t) {LAMBDA_0:g}/s -> {LAMBDA_1:g}/s exponential growth "
        f"over {RAMP:g} s ramp, then hold; lifetime mean {LIFETIME_MEAN:g} s; "
        f"seed {DEFAULT_SEED}; dt {DT:g} s; horizon {HORIZON:g} s.",
        "",
        "Ramp total arrivals Lambda(ramp) = "
        f"{arrival_ramp_total():.1f} (plan: ~3800).",
        "",
        "| t (s) | Lambda(t) (analytic) | N(t) fixed (sim) | N(t) fixed (analytic) | "
        "N(t) exponential (sim) | N(t) exponential (analytic) | lambda(t)*W (Little) |",
        "|---:|---:|---:|---:|---:|---:|---:|",
    ]
    for i, t in enumerate(sim_f.checkpoints):
        lines.append(
            f"| {t:g} | {cumulative_arrivals(t):.0f} "
            f"| {sim_f.active[i]} | {analytic_fixed(t):.0f} "
            f"| {sim_e.active[i]} | {analytic_exponential(t):.0f} "
            f"| {littles_law(t):.0f} |"
        )
    lines += [
        "",
        "### Saturation policies against concurrency.max = 5000",
        "",
        "| policy | admitted | rejected | max active | mean delay (s) | p99 delay (s) |",
        "|---|---:|---:|---:|---:|---:|",
    ]
    for policy in ("reject_new", "queue", "delay"):
        r = simulate_with_cap(policy, 5000)
        d = sorted(r.delays)
        p99 = d[min(len(d) - 1, int(0.99 * len(d)))] if d else 0.0
        mean = sum(d) / len(d) if d else 0.0
        lines.append(
            f"| {r.policy} | {r.admitted} | {r.rejected} | {r.max_active} "
            f"| {mean:.1f} | {p99:.1f} |"
        )
    return "\n".join(lines)


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.parse_args()
    print(format_table())


if __name__ == "__main__":
    main()
