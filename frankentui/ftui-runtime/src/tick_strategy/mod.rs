//! Tick strategy primitives for selective background ticking.
//!
//! The runtime always ticks the active screen. A [`TickStrategy`] decides
//! whether each inactive screen should receive a tick on a given frame.
//!
//! # Standalone structs vs. convenience enum
//!
//! Each built-in strategy has a standalone struct ([`ActiveOnly`], [`Uniform`],
//! [`ActivePlusAdjacent`]) that implements [`TickStrategy`] directly. For
//! quick selection among built-ins, use [`TickStrategyKind`] which delegates
//! to the same logic.

mod active_only;
mod active_plus_adjacent;
mod custom;
mod markov_predictor;
#[cfg(any(feature = "state-persistence", test))]
pub mod persistence;
mod predictive;
mod tick_allocation;
mod transition_counter;
mod transition_history;
mod uniform;

pub use active_only::ActiveOnly;
pub use active_plus_adjacent::ActivePlusAdjacent;
pub use custom::Custom;
pub use markov_predictor::{DecayConfig, MarkovPredictor, ScreenPrediction};
#[cfg(feature = "state-persistence")]
pub use persistence::{load_transitions, save_transitions};
// Note: persistence module also compiles under #[cfg(test)] since serde is in dev-deps.
pub use predictive::{Predictive, PredictiveStrategyConfig};
pub use tick_allocation::{AllocationCurve, TickAllocation};
pub use transition_counter::TransitionCounter;
pub use transition_history::{TransitionEntry, TransitionHistory};
pub use uniform::Uniform;

/// Decision returned by a [`TickStrategy`] for an inactive screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TickDecision {
    /// Tick this screen on this frame.
    Tick,
    /// Skip this screen on this frame.
    Skip,
}

/// Controls which inactive screens get ticked on each frame.
///
/// The runtime owns the invariant that the active screen is always ticked.
/// Implementations should assume `should_tick` is only called for inactive
/// screens that are still eligible for work.
pub trait TickStrategy: Send {
    /// Decide whether to tick an inactive screen on this frame.
    fn should_tick(
        &mut self,
        screen_id: &str,
        tick_count: u64,
        active_screen: &str,
    ) -> TickDecision;

    /// Called when the runtime observes a screen transition.
    fn on_screen_transition(&mut self, _from: &str, _to: &str) {}

    /// Called periodically for maintenance work.
    fn maintenance_tick(&mut self, _tick_count: u64) {}

    /// Called during clean shutdown.
    fn shutdown(&mut self) {}

    /// Human-readable strategy name for logs/debugging.
    fn name(&self) -> &str;

    /// Optional key-value debug stats.
    fn debug_stats(&self) -> Vec<(String, String)> {
        Vec::new()
    }
}

/// Minimal predictive strategy config used by [`TickStrategyKind`].
///
/// Full predictive tuning fields are added by follow-up tasks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PredictiveConfig {
    /// Fallback divisor when no predictive signal is available.
    pub fallback_divisor: u64,
}

impl PredictiveConfig {
    /// Construct a predictive config with an explicit fallback divisor.
    #[must_use]
    pub const fn new(fallback_divisor: u64) -> Self {
        Self { fallback_divisor }
    }

    #[must_use]
    const fn normalized_fallback_divisor(self) -> u64 {
        if self.fallback_divisor == 0 {
            1
        } else {
            self.fallback_divisor
        }
    }
}

impl Default for PredictiveConfig {
    fn default() -> Self {
        Self {
            fallback_divisor: 5,
        }
    }
}

/// Built-in strategy selection convenience enum.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TickStrategyKind {
    /// Tick only the active screen; all inactive screens are skipped.
    ActiveOnly,
    /// Tick all inactive screens every `divisor` frames.
    Uniform { divisor: u64 },
    /// Tick declared adjacent screens each frame; all others use a divisor.
    ActivePlusAdjacent {
        /// Screen ids adjacent to the active screen.
        screens: Vec<String>,
        /// Divisor for non-adjacent inactive screens.
        background_divisor: u64,
    },
    /// Predictive strategy using current config.
    Predictive { config: PredictiveConfig },
}

impl TickStrategyKind {
    #[must_use]
    const fn normalized_divisor(divisor: u64) -> u64 {
        if divisor == 0 { 1 } else { divisor }
    }
}

impl TickStrategy for TickStrategyKind {
    fn should_tick(
        &mut self,
        screen_id: &str,
        tick_count: u64,
        _active_screen: &str,
    ) -> TickDecision {
        match self {
            Self::ActiveOnly => TickDecision::Skip,
            Self::Uniform { divisor } => {
                if tick_count.is_multiple_of(Self::normalized_divisor(*divisor)) {
                    TickDecision::Tick
                } else {
                    TickDecision::Skip
                }
            }
            Self::ActivePlusAdjacent {
                screens,
                background_divisor,
            } => {
                if screens.iter().any(|adjacent| adjacent == screen_id)
                    || tick_count.is_multiple_of(Self::normalized_divisor(*background_divisor))
                {
                    TickDecision::Tick
                } else {
                    TickDecision::Skip
                }
            }
            Self::Predictive { config } => {
                if tick_count.is_multiple_of(config.normalized_fallback_divisor()) {
                    TickDecision::Tick
                } else {
                    TickDecision::Skip
                }
            }
        }
    }

    fn name(&self) -> &str {
        match self {
            Self::ActiveOnly => "ActiveOnly",
            Self::Uniform { .. } => "Uniform",
            Self::ActivePlusAdjacent { .. } => "ActivePlusAdjacent",
            Self::Predictive { .. } => "Predictive",
        }
    }

    fn debug_stats(&self) -> Vec<(String, String)> {
        match self {
            Self::ActiveOnly => vec![("strategy".into(), "ActiveOnly".into())],
            Self::Uniform { divisor } => vec![
                ("strategy".into(), "Uniform".into()),
                (
                    "divisor".into(),
                    Self::normalized_divisor(*divisor).to_string(),
                ),
            ],
            Self::ActivePlusAdjacent {
                screens,
                background_divisor,
            } => vec![
                ("strategy".into(), "ActivePlusAdjacent".into()),
                (
                    "background_divisor".into(),
                    Self::normalized_divisor(*background_divisor).to_string(),
                ),
                ("adjacent_screen_count".into(), screens.len().to_string()),
            ],
            Self::Predictive { config } => vec![
                ("strategy".into(), "Predictive".into()),
                (
                    "fallback_divisor".into(),
                    config.normalized_fallback_divisor().to_string(),
                ),
            ],
        }
    }
}

/// Implemented by [`Model`](crate::program::Model)s that manage multiple
/// screens and want per-screen tick control via [`TickStrategy`].
///
/// The runtime checks for this trait via
/// [`Model::as_screen_tick_dispatch`](crate::program::Model::as_screen_tick_dispatch).
/// When present, the runtime ticks individual screens instead of calling a
/// monolithic `update(Tick)`.
pub trait ScreenTickDispatch {
    /// Returns IDs of all currently registered screens.
    fn screen_ids(&self) -> Vec<String>;

    /// Returns the ID of the currently active/visible screen.
    fn active_screen_id(&self) -> String;

    /// Tick a specific screen by ID.
    ///
    /// Called by the runtime for each screen the [`TickStrategy`] approves.
    /// Unknown screen IDs should be silently ignored.
    fn tick_screen(&mut self, screen_id: &str, tick_count: u64);
}

#[cfg(test)]
mod tests {
    use super::{PredictiveConfig, TickDecision, TickStrategy, TickStrategyKind};

    struct NoopStrategy;

    impl TickStrategy for NoopStrategy {
        fn should_tick(
            &mut self,
            _screen_id: &str,
            _tick_count: u64,
            _active_screen: &str,
        ) -> TickDecision {
            TickDecision::Skip
        }

        fn name(&self) -> &str {
            "Noop"
        }
    }

    #[test]
    fn tick_decision_copy_and_eq() {
        let decision = TickDecision::Tick;
        let copied = decision;
        assert_eq!(copied, TickDecision::Tick);
        assert_ne!(TickDecision::Tick, TickDecision::Skip);
        assert!(format!("{decision:?}").contains("Tick"));
    }

    #[test]
    fn default_trait_hooks_are_noops() {
        let mut strategy = NoopStrategy;
        strategy.on_screen_transition("A", "B");
        strategy.maintenance_tick(123);
        strategy.shutdown();
        assert!(strategy.debug_stats().is_empty());
    }

    #[test]
    fn tick_strategy_kind_delegates_should_tick() {
        let mut active_only = TickStrategyKind::ActiveOnly;
        assert_eq!(
            active_only.should_tick("ScreenA", 10, "ScreenB"),
            TickDecision::Skip
        );

        let mut uniform = TickStrategyKind::Uniform { divisor: 5 };
        assert_eq!(
            uniform.should_tick("ScreenA", 10, "ScreenB"),
            TickDecision::Tick
        );
        assert_eq!(
            uniform.should_tick("ScreenA", 11, "ScreenB"),
            TickDecision::Skip
        );

        let mut uniform_zero = TickStrategyKind::Uniform { divisor: 0 };
        assert_eq!(
            uniform_zero.should_tick("ScreenA", 3, "ScreenB"),
            TickDecision::Tick
        );

        let mut active_plus_adjacent = TickStrategyKind::ActivePlusAdjacent {
            screens: vec!["Messages".into(), "Threads".into()],
            background_divisor: 4,
        };
        assert_eq!(
            active_plus_adjacent.should_tick("Messages", 1, "Dashboard"),
            TickDecision::Tick
        );
        assert_eq!(
            active_plus_adjacent.should_tick("Settings", 4, "Dashboard"),
            TickDecision::Tick
        );
        assert_eq!(
            active_plus_adjacent.should_tick("Settings", 5, "Dashboard"),
            TickDecision::Skip
        );

        let mut predictive = TickStrategyKind::Predictive {
            config: PredictiveConfig::new(3),
        };
        assert_eq!(
            predictive.should_tick("ScreenA", 6, "ScreenB"),
            TickDecision::Tick
        );
        assert_eq!(
            predictive.should_tick("ScreenA", 7, "ScreenB"),
            TickDecision::Skip
        );
    }

    #[test]
    fn tick_strategy_kind_names_are_stable() {
        assert_eq!(TickStrategyKind::ActiveOnly.name(), "ActiveOnly");
        assert_eq!(TickStrategyKind::Uniform { divisor: 5 }.name(), "Uniform");
        assert_eq!(
            TickStrategyKind::ActivePlusAdjacent {
                screens: vec![],
                background_divisor: 5,
            }
            .name(),
            "ActivePlusAdjacent"
        );
        assert_eq!(
            TickStrategyKind::Predictive {
                config: PredictiveConfig::default(),
            }
            .name(),
            "Predictive"
        );
    }

    #[test]
    fn predictive_default_config_matches_design() {
        assert_eq!(PredictiveConfig::default().fallback_divisor, 5);
    }

    // ========================================================================
    // ScreenTickDispatch tests
    // ========================================================================

    use super::ScreenTickDispatch;

    struct MockMultiScreen {
        active: String,
        screens: Vec<String>,
        ticked: Vec<(String, u64)>,
    }

    impl MockMultiScreen {
        fn new(active: &str, screens: &[&str]) -> Self {
            Self {
                active: active.to_owned(),
                screens: screens.iter().map(|s| (*s).to_owned()).collect(),
                ticked: Vec::new(),
            }
        }
    }

    impl ScreenTickDispatch for MockMultiScreen {
        fn screen_ids(&self) -> Vec<String> {
            self.screens.clone()
        }

        fn active_screen_id(&self) -> String {
            self.active.clone()
        }

        fn tick_screen(&mut self, screen_id: &str, tick_count: u64) {
            self.ticked.push((screen_id.to_owned(), tick_count));
        }
    }

    #[test]
    fn screen_tick_dispatch_returns_all_screens() {
        let mock = MockMultiScreen::new("A", &["A", "B", "C"]);
        assert_eq!(mock.screen_ids(), vec!["A", "B", "C"]);
    }

    #[test]
    fn screen_tick_dispatch_reports_active() {
        let mock = MockMultiScreen::new("B", &["A", "B", "C"]);
        assert_eq!(mock.active_screen_id(), "B");
    }

    #[test]
    fn screen_tick_dispatch_records_ticks() {
        let mut mock = MockMultiScreen::new("A", &["A", "B", "C"]);
        mock.tick_screen("B", 5);
        mock.tick_screen("C", 5);
        assert_eq!(mock.ticked.len(), 2);
        assert_eq!(mock.ticked[0], ("B".to_owned(), 5));
        assert_eq!(mock.ticked[1], ("C".to_owned(), 5));
    }

    #[test]
    fn screen_tick_dispatch_unknown_id_is_noop() {
        let mut mock = MockMultiScreen::new("A", &["A", "B"]);
        mock.tick_screen("UNKNOWN", 10);
        // Implementation records it; the trait contract says "silently ignore"
        // which the concrete impl decides. Our mock doesn't filter.
        assert_eq!(mock.ticked.len(), 1);
    }

    // ========================================================================
    // Cross-strategy invariant tests (I.4 coverage)
    // ========================================================================

    use super::{
        ActiveOnly, ActivePlusAdjacent, Custom, Predictive, PredictiveStrategyConfig, Uniform,
    };

    /// Compile-time assertion: all strategies implement Send (required by the
    /// `TickStrategy: Send` supertrait bound).
    #[test]
    fn all_strategies_implement_send() {
        fn assert_send<T: Send>() {}

        assert_send::<ActiveOnly>();
        assert_send::<Uniform>();
        assert_send::<ActivePlusAdjacent>();
        assert_send::<Predictive>();
        assert_send::<Custom>();
        assert_send::<TickStrategyKind>();
    }

    /// All strategies can be boxed as `dyn TickStrategy`.
    #[test]
    fn all_strategies_boxable_as_dyn_tick_strategy() {
        let strategies: Vec<Box<dyn TickStrategy>> = vec![
            Box::new(ActiveOnly),
            Box::new(Uniform::new(5)),
            Box::new(ActivePlusAdjacent::new(5)),
            Box::new(Predictive::new(PredictiveStrategyConfig::default())),
            Box::new(Custom::new("test", |_, _, _| TickDecision::Skip)),
            Box::new(TickStrategyKind::ActiveOnly),
        ];

        for mut s in strategies {
            // should_tick is callable through the trait object
            let _ = s.should_tick("screen", 0, "active");
            // name() is callable
            assert!(!s.name().is_empty());
        }
    }

    /// Default lifecycle hooks (on_screen_transition, maintenance_tick,
    /// shutdown) are no-ops for strategies that don't override them.
    #[test]
    fn lifecycle_hooks_are_safe_for_all_strategies() {
        let mut strategies: Vec<Box<dyn TickStrategy>> = vec![
            Box::new(ActiveOnly),
            Box::new(Uniform::new(5)),
            Box::new(ActivePlusAdjacent::new(5)),
            Box::new(Custom::new("test", |_, _, _| TickDecision::Skip)),
            Box::new(TickStrategyKind::Uniform { divisor: 5 }),
        ];

        for s in &mut strategies {
            s.on_screen_transition("A", "B");
            s.maintenance_tick(100);
            s.shutdown();
            // No panics = success
        }
    }
}
