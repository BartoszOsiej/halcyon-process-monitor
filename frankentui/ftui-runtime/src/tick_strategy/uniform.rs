//! [`Uniform`] strategy: tick all inactive screens at a fixed divisor.

use super::{TickDecision, TickStrategy};

/// Tick every inactive screen at a fixed reduced rate: every Nth frame.
///
/// This is the direct framework-level equivalent of a global
/// `INACTIVE_SCREEN_TICK_DIVISOR`.
///
/// - `divisor=1`: every frame (no throttling, matches current ftui default)
/// - `divisor=5`: every 5th frame (recommended default)
/// - `divisor=10`: every 10th frame
#[derive(Debug, Clone, Copy)]
pub struct Uniform {
    divisor: u64,
}

impl Uniform {
    /// Create a uniform strategy with the given divisor.
    ///
    /// A divisor of 0 is clamped to 1 (tick every frame).
    #[must_use]
    pub const fn new(divisor: u64) -> Self {
        Self {
            divisor: if divisor == 0 { 1 } else { divisor },
        }
    }

    /// Return the effective divisor.
    #[must_use]
    pub const fn divisor(&self) -> u64 {
        self.divisor
    }
}

impl Default for Uniform {
    fn default() -> Self {
        Self::new(5)
    }
}

impl TickStrategy for Uniform {
    fn should_tick(
        &mut self,
        _screen_id: &str,
        tick_count: u64,
        _active_screen: &str,
    ) -> TickDecision {
        if tick_count.is_multiple_of(self.divisor) {
            TickDecision::Tick
        } else {
            TickDecision::Skip
        }
    }

    fn name(&self) -> &str {
        "Uniform"
    }

    fn debug_stats(&self) -> Vec<(String, String)> {
        vec![
            ("strategy".into(), "Uniform".into()),
            ("divisor".into(), self.divisor.to_string()),
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn divisor_1_always_ticks() {
        let mut s = Uniform::new(1);
        for tick in 0..20 {
            assert_eq!(
                s.should_tick("x", tick, "active"),
                TickDecision::Tick,
                "divisor=1 should always Tick, failed at tick={tick}"
            );
        }
    }

    #[test]
    fn divisor_5_pattern() {
        let mut s = Uniform::new(5);
        for tick in 0..20 {
            let expected = if tick % 5 == 0 {
                TickDecision::Tick
            } else {
                TickDecision::Skip
            };
            assert_eq!(
                s.should_tick("x", tick, "active"),
                expected,
                "divisor=5, tick={tick}"
            );
        }
    }

    #[test]
    fn divisor_0_clamped_to_1() {
        let s = Uniform::new(0);
        assert_eq!(s.divisor(), 1);
    }

    #[test]
    fn consistent_across_screen_ids() {
        let mut s = Uniform::new(3);
        let d1 = s.should_tick("alpha", 6, "active");
        let d2 = s.should_tick("beta", 6, "active");
        assert_eq!(d1, d2);
    }

    #[test]
    fn name_is_stable() {
        assert_eq!(Uniform::new(5).name(), "Uniform");
    }

    #[test]
    fn debug_stats_reports_divisor() {
        let stats = Uniform::new(7).debug_stats();
        assert!(stats.iter().any(|(k, v)| k == "divisor" && v == "7"));
    }

    #[test]
    fn default_is_divisor_5() {
        assert_eq!(Uniform::default().divisor(), 5);
    }
}
