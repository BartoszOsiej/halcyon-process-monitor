//! Performance Regression Tests for Resize Storms (bd-1rz0.11)
//!
//! These tests enforce budget thresholds and fail if performance degrades.
//! Unlike benchmarks (which measure), these tests assert budget compliance.
//!
//! ## Test Categories
//!
//! 1. **Avoidance ratio budgets** - Allocation reuse during resize storms
//! 2. **Memory efficiency budgets** - Capacity overhead limits
//! 3. **Determinism verification** - Same seed produces identical results
//!
//! Run with: cargo test -p ftui-render --test resize_storm_regression

use ftui_harness::resize_storm::{ResizeStorm, StormConfig, StormPattern};
use ftui_render::buffer::AdaptiveDoubleBuffer;

// =============================================================================
// Avoidance Ratio Budget Tests
// =============================================================================

#[test]
fn burst_storm_achieves_avoidance_budget() {
    let config = StormConfig::default()
        .with_seed(42)
        .with_pattern(StormPattern::Burst { count: 100 })
        .with_initial_size(80, 24);

    let storm = ResizeStorm::new(config);
    let mut adb = AdaptiveDoubleBuffer::new(80, 24);

    for event in storm.events() {
        adb.resize(event.width, event.height);
    }

    let ratio = adb.stats().avoidance_ratio();
    // Budget based on observed performance: 57% typical for burst patterns
    assert!(
        ratio >= 0.50,
        "Burst storm avoidance ratio {:.1}% below budget (50%)",
        ratio * 100.0
    );
}

#[test]
fn oscillate_within_capacity_achieves_high_avoidance() {
    let config = StormConfig::default()
        .with_seed(42)
        .with_pattern(StormPattern::Oscillate {
            size_a: (80, 24),
            size_b: (90, 28), // Within initial capacity (100x30)
            cycles: 50,
        })
        .with_initial_size(80, 24);

    let storm = ResizeStorm::new(config);
    let mut adb = AdaptiveDoubleBuffer::new(80, 24);

    for event in storm.events() {
        adb.resize(event.width, event.height);
    }

    let ratio = adb.stats().avoidance_ratio();
    assert!(
        ratio >= 0.95,
        "Oscillate within capacity avoidance ratio {:.1}% below budget (95%)",
        ratio * 100.0
    );
}

#[test]
fn sweep_storm_maintains_reasonable_avoidance() {
    let config = StormConfig::default()
        .with_pattern(StormPattern::Sweep {
            start_width: 40,
            start_height: 12,
            end_width: 200,
            end_height: 60,
            steps: 50,
        })
        .with_initial_size(40, 12);

    let storm = ResizeStorm::new(config);
    let mut adb = AdaptiveDoubleBuffer::new(40, 12);

    for event in storm.events() {
        adb.resize(event.width, event.height);
    }

    let ratio = adb.stats().avoidance_ratio();
    // Sweep grows continuously, so lower avoidance is expected
    assert!(
        ratio >= 0.50,
        "Sweep storm avoidance ratio {:.1}% below budget (50%)",
        ratio * 100.0
    );
}

#[test]
fn pathological_storm_survives_without_panic() {
    let config = StormConfig::default()
        .with_seed(42)
        .with_pattern(StormPattern::Pathological { count: 50 })
        .with_initial_size(80, 24);

    let storm = ResizeStorm::new(config);
    let mut adb = AdaptiveDoubleBuffer::new(80, 24);

    // Should not panic even with pathological inputs
    for event in storm.events() {
        adb.resize(event.width, event.height);
    }

    // Just verify we completed without panic
    assert!(adb.stats().resize_avoided + adb.stats().resize_reallocated > 0);
}

// =============================================================================
// Memory Efficiency Budget Tests
// =============================================================================

#[test]
fn memory_efficiency_after_storm_within_budget() {
    let config = StormConfig::default()
        .with_seed(42)
        .with_pattern(StormPattern::Mixed { count: 100 })
        .with_initial_size(80, 24);

    let storm = ResizeStorm::new(config);
    let mut adb = AdaptiveDoubleBuffer::new(80, 24);

    for event in storm.events() {
        adb.resize(event.width, event.height);
    }

    let efficiency = adb.memory_efficiency();
    assert!(
        efficiency >= 0.35,
        "Memory efficiency {:.1}% after mixed storm below budget (35%)",
        efficiency * 100.0
    );
}

#[test]
fn large_buffer_memory_efficiency() {
    let adb = AdaptiveDoubleBuffer::new(500, 200);
    let efficiency = adb.memory_efficiency();

    // Large buffers should have higher efficiency due to capped overage
    // Budget based on observed performance: 64% typical for 500x200
    assert!(
        efficiency >= 0.60,
        "Large buffer (500x200) efficiency {:.1}% below budget (60%)",
        efficiency * 100.0
    );
}

#[test]
fn small_buffer_reasonable_efficiency() {
    let adb = AdaptiveDoubleBuffer::new(20, 10);
    let efficiency = adb.memory_efficiency();

    // Small buffers have relatively higher overhead but should be reasonable
    assert!(
        efficiency >= 0.40,
        "Small buffer (20x10) efficiency {:.1}% below budget (40%)",
        efficiency * 100.0
    );
}

// =============================================================================
// Determinism Verification Tests
// =============================================================================

#[test]
fn storm_generation_is_deterministic() {
    let config = StormConfig::default()
        .with_seed(12345)
        .with_pattern(StormPattern::Burst { count: 100 });

    let storm1 = ResizeStorm::new(config.clone());
    let storm2 = ResizeStorm::new(config);

    assert_eq!(
        storm1.sequence_checksum(),
        storm2.sequence_checksum(),
        "Same seed must produce identical checksum"
    );

    assert_eq!(
        storm1.events().len(),
        storm2.events().len(),
        "Same seed must produce same event count"
    );

    for (i, (e1, e2)) in storm1.events().iter().zip(storm2.events()).enumerate() {
        assert_eq!(
            (e1.width, e1.height),
            (e2.width, e2.height),
            "Event {} differs between runs",
            i
        );
    }
}

#[test]
fn different_seeds_produce_different_sequences() {
    let config1 = StormConfig::default()
        .with_seed(1)
        .with_pattern(StormPattern::Burst { count: 50 });
    let config2 = StormConfig::default()
        .with_seed(2)
        .with_pattern(StormPattern::Burst { count: 50 });

    let storm1 = ResizeStorm::new(config1);
    let storm2 = ResizeStorm::new(config2);

    assert_ne!(
        storm1.sequence_checksum(),
        storm2.sequence_checksum(),
        "Different seeds must produce different checksums"
    );
}

#[test]
fn avoidance_ratio_is_deterministic() {
    let config = StormConfig::default()
        .with_seed(42)
        .with_pattern(StormPattern::Burst { count: 50 });

    let storm = ResizeStorm::new(config);

    let mut adb1 = AdaptiveDoubleBuffer::new(80, 24);
    let mut adb2 = AdaptiveDoubleBuffer::new(80, 24);

    for event in storm.events() {
        adb1.resize(event.width, event.height);
    }
    for event in storm.events() {
        adb2.resize(event.width, event.height);
    }

    assert_eq!(
        adb1.stats().avoidance_ratio(),
        adb2.stats().avoidance_ratio(),
        "Avoidance ratio must be deterministic for same inputs"
    );
}

// =============================================================================
// Edge Cases
// =============================================================================

#[test]
fn resize_to_same_size_is_noop() {
    let mut adb = AdaptiveDoubleBuffer::new(80, 24);
    let initial_stats = adb.stats().clone();

    // Multiple resizes to same size
    for _ in 0..10 {
        adb.resize(80, 24);
    }

    let final_stats = adb.stats();
    assert_eq!(
        initial_stats.resize_avoided + initial_stats.resize_reallocated,
        final_stats.resize_avoided + final_stats.resize_reallocated,
        "Resize to same size should not increment counters"
    );
}

#[test]
fn shrink_grow_cycle_no_thrash() {
    let mut adb = AdaptiveDoubleBuffer::new(100, 40);

    // Shrink to 80x30, then grow back to 100x40
    // Should stay within the original capacity
    for _ in 0..10 {
        adb.resize(80, 30);
        adb.resize(100, 40);
    }

    let ratio = adb.stats().avoidance_ratio();
    assert!(
        ratio >= 0.90,
        "Shrink-grow cycle avoidance ratio {:.1}% below budget (90%)",
        ratio * 100.0
    );
}

#[test]
fn gradual_growth_reuses_capacity() {
    let mut adb = AdaptiveDoubleBuffer::new(80, 24);

    // Grow by 1 cell at a time (should reuse capacity)
    for i in 1u16..=15 {
        adb.resize(80 + i, 24);
    }

    let stats = adb.stats();
    assert!(
        stats.resize_avoided >= 10,
        "Expected >= 10 avoided resizes during gradual growth, got {}",
        stats.resize_avoided
    );
}
