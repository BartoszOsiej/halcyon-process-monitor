//! Property-based invariant tests for geometry primitives (Rect, Size, Sides).
//!
//! These tests verify algebraic and structural invariants that must hold for
//! any valid inputs:
//!
//! 1. Intersection is commutative.
//! 2. Intersection is idempotent (A ∩ A = A).
//! 3. Intersection result fits within both inputs.
//! 4. Union is commutative.
//! 5. Union is idempotent (A ∪ A = A).
//! 6. Union contains both inputs.
//! 7. Contains agrees with intersection (point in rect ↔ point in intersection).
//! 8. Inner margin shrinks dimensions.
//! 9. Right/bottom edges are consistent with x+width, y+height.
//! 10. Size clamp_max/clamp_min monotonicity.
//! 11. Area is width * height.
//! 12. No panics on extreme u16 values.

use ftui_core::geometry::{Rect, Sides, Size};
use proptest::prelude::*;

// ── Helpers ─────────────────────────────────────────────────────────────

fn rect_strategy() -> impl Strategy<Value = Rect> {
    (any::<u16>(), any::<u16>(), any::<u16>(), any::<u16>())
        .prop_map(|(x, y, w, h)| Rect::new(x, y, w, h))
}

fn small_rect_strategy() -> impl Strategy<Value = Rect> {
    (0u16..=500, 0u16..=500, 0u16..=500, 0u16..=500).prop_map(|(x, y, w, h)| Rect::new(x, y, w, h))
}

fn sides_strategy() -> impl Strategy<Value = Sides> {
    (any::<u16>(), any::<u16>(), any::<u16>(), any::<u16>())
        .prop_map(|(t, r, b, l)| Sides::new(t, r, b, l))
}

fn size_strategy() -> impl Strategy<Value = Size> {
    (any::<u16>(), any::<u16>()).prop_map(|(w, h)| Size::new(w, h))
}

// ═════════════════════════════════════════════════════════════════════════
// 1. Intersection is commutative
// ═════════════════════════════════════════════════════════════════════════

proptest! {
    #[test]
    fn intersection_commutative(a in small_rect_strategy(), b in small_rect_strategy()) {
        prop_assert_eq!(
            a.intersection(&b),
            b.intersection(&a),
            "intersection is not commutative: a={:?}, b={:?}",
            a, b
        );
    }
}

// ═════════════════════════════════════════════════════════════════════════
// 2. Intersection is idempotent
// ═════════════════════════════════════════════════════════════════════════

proptest! {
    #[test]
    fn intersection_idempotent(a in small_rect_strategy()) {
        let result = a.intersection(&a);
        if a.is_empty() {
            // Empty rects have no overlap with anything, even themselves
            prop_assert!(result.is_empty(), "Empty rect intersection should be empty");
        } else {
            prop_assert_eq!(result, a, "A ∩ A should equal A for {:?}", a);
        }
    }
}

// ═════════════════════════════════════════════════════════════════════════
// 3. Intersection result fits within both inputs
// ═════════════════════════════════════════════════════════════════════════

proptest! {
    #[test]
    fn intersection_fits_within_both(a in small_rect_strategy(), b in small_rect_strategy()) {
        let inter = a.intersection(&b);
        if !inter.is_empty() {
            prop_assert!(inter.left() >= a.left() && inter.left() >= b.left());
            prop_assert!(inter.top() >= a.top() && inter.top() >= b.top());
            prop_assert!(inter.right() <= a.right() && inter.right() <= b.right());
            prop_assert!(inter.bottom() <= a.bottom() && inter.bottom() <= b.bottom());
        }
    }
}

// ═════════════════════════════════════════════════════════════════════════
// 4. Union is commutative
// ═════════════════════════════════════════════════════════════════════════

proptest! {
    #[test]
    fn union_commutative(a in small_rect_strategy(), b in small_rect_strategy()) {
        prop_assert_eq!(
            a.union(&b),
            b.union(&a),
            "union is not commutative: a={:?}, b={:?}",
            a, b
        );
    }
}

// ═════════════════════════════════════════════════════════════════════════
// 5. Union is idempotent
// ═════════════════════════════════════════════════════════════════════════

proptest! {
    #[test]
    fn union_idempotent(a in small_rect_strategy()) {
        prop_assert_eq!(
            a.union(&a),
            a,
            "A ∪ A should equal A for {:?}",
            a
        );
    }
}

// ═════════════════════════════════════════════════════════════════════════
// 6. Union contains both inputs
// ═════════════════════════════════════════════════════════════════════════

proptest! {
    #[test]
    fn union_contains_both(a in small_rect_strategy(), b in small_rect_strategy()) {
        let u = a.union(&b);
        if !a.is_empty() {
            prop_assert!(u.left() <= a.left());
            prop_assert!(u.top() <= a.top());
            prop_assert!(u.right() >= a.right());
            prop_assert!(u.bottom() >= a.bottom());
        }
        if !b.is_empty() {
            prop_assert!(u.left() <= b.left());
            prop_assert!(u.top() <= b.top());
            prop_assert!(u.right() >= b.right());
            prop_assert!(u.bottom() >= b.bottom());
        }
    }
}

// ═════════════════════════════════════════════════════════════════════════
// 7. Contains agrees with intersection
// ═════════════════════════════════════════════════════════════════════════

proptest! {
    #[test]
    fn contains_agrees_with_intersection(
        a in small_rect_strategy(),
        px in 0u16..=600,
        py in 0u16..=600,
    ) {
        let point_rect = Rect::new(px, py, 1, 1);
        let inter = a.intersection(&point_rect);

        if a.contains(px, py) {
            prop_assert!(
                !inter.is_empty(),
                "contains({},{}) is true but intersection is empty for {:?}",
                px, py, a
            );
        }
        // Note: the converse is also true but only for non-empty intersections
        // with 1x1 rects, which is exactly the point containment test.
        if !inter.is_empty() {
            prop_assert!(
                a.contains(px, py),
                "intersection non-empty but contains({},{}) is false for {:?}",
                px, py, a
            );
        }
    }
}

// ═════════════════════════════════════════════════════════════════════════
// 8. Inner margin shrinks dimensions
// ═════════════════════════════════════════════════════════════════════════

proptest! {
    #[test]
    fn inner_margin_shrinks(
        rect in small_rect_strategy(),
        sides in (0u16..=100, 0u16..=100, 0u16..=100, 0u16..=100)
            .prop_map(|(t, r, b, l)| Sides::new(t, r, b, l)),
    ) {
        let inner = rect.inner(sides);
        prop_assert!(
            inner.width <= rect.width,
            "inner width {} > outer width {} with margin {:?}",
            inner.width, rect.width, sides
        );
        prop_assert!(
            inner.height <= rect.height,
            "inner height {} > outer height {} with margin {:?}",
            inner.height, rect.height, sides
        );
    }
}

// ═════════════════════════════════════════════════════════════════════════
// 9. Right/bottom edge consistency
// ═════════════════════════════════════════════════════════════════════════

proptest! {
    #[test]
    fn right_bottom_consistent(rect in rect_strategy()) {
        // right() uses saturating_add, so right >= x always
        prop_assert!(rect.right() >= rect.x);
        prop_assert!(rect.bottom() >= rect.y);

        // right - x gives width (or saturates)
        let computed_width = rect.right().saturating_sub(rect.x);
        // Due to saturating_add, right() may be clamped to u16::MAX
        if rect.x as u32 + rect.width as u32 <= u16::MAX as u32 {
            prop_assert_eq!(
                computed_width, rect.width,
                "right()-x should equal width when no saturation"
            );
        }
    }
}

// ═════════════════════════════════════════════════════════════════════════
// 10. Size clamp monotonicity
// ═════════════════════════════════════════════════════════════════════════

proptest! {
    #[test]
    fn size_clamp_max_monotone(s in size_strategy(), max in size_strategy()) {
        let clamped = s.clamp_max(max);
        prop_assert!(clamped.width <= s.width);
        prop_assert!(clamped.height <= s.height);
        prop_assert!(clamped.width <= max.width);
        prop_assert!(clamped.height <= max.height);
    }

    #[test]
    fn size_clamp_min_monotone(s in size_strategy(), min in size_strategy()) {
        let clamped = s.clamp_min(min);
        prop_assert!(clamped.width >= s.width || clamped.width >= min.width);
        prop_assert!(clamped.height >= s.height || clamped.height >= min.height);
        prop_assert!(clamped.width >= min.width);
        prop_assert!(clamped.height >= min.height);
    }
}

// ═════════════════════════════════════════════════════════════════════════
// 11. Area is width * height
// ═════════════════════════════════════════════════════════════════════════

proptest! {
    #[test]
    fn rect_area_is_product(rect in rect_strategy()) {
        prop_assert_eq!(
            rect.area(),
            rect.width as u32 * rect.height as u32,
            "area() != width*height for {:?}",
            rect
        );
    }

    #[test]
    fn size_area_is_product(s in size_strategy()) {
        prop_assert_eq!(
            s.area(),
            s.width as u32 * s.height as u32,
            "area() != width*height for {:?}",
            s
        );
    }
}

// ═════════════════════════════════════════════════════════════════════════
// 12. No panics on extreme values
// ═════════════════════════════════════════════════════════════════════════

proptest! {
    #[test]
    fn no_panic_rect_operations(a in rect_strategy(), b in rect_strategy(), sides in sides_strategy()) {
        let _ = a.intersection(&b);
        let _ = a.intersection_opt(&b);
        let _ = a.union(&b);
        let _ = a.inner(sides);
        let _ = a.contains(b.x, b.y);
        let _ = a.left();
        let _ = a.top();
        let _ = a.right();
        let _ = a.bottom();
        let _ = a.area();
        let _ = a.is_empty();
    }

    #[test]
    fn no_panic_size_operations(a in size_strategy(), b in size_strategy()) {
        let _ = a.clamp_max(b);
        let _ = a.clamp_min(b);
        let _ = a.area();
        let _ = a.is_empty();
    }
}

// ═════════════════════════════════════════════════════════════════════════
// 13. Intersection and union absorption laws
// ═════════════════════════════════════════════════════════════════════════

proptest! {
    #[test]
    fn intersection_with_union_absorption(a in small_rect_strategy(), b in small_rect_strategy()) {
        // A ∩ (A ∪ B) = A (absorption law, holds for non-empty rects)
        if !a.is_empty() {
            let union_ab = a.union(&b);
            let result = a.intersection(&union_ab);
            prop_assert_eq!(
                result, a,
                "A ∩ (A ∪ B) should equal A for a={:?}, b={:?}",
                a, b
            );
        }
    }
}

// ═════════════════════════════════════════════════════════════════════════
// 14. Empty rect is intersection identity exception
// ═════════════════════════════════════════════════════════════════════════

proptest! {
    #[test]
    fn empty_rect_is_empty(x in any::<u16>(), y in any::<u16>()) {
        let zero_w = Rect::new(x, y, 0, 1);
        let zero_h = Rect::new(x, y, 1, 0);
        let zero_both = Rect::new(x, y, 0, 0);

        prop_assert!(zero_w.is_empty());
        prop_assert!(zero_h.is_empty());
        prop_assert!(zero_both.is_empty());
        prop_assert_eq!(zero_w.area(), 0);
        prop_assert_eq!(zero_h.area(), 0);
    }
}

// ═════════════════════════════════════════════════════════════════════════
// 15. Sides horizontal/vertical sum consistency
// ═════════════════════════════════════════════════════════════════════════

proptest! {
    #[test]
    fn sides_sums_consistent(sides in sides_strategy()) {
        let h = sides.horizontal_sum();
        let v = sides.vertical_sum();

        // Sums are defined via saturating_add.
        prop_assert_eq!(h, sides.left.saturating_add(sides.right));
        prop_assert_eq!(v, sides.top.saturating_add(sides.bottom));
    }
}
