//! Property-based invariant tests for the ftui-layout constraint solver.
//!
//! These tests verify structural invariants of the Flex layout system that must
//! hold for **any** combination of constraints and available space:
//!
//! 1. Sum conservation: allocated sizes never exceed available space.
//! 2. Constraint solver is deterministic.
//! 3. Rect count matches constraint count.
//! 4. All rects fit within the parent area.
//! 5. Max constraints are never exceeded.
//! 6. Fixed constraints get exactly their size (when space allows).
//! 7. Empty area produces zero-area rects.
//! 8. No constraints produces empty vec.
//! 9. Breakpoint classification is monotonic.
//! 10. round_layout_stable preserves exact sum.
//! 11. round_layout_stable bounded displacement (|x_i - r_i| < 1).
//! 12. round_layout_stable determinism.
//! 13. Vertical and horizontal layouts are structurally symmetric.
//! 14. Gap handling: total allocation + gaps <= available.

use ftui_core::geometry::{Rect, Sides};
use ftui_layout::{
    Alignment, Breakpoint, Breakpoints, Constraint, Direction, Flex, round_layout_stable,
};
use proptest::prelude::*;

// ── Helpers ─────────────────────────────────────────────────────────────

fn constraint_strategy() -> impl Strategy<Value = Constraint> {
    prop_oneof![
        (0u16..=500).prop_map(Constraint::Fixed),
        (0.0f32..=100.0).prop_map(Constraint::Percentage),
        (0u16..=500).prop_map(Constraint::Min),
        (0u16..=500).prop_map(Constraint::Max),
        (0u32..=100, 1u32..=100).prop_map(|(n, d)| Constraint::Ratio(n, d)),
        Just(Constraint::Fill),
    ]
}

fn constraint_list(max_len: usize) -> impl Strategy<Value = Vec<Constraint>> {
    proptest::collection::vec(constraint_strategy(), 1..=max_len)
}

fn alignment_strategy() -> impl Strategy<Value = Alignment> {
    prop_oneof![
        Just(Alignment::Start),
        Just(Alignment::Center),
        Just(Alignment::End),
        Just(Alignment::SpaceBetween),
        Just(Alignment::SpaceAround),
    ]
}

fn direction_strategy() -> impl Strategy<Value = Direction> {
    prop_oneof![Just(Direction::Horizontal), Just(Direction::Vertical),]
}

fn area_strategy() -> impl Strategy<Value = Rect> {
    (0u16..=100, 0u16..=100, 1u16..=500, 1u16..=200).prop_map(|(x, y, w, h)| Rect::new(x, y, w, h))
}

// ═════════════════════════════════════════════════════════════════════════
// 1. Sum conservation: allocated sizes never exceed available
// ═════════════════════════════════════════════════════════════════════════

proptest! {
    #[test]
    fn sum_never_exceeds_available(
        constraints in constraint_list(10),
        area in area_strategy(),
        alignment in alignment_strategy(),
        direction in direction_strategy(),
        gap in 0u16..=20,
    ) {
        let flex = Flex::horizontal()
            .direction(direction)
            .constraints(constraints)
            .alignment(alignment)
            .gap(gap);
        let rects = flex.split(area);

        let total: u16 = match direction {
            Direction::Horizontal => rects.iter().map(|r| r.width).sum(),
            Direction::Vertical => rects.iter().map(|r| r.height).sum(),
        };

        let available = match direction {
            Direction::Horizontal => area.width,
            Direction::Vertical => area.height,
        };

        prop_assert!(
            total <= available,
            "Total size {} exceeded available {} (direction={:?}, alignment={:?}, gap={})",
            total, available, direction, alignment, gap
        );
    }
}

// ═════════════════════════════════════════════════════════════════════════
// 2. Determinism: same inputs always produce same output
// ═════════════════════════════════════════════════════════════════════════

proptest! {
    #[test]
    fn solver_is_deterministic(
        constraints in constraint_list(8),
        width in 1u16..=500,
        alignment in alignment_strategy(),
    ) {
        let flex = Flex::horizontal()
            .constraints(constraints)
            .alignment(alignment);
        let area = Rect::new(0, 0, width, 10);

        let rects1 = flex.split(area);
        let rects2 = flex.split(area);

        prop_assert_eq!(rects1, rects2, "Two calls produced different results");
    }
}

// ═════════════════════════════════════════════════════════════════════════
// 3. Rect count matches constraint count
// ═════════════════════════════════════════════════════════════════════════

proptest! {
    #[test]
    fn rect_count_matches_constraints(
        constraints in constraint_list(15),
        area in area_strategy(),
    ) {
        let count = constraints.len();
        let flex = Flex::horizontal().constraints(constraints);
        let rects = flex.split(area);
        prop_assert_eq!(
            rects.len(),
            count,
            "Expected {} rects, got {}",
            count,
            rects.len()
        );
    }
}

// ═════════════════════════════════════════════════════════════════════════
// 4. All rects fit within the parent area (horizontal layout)
// ═════════════════════════════════════════════════════════════════════════

proptest! {
    #[test]
    fn horizontal_rects_fit_within_parent(
        constraints in constraint_list(8),
        width in 1u16..=500,
        height in 1u16..=200,
    ) {
        let area = Rect::new(0, 0, width, height);
        let flex = Flex::horizontal().constraints(constraints);
        let rects = flex.split(area);

        for (i, r) in rects.iter().enumerate() {
            prop_assert!(
                r.y == area.y,
                "Rect {} y={} != area y={}",
                i, r.y, area.y
            );
            prop_assert!(
                r.height == area.height || r.height == 0,
                "Rect {} height={} != area height={} (and not 0)",
                i, r.height, area.height
            );
        }
    }

    #[test]
    fn vertical_rects_fit_within_parent(
        constraints in constraint_list(8),
        width in 1u16..=500,
        height in 1u16..=200,
    ) {
        let area = Rect::new(0, 0, width, height);
        let flex = Flex::vertical().constraints(constraints);
        let rects = flex.split(area);

        for (i, r) in rects.iter().enumerate() {
            prop_assert!(
                r.x == area.x,
                "Rect {} x={} != area x={}",
                i, r.x, area.x
            );
            prop_assert!(
                r.width == area.width || r.width == 0,
                "Rect {} width={} != area width={} (and not 0)",
                i, r.width, area.width
            );
        }
    }
}

// ═════════════════════════════════════════════════════════════════════════
// 5. Max constraints are never exceeded
// ═════════════════════════════════════════════════════════════════════════

proptest! {
    #[test]
    fn max_constraints_respected(
        max_val in 1u16..=200,
        extra_constraints in constraint_list(5),
        width in 1u16..=500,
    ) {
        let mut constraints = vec![Constraint::Max(max_val)];
        constraints.extend(extra_constraints);
        let flex = Flex::horizontal().constraints(constraints);
        let rects = flex.split(Rect::new(0, 0, width, 10));

        prop_assert!(
            rects[0].width <= max_val,
            "Max({}) constraint violated: got width {}",
            max_val, rects[0].width
        );
    }
}

// ═════════════════════════════════════════════════════════════════════════
// 6. Fixed constraints get exact size (when ample space)
// ═════════════════════════════════════════════════════════════════════════

proptest! {
    #[test]
    fn fixed_gets_exact_size_when_space_allows(
        fixed_size in 1u16..=100,
    ) {
        // Provide ample space (2x the fixed size)
        let width = fixed_size.saturating_mul(2).max(fixed_size + 1);
        let flex = Flex::horizontal()
            .constraints([Constraint::Fixed(fixed_size)]);
        let rects = flex.split(Rect::new(0, 0, width, 10));

        prop_assert_eq!(
            rects[0].width, fixed_size,
            "Fixed({}) should get exactly that size with width={}",
            fixed_size, width
        );
    }
}

// ═════════════════════════════════════════════════════════════════════════
// 7. Empty area produces zero-area rects
// ═════════════════════════════════════════════════════════════════════════

proptest! {
    #[test]
    fn empty_area_produces_zero_rects(
        constraints in constraint_list(10),
    ) {
        let flex = Flex::horizontal().constraints(constraints);
        let rects = flex.split(Rect::new(0, 0, 0, 0));
        for (i, r) in rects.iter().enumerate() {
            prop_assert!(
                r.is_empty(),
                "Rect {} should be empty for zero-area input, got {:?}",
                i, r
            );
        }
    }
}

// ═════════════════════════════════════════════════════════════════════════
// 8. No constraints produces empty vec
// ═════════════════════════════════════════════════════════════════════════

proptest! {
    #[test]
    fn no_constraints_empty_output(
        area in area_strategy(),
    ) {
        let flex = Flex::horizontal().constraints(std::iter::empty::<Constraint>());
        let rects = flex.split(area);
        prop_assert!(rects.is_empty(), "Expected empty vec, got {} rects", rects.len());
    }
}

// ═════════════════════════════════════════════════════════════════════════
// 9. Breakpoint classification is monotonic
// ═════════════════════════════════════════════════════════════════════════

proptest! {
    #[test]
    fn breakpoint_monotonic(
        sm in 1u16..=200,
        md_offset in 0u16..=200,
        lg_offset in 0u16..=200,
        w1 in 0u16..=600,
        w2 in 0u16..=600,
    ) {
        let md = sm.saturating_add(md_offset);
        let lg = md.saturating_add(lg_offset);
        let bp = Breakpoints::new(sm, md, lg);

        let (lo, hi) = if w1 <= w2 { (w1, w2) } else { (w2, w1) };
        let bp_lo = bp.classify_width(lo);
        let bp_hi = bp.classify_width(hi);

        prop_assert!(
            bp_lo <= bp_hi,
            "Monotonicity violated: classify({}) = {:?} > classify({}) = {:?}",
            lo, bp_lo, hi, bp_hi
        );
    }

    #[test]
    fn breakpoint_all_reachable(
        sm in 10u16..=100,
        md_offset in 10u16..=100,
        lg_offset in 10u16..=100,
    ) {
        let md = sm.saturating_add(md_offset);
        let lg = md.saturating_add(lg_offset);
        let bp = Breakpoints::new(sm, md, lg);

        prop_assert_eq!(bp.classify_width(0), Breakpoint::Xs);
        prop_assert_eq!(bp.classify_width(sm), Breakpoint::Sm);
        prop_assert_eq!(bp.classify_width(md), Breakpoint::Md);
        prop_assert_eq!(bp.classify_width(lg), Breakpoint::Lg);
        // Xl threshold is lg + 40 per Breakpoints::new
        let xl_threshold = bp.threshold(Breakpoint::Xl);
        prop_assert_eq!(bp.classify_width(xl_threshold), Breakpoint::Xl);
    }
}

// ═════════════════════════════════════════════════════════════════════════
// 10. round_layout_stable: exact sum conservation
// ═════════════════════════════════════════════════════════════════════════

proptest! {
    #[test]
    fn rounding_preserves_sum(
        count in 1usize..=20,
        total in 1u16..=500,
    ) {
        // Generate targets that sum to approximately `total` so the
        // algorithm can actually achieve exact sum conservation.
        let target_each = total as f64 / count as f64;
        let targets: Vec<f64> = (0..count).map(|i| {
            // Add slight jitter to make it interesting
            let jitter = (i as f64 * 0.3).sin() * 0.4;
            (target_each + jitter).max(0.0)
        }).collect();
        let result = round_layout_stable(&targets, total, None);
        let actual_sum: u16 = result.iter().copied().sum();

        prop_assert_eq!(
            actual_sum, total,
            "Sum {} != expected {} for targets {:?}",
            actual_sum, total, targets
        );
    }
}

// ═════════════════════════════════════════════════════════════════════════
// 11. round_layout_stable: bounded displacement
// ═════════════════════════════════════════════════════════════════════════

proptest! {
    #[test]
    fn rounding_bounded_displacement(
        total in 10u16..=300,
        count in 1usize..=10,
    ) {
        // Generate targets that sum close to total
        let target_each = total as f64 / count as f64;
        let targets: Vec<f64> = (0..count).map(|_| target_each).collect();
        let result = round_layout_stable(&targets, total, None);

        for (i, (&rounded, &target)) in result.iter().zip(targets.iter()).enumerate() {
            let diff = (rounded as f64 - target).abs();
            prop_assert!(
                diff < 1.0 + f64::EPSILON,
                "Displacement {} for item {} exceeds 1.0 (target={}, rounded={})",
                diff, i, target, rounded
            );
        }
    }
}

// ═════════════════════════════════════════════════════════════════════════
// 12. round_layout_stable: determinism
// ═════════════════════════════════════════════════════════════════════════

proptest! {
    #[test]
    fn rounding_deterministic(
        targets in proptest::collection::vec(0.0f64..=100.0, 1..=15),
        total in 0u16..=500,
    ) {
        let r1 = round_layout_stable(&targets, total, None);
        let r2 = round_layout_stable(&targets, total, None);
        prop_assert_eq!(r1, r2, "Rounding is non-deterministic");
    }
}

// ═════════════════════════════════════════════════════════════════════════
// 13. Structural symmetry: horizontal vs vertical
// ═════════════════════════════════════════════════════════════════════════

proptest! {
    #[test]
    fn direction_symmetry(
        constraints in constraint_list(8),
        size in 10u16..=500,
    ) {
        let h_flex = Flex::horizontal().constraints(constraints.clone());
        let v_flex = Flex::vertical().constraints(constraints);

        let h_rects = h_flex.split(Rect::new(0, 0, size, size));
        let v_rects = v_flex.split(Rect::new(0, 0, size, size));

        // With a square area, horizontal widths should equal vertical heights
        let h_sizes: Vec<u16> = h_rects.iter().map(|r| r.width).collect();
        let v_sizes: Vec<u16> = v_rects.iter().map(|r| r.height).collect();

        prop_assert_eq!(
            h_sizes, v_sizes,
            "Horizontal widths and vertical heights should match for square area"
        );
    }
}

// ═════════════════════════════════════════════════════════════════════════
// 14. Gap handling: sizes + gaps fit within available
// ═════════════════════════════════════════════════════════════════════════

proptest! {
    #[test]
    fn gap_does_not_cause_overflow(
        constraints in constraint_list(8),
        width in 1u16..=500,
        gap in 0u16..=50,
    ) {
        let flex = Flex::horizontal()
            .gap(gap)
            .constraints(constraints.clone());
        let rects = flex.split(Rect::new(0, 0, width, 10));

        // The constraint solver subtracts total gaps from available space,
        // so the sum of allocated widths should not exceed (width - total_gaps).
        let count = constraints.len();
        let gap_count = count.saturating_sub(1);
        let total_gap = (gap_count as u64 * gap as u64).min(u16::MAX as u64) as u16;
        let available_after_gaps = width.saturating_sub(total_gap);

        let total_width: u16 = rects.iter().map(|r| r.width).sum();
        prop_assert!(
            total_width <= available_after_gaps,
            "Total width {} exceeds available after gaps {} (width={}, gap={}, count={})",
            total_width, available_after_gaps, width, gap, count
        );
    }
}

// ═════════════════════════════════════════════════════════════════════════
// 15. Margin handling: inner area is always <= outer area
// ═════════════════════════════════════════════════════════════════════════

proptest! {
    #[test]
    fn margin_shrinks_area(
        constraints in constraint_list(5),
        width in 1u16..=500,
        height in 1u16..=200,
        margin in 0u16..=20,
    ) {
        let flex = Flex::horizontal()
            .constraints(constraints)
            .margin(Sides::all(margin));
        let rects = flex.split(Rect::new(0, 0, width, height));

        let total_width: u16 = rects.iter().map(|r| r.width).sum();
        let effective_width = width.saturating_sub(margin.saturating_mul(2));

        prop_assert!(
            total_width <= effective_width,
            "Total width {} exceeds effective width {} (margin={})",
            total_width, effective_width, margin
        );
    }
}

// ═════════════════════════════════════════════════════════════════════════
// 16. Never panics on extreme values
// ═════════════════════════════════════════════════════════════════════════

proptest! {
    #[test]
    fn no_panic_on_extreme_values(
        constraints in constraint_list(15),
        width in prop_oneof![Just(0u16), Just(1u16), Just(u16::MAX), 0u16..=u16::MAX],
        height in prop_oneof![Just(0u16), Just(1u16), Just(u16::MAX), 0u16..=u16::MAX],
        gap in prop_oneof![Just(0u16), Just(u16::MAX), 0u16..=1000],
        alignment in alignment_strategy(),
    ) {
        let flex = Flex::horizontal()
            .constraints(constraints)
            .alignment(alignment)
            .gap(gap);
        // Must not panic
        let _ = flex.split(Rect::new(0, 0, width, height));
    }
}

// ═════════════════════════════════════════════════════════════════════════
// 17. round_layout_stable: empty targets
// ═════════════════════════════════════════════════════════════════════════

proptest! {
    #[test]
    fn rounding_empty_targets(total in 0u16..=500) {
        let result = round_layout_stable(&[], total, None);
        prop_assert!(result.is_empty(), "Empty targets should produce empty result");
    }
}

// ═════════════════════════════════════════════════════════════════════════
// 18. round_layout_stable: temporal coherence
// ═════════════════════════════════════════════════════════════════════════

proptest! {
    #[test]
    fn rounding_temporal_coherence(
        total in 10u16..=200,
        count in 2usize..=8,
    ) {
        let target_each = total as f64 / count as f64;
        let targets: Vec<f64> = (0..count).map(|_| target_each).collect();

        // First round: no previous
        let r1 = round_layout_stable(&targets, total, None);

        // Second round: same targets with previous allocation
        let r2 = round_layout_stable(&targets, total, Some(r1.clone()));

        // With identical targets and previous allocation, result should be identical
        prop_assert_eq!(
            r1, r2,
            "Same targets with previous allocation should produce same result"
        );
    }
}
