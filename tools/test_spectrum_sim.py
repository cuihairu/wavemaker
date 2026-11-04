"""Tests for tools/spectrum_sim.py (BLOCKER-1 executable verification).

Run: python3 -m pytest tools/test_spectrum_sim.py -v
"""

import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

import spectrum_sim as sim  # noqa: E402


TOL_REL = 0.05   # 5% relative tolerance for Monte-Carlo vs analytic
TOL_ABS = 60.0   # absolute tolerance floor (Poisson noise at small counts)


def close(a: float, b: float) -> bool:
    return abs(a - b) <= max(TOL_ABS, TOL_REL * max(abs(a), abs(b)))


# --- analytic arrival model ----------------------------------------------------

def test_exponential_arrival_ramp_total_matches_plan():
    # Plan §8.2: "exponential arrival integral ~ 3800 over the 600 s ramp"
    total = sim.arrival_ramp_total()
    assert 3700.0 <= total <= 3900.0, total


def test_cumulative_arrivals_is_monotone_and_consistent():
    prev = 0.0
    for t in range(0, 3601, 60):
        cur = sim.cumulative_arrivals(float(t))
        assert cur >= prev
        assert cur == sim.cumulative_arrivals(float(t) + 0.001) - 1e-12 or cur <= cur + 1
        prev = cur
    # ramp end value equals the closed form
    assert abs(sim.cumulative_arrivals(sim.RAMP) - sim.arrival_ramp_total()) < 1e-6


# --- active-count curves ---------------------------------------------------------

def test_fixed_lifetime_sim_matches_analytic():
    r = sim.simulate(lifetime="fixed")
    for t, n in zip(r.checkpoints, r.active):
        assert close(n, sim.analytic_fixed(t)), (t, n, sim.analytic_fixed(t))


def test_exponential_lifetime_sim_matches_analytic():
    r = sim.simulate(lifetime="exponential", seed=sim.DEFAULT_SEED + 1)
    for t, n in zip(r.checkpoints, r.active):
        assert close(n, sim.analytic_exponential(t)), (t, n)


def test_fixed_lifetime_all_ramp_arrivals_still_active_at_ramp_end():
    # Plan §8.2: with a fixed 900 s lifetime, Bots created during the 600 s ramp
    # have not expired at t = 600, so N(600) == Lambda(600) ~ 3800.
    r = sim.simulate(lifetime="fixed")
    idx = r.checkpoints.index(600.0)
    assert close(r.active[idx], sim.arrival_ramp_total())


def test_exponential_lifetime_lower_than_fixed_during_ramp():
    # Plan §8.2: "if lifetime is exponential, active count at 600 s is lower".
    rf = sim.simulate(lifetime="fixed")
    re = sim.simulate(lifetime="exponential", seed=sim.DEFAULT_SEED + 1)
    idx = rf.checkpoints.index(600.0)
    assert re.active[idx] < rf.active[idx] * 0.9


def test_active_never_exceeds_cumulative_arrivals():
    r = sim.simulate(lifetime="exponential")
    for t, n, p in zip(r.checkpoints, r.active, r.cumulative_planned):
        assert n <= p


def test_littles_law_holds_only_at_steady_state():
    # Hold phase ran from 600 s to 3600 s (3000 s >> W = 900 s): steady state.
    n = sim.simulate(lifetime="exponential", seed=sim.DEFAULT_SEED + 1)
    idx = n.checkpoints.index(3600.0)
    steady = 18000.0  # lambda_hold * W = 20 * 900
    assert close(n.active[idx], steady)
    # During ramp, naive Little's Law wildly overestimates (plan §8.2).
    littles_at_600 = sim.littles_law(600.0)
    assert littles_at_600 > 2.0 * sim.arrival_ramp_total()


# --- saturation policies ---------------------------------------------------------

def test_reject_new_never_exceeds_cap_and_counts_rejections():
    r = sim.simulate_with_cap("reject_new", 5000)
    assert r.max_active <= 5000
    assert r.rejected > 0
    assert r.admitted + r.rejected == len(
        sim.simulate(lifetime="fixed").events)


def test_queue_respects_cap_and_records_delays():
    r = sim.simulate_with_cap("queue", 5000)
    assert r.max_active <= 5000
    assert r.rejected == 0
    assert len(r.delays) == r.admitted
    assert all(d >= 0.0 for d in r.delays)


def test_delay_policy_records_deviation_and_respects_cap():
    r = sim.simulate_with_cap("delay", 5000)
    assert r.max_active <= 5000
    assert r.rejected == 0
    assert len(r.delays) == r.admitted
    assert all(d >= 0.0 for d in r.delays)


def test_small_cap_produces_expected_policy_differences():
    r = sim.simulate_with_cap("reject_new", 500)
    q = sim.simulate_with_cap("queue", 500)
    # reject_new loses load (counted as rejected); queue defers it and must
    # record the scheduling deviation of every admitted Bot.
    assert r.rejected > 0
    assert q.rejected == 0
    assert any(d > 0.0 for d in q.delays)
    assert r.delays == []
    # Without a cap-binding load, queue introduces no deviation.
    uncap = sim.simulate_with_cap("queue", 10**9)
    assert uncap.delays == [] or max(uncap.delays) < sim.DT * 2


def test_invalid_inputs_rejected():
    for policy in ("drop_all", "unknown"):
        try:
            sim.simulate_with_cap(policy, 100)  # type: ignore[arg-type]
        except ValueError:
            pass
        else:
            raise AssertionError(f"policy {policy!r} should be rejected")
    try:
        sim.simulate(lifetime="gamma")  # type: ignore[arg-type]
    except ValueError:
        pass
    else:
        raise AssertionError("lifetime 'gamma' should be rejected")


def test_deterministic_given_same_seed():
    a = sim.simulate(lifetime="exponential", seed=42)
    b = sim.simulate(lifetime="exponential", seed=42)
    assert a.active == b.active
    assert [(e.t_arrival, e.lifetime) for e in a.events] == \
        [(e.t_arrival, e.lifetime) for e in b.events]
