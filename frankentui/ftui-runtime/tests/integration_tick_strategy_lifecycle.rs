//! Integration test: full lifecycle with MockMultiScreenModel.
//!
//! Exercises the entire tick strategy system end-to-end: strategy creation,
//! screen transition detection, tick dispatch, prediction learning,
//! and persistence round-trip.

use std::collections::HashMap;

use ftui_runtime::{
    ActiveOnly, Predictive, PredictiveStrategyConfig, ScreenTickDispatch, TickDecision,
    TickStrategy, TickStrategyKind, Uniform,
};

// ── MockMultiScreenModel ──────────────────────────────────────────────────

struct MockMultiScreenModel {
    screens: Vec<String>,
    active: usize,
    tick_counts: HashMap<String, u64>,
    total_ticks: u64,
}

impl MockMultiScreenModel {
    fn new(screen_names: &[&str]) -> Self {
        Self {
            screens: screen_names.iter().map(|s| s.to_string()).collect(),
            active: 0,
            tick_counts: HashMap::new(),
            total_ticks: 0,
        }
    }

    fn switch_to(&mut self, screen_name: &str) {
        if let Some(idx) = self.screens.iter().position(|s| s == screen_name) {
            self.active = idx;
        }
    }

    fn log_distribution(&self, label: &str) {
        eprintln!("=== Tick distribution: {label} ===");
        let mut entries: Vec<_> = self.tick_counts.iter().collect();
        entries.sort_by_key(|(name, _)| (*name).clone());
        for (screen, count) in entries {
            let pct = if self.total_ticks > 0 {
                *count as f64 / self.total_ticks as f64 * 100.0
            } else {
                0.0
            };
            eprintln!("  {screen}: {count} ticks ({pct:.1}%)");
        }
        eprintln!("  total: {} ticks", self.total_ticks);
    }
}

impl ScreenTickDispatch for MockMultiScreenModel {
    fn screen_ids(&self) -> Vec<String> {
        self.screens.clone()
    }

    fn active_screen_id(&self) -> String {
        self.screens[self.active].clone()
    }

    fn tick_screen(&mut self, screen_id: &str, _tick_count: u64) {
        *self.tick_counts.entry(screen_id.to_string()).or_default() += 1;
        self.total_ticks += 1;
    }
}

// ── Simulation helper ─────────────────────────────────────────────────────

/// Simulate the runtime tick dispatch loop for `frames` frames.
///
/// This mirrors the logic in program.rs: active screen always ticks,
/// inactive screens tick according to the strategy.
fn simulate_frames(
    model: &mut MockMultiScreenModel,
    strategy: &mut dyn TickStrategy,
    frames: u64,
    start_tick: u64,
) {
    for tick in start_tick..start_tick + frames {
        let active = model.active_screen_id();
        let all_screens = model.screen_ids();

        // Active screen always ticks
        model.tick_screen(&active, tick);

        // Inactive screens consult the strategy
        for screen_id in &all_screens {
            if *screen_id != active
                && strategy.should_tick(screen_id, tick, &active) == TickDecision::Tick
            {
                model.tick_screen(screen_id, tick);
            }
        }

        // Maintenance
        strategy.maintenance_tick(tick);
    }
}

// ── Test 1: No strategy baseline ──────────────────────────────────────────

#[test]
fn baseline_no_strategy_all_screens_tick_every_frame() {
    let mut model = MockMultiScreenModel::new(&["A", "B", "C", "D"]);
    let frames = 100;

    // Without a strategy, simulate "tick everything every frame"
    for tick in 0..frames {
        for screen in &model.screens.clone() {
            model.tick_screen(screen, tick);
        }
    }

    model.log_distribution("baseline (no strategy)");

    // Every screen should get exactly `frames` ticks
    for screen in &model.screens {
        assert_eq!(
            model.tick_counts.get(screen).copied().unwrap_or(0),
            frames,
            "screen {screen} should have {frames} ticks"
        );
    }
    assert_eq!(model.total_ticks, frames * 4);
}

// ── Test 2: ActiveOnly ────────────────────────────────────────────────────

#[test]
fn active_only_strategy_ticks_only_active() {
    let mut model = MockMultiScreenModel::new(&["A", "B", "C", "D"]);
    let mut strategy = ActiveOnly;
    let frames = 100;

    simulate_frames(&mut model, &mut strategy, frames, 0);
    model.log_distribution("ActiveOnly");

    // Only active screen ("A") should have ticks
    assert_eq!(model.tick_counts.get("A").copied().unwrap_or(0), frames);
    assert_eq!(model.tick_counts.get("B").copied().unwrap_or(0), 0);
    assert_eq!(model.tick_counts.get("C").copied().unwrap_or(0), 0);
    assert_eq!(model.tick_counts.get("D").copied().unwrap_or(0), 0);
    assert_eq!(model.total_ticks, frames);
}

// ── Test 3: Uniform(5) ───────────────────────────────────────────────────

#[test]
fn uniform_strategy_ticks_inactive_at_divisor_rate() {
    let mut model = MockMultiScreenModel::new(&["A", "B", "C"]);
    let mut strategy = Uniform::new(5);
    let frames = 100;

    simulate_frames(&mut model, &mut strategy, frames, 0);
    model.log_distribution("Uniform(5)");

    // Active "A" gets every frame
    assert_eq!(model.tick_counts.get("A").copied().unwrap_or(0), frames);

    // Inactive screens get every 5th frame: 0,5,10,...95 = 20 ticks
    let expected_inactive = frames / 5;
    assert_eq!(
        model.tick_counts.get("B").copied().unwrap_or(0),
        expected_inactive
    );
    assert_eq!(
        model.tick_counts.get("C").copied().unwrap_or(0),
        expected_inactive
    );
}

// ── Test 4: Predictive cold start ─────────────────────────────────────────

#[test]
fn predictive_cold_start_uses_fallback_divisor() {
    let mut model = MockMultiScreenModel::new(&["A", "B", "C"]);
    let config = PredictiveStrategyConfig {
        fallback_divisor: 10,
        min_observations: 50, // high threshold → cold start
        ..PredictiveStrategyConfig::default()
    };
    let mut strategy = Predictive::new(config);
    let frames = 100;

    simulate_frames(&mut model, &mut strategy, frames, 0);
    model.log_distribution("Predictive (cold start, fallback=10)");

    // Active gets every frame
    assert_eq!(model.tick_counts.get("A").copied().unwrap_or(0), frames);

    // Inactive screens use fallback divisor of 10 → 10 ticks each
    let expected_inactive = frames / 10;
    assert_eq!(
        model.tick_counts.get("B").copied().unwrap_or(0),
        expected_inactive
    );
    assert_eq!(
        model.tick_counts.get("C").copied().unwrap_or(0),
        expected_inactive
    );
}

// ── Test 5: Predictive warm ───────────────────────────────────────────────

#[test]
fn predictive_warm_favors_likely_targets() {
    let mut model = MockMultiScreenModel::new(&["A", "B", "C", "D"]);
    let config = PredictiveStrategyConfig {
        min_observations: 5,
        fallback_divisor: 20,
        decay_interval: 10_000, // no decay during test
        ..PredictiveStrategyConfig::default()
    };
    let mut strategy = Predictive::new(config);

    // Train: A→B (80%), A→C (15%), A→D (5%)
    for _ in 0..80 {
        strategy.on_screen_transition("A", "B");
    }
    for _ in 0..15 {
        strategy.on_screen_transition("A", "C");
    }
    for _ in 0..5 {
        strategy.on_screen_transition("A", "D");
    }

    let frames = 200;
    simulate_frames(&mut model, &mut strategy, frames, 0);
    model.log_distribution("Predictive (warm, A active)");

    let b_ticks = model.tick_counts.get("B").copied().unwrap_or(0);
    let c_ticks = model.tick_counts.get("C").copied().unwrap_or(0);
    let d_ticks = model.tick_counts.get("D").copied().unwrap_or(0);

    // B (80%) should tick the most among inactive screens
    assert!(
        b_ticks > c_ticks,
        "B ({b_ticks}) should tick more than C ({c_ticks})"
    );
    assert!(
        b_ticks > d_ticks,
        "B ({b_ticks}) should tick more than D ({d_ticks})"
    );
    // C (15%) should tick more than D (5%)
    assert!(
        c_ticks >= d_ticks,
        "C ({c_ticks}) should tick >= D ({d_ticks})"
    );
}

// ── Test 6: Screen switch force-tick ──────────────────────────────────────

#[test]
fn screen_switch_changes_active_ticking() {
    let mut model = MockMultiScreenModel::new(&["A", "B", "C"]);
    let mut strategy = ActiveOnly;

    // Run 50 frames with A active
    simulate_frames(&mut model, &mut strategy, 50, 0);

    let a_ticks_before = model.tick_counts.get("A").copied().unwrap_or(0);
    assert_eq!(a_ticks_before, 50);
    assert_eq!(model.tick_counts.get("B").copied().unwrap_or(0), 0);

    // Switch to B
    model.switch_to("B");

    // Simulate the force-tick that the runtime does on screen transition
    model.tick_screen("B", 50);

    // Run 50 more frames with B active
    simulate_frames(&mut model, &mut strategy, 50, 50);

    model.log_distribution("screen switch A→B");

    // A had 50 ticks before switch, 0 after (ActiveOnly)
    assert_eq!(model.tick_counts.get("A").copied().unwrap_or(0), 50);
    // B had 0 before, 1 force-tick + 50 regular = 51
    assert_eq!(model.tick_counts.get("B").copied().unwrap_or(0), 51);
}

// ── Test 7: Screen switch updates predictions ─────────────────────────────

#[test]
fn screen_switch_updates_predictions() {
    let config = PredictiveStrategyConfig {
        min_observations: 5,
        fallback_divisor: 20,
        decay_interval: 10_000,
        ..PredictiveStrategyConfig::default()
    };
    let mut strategy = Predictive::new(config);

    // Record 50 A→B transitions
    for _ in 0..50 {
        strategy.on_screen_transition("A", "B");
    }

    // Predictions from A should heavily favor B
    let predictions = strategy.predictor().predict(&"A".to_string());
    let b_pred = predictions.iter().find(|p| p.screen == "B");
    assert!(b_pred.is_some(), "B should be in predictions from A");
    let b_prob = b_pred.unwrap().probability;
    assert!(
        b_prob > 0.8,
        "B probability from A should be >0.8, got {b_prob}"
    );

    // The Predictive strategy should also give B a low divisor when A is active
    let b_decision_tick0 = strategy.should_tick("B", 0, "A");
    assert_eq!(
        b_decision_tick0,
        TickDecision::Tick,
        "B should tick on frame 0 when A is active (high probability target)"
    );
}

// ── Test 8: Persistence round-trip ────────────────────────────────────────

#[cfg(feature = "state-persistence")]
#[test]
fn persistence_round_trip_preserves_predictions() {
    use ftui_runtime::{load_transitions, save_transitions};

    let config = PredictiveStrategyConfig {
        min_observations: 5,
        fallback_divisor: 20,
        decay_interval: 10_000,
        ..PredictiveStrategyConfig::default()
    };

    // Create strategy and train it
    let mut original = Predictive::new(config.clone());
    for _ in 0..60 {
        original.on_screen_transition("A", "B");
    }
    for _ in 0..30 {
        original.on_screen_transition("A", "C");
    }
    for _ in 0..10 {
        original.on_screen_transition("A", "D");
    }

    // Save
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("transitions.json");
    save_transitions(original.counter(), &path).unwrap();

    // Load into new strategy
    let loaded_counter = load_transitions(&path).unwrap();
    let mut restored = Predictive::with_counter(config, loaded_counter);

    // Compare predictions
    let orig_preds = original.predictor().predict(&"A".to_string());
    let restored_preds = restored.predictor().predict(&"A".to_string());

    assert_eq!(
        orig_preds.len(),
        restored_preds.len(),
        "prediction count should match"
    );

    for (o, r) in orig_preds.iter().zip(restored_preds.iter()) {
        assert_eq!(o.screen, r.screen, "screen names should match");
        assert!(
            (o.probability - r.probability).abs() < 1e-9,
            "probabilities should match for {}: {} vs {}",
            o.screen,
            o.probability,
            r.probability
        );
    }

    // Run simulation with both and compare tick distributions
    let mut model_orig = MockMultiScreenModel::new(&["A", "B", "C", "D"]);
    let mut model_restored = MockMultiScreenModel::new(&["A", "B", "C", "D"]);
    let frames = 100;

    simulate_frames(&mut model_orig, &mut original, frames, 0);
    simulate_frames(&mut model_restored, &mut restored, frames, 0);

    model_orig.log_distribution("original");
    model_restored.log_distribution("restored from disk");

    // Tick counts should be identical
    for screen in &["A", "B", "C", "D"] {
        let o = model_orig.tick_counts.get(*screen).copied().unwrap_or(0);
        let r = model_restored
            .tick_counts
            .get(*screen)
            .copied()
            .unwrap_or(0);
        assert_eq!(o, r, "tick count mismatch for screen {screen}: {o} vs {r}");
    }
}

// ── Test 9: TickStrategyKind enum delegates correctly ─────────────────────

#[test]
fn tick_strategy_kind_delegates_full_lifecycle() {
    // Test via the TickStrategyKind enum to verify delegation works
    let mut model = MockMultiScreenModel::new(&["A", "B", "C"]);
    let mut strategy = TickStrategyKind::Uniform { divisor: 4 };
    let frames = 100;

    simulate_frames(&mut model, &mut strategy, frames, 0);
    model.log_distribution("TickStrategyKind::Uniform(4)");

    assert_eq!(model.tick_counts.get("A").copied().unwrap_or(0), frames);
    // Every 4th frame: 0,4,8,...96 = 25
    assert_eq!(model.tick_counts.get("B").copied().unwrap_or(0), 25);
    assert_eq!(model.tick_counts.get("C").copied().unwrap_or(0), 25);
}

// ── Test 10: Multi-switch simulation ──────────────────────────────────────

#[test]
fn multi_switch_simulation_distributes_ticks() {
    let mut model = MockMultiScreenModel::new(&["Home", "Messages", "Settings", "Profile"]);
    let config = PredictiveStrategyConfig {
        min_observations: 3,
        fallback_divisor: 10,
        decay_interval: 10_000,
        ..PredictiveStrategyConfig::default()
    };
    let mut strategy = Predictive::new(config);

    // Simulate realistic user behavior:
    // Home is the hub; user goes Home→Messages→Home→Settings→Home→Messages...
    let navigation = [
        ("Home", 20),
        ("Messages", 10),
        ("Home", 15),
        ("Settings", 5),
        ("Home", 10),
        ("Messages", 15),
        ("Home", 10),
        ("Profile", 5),
        ("Home", 10),
    ];

    let mut tick = 0u64;
    let mut prev_screen: Option<String> = None;

    for (screen, frames) in &navigation {
        // Notify strategy of screen transition
        if let Some(ref prev) = prev_screen
            && prev != screen
        {
            strategy.on_screen_transition(prev, screen);
        }
        model.switch_to(screen);

        simulate_frames(&mut model, &mut strategy, *frames, tick);
        tick += frames;
        prev_screen = Some(screen.to_string());
    }

    model.log_distribution("multi-switch realistic navigation");

    // Home should have the most ticks (it's active most of the time)
    let home_ticks = model.tick_counts.get("Home").copied().unwrap_or(0);
    let msgs_ticks = model.tick_counts.get("Messages").copied().unwrap_or(0);
    let _settings_ticks = model.tick_counts.get("Settings").copied().unwrap_or(0);
    let profile_ticks = model.tick_counts.get("Profile").copied().unwrap_or(0);

    assert!(
        home_ticks > msgs_ticks,
        "Home ({home_ticks}) should have more ticks than Messages ({msgs_ticks})"
    );
    assert!(
        msgs_ticks > profile_ticks,
        "Messages ({msgs_ticks}) should have more ticks than Profile ({profile_ticks})"
    );

    // Verify all screens received at least some ticks
    assert!(home_ticks > 0, "Home should have ticks");
    assert!(msgs_ticks > 0, "Messages should have ticks");

    // After this navigation, strategy should predict Home→Messages as most likely
    let home_predictions = strategy.predictor().predict(&"Home".to_string());
    if !home_predictions.is_empty() {
        let top = &home_predictions[0];
        eprintln!(
            "Top prediction from Home: {} (p={:.3})",
            top.screen, top.probability
        );
        // Messages was visited from Home most frequently
        assert_eq!(
            top.screen, "Messages",
            "Messages should be top prediction from Home"
        );
    }
}

// ── Test 11: Decay reduces old transition influence ───────────────────────

#[test]
fn decay_reduces_old_transition_influence() {
    let config = PredictiveStrategyConfig {
        min_observations: 3,
        fallback_divisor: 10,
        decay_interval: 10,
        decay_factor: 0.1, // aggressive decay
        ..PredictiveStrategyConfig::default()
    };
    let mut strategy = Predictive::new(config);

    // Record old transitions: A→B heavily
    for _ in 0..50 {
        strategy.on_screen_transition("A", "B");
    }

    let total_before = strategy.counter().total();
    eprintln!("total before decay: {total_before}");

    // Trigger maintenance ticks to cause decay
    for tick in 0..20 {
        strategy.maintenance_tick(tick);
    }

    let total_after = strategy.counter().total();
    eprintln!("total after decay: {total_after}");
    assert!(
        total_after < total_before,
        "decay should reduce total: {total_after} < {total_before}"
    );
}
