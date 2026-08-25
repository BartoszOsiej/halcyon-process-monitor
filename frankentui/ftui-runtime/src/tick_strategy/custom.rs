//! [`Custom`] strategy: closure-based for app-specific tick logic.
//!
//! An escape hatch for apps that need custom tick decisions without
//! implementing the full [`TickStrategy`] trait. Wraps a user-provided
//! closure `(screen_id, tick_count, active_screen) -> TickDecision`.

use super::{TickDecision, TickStrategy};

/// Type alias for the custom tick decision function.
type DeciderFn = dyn Fn(&str, u64, &str) -> TickDecision + Send;

/// Closure-based tick strategy for app-specific logic.
///
/// # Example
///
/// ```
/// use ftui_runtime::tick_strategy::{Custom, TickDecision};
///
/// let strategy = Custom::new("PriorityBased", |screen_id, tick_count, _active| {
///     let divisor: u64 = match screen_id {
///         "Dashboard" | "Messages" => 2,
///         "Analytics" => 10,
///         _ => 5,
///     };
///     if tick_count.is_multiple_of(divisor) {
///         TickDecision::Tick
///     } else {
///         TickDecision::Skip
///     }
/// });
/// ```
pub struct Custom {
    decider: Box<DeciderFn>,
    label: String,
}

impl Custom {
    /// Create a custom strategy with the given label and decision function.
    ///
    /// The closure receives `(screen_id, tick_count, active_screen)` and
    /// returns [`TickDecision::Tick`] or [`TickDecision::Skip`].
    pub fn new<F>(label: impl Into<String>, f: F) -> Self
    where
        F: Fn(&str, u64, &str) -> TickDecision + Send + 'static,
    {
        Self {
            decider: Box::new(f),
            label: label.into(),
        }
    }
}

impl std::fmt::Debug for Custom {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Custom")
            .field("label", &self.label)
            .finish_non_exhaustive()
    }
}

impl TickStrategy for Custom {
    fn should_tick(
        &mut self,
        screen_id: &str,
        tick_count: u64,
        active_screen: &str,
    ) -> TickDecision {
        (self.decider)(screen_id, tick_count, active_screen)
    }

    fn name(&self) -> &str {
        &self.label
    }

    fn debug_stats(&self) -> Vec<(String, String)> {
        vec![("strategy".into(), self.label.clone())]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn custom_closure_receives_correct_args() {
        let mut s = Custom::new("Test", |screen_id, tick_count, active| {
            assert_eq!(screen_id, "bg");
            assert_eq!(tick_count, 42);
            assert_eq!(active, "fg");
            TickDecision::Tick
        });
        assert_eq!(s.should_tick("bg", 42, "fg"), TickDecision::Tick);
    }

    #[test]
    fn custom_return_value_is_respected() {
        let mut always_tick = Custom::new("AlwaysTick", |_, _, _| TickDecision::Tick);
        assert_eq!(always_tick.should_tick("x", 0, "y"), TickDecision::Tick);

        let mut always_skip = Custom::new("AlwaysSkip", |_, _, _| TickDecision::Skip);
        assert_eq!(always_skip.should_tick("x", 0, "y"), TickDecision::Skip);
    }

    #[test]
    fn name_returns_label() {
        let s = Custom::new("MyCustom", |_, _, _| TickDecision::Skip);
        assert_eq!(s.name(), "MyCustom");
    }

    #[test]
    fn debug_stats_contains_label() {
        let s = Custom::new("Labeled", |_, _, _| TickDecision::Skip);
        let stats = s.debug_stats();
        assert_eq!(stats.len(), 1);
        assert_eq!(stats[0], ("strategy".to_owned(), "Labeled".to_owned()));
    }

    #[test]
    fn custom_can_be_boxed_as_dyn_tick_strategy() {
        let s = Custom::new("Boxable", |_, _, _| TickDecision::Tick);
        let mut boxed: Box<dyn TickStrategy> = Box::new(s);
        assert_eq!(boxed.should_tick("a", 0, "b"), TickDecision::Tick);
        assert_eq!(boxed.name(), "Boxable");
    }

    #[test]
    fn custom_debug_format() {
        let s = Custom::new("Dbg", |_, _, _| TickDecision::Skip);
        let dbg = format!("{s:?}");
        assert!(dbg.contains("Custom"));
        assert!(dbg.contains("Dbg"));
    }

    #[test]
    fn custom_divisor_logic() {
        let mut s = Custom::new("DivisorBased", |screen_id, tick_count, _| {
            let divisor: u64 = match screen_id {
                "fast" => 2,
                "slow" => 10,
                _ => 5,
            };
            if tick_count.is_multiple_of(divisor) {
                TickDecision::Tick
            } else {
                TickDecision::Skip
            }
        });

        // fast: divisor=2, tick 4 → Tick
        assert_eq!(s.should_tick("fast", 4, "active"), TickDecision::Tick);
        // fast: divisor=2, tick 3 → Skip
        assert_eq!(s.should_tick("fast", 3, "active"), TickDecision::Skip);
        // slow: divisor=10, tick 10 → Tick
        assert_eq!(s.should_tick("slow", 10, "active"), TickDecision::Tick);
        // slow: divisor=10, tick 5 → Skip
        assert_eq!(s.should_tick("slow", 5, "active"), TickDecision::Skip);
    }
}
