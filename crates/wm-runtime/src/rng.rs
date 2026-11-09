//! Seeded, reproducible random sources.
//!
//! Every runtime draw (think time, arrival jitter, payload choice) comes from
//! an injected [`SeededRng`], never from the OS entropy pool. A plan plus a
//! seed plus a deterministic clock replays identically.

/// SplitMix64 PRNG: tiny, fast, statistically solid seeding/stepping core.
///
/// Not cryptographic — load simulation only.
#[derive(Debug, Clone)]
pub struct SeededRng {
    state: u64,
}

impl SeededRng {
    /// Derive an independent stream from `(plan seed, stream name)` so that
    /// adding a new consumer never shifts an existing consumer's sequence.
    pub fn for_stream(seed: u64, stream: &str) -> Self {
        // Split a 64-bit plan seed and a name into one mixed state via FNV-1a
        // over the name folded into the seed, then mixed by splitmix64 steps.
        let mut hash = 0xcbf2_9ce4_8422_2325u64;
        for byte in stream.as_bytes() {
            hash ^= u64::from(*byte);
            hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
        }
        Self::new(seed ^ hash)
    }

    pub fn new(seed: u64) -> Self {
        Self { state: seed }
    }

    /// Next raw 64-bit value (SplitMix64 step).
    pub fn next_u64(&mut self) -> u64 {
        self.state = self.state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// Uniform in `[0, 1)`.
    pub fn next_f64(&mut self) -> f64 {
        // 53-bit mantissa draw: strictly below 1.0.
        (self.next_u64() >> 11) as f64 * (1.0 / (1u64 << 53) as f64)
    }

    /// Uniform in `[low, high)` (empty range yields `low`).
    ///
    /// # Panics
    /// Panics if `high < low` or the range is not finite.
    pub fn next_range_f64(&mut self, low: f64, high: f64) -> f64 {
        assert!(high >= low, "invalid range [{low}, {high})");
        assert!(
            low.is_finite() && high.is_finite(),
            "range bounds must be finite"
        );
        low + self.next_f64() * (high - low)
    }

    /// Uniform integer in `[low, high)` (empty range yields `low`).
    ///
    /// # Panics
    /// Panics if `high < low`.
    pub fn next_range_u64(&mut self, low: u64, high: u64) -> u64 {
        assert!(high >= low, "invalid range [{low}, {high})");
        let span = high - low;
        if span == 0 {
            return low;
        }
        // Rejection sampling keeps the distribution uniform without modulo bias.
        let zone = u64::MAX - (u64::MAX % span) - 1;
        loop {
            let draw = self.next_u64();
            if draw <= zone {
                return low + (draw % span);
            }
        }
    }

    /// Exponential draw with rate `lambda` (mean `1 / lambda`), e.g. for
    /// Poisson think times. `lambda` must be positive and finite.
    ///
    /// # Panics
    /// Panics if `lambda <= 0` or not finite.
    pub fn next_exponential(&mut self, lambda: f64) -> f64 {
        assert!(
            lambda > 0.0 && lambda.is_finite(),
            "lambda must be positive and finite"
        );
        let u = self.next_f64();
        // u in [0,1); use 1-u to avoid log(0).
        -(1.0 - u).ln() / lambda
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_seed_replays_identically() {
        let mut a = SeededRng::new(42);
        let mut b = SeededRng::new(42);
        for _ in 0..1000 {
            assert_eq!(a.next_u64(), b.next_u64());
        }
    }

    #[test]
    fn different_seeds_diverge() {
        let mut a = SeededRng::new(1);
        let mut b = SeededRng::new(2);
        let seq_a: Vec<u64> = (0..16).map(|_| a.next_u64()).collect();
        let seq_b: Vec<u64> = (0..16).map(|_| b.next_u64()).collect();
        assert_ne!(seq_a, seq_b);
    }

    #[test]
    fn stream_derivation_is_stable_and_order_independent() {
        // Adding a stream later must not shift earlier streams.
        let mut first = SeededRng::for_stream(7, "arrivals");
        let mut second = SeededRng::for_stream(7, "think_time");
        let first_draw = first.next_u64();
        let second_draw = second.next_u64();
        let mut first_again = SeededRng::for_stream(7, "arrivals");
        assert_ne!(first_draw, second_draw);
        assert_eq!(first_draw, first_again.next_u64());
    }

    #[test]
    fn unit_interval_is_half_open() {
        let mut rng = SeededRng::new(9);
        for _ in 0..10_000 {
            let u = rng.next_f64();
            assert!((0.0..1.0).contains(&u), "draw {u} outside [0,1)");
        }
    }

    #[test]
    fn uniform_range_respects_bounds_and_mean() {
        let mut rng = SeededRng::new(11);
        let n = 20_000;
        let mut sum = 0.0;
        for _ in 0..n {
            let draw = rng.next_range_f64(5.0, 15.0);
            assert!((5.0..15.0).contains(&draw));
            sum += draw;
        }
        let mean = sum / n as f64;
        assert!((9.5..10.5).contains(&mean), "uniform mean drifted: {mean}");
    }

    #[test]
    fn integer_range_has_no_modulo_bias() {
        let mut rng = SeededRng::new(13);
        let mut seen = [false; 10];
        for _ in 0..10_000 {
            let draw = rng.next_range_u64(3, 13) as usize;
            assert!((3..13).contains(&draw));
            seen[draw - 3] = true;
        }
        assert!(seen.iter().all(|s| *s), "some values never drawn");
    }

    #[test]
    fn empty_ranges_yield_low() {
        let mut rng = SeededRng::new(17);
        assert_eq!(rng.next_range_u64(4, 4), 4);
        assert_eq!(rng.next_range_f64(2.0, 2.0), 2.0);
    }

    #[test]
    fn exponential_mean_tracks_lambda() {
        let mut rng = SeededRng::new(19);
        let lambda = 2.0;
        let n = 50_000;
        let mut sum = 0.0;
        for _ in 0..n {
            let draw = rng.next_exponential(lambda);
            assert!(draw >= 0.0);
            sum += draw;
        }
        let mean = sum / n as f64;
        let expected = 1.0 / lambda;
        assert!(
            (mean - expected).abs() < 0.05 * expected,
            "exponential mean drifted: {mean} vs {expected}"
        );
    }

    #[test]
    #[should_panic(expected = "lambda must be positive")]
    fn exponential_rejects_non_positive_lambda() {
        SeededRng::new(1).next_exponential(0.0);
    }

    #[test]
    #[should_panic(expected = "invalid range")]
    fn range_rejects_inverted_bounds() {
        SeededRng::new(1).next_range_f64(10.0, 5.0);
    }
}
