//! Property-based invariant tests for the tick strategy system.
//!
//! Verifies mathematical invariants of the Markov chain prediction engine,
//! transition counter, tick allocation, and persistence layer.
//!
//! ## Invariants
//!
//! 1. Probability normalization: outgoing probabilities sum to 1.0 (±epsilon)
//! 2. Monotonicity: higher count implies higher or equal probability
//! 3. Divisor monotonicity: higher probability → lower or equal divisor
//! 4. Decay preserves relative order
//! 5. Merge commutativity
//! 6. Merge associativity
//! 7. Round-trip serialization
//! 8. Confidence bounds: 0.0 <= confidence <= 1.0
//! 9. Divisor bounds: min_divisor <= divisor <= max_divisor
//! 10. Decay monotonic: smaller factor → smaller counts

use ftui_runtime::{MarkovPredictor, TickAllocation, TransitionCounter};
use proptest::prelude::*;

// ── Strategies ────────────────────────────────────────────────────────────

fn arb_screen_id() -> impl Strategy<Value = String> {
    prop::string::string_regex("[A-Z][a-z]{2,8}").unwrap()
}

fn arb_transition_pair() -> impl Strategy<Value = (String, String)> {
    (arb_screen_id(), arb_screen_id())
}

fn arb_transitions(max_n: usize) -> impl Strategy<Value = Vec<(String, String)>> {
    prop::collection::vec(arb_transition_pair(), 1..max_n)
}

fn arb_counter(max_transitions: usize) -> impl Strategy<Value = TransitionCounter<String>> {
    arb_transitions(max_transitions).prop_map(|transitions| {
        let mut counter = TransitionCounter::new();
        for (from, to) in transitions {
            counter.record(from, to);
        }
        counter
    })
}

fn arb_probability() -> impl Strategy<Value = f64> {
    (0u32..=1000).prop_map(|x| x as f64 / 1000.0)
}

fn arb_decay_factor() -> impl Strategy<Value = f64> {
    (1u32..=999).prop_map(|x| x as f64 / 1000.0)
}

// ── 1. Probability normalization ──────────────────────────────────────────

proptest! {
    #![proptest_config(ProptestConfig::with_cases(200))]

    #[test]
    fn probabilities_sum_to_one(transitions in arb_transitions(100)) {
        let mut counter = TransitionCounter::new();
        for (from, to) in &transitions {
            counter.record(from.clone(), to.clone());
        }
        for from in counter.state_ids() {
            let targets = counter.all_targets_ranked(&from);
            if targets.is_empty() {
                continue;
            }
            let sum: f64 = targets.iter().map(|(_, p)| p).sum();
            prop_assert!(
                (sum - 1.0).abs() < 1e-9,
                "sum={sum} for screen {from}"
            );
        }
    }
}

// ── 2. Count-probability monotonicity ─────────────────────────────────────

proptest! {
    #![proptest_config(ProptestConfig::with_cases(200))]

    #[test]
    fn higher_count_implies_higher_probability(
        base_transitions in arb_transitions(50),
        extra_count in 1u32..50,
    ) {
        let mut counter = TransitionCounter::new();
        for (from, to) in &base_transitions {
            counter.record(from.clone(), to.clone());
        }

        // Pick any source screen that has at least 2 targets
        for from in counter.state_ids() {
            let targets = counter.all_targets_ranked(&from);
            if targets.len() < 2 {
                continue;
            }
            // all_targets_ranked returns descending by probability
            for w in targets.windows(2) {
                let (_, p_high) = &w[0];
                let (_, p_low) = &w[1];
                prop_assert!(
                    p_high >= p_low,
                    "ranked order violated: {p_high} < {p_low} from screen {from}"
                );
            }
        }

        // Now add extra transitions to boost one target
        let from = &base_transitions[0].0;
        let to = &base_transitions[0].1;
        for _ in 0..extra_count {
            counter.record(from.clone(), to.clone());
        }
        let boosted_prob = counter.probability(from, to);
        // This target should now have >= its original probability
        // (it got more observations)
        let targets = counter.all_targets_ranked(from);
        for (target, prob) in &targets {
            if target != to {
                // Boosted target should have >= probability than others
                // only if it has more raw counts
                let boosted_count = counter.count(from, to);
                let other_count = counter.count(from, target);
                if boosted_count > other_count {
                    prop_assert!(
                        boosted_prob >= *prob,
                        "boosted target {to} (count={boosted_count}, prob={boosted_prob}) \
                         should have >= prob than {target} (count={other_count}, prob={prob})"
                    );
                }
            }
        }
    }
}

// ── 3. Divisor monotonicity ───────────────────────────────────────────────

proptest! {
    #![proptest_config(ProptestConfig::with_cases(300))]

    #[test]
    fn higher_probability_gives_lower_divisor(
        p1 in arb_probability(),
        p2 in arb_probability(),
        min_div in 1u64..=5,
        max_div in 6u64..=100,
        exponent in 0.5f64..=5.0,
    ) {
        let alloc = TickAllocation::exponential(min_div, max_div, exponent);
        let d1 = alloc.divisor_for(p1);
        let d2 = alloc.divisor_for(p2);

        if p1 > p2 {
            prop_assert!(
                d1 <= d2,
                "p1={p1} > p2={p2} but d1={d1} > d2={d2}"
            );
        } else if p2 > p1 {
            prop_assert!(
                d2 <= d1,
                "p2={p2} > p1={p1} but d2={d2} > d1={d1}"
            );
        }
    }
}

// ── 4. Decay preserves relative order ─────────────────────────────────────

proptest! {
    #![proptest_config(ProptestConfig::with_cases(200))]

    #[test]
    fn decay_preserves_relative_order(
        transitions in arb_transitions(80),
        factor in arb_decay_factor(),
    ) {
        let mut counter = TransitionCounter::new();
        for (from, to) in &transitions {
            counter.record(from.clone(), to.clone());
        }

        // Record count ratios before decay
        let mut ratios = Vec::new();
        for from in counter.state_ids() {
            let targets = counter.all_targets_ranked(&from);
            if targets.len() >= 2 {
                let (t1, _) = &targets[0];
                let (t2, _) = &targets[1];
                let c1 = counter.count(&from, t1);
                let c2 = counter.count(&from, t2);
                if c1 > c2 {
                    ratios.push((from.clone(), t1.clone(), t2.clone()));
                }
            }
        }

        counter.decay(factor);

        // After decay, relative order should be preserved
        for (from, t1, t2) in &ratios {
            let c1 = counter.count(from, t1);
            let c2 = counter.count(from, t2);
            prop_assert!(
                c1 >= c2,
                "decay changed order: {t1}={c1} < {t2}={c2} from {from}"
            );
        }
    }
}

// ── 5. Merge commutativity ────────────────────────────────────────────────

proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]

    #[test]
    fn merge_is_commutative(
        a in arb_counter(50),
        b in arb_counter(50),
    ) {
        let mut ab = a.clone();
        ab.merge(&b);

        let mut ba = b.clone();
        ba.merge(&a);

        // Check totals match
        prop_assert!(
            (ab.total() - ba.total()).abs() < 1e-9,
            "merge not commutative: ab.total={}, ba.total={}",
            ab.total(), ba.total()
        );

        // Check all individual counts match
        let all_ids: std::collections::HashSet<_> = ab.state_ids()
            .union(&ba.state_ids())
            .cloned()
            .collect();
        for from in &all_ids {
            for to in &all_ids {
                let c_ab = ab.count(from, to);
                let c_ba = ba.count(from, to);
                prop_assert!(
                    (c_ab - c_ba).abs() < 1e-9,
                    "merge not commutative for {from}->{to}: {c_ab} vs {c_ba}"
                );
            }
        }
    }
}

// ── 6. Merge associativity ────────────────────────────────────────────────

proptest! {
    #![proptest_config(ProptestConfig::with_cases(80))]

    #[test]
    fn merge_is_associative(
        a in arb_counter(30),
        b in arb_counter(30),
        c in arb_counter(30),
    ) {
        // (a merge b) merge c
        let mut ab = a.clone();
        ab.merge(&b);
        let mut ab_c = ab;
        ab_c.merge(&c);

        // a merge (b merge c)
        let mut bc = b.clone();
        bc.merge(&c);
        let mut a_bc = a.clone();
        a_bc.merge(&bc);

        prop_assert!(
            (ab_c.total() - a_bc.total()).abs() < 1e-9,
            "merge not associative: {}, {}",
            ab_c.total(), a_bc.total()
        );

        let all_ids: std::collections::HashSet<_> = ab_c.state_ids()
            .union(&a_bc.state_ids())
            .cloned()
            .collect();
        for from in &all_ids {
            for to in &all_ids {
                let c1 = ab_c.count(from, to);
                let c2 = a_bc.count(from, to);
                prop_assert!(
                    (c1 - c2).abs() < 1e-9,
                    "merge not associative for {from}->{to}: {c1} vs {c2}"
                );
            }
        }
    }
}

// ── 7. Round-trip serialization ───────────────────────────────────────────

#[cfg(feature = "state-persistence")]
proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]

    #[test]
    fn round_trip_serialization_preserves_counts(
        transitions in arb_transitions(80),
    ) {
        use ftui_runtime::{save_transitions, load_transitions};

        let mut counter = TransitionCounter::new();
        for (from, to) in &transitions {
            counter.record(from.clone(), to.clone());
        }

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("prop_test.json");

        save_transitions(&counter, &path).unwrap();
        let loaded = load_transitions(&path).unwrap();

        prop_assert!(
            (counter.total() - loaded.total()).abs() < 1e-9,
            "total mismatch: {} vs {}",
            counter.total(), loaded.total()
        );

        for from in counter.state_ids() {
            for (to, _) in counter.all_targets_ranked(&from) {
                let orig = counter.count(&from, &to);
                let rt = loaded.count(&from, &to);
                prop_assert!(
                    (orig - rt).abs() < 1e-9,
                    "count mismatch for {from}->{to}: {orig} vs {rt}"
                );
            }
        }
    }
}

// ── 8. Confidence bounds ──────────────────────────────────────────────────

proptest! {
    #![proptest_config(ProptestConfig::with_cases(200))]

    #[test]
    fn confidence_always_in_unit_range(
        transitions in arb_transitions(100),
        min_obs in 1u64..=100,
    ) {
        let mut predictor = MarkovPredictor::with_min_observations(min_obs);
        for (from, to) in &transitions {
            predictor.record_transition(from.clone(), to.clone());
        }

        for screen in predictor.counter().state_ids() {
            let conf = predictor.confidence(&screen);
            prop_assert!(
                (0.0..=1.0).contains(&conf),
                "confidence out of range: {conf} for screen {screen}"
            );
        }
    }
}

// ── 9. Divisor bounds ─────────────────────────────────────────────────────

proptest! {
    #![proptest_config(ProptestConfig::with_cases(300))]

    #[test]
    fn divisor_always_in_range(
        p in arb_probability(),
        min_div in 1u64..=10,
        max_div in 11u64..=200,
        exponent in 0.1f64..=10.0,
    ) {
        let alloc = TickAllocation::exponential(min_div, max_div, exponent);
        let d = alloc.divisor_for(p);
        prop_assert!(
            d >= min_div && d <= max_div,
            "divisor {d} out of [{min_div}, {max_div}] for p={p}, exp={exponent}"
        );
    }

    #[test]
    fn linear_divisor_always_in_range(
        p in arb_probability(),
        min_div in 1u64..=10,
        max_div in 11u64..=200,
    ) {
        let alloc = TickAllocation::linear(min_div, max_div);
        let d = alloc.divisor_for(p);
        prop_assert!(
            d >= min_div && d <= max_div,
            "linear divisor {d} out of [{min_div}, {max_div}] for p={p}"
        );
    }
}

// ── 10. Decay monotonic: smaller factor → smaller counts ──────────────────

proptest! {
    #![proptest_config(ProptestConfig::with_cases(200))]

    #[test]
    fn smaller_decay_factor_produces_smaller_counts(
        transitions in arb_transitions(50),
        f1_raw in 1u32..=499,
        f2_raw in 500u32..=999,
    ) {
        let f1 = f1_raw as f64 / 1000.0; // smaller factor
        let f2 = f2_raw as f64 / 1000.0; // larger factor
        prop_assume!(f1 < f2);

        let mut base = TransitionCounter::new();
        for (from, to) in &transitions {
            base.record(from.clone(), to.clone());
        }

        let mut counter1 = base.clone();
        counter1.decay(f1);

        let mut counter2 = base.clone();
        counter2.decay(f2);

        // f1 < f2, so counter1 should have smaller or equal total
        prop_assert!(
            counter1.total() <= counter2.total() + 1e-9,
            "smaller factor {f1} gave larger total {} vs factor {f2} total {}",
            counter1.total(), counter2.total()
        );

        // Also check individual counts
        for from in base.state_ids() {
            for (to, _) in base.all_targets_ranked(&from) {
                let c1 = counter1.count(&from, &to);
                let c2 = counter2.count(&from, &to);
                prop_assert!(
                    c1 <= c2 + 1e-9,
                    "smaller factor {f1} gave larger count for {from}->{to}: {c1} vs {c2}"
                );
            }
        }
    }
}

// ── Additional: Prediction probabilities sum to 1 ─────────────────────────

proptest! {
    #![proptest_config(ProptestConfig::with_cases(200))]

    #[test]
    fn prediction_probabilities_sum_to_one(
        transitions in arb_transitions(80),
        min_obs in 1u64..=50,
    ) {
        let mut predictor = MarkovPredictor::with_min_observations(min_obs);
        for (from, to) in &transitions {
            predictor.record_transition(from.clone(), to.clone());
        }

        for screen in predictor.counter().state_ids() {
            let predictions = predictor.predict(&screen);
            if predictions.is_empty() {
                continue;
            }
            let sum: f64 = predictions.iter().map(|p| p.probability).sum();
            prop_assert!(
                (sum - 1.0).abs() < 1e-6,
                "prediction probabilities sum to {sum} for screen {screen}"
            );
        }
    }

    #[test]
    fn prediction_probabilities_always_positive(
        transitions in arb_transitions(80),
        min_obs in 1u64..=50,
    ) {
        let mut predictor = MarkovPredictor::with_min_observations(min_obs);
        for (from, to) in &transitions {
            predictor.record_transition(from.clone(), to.clone());
        }

        for screen in predictor.counter().state_ids() {
            for pred in predictor.predict(&screen) {
                prop_assert!(
                    pred.probability > 0.0,
                    "non-positive probability {} for {}->{}", pred.probability, screen, pred.screen
                );
                prop_assert!(
                    pred.probability <= 1.0,
                    "probability > 1: {} for {}->{}", pred.probability, screen, pred.screen
                );
            }
        }
    }
}
