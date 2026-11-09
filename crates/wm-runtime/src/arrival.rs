//! Arrival Scheduler (WM-0306).
//!
//! Bot arrivals are a non-homogeneous Poisson process whose rate follows the
//! Spectrum ramp model (docs/spec/spectrum-semantics.md §3): exponential
//! growth `λ(t) = λ₀·e^{kt}` from `from_per_second` to `to_per_second` over
//! `ramp_duration`, then hold at `to_per_second` afterwards.
//!
//! Arrivals are generated exactly (not by fixed-step Bernoulli thinning) via
//! the standard inversion method in cumulative-intensity space: an Exp(1)
//! draw is added to `Λ(t) = ∫λ(s)ds` and the arrival time is `Λ⁻¹` of the
//! result. The same plan plus the same seed replays identically, matching the
//! Python reference (`tools/spectrum_sim.py`) in expectation.

const NANOS_PER_SEC: f64 = 1_000_000_000.0;

/// Rate-curve parameters for one arrival plan.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ArrivalPlan {
    from_per_second: f64,
    to_per_second: f64,
    ramp_duration_ns: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum ArrivalError {
    #[error("rates must be positive and finite")]
    InvalidRate,
    #[error("ramp duration must be finite")]
    InvalidRamp,
}

impl ArrivalPlan {
    /// Build a ramp plan. `ramp_duration_ns == 0` or equal rates degrade to a
    /// constant rate of `to_per_second`.
    pub fn new(
        from_per_second: f64,
        to_per_second: f64,
        ramp_duration_ns: u64,
    ) -> Result<Self, ArrivalError> {
        if !from_per_second.is_finite()
            || !to_per_second.is_finite()
            || from_per_second <= 0.0
            || to_per_second <= 0.0
        {
            return Err(ArrivalError::InvalidRate);
        }
        Ok(Self {
            from_per_second,
            to_per_second,
            ramp_duration_ns,
        })
    }

    /// Constant-rate plan.
    pub fn constant(per_second: f64) -> Result<Self, ArrivalError> {
        Self::new(per_second, per_second, 0)
    }

    pub fn from_per_second(&self) -> f64 {
        self.from_per_second
    }

    pub fn to_per_second(&self) -> f64 {
        self.to_per_second
    }

    pub fn ramp_duration_ns(&self) -> u64 {
        self.ramp_duration_ns
    }

    fn ramp_secs(&self) -> f64 {
        self.ramp_duration_ns as f64 / NANOS_PER_SEC
    }

    fn is_constant(&self) -> bool {
        self.ramp_duration_ns == 0 || self.from_per_second == self.to_per_second
    }

    fn growth_k(&self) -> f64 {
        (self.to_per_second / self.from_per_second).ln() / self.ramp_secs()
    }

    /// Arrival rate λ(t) in arrivals per second at clock time `t_ns`.
    pub fn rate_at(&self, t_ns: u64) -> f64 {
        let t = t_ns as f64 / NANOS_PER_SEC;
        if self.is_constant() {
            return self.to_per_second;
        }
        if t <= self.ramp_secs() {
            self.from_per_second * (self.growth_k() * t).exp()
        } else {
            self.to_per_second
        }
    }

    /// Cumulative intensity Λ(t) = ∫₀ᵗ λ(s) ds (expected arrivals by `t_ns`).
    pub fn cumulative(&self, t_ns: u64) -> f64 {
        let t = t_ns as f64 / NANOS_PER_SEC;
        if self.is_constant() {
            return self.to_per_second * t;
        }
        if t <= self.ramp_secs() {
            self.from_per_second * (self.growth_k() * t).exp_m1() / self.growth_k()
        } else {
            let ramp = self.ramp_cumulative();
            ramp + self.to_per_second * (t - self.ramp_secs())
        }
    }

    fn ramp_cumulative(&self) -> f64 {
        // Λ(T) = λ₀(e^{kT} − 1)/k = (λ₁ − λ₀)/k.
        self.from_per_second * (self.growth_k() * self.ramp_secs()).exp_m1() / self.growth_k()
    }

    /// Inverse of [`ArrivalPlan::cumulative`]: the time (ns) at which Λ equals
    /// `x` arrivals, rounded up to the next whole nanosecond.
    pub fn inverse_cumulative(&self, x: f64) -> u64 {
        let x = x.max(0.0);
        let t_secs = if self.is_constant() {
            x / self.to_per_second
        } else {
            let ramp = self.ramp_cumulative();
            if x <= ramp {
                (1.0 + self.growth_k() * x / self.from_per_second).ln() / self.growth_k()
            } else {
                self.ramp_secs() + (x - ramp) / self.to_per_second
            }
        };
        (t_secs * NANOS_PER_SEC).ceil() as u64
    }
}

/// Deterministic arrival generator for one [`ArrivalPlan`].
///
/// Emits arrival times in nanosecond clock time. Sequential draws are exact
/// NHPP samples; a draw re-anchored at a later `after_ns` is a valid
/// memoryless skip-ahead sample.
#[derive(Debug, Clone)]
pub struct ArrivalScheduler {
    plan: ArrivalPlan,
    rng: crate::rng::SeededRng,
    last_arrival_ns: Option<u64>,
    arrived: u64,
}

impl ArrivalScheduler {
    pub fn new(plan: ArrivalPlan, seed: u64) -> Self {
        Self {
            plan,
            rng: crate::rng::SeededRng::for_stream(seed, "arrivals"),
            last_arrival_ns: None,
            arrived: 0,
        }
    }

    pub fn plan(&self) -> &ArrivalPlan {
        &self.plan
    }

    /// Total arrivals emitted so far.
    pub fn arrived(&self) -> u64 {
        self.arrived
    }

    /// Last emitted arrival time, if any.
    pub fn last_arrival(&self) -> Option<u64> {
        self.last_arrival_ns
    }

    /// Next arrival strictly after `after_ns`.
    ///
    /// Re-anchors the Exp(1) draw at `after_ns`; because the NHPP is
    /// memoryless in Λ-space this is a valid sample whether `after_ns` is the
    /// previous arrival (sequential) or a later time (skip-ahead).
    pub fn next_arrival_after(&mut self, after_ns: u64) -> u64 {
        let base = self.plan.cumulative(after_ns);
        let x = base + self.rng.next_exponential(1.0);
        let t = self.plan.inverse_cumulative(x);
        debug_assert!(t > after_ns, "Exp(1) draw must advance time");
        self.last_arrival_ns = Some(t);
        self.arrived += 1;
        t
    }

    /// Next arrival after the last emitted one (or after t=0 for the first).
    pub fn next_arrival(&mut self) -> u64 {
        let after = self.last_arrival_ns.unwrap_or(0);
        self.next_arrival_after(after)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Spec §3 reference scenario: 1/s -> 20/s over 600 s, hold after.
    fn spec_plan() -> ArrivalPlan {
        ArrivalPlan::new(1.0, 20.0, 600_000_000_000).unwrap()
    }

    #[test]
    fn rate_follows_ramp_then_holds() {
        let plan = spec_plan();
        let t_ramp = 600_000_000_000u64;
        assert_eq!(plan.rate_at(0), 1.0);
        let at_ramp = plan.rate_at(t_ramp);
        assert!((at_ramp - 20.0).abs() < 1e-6, "rate at ramp end {at_ramp}");
        let after = plan.rate_at(3_600_000_000_000);
        assert!((after - 20.0).abs() < 1e-9, "hold rate {after}");
        // Growth is monotone inside the ramp.
        let mid = plan.rate_at(t_ramp / 2);
        assert!((1.0..20.0).contains(&mid), "mid-ramp rate {mid}");
    }

    #[test]
    fn constant_plan_rate_is_flat() {
        let plan = ArrivalPlan::constant(7.0).unwrap();
        assert_eq!(plan.rate_at(0), 7.0);
        assert_eq!(plan.rate_at(1_000_000_000), 7.0);
        assert_eq!(plan.cumulative(2_000_000_000), 14.0);
    }

    #[test]
    fn ramp_cumulative_matches_spec_integral() {
        // Λ(T) = (λ₁ − λ₀)/k = 11400 / ln(20) ≈ 3805.4 (spec §3).
        let plan = spec_plan();
        let ramp = plan.cumulative(600_000_000_000);
        assert!((ramp - 3805.4).abs() < 1.0, "ramp integral {ramp}");
        // Hold phase adds λ₁·(t−T) linearly.
        let horizon = plan.cumulative(3_600_000_000_000);
        let expected = 3805.4 + 20.0 * 3000.0;
        assert!(
            (horizon - expected).abs() < 1.0,
            "horizon cumulative {horizon} vs {expected}"
        );
    }

    #[test]
    fn inverse_round_trips_cumulative() {
        let plan = spec_plan();
        for t_ns in [
            0u64,
            60_000_000_000,
            600_000_000_000,
            1_200_000_000_000,
            3_600_000_000_000,
        ] {
            let x = plan.cumulative(t_ns);
            let back = plan.inverse_cumulative(x);
            let diff_ns = back.abs_diff(t_ns);
            assert!(
                diff_ns <= 1,
                "round trip at {t_ns}ns landed {diff_ns}ns off"
            );
        }
        assert_eq!(plan.inverse_cumulative(0.0), 0);
    }

    #[test]
    fn inverse_after_ramp_is_linear() {
        let plan = spec_plan();
        let t_ramp = 600_000_000_000u64;
        let ramp = plan.cumulative(t_ramp);
        // One second of hold-phase intensity advances time by one second.
        let t1 = plan.inverse_cumulative(ramp + 20.0);
        assert!((t1 as f64 - (t_ramp as f64 + NANOS_PER_SEC)).abs() < 2.0);
    }

    #[test]
    fn next_arrivals_are_strictly_increasing_and_deterministic() {
        let mut a = ArrivalScheduler::new(spec_plan(), 20261104);
        let mut b = ArrivalScheduler::new(spec_plan(), 20261104);
        let mut prev = 0u64;
        for _ in 0..1000 {
            let t = a.next_arrival();
            let t2 = b.next_arrival();
            assert_eq!(t, t2, "same seed must replay identically");
            assert!(t > prev, "arrivals strictly increase: {t} <= {prev}");
            prev = t;
        }
        assert_eq!(a.arrived(), 1000);
        assert_eq!(a.last_arrival(), Some(prev));
    }

    #[test]
    fn arrival_count_tracks_cumulative_intensity() {
        // Over a long horizon the Poisson count ≈ Λ(horizon) within noise.
        let plan = ArrivalPlan::new(1.0, 20.0, 600_000_000_000).unwrap();
        let mut sched = ArrivalScheduler::new(plan, 7);
        let horizon = 3_600_000_000_000u64;
        let mut count = 0u64;
        loop {
            let t = sched.next_arrival();
            if t > horizon {
                break;
            }
            count += 1;
        }
        let expected = plan.cumulative(horizon);
        let rel = (count as f64 - expected).abs() / expected;
        assert!(
            rel < 0.03,
            "count {count} vs expected {expected} (rel {rel})"
        );
    }

    #[test]
    fn constant_rate_interarrival_mean_tracks_rate() {
        // Exp(λ) inter-arrival: mean 1/λ.
        let plan = ArrivalPlan::constant(50.0).unwrap();
        let mut sched = ArrivalScheduler::new(plan, 99);
        let n = 20_000;
        let mut sum = 0u128;
        let mut prev = sched.next_arrival();
        for _ in 0..n {
            let t = sched.next_arrival();
            sum += (t - prev) as u128;
            prev = t;
        }
        let mean = sum as f64 / n as f64;
        let expected = NANOS_PER_SEC / 50.0;
        assert!(
            (mean - expected).abs() < 0.05 * expected,
            "inter-arrival mean {mean} vs {expected}"
        );
    }

    #[test]
    fn skip_ahead_is_memoryless_valid() {
        // Anchoring far ahead still yields a later, valid arrival.
        let mut sched = ArrivalScheduler::new(spec_plan(), 5);
        let t = sched.next_arrival_after(2_000_000_000_000);
        assert!(t > 2_000_000_000_000);
        assert_eq!(sched.arrived(), 1);
    }

    #[test]
    fn zero_duration_ramp_is_constant_at_to() {
        let plan = ArrivalPlan::new(1.0, 30.0, 0).unwrap();
        assert_eq!(plan.rate_at(0), 30.0);
        assert_eq!(plan.cumulative(1_000_000_000), 30.0);
    }

    #[test]
    fn rejects_invalid_rates() {
        assert!(matches!(
            ArrivalPlan::new(0.0, 20.0, 1),
            Err(ArrivalError::InvalidRate)
        ));
        assert!(matches!(
            ArrivalPlan::new(1.0, -1.0, 1),
            Err(ArrivalError::InvalidRate)
        ));
        assert!(matches!(
            ArrivalPlan::new(f64::NAN, 20.0, 1),
            Err(ArrivalError::InvalidRate)
        ));
        assert!(matches!(
            ArrivalPlan::new(f64::INFINITY, 20.0, 1),
            Err(ArrivalError::InvalidRate)
        ));
    }
}
