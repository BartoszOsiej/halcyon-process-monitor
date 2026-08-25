//! Probability-to-divisor tick rate allocation.
//!
//! [`TickAllocation`] maps a transition probability (0.0..1.0) to a tick
//! divisor (min..max). This bridges the gap between "how likely is the user
//! to switch to screen X?" and "how often should we tick screen X?"
//!
//! Three allocation curves are provided:
//!
//! - [`AllocationCurve::Linear`]: `divisor = max - (max - min) * prob`
//! - [`AllocationCurve::Exponential`]: concentrates budget on likely screens
//! - [`AllocationCurve::Stepped`]: explicit threshold-to-divisor tiers

/// Maps transition probability to tick divisor.
#[derive(Debug, Clone)]
pub struct TickAllocation {
    /// Ceiling: least-likely screens tick at most every max_divisor frames.
    pub max_divisor: u64,
    /// Floor: most-likely screens tick at least every min_divisor frames.
    pub min_divisor: u64,
    /// How probability maps to divisor.
    pub curve: AllocationCurve,
}

/// Curve controlling how probability maps to divisor.
#[derive(Debug, Clone)]
pub enum AllocationCurve {
    /// Linear: `divisor = max - (max - min) * probability`.
    Linear,
    /// Exponential: `divisor = max * (1 - probability)^exponent`.
    /// Gives more budget to high-probability screens with sharp falloff.
    Exponential {
        /// Controls curve steepness (typically 2.0).
        exponent: f64,
    },
    /// Stepped: bucket probabilities into tiers.
    ///
    /// Thresholds are checked in descending order; first match wins.
    /// Must be sorted descending by threshold value.
    ///
    /// Example: `[(0.3, 1), (0.1, 2), (0.03, 5), (0.0, 20)]`
    Stepped {
        /// `(threshold, divisor)` pairs, sorted descending by threshold.
        thresholds: Vec<(f64, u64)>,
    },
}

impl TickAllocation {
    /// Create with exponential curve (recommended default).
    ///
    /// Exponent=2.0, max_divisor=20, min_divisor=1.
    #[must_use]
    pub fn new() -> Self {
        Self {
            max_divisor: 20,
            min_divisor: 1,
            curve: AllocationCurve::Exponential { exponent: 2.0 },
        }
    }

    /// Create with a linear curve.
    #[must_use]
    pub fn linear(min_divisor: u64, max_divisor: u64) -> Self {
        Self {
            max_divisor: max_divisor.max(1),
            min_divisor: min_divisor.max(1),
            curve: AllocationCurve::Linear,
        }
    }

    /// Create with an exponential curve.
    #[must_use]
    pub fn exponential(min_divisor: u64, max_divisor: u64, exponent: f64) -> Self {
        Self {
            max_divisor: max_divisor.max(1),
            min_divisor: min_divisor.max(1),
            curve: AllocationCurve::Exponential {
                exponent: exponent.max(0.1),
            },
        }
    }

    /// Create with stepped thresholds.
    ///
    /// # Panics
    ///
    /// Panics if thresholds are not sorted descending by threshold value.
    #[must_use]
    pub fn stepped(thresholds: Vec<(f64, u64)>) -> Self {
        // Validate descending order
        for window in thresholds.windows(2) {
            assert!(
                window[0].0 >= window[1].0,
                "Stepped thresholds must be sorted descending: {} >= {} violated",
                window[0].0,
                window[1].0,
            );
        }

        let max_divisor = thresholds
            .iter()
            .map(|(_, d)| *d)
            .max()
            .unwrap_or(20)
            .max(1);
        let min_divisor = thresholds.iter().map(|(_, d)| *d).min().unwrap_or(1).max(1);

        Self {
            max_divisor,
            min_divisor,
            curve: AllocationCurve::Stepped { thresholds },
        }
    }

    /// Map a probability (0.0..1.0) to a tick divisor.
    ///
    /// Higher probability → lower divisor (faster ticking).
    /// Result is always clamped to `[min_divisor, max_divisor]`.
    #[must_use]
    pub fn divisor_for(&self, probability: f64) -> u64 {
        let prob = probability.clamp(0.0, 1.0);
        let min = self.min_divisor.max(1);
        let max = self.max_divisor.max(min);

        let raw = match &self.curve {
            AllocationCurve::Linear => {
                // divisor = max - (max - min) * prob
                let range = (max - min) as f64;
                max as f64 - range * prob
            }
            AllocationCurve::Exponential { exponent } => {
                // divisor = min + (max - min) * (1 - prob)^exponent
                let range = (max - min) as f64;
                min as f64 + range * (1.0 - prob).powf(*exponent)
            }
            AllocationCurve::Stepped { thresholds } => {
                // First threshold where prob >= threshold wins
                for &(threshold, divisor) in thresholds {
                    if prob >= threshold {
                        return divisor.clamp(min, max);
                    }
                }
                // No threshold matched → max divisor
                max as f64
            }
        };

        (raw.round() as u64).clamp(min, max)
    }
}

impl Default for TickAllocation {
    fn default() -> Self {
        Self::new()
    }
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_probability_one_returns_min() {
        let alloc = TickAllocation::new();
        assert_eq!(alloc.divisor_for(1.0), 1);
    }

    #[test]
    fn default_probability_zero_returns_max() {
        let alloc = TickAllocation::new();
        assert_eq!(alloc.divisor_for(0.0), 20);
    }

    #[test]
    fn monotonically_decreasing() {
        let alloc = TickAllocation::new();
        let mut prev = u64::MAX;
        for i in 0..=100 {
            let prob = i as f64 / 100.0;
            let div = alloc.divisor_for(prob);
            assert!(
                div <= prev,
                "not monotonic at prob={prob}: div={div}, prev={prev}"
            );
            prev = div;
        }
    }

    #[test]
    fn linear_curve() {
        let alloc = TickAllocation::linear(1, 20);

        assert_eq!(alloc.divisor_for(1.0), 1);
        assert_eq!(alloc.divisor_for(0.0), 20);
        // 0.5 → 20 - 19 * 0.5 = 10.5 → 11
        assert_eq!(alloc.divisor_for(0.5), 11);
    }

    #[test]
    fn linear_monotonic() {
        let alloc = TickAllocation::linear(1, 100);
        let mut prev = u64::MAX;
        for i in 0..=100 {
            let prob = i as f64 / 100.0;
            let div = alloc.divisor_for(prob);
            assert!(div <= prev);
            prev = div;
        }
    }

    #[test]
    fn exponential_curve() {
        let alloc = TickAllocation::exponential(1, 20, 2.0);

        assert_eq!(alloc.divisor_for(1.0), 1);
        assert_eq!(alloc.divisor_for(0.0), 20);

        // 0.5 → 1 + 19 * (0.5)^2 = 1 + 19*0.25 = 1 + 4.75 = 5.75 → 6
        assert_eq!(alloc.divisor_for(0.5), 6);
    }

    #[test]
    fn exponential_monotonic() {
        let alloc = TickAllocation::exponential(1, 20, 2.0);
        let mut prev = u64::MAX;
        for i in 0..=100 {
            let prob = i as f64 / 100.0;
            let div = alloc.divisor_for(prob);
            assert!(div <= prev);
            prev = div;
        }
    }

    #[test]
    fn exponential_default_table() {
        // Verify the recommended defaults match the design table
        let alloc = TickAllocation::new(); // exponential, exp=2.0, max=20, min=1

        // p=0.50 → 1 + 19*(0.5)^2 = 5.75 → 6
        assert_eq!(alloc.divisor_for(0.50), 6);
        // p=0.30 → 1 + 19*(0.7)^2 = 1 + 19*0.49 = 10.31 → 10
        assert_eq!(alloc.divisor_for(0.30), 10);
        // p=0.05 → 1 + 19*(0.95)^2 = 1 + 19*0.9025 = 18.15 → 18
        assert_eq!(alloc.divisor_for(0.05), 18);
    }

    #[test]
    fn stepped_curve() {
        let alloc = TickAllocation::stepped(vec![(0.30, 1), (0.10, 2), (0.03, 5), (0.00, 20)]);

        assert_eq!(alloc.divisor_for(0.50), 1); // > 0.30
        assert_eq!(alloc.divisor_for(0.31), 1); // > 0.30
        assert_eq!(alloc.divisor_for(0.20), 2); // > 0.10
        assert_eq!(alloc.divisor_for(0.05), 5); // > 0.03
        assert_eq!(alloc.divisor_for(0.01), 20); // > 0.00
    }

    #[test]
    fn stepped_first_match_wins() {
        let alloc = TickAllocation::stepped(vec![(0.50, 1), (0.25, 5), (0.00, 10)]);
        // 0.60 > 0.50, so first threshold matches
        assert_eq!(alloc.divisor_for(0.60), 1);
    }

    #[test]
    fn stepped_threshold_is_inclusive() {
        let alloc = TickAllocation::stepped(vec![(0.30, 1), (0.10, 2), (0.00, 20)]);
        assert_eq!(alloc.divisor_for(0.30), 1);
        assert_eq!(alloc.divisor_for(0.10), 2);
        assert_eq!(alloc.divisor_for(0.00), 20);
    }

    #[test]
    #[should_panic(expected = "sorted descending")]
    fn stepped_panics_on_unsorted() {
        let _ = TickAllocation::stepped(vec![
            (0.10, 2), // wrong: should be higher first
            (0.30, 1),
            (0.00, 20),
        ]);
    }

    #[test]
    fn clamps_to_range() {
        let alloc = TickAllocation::exponential(2, 15, 1.0);
        // Even extreme probabilities stay in [2, 15]
        assert!(alloc.divisor_for(1.0) >= 2);
        assert!(alloc.divisor_for(0.0) <= 15);
        assert!(alloc.divisor_for(1.5) >= 2); // clamped input
        assert!(alloc.divisor_for(-0.5) <= 15); // clamped input
    }

    #[test]
    fn all_curves_in_range() {
        let curves: Vec<TickAllocation> = vec![
            TickAllocation::linear(1, 20),
            TickAllocation::exponential(1, 20, 2.0),
            TickAllocation::stepped(vec![(0.5, 1), (0.0, 20)]),
        ];

        for alloc in &curves {
            for i in 0..=100 {
                let prob = i as f64 / 100.0;
                let div = alloc.divisor_for(prob);
                assert!(
                    div >= alloc.min_divisor && div <= alloc.max_divisor,
                    "out of range: div={div}, min={}, max={}, prob={prob}",
                    alloc.min_divisor,
                    alloc.max_divisor,
                );
            }
        }
    }

    #[test]
    fn default_impl() {
        let alloc = TickAllocation::default();
        assert_eq!(alloc.max_divisor, 20);
        assert_eq!(alloc.min_divisor, 1);
    }

    #[test]
    fn empty_stepped_returns_max() {
        let alloc = TickAllocation::stepped(vec![]);
        // No thresholds → divisor_for should return max (which defaults based on empty vec)
        let div = alloc.divisor_for(0.5);
        assert_eq!(div, alloc.max_divisor);
    }

    // ========================================================================
    // Additional tests (I.3 coverage)
    // ========================================================================

    #[test]
    fn max_divisor_one_always_returns_one() {
        // When max_divisor=1 (no throttling), every screen gets ticked every frame.
        let linear = TickAllocation::linear(1, 1);
        let exp = TickAllocation::exponential(1, 1, 2.0);

        for prob in [0.0, 0.25, 0.5, 0.75, 1.0] {
            let d_lin = linear.divisor_for(prob);
            let d_exp = exp.divisor_for(prob);
            eprintln!("prob={prob}: linear={d_lin}, exponential={d_exp}");
            assert_eq!(
                d_lin, 1,
                "linear with max=1 should return 1 for prob={prob}"
            );
            assert_eq!(
                d_exp, 1,
                "exponential with max=1 should return 1 for prob={prob}"
            );
        }
    }

    #[test]
    fn exponential_high_exponent_concentrates_budget() {
        // With a high exponent, the curve drops steeply — only very high
        // probabilities get low divisors.
        let steep = TickAllocation::exponential(1, 100, 5.0);
        let shallow = TickAllocation::exponential(1, 100, 1.0);

        let p = 0.5;
        let d_steep = steep.divisor_for(p);
        let d_shallow = shallow.divisor_for(p);

        eprintln!("p={p}: steep(exp=5)={d_steep}, shallow(exp=1)={d_shallow}");
        // Steep exponent: (1-0.5)^5 = 0.03125 → 1 + 99*0.03125 ≈ 4
        // Shallow exponent: (1-0.5)^1 = 0.5 → 1 + 99*0.5 ≈ 51
        assert!(
            d_steep < d_shallow,
            "steep exponent should give lower divisor (more budget) for p={p}"
        );
    }
}
