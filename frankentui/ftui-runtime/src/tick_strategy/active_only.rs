//! [`ActiveOnly`] strategy: only the active screen ticks.

use super::{TickDecision, TickStrategy};

/// Simplest strategy — all inactive screens are skipped every frame.
///
/// Use when screen tick methods are expensive (heavy DB queries, network
/// calls) and force-tick-on-activation provides sufficient freshness.
#[derive(Debug, Clone, Copy, Default)]
pub struct ActiveOnly;

impl TickStrategy for ActiveOnly {
    fn should_tick(
        &mut self,
        _screen_id: &str,
        _tick_count: u64,
        _active_screen: &str,
    ) -> TickDecision {
        TickDecision::Skip
    }

    fn name(&self) -> &str {
        "ActiveOnly"
    }

    fn debug_stats(&self) -> Vec<(String, String)> {
        vec![
            ("strategy".into(), "ActiveOnly".into()),
            ("background_ticks".into(), "0".into()),
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn always_skips() {
        let mut s = ActiveOnly;
        for tick in 0..100 {
            assert_eq!(
                s.should_tick("screen_a", tick, "screen_b"),
                TickDecision::Skip
            );
        }
    }

    #[test]
    fn name_is_stable() {
        assert_eq!(ActiveOnly.name(), "ActiveOnly");
    }

    #[test]
    fn default_hooks_are_noops() {
        let mut s = ActiveOnly;
        s.on_screen_transition("a", "b");
        s.maintenance_tick(42);
        s.shutdown();
        let stats = s.debug_stats();
        assert!(
            stats
                .iter()
                .any(|(k, v)| k == "strategy" && v == "ActiveOnly")
        );
        assert!(
            stats
                .iter()
                .any(|(k, v)| k == "background_ticks" && v == "0")
        );
    }
}
