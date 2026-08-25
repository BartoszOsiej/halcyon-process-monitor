//! Property-based invariant tests for the spatial hit-test index.
//!
//! These tests verify that the spatial hit index correctly implements:
//!
//! 1. Hit-test returns topmost (highest z-order) widget at any point.
//! 2. Points outside all registered regions return None.
//! 3. Hit-test is deterministic (same state → same result).
//! 4. Cache consistency: hit_test and hit_test_readonly agree.
//! 5. Register/remove round-trip leaves index clean.
//! 6. Z-order is respected for overlapping widgets.
//! 7. No panics on any valid coordinate query.

use ftui_core::geometry::Rect;
use ftui_render::frame::{HitData, HitId, HitRegion};
use ftui_render::spatial_hit_index::SpatialHitIndex;
use proptest::prelude::*;

// ── Helpers ─────────────────────────────────────────────────────────────

fn screen_dims() -> impl Strategy<Value = (u16, u16)> {
    (1u16..=200, 1u16..=100)
}

fn rect_within(w: u16, h: u16) -> impl Strategy<Value = Rect> {
    (0..w, 0..h).prop_flat_map(move |(x, y)| {
        let max_w = (w - x).max(1);
        let max_h = (h - y).max(1);
        (Just(x), Just(y), 1..=max_w, 1..=max_h).prop_map(|(x, y, width, height)| Rect {
            x,
            y,
            width,
            height,
        })
    })
}

fn widget_set(w: u16, h: u16, count: usize) -> impl Strategy<Value = Vec<(Rect, u16)>> {
    proptest::collection::vec((rect_within(w, h), 0u16..10), 0..count)
}

// ═════════════════════════════════════════════════════════════════════════
// 1. Hit-test never panics on any coordinate
// ═════════════════════════════════════════════════════════════════════════

proptest! {
    #[test]
    fn hit_test_never_panics(
        (w, h) in screen_dims(),
        widgets in widget_set(200, 100, 50),
        qx in 0u16..300,
        qy in 0u16..300,
    ) {
        let mut index = SpatialHitIndex::with_defaults(w, h);
        for (i, (rect, z)) in widgets.iter().enumerate() {
            index.register(
                HitId::new(i as u32 + 1),
                *rect,
                HitRegion::Content,
                i as HitData,
                *z,
            );
        }
        // Must not panic
        let _ = index.hit_test(qx, qy);
    }

    #[test]
    fn hit_test_readonly_never_panics(
        (w, h) in screen_dims(),
        widgets in widget_set(200, 100, 50),
        qx in 0u16..300,
        qy in 0u16..300,
    ) {
        let mut index = SpatialHitIndex::with_defaults(w, h);
        for (i, (rect, z)) in widgets.iter().enumerate() {
            index.register(
                HitId::new(i as u32 + 1),
                *rect,
                HitRegion::Content,
                i as HitData,
                *z,
            );
        }
        let _ = index.hit_test_readonly(qx, qy);
    }
}

// ═════════════════════════════════════════════════════════════════════════
// 2. Out-of-bounds queries always return None
// ═════════════════════════════════════════════════════════════════════════

proptest! {
    #[test]
    fn out_of_bounds_returns_none(
        (w, h) in screen_dims(),
        widgets in widget_set(200, 100, 20),
    ) {
        let mut index = SpatialHitIndex::with_defaults(w, h);
        for (i, (rect, z)) in widgets.iter().enumerate() {
            index.register(
                HitId::new(i as u32 + 1),
                *rect,
                HitRegion::Content,
                i as HitData,
                *z,
            );
        }
        // Query at width/height (just out of bounds)
        prop_assert!(index.hit_test(w, 0).is_none(),
            "x={} (== width) should return None", w);
        prop_assert!(index.hit_test(0, h).is_none(),
            "y={} (== height) should return None", h);
        prop_assert!(index.hit_test(w, h).is_none(),
            "({},{}) should return None", w, h);
        // Far out of bounds
        prop_assert!(index.hit_test(u16::MAX, u16::MAX).is_none());
    }
}

// ═════════════════════════════════════════════════════════════════════════
// 3. Topmost widget wins (z-order + registration order tie-breaking)
// ═════════════════════════════════════════════════════════════════════════

proptest! {
    #[test]
    fn topmost_widget_wins(
        (w, h) in (10u16..100, 10u16..60),
        widgets in widget_set(100, 60, 30),
        qx in 0u16..100,
        qy in 0u16..60,
    ) {
        if qx >= w || qy >= h {
            return Ok(());
        }

        let mut index = SpatialHitIndex::with_defaults(w, h);
        for (i, (rect, z)) in widgets.iter().enumerate() {
            index.register(
                HitId::new(i as u32 + 1),
                *rect,
                HitRegion::Content,
                i as HitData,
                *z,
            );
        }

        let result = index.hit_test(qx, qy);

        // Verify independently: find the topmost widget containing (qx, qy).
        let mut expected_id = None;
        let mut best_z: Option<(u16, u32)> = None; // (z_order, registration_order)

        for (i, (rect, z)) in widgets.iter().enumerate() {
            let in_rect = qx >= rect.x
                && qx < rect.x.saturating_add(rect.width)
                && qy >= rect.y
                && qy < rect.y.saturating_add(rect.height);
            if in_rect {
                let order = i as u32;
                let better = match best_z {
                    None => true,
                    Some((best_z_val, best_ord)) => {
                        (*z, order) > (best_z_val, best_ord)
                    }
                };
                if better {
                    expected_id = Some(HitId::new(i as u32 + 1));
                    best_z = Some((*z, order));
                }
            }
        }

        match (result, expected_id) {
            (Some((id, _, _)), Some(exp)) => {
                prop_assert_eq!(id, exp,
                    "At ({},{}): expected {:?}, got {:?}", qx, qy, exp, id);
            }
            (None, None) => {} // Both agree no widget
            (Some((id, _, _)), None) => {
                prop_assert!(false,
                    "At ({},{}): got {:?} but expected None", qx, qy, id);
            }
            (None, Some(exp)) => {
                prop_assert!(false,
                    "At ({},{}): got None but expected {:?}", qx, qy, exp);
            }
        }
    }
}

// ═════════════════════════════════════════════════════════════════════════
// 4. hit_test and hit_test_readonly agree
// ═════════════════════════════════════════════════════════════════════════

proptest! {
    #[test]
    fn cached_and_readonly_agree(
        (w, h) in (10u16..100, 10u16..60),
        widgets in widget_set(100, 60, 20),
        qx in 0u16..100,
        qy in 0u16..60,
    ) {
        let mut index = SpatialHitIndex::with_defaults(w, h);
        for (i, (rect, z)) in widgets.iter().enumerate() {
            index.register(
                HitId::new(i as u32 + 1),
                *rect,
                HitRegion::Content,
                i as HitData,
                *z,
            );
        }

        let readonly_result = index.hit_test_readonly(qx, qy);
        let cached_result = index.hit_test(qx, qy);

        prop_assert_eq!(
            readonly_result.map(|(id, _, _)| id),
            cached_result.map(|(id, _, _)| id),
            "hit_test and hit_test_readonly disagree at ({},{})", qx, qy
        );
    }
}

// ═════════════════════════════════════════════════════════════════════════
// 5. Register + remove round-trip
// ═════════════════════════════════════════════════════════════════════════

proptest! {
    #[test]
    fn register_remove_roundtrip(
        (w, h) in screen_dims(),
        widgets in widget_set(200, 100, 30),
    ) {
        let mut index = SpatialHitIndex::with_defaults(w, h);

        // Register all
        for (i, (rect, z)) in widgets.iter().enumerate() {
            index.register(
                HitId::new(i as u32 + 1),
                *rect,
                HitRegion::Content,
                i as HitData,
                *z,
            );
        }
        prop_assert_eq!(index.len(), widgets.len());

        // Remove all
        for i in 0..widgets.len() {
            let removed = index.remove(HitId::new(i as u32 + 1));
            prop_assert!(removed, "Widget {} should have been removable", i);
        }
        prop_assert_eq!(index.len(), 0, "All widgets should be removed");
    }
}

// ═════════════════════════════════════════════════════════════════════════
// 6. Empty index returns None for any query
// ═════════════════════════════════════════════════════════════════════════

proptest! {
    #[test]
    fn empty_index_returns_none(
        (w, h) in screen_dims(),
        qx in 0u16..200,
        qy in 0u16..100,
    ) {
        let mut index = SpatialHitIndex::with_defaults(w, h);
        prop_assert!(index.hit_test(qx, qy).is_none(),
            "Empty index should return None for ({},{})", qx, qy);
    }
}

// ═════════════════════════════════════════════════════════════════════════
// 7. Determinism: same operations produce same results
// ═════════════════════════════════════════════════════════════════════════

proptest! {
    #[test]
    fn hit_test_deterministic(
        (w, h) in (10u16..80, 10u16..40),
        widgets in widget_set(80, 40, 20),
        queries in proptest::collection::vec((0u16..80, 0u16..40), 1..20),
    ) {
        // Run 1
        let mut idx1 = SpatialHitIndex::with_defaults(w, h);
        for (i, (rect, z)) in widgets.iter().enumerate() {
            idx1.register(HitId::new(i as u32 + 1), *rect, HitRegion::Content, i as HitData, *z);
        }
        let results1: Vec<_> = queries.iter()
            .map(|&(x, y)| idx1.hit_test_readonly(x, y).map(|(id, _, _)| id))
            .collect();

        // Run 2
        let mut idx2 = SpatialHitIndex::with_defaults(w, h);
        for (i, (rect, z)) in widgets.iter().enumerate() {
            idx2.register(HitId::new(i as u32 + 1), *rect, HitRegion::Content, i as HitData, *z);
        }
        let results2: Vec<_> = queries.iter()
            .map(|&(x, y)| idx2.hit_test_readonly(x, y).map(|(id, _, _)| id))
            .collect();

        prop_assert_eq!(results1, results2, "Hit test results differ between identical setups");
    }
}

// ═════════════════════════════════════════════════════════════════════════
// 8. Clear empties the index completely
// ═════════════════════════════════════════════════════════════════════════

proptest! {
    #[test]
    fn clear_empties_index(
        (w, h) in screen_dims(),
        widgets in widget_set(200, 100, 30),
        qx in 0u16..200,
        qy in 0u16..100,
    ) {
        let mut index = SpatialHitIndex::with_defaults(w, h);
        for (i, (rect, z)) in widgets.iter().enumerate() {
            index.register(HitId::new(i as u32 + 1), *rect, HitRegion::Content, i as HitData, *z);
        }

        index.clear();

        prop_assert_eq!(index.len(), 0);
        prop_assert!(index.is_empty());
        prop_assert!(index.hit_test(qx, qy).is_none(),
            "Cleared index should return None");
    }
}

// ═════════════════════════════════════════════════════════════════════════
// 9. Widget count accuracy
// ═════════════════════════════════════════════════════════════════════════

proptest! {
    #[test]
    fn len_tracks_registrations(
        (w, h) in screen_dims(),
        widgets in widget_set(200, 100, 50),
    ) {
        let mut index = SpatialHitIndex::with_defaults(w, h);
        for (i, (rect, z)) in widgets.iter().enumerate() {
            index.register(HitId::new(i as u32 + 1), *rect, HitRegion::Content, i as HitData, *z);
            prop_assert_eq!(index.len(), i + 1, "len should be {} after {} registrations", i + 1, i + 1);
        }
    }
}

// ═════════════════════════════════════════════════════════════════════════
// 10. Single widget hit within its rect, miss outside
// ═════════════════════════════════════════════════════════════════════════

proptest! {
    #[test]
    fn single_widget_hit_within_miss_outside(
        (w, h) in (20u16..120, 20u16..60),
        rect in rect_within(120, 60),
    ) {
        let mut index = SpatialHitIndex::with_defaults(w, h);
        index.register(HitId::new(1), rect, HitRegion::Content, 42, 0);

        // Inside: should hit (if within screen bounds)
        let mid_x = rect.x + rect.width / 2;
        let mid_y = rect.y + rect.height / 2;
        if mid_x < w && mid_y < h {
            let result = index.hit_test(mid_x, mid_y);
            prop_assert!(result.is_some(),
                "Should hit widget at ({},{}) inside rect {:?}", mid_x, mid_y, rect);
            if let Some((id, _, _)) = result {
                prop_assert_eq!(id, HitId::new(1));
            }
        }

        // Just outside right edge (if within screen)
        let right = rect.x.saturating_add(rect.width);
        if right < w && rect.y < h {
            let result = index.hit_test(right, rect.y);
            prop_assert!(result.is_none(),
                "Should miss at right edge ({},{}) for rect {:?}", right, rect.y, rect);
        }

        // Just outside bottom edge (if within screen)
        let bottom = rect.y.saturating_add(rect.height);
        if rect.x < w && bottom < h {
            let result = index.hit_test(rect.x, bottom);
            prop_assert!(result.is_none(),
                "Should miss at bottom edge ({},{}) for rect {:?}", rect.x, bottom, rect);
        }
    }
}
