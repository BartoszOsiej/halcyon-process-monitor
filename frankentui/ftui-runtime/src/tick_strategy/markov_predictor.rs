//! Markov-chain prediction API for screen transitions.
//!
//! [`MarkovPredictor<S>`] wraps a [`TransitionCounter<S>`] and provides a clean
//! prediction interface with confidence-weighted probabilities. The Predictive
//! tick strategy (C.5) queries this to determine tick rates for each screen.
//!
//! # Confidence model
//!
//! Predictions blend between uniform (cold start) and observed (warm) based on
//! the number of transitions recorded from the current screen:
//!
//! ```text
//! confidence = min(1.0, observations / min_observations)
//! effective  = confidence * predicted + (1 - confidence) * uniform
//! ```
//!
//! This ensures smooth warm-up from equal-probability to learned behavior.

use std::hash::Hash;

use super::TransitionCounter;

/// Default minimum observations before predictions are fully trusted.
const DEFAULT_MIN_OBSERVATIONS: u64 = 20;

/// Configuration for automatic temporal decay in [`MarkovPredictor`].
///
/// Decay is triggered every `interval` calls to `record_transition()`,
/// using a separate monotonic u64 counter (not the f64 total, which
/// shrinks after each decay).
#[derive(Debug, Clone)]
pub struct DecayConfig {
    /// Multiplicative decay factor applied to all counts (0.0..1.0).
    pub factor: f64,
    /// Number of `record_transition()` calls between decay rounds.
    pub interval: u64,
}

impl Default for DecayConfig {
    fn default() -> Self {
        Self {
            factor: 0.85,
            interval: 500,
        }
    }
}

/// Internal state tracking automatic decay scheduling.
#[derive(Debug, Clone)]
struct DecayState {
    config: Option<DecayConfig>,
    transitions_since_last_decay: u64,
}

impl DecayState {
    fn disabled() -> Self {
        Self {
            config: None,
            transitions_since_last_decay: 0,
        }
    }

    fn with_config(config: DecayConfig) -> Self {
        Self {
            config: Some(config),
            transitions_since_last_decay: 0,
        }
    }

    /// Increment the call counter and trigger decay if the interval is reached.
    /// Returns `true` if decay was performed.
    fn maybe_decay<S: Eq + Hash + Clone>(&mut self, counter: &mut TransitionCounter<S>) -> bool {
        let config = match &self.config {
            Some(c) => c,
            None => return false,
        };

        self.transitions_since_last_decay += 1;
        if self.transitions_since_last_decay >= config.interval {
            counter.decay(config.factor);
            self.transitions_since_last_decay = 0;
            true
        } else {
            false
        }
    }
}

/// A single screen prediction with probability and confidence.
#[derive(Debug, Clone)]
pub struct ScreenPrediction<S> {
    /// The predicted next screen.
    pub screen: S,
    /// Probability of transitioning to this screen (0.0..1.0).
    pub probability: f64,
    /// Confidence in this prediction (0.0..1.0), based on observation count.
    pub confidence: f64,
}

/// Markov-chain predictor wrapping a [`TransitionCounter`].
///
/// Provides prediction queries with confidence-aware blending between
/// observed transition probabilities and a uniform fallback.
#[derive(Debug, Clone)]
pub struct MarkovPredictor<S: Eq + Hash + Clone> {
    counter: TransitionCounter<S>,
    min_observations: u64,
    decay_state: DecayState,
}

impl<S: Eq + Hash + Clone> MarkovPredictor<S> {
    /// Create a predictor with default settings (min_observations=20, no auto-decay).
    #[must_use]
    pub fn new() -> Self {
        Self {
            counter: TransitionCounter::new(),
            min_observations: DEFAULT_MIN_OBSERVATIONS,
            decay_state: DecayState::disabled(),
        }
    }

    /// Create a predictor with a custom observation threshold (no auto-decay).
    #[must_use]
    pub fn with_min_observations(n: u64) -> Self {
        Self {
            counter: TransitionCounter::new(),
            min_observations: n.max(1),
            decay_state: DecayState::disabled(),
        }
    }

    /// Create a predictor with a pre-loaded counter (e.g., from persistence).
    #[must_use]
    pub fn with_counter(counter: TransitionCounter<S>, min_observations: u64) -> Self {
        Self {
            counter,
            min_observations: min_observations.max(1),
            decay_state: DecayState::disabled(),
        }
    }

    /// Enable automatic temporal decay on `record_transition()` calls.
    ///
    /// Every `config.interval` record calls, all counts are multiplied by
    /// `config.factor`, causing old transition data to fade over time.
    pub fn enable_auto_decay(&mut self, config: DecayConfig) {
        self.decay_state = DecayState::with_config(config);
    }

    /// Record a screen transition.
    ///
    /// If auto-decay is enabled, this may trigger a decay cycle after
    /// every `interval` calls.
    pub fn record_transition(&mut self, from: S, to: S) {
        self.counter.record(from, to);
        self.decay_state.maybe_decay(&mut self.counter);
    }

    /// Predict likely next screens from `current_screen`.
    ///
    /// Returns predictions sorted by effective probability (descending).
    /// Each prediction blends observed and uniform probabilities based on
    /// confidence.
    #[must_use]
    pub fn predict(&self, current_screen: &S) -> Vec<ScreenPrediction<S>> {
        let confidence = self.confidence(current_screen);
        let ranked = self.counter.all_targets_ranked(current_screen);

        if ranked.is_empty() {
            return Vec::new();
        }

        let n_targets = ranked.len() as f64;
        let uniform_prob = 1.0 / n_targets;

        let mut predictions: Vec<ScreenPrediction<S>> = ranked
            .into_iter()
            .map(|(screen, raw_prob)| {
                let effective = confidence * raw_prob + (1.0 - confidence) * uniform_prob;
                ScreenPrediction {
                    screen,
                    probability: effective,
                    confidence,
                }
            })
            .collect();

        // Re-sort by effective probability (blending can change order during warm-up)
        predictions.sort_by(|a, b| {
            b.probability
                .partial_cmp(&a.probability)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        predictions
    }

    /// Check whether predictions from `screen` have insufficient data.
    ///
    /// Returns `true` if fewer than `min_observations` transitions have
    /// been recorded from this screen.
    #[must_use]
    pub fn is_cold_start(&self, screen: &S) -> bool {
        (self.counter.total_from(screen) as u64) < self.min_observations
    }

    /// Confidence level for predictions from `screen` (0.0..1.0).
    ///
    /// Grows linearly from 0 to 1 as observations approach `min_observations`.
    #[must_use]
    pub fn confidence(&self, screen: &S) -> f64 {
        let observations = self.counter.total_from(screen);
        (observations / self.min_observations as f64).min(1.0)
    }

    /// Access the underlying transition counter.
    #[must_use]
    pub fn counter(&self) -> &TransitionCounter<S> {
        &self.counter
    }

    /// Mutable access to the transition counter (for merging, decay, etc.).
    pub fn counter_mut(&mut self) -> &mut TransitionCounter<S> {
        &mut self.counter
    }

    /// Return the minimum observations threshold.
    #[must_use]
    pub fn min_observations(&self) -> u64 {
        self.min_observations
    }
}

impl<S: Eq + Hash + Clone> Default for MarkovPredictor<S> {
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
    fn cold_start_returns_uniform_distribution() {
        let mut mp = MarkovPredictor::with_min_observations(20);
        // Record just 1 transition (well below threshold of 20)
        mp.record_transition("a", "b");
        mp.record_transition("a", "c");

        let preds = mp.predict(&"a");
        assert_eq!(preds.len(), 2);

        // With only 2 observations out of 20 needed, confidence = 2/20 = 0.1
        // Effective probs should be close to uniform (0.5 each)
        let diff = (preds[0].probability - preds[1].probability).abs();
        assert!(
            diff < 0.15,
            "cold start should be near-uniform, diff={diff}"
        );
    }

    #[test]
    fn warm_predictions_match_observed() {
        let mut mp = MarkovPredictor::with_min_observations(10);

        // Record 30 total transitions (well above threshold)
        for _ in 0..20 {
            mp.record_transition("a", "b");
        }
        for _ in 0..10 {
            mp.record_transition("a", "c");
        }

        let preds = mp.predict(&"a");
        assert_eq!(preds.len(), 2);

        // Confidence should be 1.0 (30 >= 10)
        assert!((preds[0].confidence - 1.0).abs() < 1e-10);

        // "b" should have higher probability than "c"
        assert_eq!(preds[0].screen, "b");
        assert!(preds[0].probability > preds[1].probability);

        // With full confidence, effective = raw probability
        // P(b|a) with smoothing: (20+1)/(30+2) ≈ 0.656
        // P(c|a) with smoothing: (10+1)/(30+2) ≈ 0.344
        assert!((preds[0].probability - 21.0 / 32.0).abs() < 1e-10);
        assert!((preds[1].probability - 11.0 / 32.0).abs() < 1e-10);
    }

    #[test]
    fn confidence_increases_with_observations() {
        let mut mp = MarkovPredictor::with_min_observations(10);

        assert_eq!(mp.confidence(&"x"), 0.0); // no data

        mp.record_transition("x", "y");
        assert!((mp.confidence(&"x") - 0.1).abs() < 1e-10); // 1/10

        for _ in 0..4 {
            mp.record_transition("x", "y");
        }
        assert!((mp.confidence(&"x") - 0.5).abs() < 1e-10); // 5/10

        for _ in 0..5 {
            mp.record_transition("x", "y");
        }
        assert!((mp.confidence(&"x") - 1.0).abs() < 1e-10); // 10/10
    }

    #[test]
    fn confidence_caps_at_one() {
        let mut mp = MarkovPredictor::with_min_observations(5);
        for _ in 0..100 {
            mp.record_transition("a", "b");
        }
        assert!((mp.confidence(&"a") - 1.0).abs() < 1e-10);
    }

    #[test]
    fn is_cold_start_reflects_threshold() {
        let mut mp = MarkovPredictor::with_min_observations(5);
        assert!(mp.is_cold_start(&"x"));

        for _ in 0..4 {
            mp.record_transition("x", "y");
        }
        assert!(mp.is_cold_start(&"x")); // 4 < 5

        mp.record_transition("x", "y");
        assert!(!mp.is_cold_start(&"x")); // 5 >= 5
    }

    #[test]
    fn empty_predictor_returns_no_predictions() {
        let mp: MarkovPredictor<&str> = MarkovPredictor::new();
        let preds = mp.predict(&"x");
        assert!(preds.is_empty());
    }

    #[test]
    fn predictions_sorted_by_probability() {
        let mut mp = MarkovPredictor::with_min_observations(5);
        for _ in 0..10 {
            mp.record_transition("a", "x");
        }
        for _ in 0..5 {
            mp.record_transition("a", "y");
        }
        for _ in 0..1 {
            mp.record_transition("a", "z");
        }

        let preds = mp.predict(&"a");
        assert_eq!(preds.len(), 3);
        assert!(preds[0].probability >= preds[1].probability);
        assert!(preds[1].probability >= preds[2].probability);
    }

    #[test]
    fn probabilities_sum_to_approximately_one() {
        let mut mp = MarkovPredictor::with_min_observations(10);
        for _ in 0..15 {
            mp.record_transition("a", "b");
        }
        for _ in 0..8 {
            mp.record_transition("a", "c");
        }
        for _ in 0..3 {
            mp.record_transition("a", "d");
        }

        let preds = mp.predict(&"a");
        let sum: f64 = preds.iter().map(|p| p.probability).sum();
        assert!(
            (sum - 1.0).abs() < 1e-10,
            "probabilities should sum to 1.0, got {sum}"
        );
    }

    #[test]
    fn counter_access() {
        let mut mp = MarkovPredictor::<&str>::new();
        mp.record_transition("a", "b");

        assert_eq!(mp.counter().total(), 1.0);
        assert_eq!(mp.counter().count(&"a", &"b"), 1.0);
    }

    #[test]
    fn counter_mut_access() {
        let mut mp = MarkovPredictor::<&str>::new();
        mp.record_transition("a", "b");

        // Merge via mutable access
        let mut other = TransitionCounter::new();
        other.record("a", "c");
        mp.counter_mut().merge(&other);

        assert_eq!(mp.counter().total(), 2.0);
    }

    #[test]
    fn with_counter_constructor() {
        let mut counter = TransitionCounter::new();
        for _ in 0..50 {
            counter.record("a", "b");
        }

        let mp = MarkovPredictor::with_counter(counter, 10);
        assert!(!mp.is_cold_start(&"a"));
        assert_eq!(mp.min_observations(), 10);
    }

    #[test]
    fn default_impl() {
        let mp: MarkovPredictor<String> = MarkovPredictor::default();
        assert_eq!(mp.min_observations(), DEFAULT_MIN_OBSERVATIONS);
        assert_eq!(mp.counter().total(), 0.0);
    }

    // ========================================================================
    // Additional tests (I.2 coverage)
    // ========================================================================

    #[test]
    fn predict_returns_all_known_targets() {
        let mut mp = MarkovPredictor::with_min_observations(5);
        mp.record_transition("a", "b");
        mp.record_transition("a", "c");
        mp.record_transition("a", "d");

        let preds = mp.predict(&"a");
        let screens: Vec<_> = preds.iter().map(|p| p.screen).collect();
        eprintln!("predicted screens: {screens:?}");
        assert_eq!(preds.len(), 3);
        assert!(screens.contains(&"b"));
        assert!(screens.contains(&"c"));
        assert!(screens.contains(&"d"));
    }

    #[test]
    fn predict_zero_outgoing_returns_empty() {
        let mut mp = MarkovPredictor::with_min_observations(5);
        // "x" has never been a source — only recorded as a target
        mp.record_transition("a", "x");

        let preds = mp.predict(&"x");
        eprintln!("predictions from unseen source: len={}", preds.len());
        assert!(preds.is_empty());
    }

    #[test]
    fn record_transition_updates_predictions() {
        let mut mp = MarkovPredictor::with_min_observations(5);
        mp.record_transition("a", "b");

        let preds_before = mp.predict(&"a");
        assert_eq!(preds_before.len(), 1);
        assert_eq!(preds_before[0].screen, "b");

        // Add a new target
        mp.record_transition("a", "c");
        let preds_after = mp.predict(&"a");
        eprintln!(
            "before: {} predictions, after: {} predictions",
            preds_before.len(),
            preds_after.len()
        );
        assert_eq!(preds_after.len(), 2);
        let screens: Vec<_> = preds_after.iter().map(|p| p.screen).collect();
        assert!(screens.contains(&"b"));
        assert!(screens.contains(&"c"));
    }

    #[test]
    fn predictions_change_with_new_transitions() {
        let mut mp = MarkovPredictor::with_min_observations(5);
        for _ in 0..10 {
            mp.record_transition("a", "b");
        }
        mp.record_transition("a", "c");

        let preds1 = mp.predict(&"a");
        let prob_b1 = preds1.iter().find(|p| p.screen == "b").unwrap().probability;
        let prob_c1 = preds1.iter().find(|p| p.screen == "c").unwrap().probability;

        // Record many more transitions to "c" to shift the distribution
        for _ in 0..50 {
            mp.record_transition("a", "c");
        }

        let preds2 = mp.predict(&"a");
        let prob_b2 = preds2.iter().find(|p| p.screen == "b").unwrap().probability;
        let prob_c2 = preds2.iter().find(|p| p.screen == "c").unwrap().probability;

        eprintln!("before: P(b)={prob_b1:.4}, P(c)={prob_c1:.4}");
        eprintln!("after:  P(b)={prob_b2:.4}, P(c)={prob_c2:.4}");

        // "c" should now be more probable than before
        assert!(
            prob_c2 > prob_c1,
            "P(c) should increase with more transitions"
        );
        // "b" should now be less probable
        assert!(prob_b2 < prob_b1, "P(b) should decrease as c dominates");
    }

    #[test]
    fn decay_via_counter_reduces_old_influence() {
        let mut mp = MarkovPredictor::with_min_observations(5);
        // Record many "a→b" transitions
        for _ in 0..20 {
            mp.record_transition("a", "b");
        }
        mp.record_transition("a", "c");

        let preds_before = mp.predict(&"a");
        let prob_b_before = preds_before
            .iter()
            .find(|p| p.screen == "b")
            .unwrap()
            .probability;

        // Decay the counter heavily
        mp.counter_mut().decay(0.1);

        // Now add fresh transitions to "c"
        for _ in 0..5 {
            mp.record_transition("a", "c");
        }

        let preds_after = mp.predict(&"a");
        let prob_c_after = preds_after
            .iter()
            .find(|p| p.screen == "c")
            .unwrap()
            .probability;

        eprintln!("before decay: P(b)={prob_b_before:.4}, after fresh c: P(c)={prob_c_after:.4}");
        // After heavy decay + fresh c transitions, c should be dominant
        assert!(
            prob_c_after > prob_b_before * 0.5,
            "fresh transitions after decay should be influential"
        );
    }

    #[test]
    fn decay_shifts_predictions_toward_recent() {
        let mut mp = MarkovPredictor::with_min_observations(5);

        // Phase 1: "b" dominant
        for _ in 0..20 {
            mp.record_transition("a", "b");
        }
        for _ in 0..5 {
            mp.record_transition("a", "c");
        }

        let p1 = mp.predict(&"a");
        let p1_b = p1.iter().find(|p| p.screen == "b").unwrap().probability;

        // Decay heavily to diminish phase 1
        mp.counter_mut().decay(0.1);

        // Phase 2: "c" dominant
        for _ in 0..20 {
            mp.record_transition("a", "c");
        }
        for _ in 0..5 {
            mp.record_transition("a", "b");
        }

        let p2 = mp.predict(&"a");
        let p2_c = p2.iter().find(|p| p.screen == "c").unwrap().probability;

        eprintln!("phase1 P(b)={p1_b:.4}, phase2 P(c)={p2_c:.4}");
        // After decay + new data, c should now dominate
        assert!(
            p2_c > 0.5,
            "recent pattern should dominate after decay, got P(c)={p2_c}"
        );
    }

    #[test]
    fn screen_prediction_fields_are_populated() {
        let mut mp = MarkovPredictor::with_min_observations(10);
        for _ in 0..5 {
            mp.record_transition("a", "b");
        }
        mp.record_transition("a", "c");

        let preds = mp.predict(&"a");
        for pred in &preds {
            eprintln!(
                "screen={}, prob={:.4}, conf={:.4}",
                pred.screen, pred.probability, pred.confidence
            );
            // Probability is always positive
            assert!(pred.probability > 0.0, "probability should be > 0");
            assert!(pred.probability <= 1.0, "probability should be <= 1.0");
            // Confidence is in [0, 1]
            assert!(pred.confidence >= 0.0, "confidence should be >= 0");
            assert!(pred.confidence <= 1.0, "confidence should be <= 1.0");
        }
    }

    #[test]
    fn confidence_always_in_unit_range() {
        let mut mp = MarkovPredictor::with_min_observations(10);

        // 0 observations
        let c0 = mp.confidence(&"x");
        assert!((0.0..=1.0).contains(&c0), "confidence={c0}");

        // Partial observations
        for i in 1..=20 {
            mp.record_transition("x", "y");
            let c = mp.confidence(&"x");
            eprintln!("obs={i}, confidence={c:.4}");
            assert!((0.0..=1.0).contains(&c), "confidence out of range: {c}");
        }
    }

    #[test]
    fn probability_always_positive_with_smoothing() {
        let mut mp = MarkovPredictor::with_min_observations(5);
        for _ in 0..100 {
            mp.record_transition("a", "b");
        }
        mp.record_transition("a", "c");

        let preds = mp.predict(&"a");
        for pred in &preds {
            eprintln!("screen={}, prob={:.6}", pred.screen, pred.probability);
            assert!(
                pred.probability > 0.0,
                "all probabilities should be > 0 due to smoothing"
            );
        }
    }

    #[test]
    fn blending_transitions_smoothly() {
        // With min_observations=10, after 5 transitions we're at 50% confidence.
        // The predictions should blend 50% observed + 50% uniform.
        let mut mp = MarkovPredictor::with_min_observations(10);

        // 4 transitions A→B, 1 transition A→C (total from A = 5)
        for _ in 0..4 {
            mp.record_transition("a", "b");
        }
        mp.record_transition("a", "c");

        let preds = mp.predict(&"a");
        assert_eq!(preds.len(), 2);

        // confidence = 5/10 = 0.5
        let conf = mp.confidence(&"a");
        assert!((conf - 0.5).abs() < 1e-10);

        // Raw P(b|a) with smoothing: (4+1)/(5+2) = 5/7 ≈ 0.714
        // Raw P(c|a) with smoothing: (1+1)/(5+2) = 2/7 ≈ 0.286
        // Uniform: 0.5 each
        // Effective P(b) = 0.5 * 5/7 + 0.5 * 0.5 = 5/14 + 1/4 = 10/28 + 7/28 = 17/28 ≈ 0.607
        // Effective P(c) = 0.5 * 2/7 + 0.5 * 0.5 = 2/14 + 1/4 = 4/28 + 7/28 = 11/28 ≈ 0.393
        let expected_b = 0.5 * (5.0 / 7.0) + 0.5 * 0.5;
        let expected_c = 0.5 * (2.0 / 7.0) + 0.5 * 0.5;

        assert_eq!(preds[0].screen, "b");
        assert!(
            (preds[0].probability - expected_b).abs() < 1e-10,
            "expected {expected_b}, got {}",
            preds[0].probability
        );
        assert!(
            (preds[1].probability - expected_c).abs() < 1e-10,
            "expected {expected_c}, got {}",
            preds[1].probability
        );

        // Sum should still be ~1.0
        let sum: f64 = preds.iter().map(|p| p.probability).sum();
        assert!((sum - 1.0).abs() < 1e-10);
    }

    // ========================================================================
    // Auto-decay tests (B.4 coverage)
    // ========================================================================

    #[test]
    fn auto_decay_triggers_at_interval() {
        let mut mp = MarkovPredictor::with_min_observations(5);
        mp.enable_auto_decay(DecayConfig {
            factor: 0.5,
            interval: 10,
        });

        // Record 9 transitions — not enough to trigger decay
        for _ in 0..9 {
            mp.record_transition("a", "b");
        }
        // With f64 counts, 9 records at count=1.0 each → total = 9.0
        assert_eq!(mp.counter().total(), 9.0);

        // 10th record triggers decay: (9+1)*0.5 = 5.0 total
        mp.record_transition("a", "b");
        let total = mp.counter().total();
        eprintln!("after 10 transitions with decay(0.5): total={total}");
        assert!(
            (total - 5.0).abs() < 1e-9,
            "expected ~5.0 after decay, got {total}"
        );
    }

    #[test]
    fn auto_decay_interval_resets_after_each_cycle() {
        let mut mp = MarkovPredictor::with_min_observations(5);
        mp.enable_auto_decay(DecayConfig {
            factor: 0.5,
            interval: 5,
        });

        // First 5 transitions → decay
        for _ in 0..5 {
            mp.record_transition("a", "b");
        }
        let after_first = mp.counter().total();
        eprintln!("after first decay: {after_first}");
        assert!((after_first - 2.5).abs() < 1e-9);

        // Next 5 transitions → second decay
        for _ in 0..5 {
            mp.record_transition("a", "b");
        }
        let after_second = mp.counter().total();
        eprintln!("after second decay: {after_second}");
        // (2.5 + 5) * 0.5 = 3.75
        assert!(
            (after_second - 3.75).abs() < 1e-9,
            "expected ~3.75, got {after_second}"
        );
    }

    #[test]
    fn auto_decay_disabled_by_default() {
        let mut mp = MarkovPredictor::with_min_observations(5);

        // No auto-decay enabled — counts should grow linearly
        for _ in 0..100 {
            mp.record_transition("a", "b");
        }
        assert_eq!(mp.counter().total(), 100.0);
    }

    #[test]
    fn auto_decay_recent_transitions_dominate() {
        let mut mp = MarkovPredictor::with_min_observations(5);
        mp.enable_auto_decay(DecayConfig {
            factor: 0.1, // aggressive decay
            interval: 20,
        });

        // Phase 1: "b" dominant (20 transitions → triggers decay)
        for _ in 0..20 {
            mp.record_transition("a", "b");
        }

        // After decay: all old counts * 0.1
        let b_after_decay = mp.counter().count(&"a", &"b");
        eprintln!("b after first decay: {b_after_decay}");

        // Phase 2: "c" dominant
        for _ in 0..15 {
            mp.record_transition("a", "c");
        }

        // "c" should have higher count than "b" (which was decayed)
        let b_count = mp.counter().count(&"a", &"b");
        let c_count = mp.counter().count(&"a", &"c");
        eprintln!("b_count={b_count}, c_count={c_count}");
        assert!(
            c_count > b_count,
            "recent 'c' transitions ({c_count}) should exceed decayed 'b' ({b_count})"
        );
    }

    #[test]
    fn auto_decay_counter_consistency() {
        let mut mp = MarkovPredictor::with_min_observations(5);
        mp.enable_auto_decay(DecayConfig {
            factor: 0.8,
            interval: 10,
        });

        // Record many transitions across multiple sources
        for _ in 0..30 {
            mp.record_transition("a", "b");
            mp.record_transition("a", "c");
            mp.record_transition("x", "y");
        }

        // Verify total consistency: total should equal sum of all counts
        let total = mp.counter().total();
        let mut sum = 0.0;
        for from in mp.counter().state_ids() {
            for (to, _) in mp.counter().all_targets_ranked(&from) {
                sum += mp.counter().count(&from, &to);
            }
        }
        eprintln!("total={total}, sum={sum}");
        assert!(
            (total - sum).abs() < 1e-6,
            "total({total}) should match sum of counts({sum})"
        );
    }

    #[test]
    fn decay_config_default() {
        let config = DecayConfig::default();
        assert!((config.factor - 0.85).abs() < 1e-10);
        assert_eq!(config.interval, 500);
    }
}
