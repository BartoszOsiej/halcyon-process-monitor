//! Property-based invariant tests for the Fenwick tree (Binary Indexed Tree).
//!
//! These tests verify structural invariants that must hold for any valid inputs:
//!
//! 1. from_values matches sequential updates.
//! 2. prefix(n-1) == total().
//! 3. get(i) recovers original values after from_values.
//! 4. range(0, n-1) == total().
//! 5. range(i, i) == get(i).
//! 6. prefix(i) == sum of get(0)..=get(i).
//! 7. set followed by get recovers the set value.
//! 8. batch_update matches sequential update calls.
//! 9. rebuild produces same tree as from_values.
//! 10. resize preserves existing values.
//! 11. find_prefix returns valid index (prefix sum <= target).
//! 12. Determinism: same operations always produce same results.
//! 13. No panics on valid-range operations.

use ftui_widgets::fenwick::FenwickTree;
use proptest::prelude::*;

// ── Helpers ─────────────────────────────────────────────────────────────

fn small_values(max_len: usize) -> impl Strategy<Value = Vec<u32>> {
    proptest::collection::vec(0u32..=1000, 1..=max_len)
}

fn naive_prefix_sum(values: &[u32], i: usize) -> u32 {
    values[..=i].iter().copied().fold(0u32, u32::wrapping_add)
}

fn naive_total(values: &[u32]) -> u32 {
    values.iter().copied().fold(0u32, u32::wrapping_add)
}

// ═════════════════════════════════════════════════════════════════════════
// 1. from_values matches sequential updates
// ═════════════════════════════════════════════════════════════════════════

proptest! {
    #[test]
    fn from_values_matches_sequential(values in small_values(100)) {
        let ft_bulk = FenwickTree::from_values(&values);

        let mut ft_seq = FenwickTree::new(values.len());
        for (i, &v) in values.iter().enumerate() {
            ft_seq.update(i, v as i32);
        }

        for i in 0..values.len() {
            prop_assert_eq!(
                ft_bulk.get(i), ft_seq.get(i),
                "Mismatch at index {} for values {:?}", i, &values[..values.len().min(20)]
            );
        }
    }
}

// ═════════════════════════════════════════════════════════════════════════
// 2. prefix(n-1) == total()
// ═════════════════════════════════════════════════════════════════════════

proptest! {
    #[test]
    fn prefix_last_equals_total(values in small_values(100)) {
        let ft = FenwickTree::from_values(&values);
        prop_assert_eq!(
            ft.prefix(values.len() - 1),
            ft.total(),
            "prefix(n-1) != total() for {} values",
            values.len()
        );
    }
}

// ═════════════════════════════════════════════════════════════════════════
// 3. get(i) recovers original values
// ═════════════════════════════════════════════════════════════════════════

proptest! {
    #[test]
    fn get_recovers_original(values in small_values(100)) {
        let ft = FenwickTree::from_values(&values);
        for (i, &v) in values.iter().enumerate() {
            prop_assert_eq!(
                ft.get(i), v,
                "get({}) = {} but expected {} from values",
                i, ft.get(i), v
            );
        }
    }
}

// ═════════════════════════════════════════════════════════════════════════
// 4. range(0, n-1) == total()
// ═════════════════════════════════════════════════════════════════════════

proptest! {
    #[test]
    fn full_range_equals_total(values in small_values(100)) {
        let ft = FenwickTree::from_values(&values);
        prop_assert_eq!(
            ft.range(0, values.len() - 1),
            ft.total(),
            "range(0, n-1) != total()"
        );
    }
}

// ═════════════════════════════════════════════════════════════════════════
// 5. range(i, i) == get(i)
// ═════════════════════════════════════════════════════════════════════════

proptest! {
    #[test]
    fn singleton_range_equals_get(
        values in small_values(50),
        idx_frac in 0.0f64..1.0,
    ) {
        let ft = FenwickTree::from_values(&values);
        let i = (idx_frac * values.len() as f64) as usize;
        let i = i.min(values.len() - 1);
        prop_assert_eq!(
            ft.range(i, i),
            ft.get(i),
            "range({},{}) != get({})",
            i, i, i
        );
    }
}

// ═════════════════════════════════════════════════════════════════════════
// 6. prefix(i) agrees with naive sum
// ═════════════════════════════════════════════════════════════════════════

proptest! {
    #[test]
    fn prefix_agrees_with_naive(values in small_values(100)) {
        let ft = FenwickTree::from_values(&values);
        for i in 0..values.len() {
            let expected = naive_prefix_sum(&values, i);
            prop_assert_eq!(
                ft.prefix(i), expected,
                "prefix({}) = {} but naive sum = {}",
                i, ft.prefix(i), expected
            );
        }
    }
}

// ═════════════════════════════════════════════════════════════════════════
// 7. set followed by get recovers value
// ═════════════════════════════════════════════════════════════════════════

proptest! {
    #[test]
    fn set_then_get_roundtrip(
        values in small_values(50),
        idx_frac in 0.0f64..1.0,
        new_val in any::<u32>(),
    ) {
        let mut ft = FenwickTree::from_values(&values);
        let i = (idx_frac * values.len() as f64) as usize;
        let i = i.min(values.len() - 1);

        ft.set(i, new_val);
        prop_assert_eq!(
            ft.get(i), new_val,
            "set({}, {}) then get({}) returned {}",
            i, new_val, i, ft.get(i)
        );
    }
}

// ═════════════════════════════════════════════════════════════════════════
// 8. batch_update matches sequential updates
// ═════════════════════════════════════════════════════════════════════════

proptest! {
    #[test]
    fn batch_equals_sequential(
        values in small_values(50),
        deltas in proptest::collection::vec(
            (0.0f64..1.0, -100i32..=100),
            0..=20,
        ),
    ) {
        let n = values.len();
        let mapped_deltas: Vec<(usize, i32)> = deltas.iter()
            .map(|&(frac, d)| {
                let idx = (frac * n as f64) as usize;
                (idx.min(n - 1), d)
            })
            .collect();

        let mut ft_seq = FenwickTree::from_values(&values);
        for &(i, d) in &mapped_deltas {
            ft_seq.update(i, d);
        }

        let mut ft_batch = FenwickTree::from_values(&values);
        ft_batch.batch_update(&mapped_deltas);

        for i in 0..n {
            prop_assert_eq!(
                ft_seq.get(i), ft_batch.get(i),
                "batch vs sequential mismatch at index {}",
                i
            );
        }
    }
}

// ═════════════════════════════════════════════════════════════════════════
// 9. rebuild produces same tree as from_values
// ═════════════════════════════════════════════════════════════════════════

proptest! {
    #[test]
    fn rebuild_matches_from_values(
        values_a in small_values(50),
        values_b in small_values(50),
    ) {
        // Use same length for both
        let n = values_a.len().min(values_b.len());
        let a = &values_a[..n];
        let b = &values_b[..n];

        let ft_fresh = FenwickTree::from_values(b);
        let mut ft_rebuilt = FenwickTree::from_values(a);
        ft_rebuilt.rebuild(b);

        for i in 0..n {
            prop_assert_eq!(
                ft_fresh.get(i), ft_rebuilt.get(i),
                "rebuild mismatch at index {}",
                i
            );
        }
        prop_assert_eq!(ft_fresh.total(), ft_rebuilt.total());
    }
}

// ═════════════════════════════════════════════════════════════════════════
// 10. resize preserves existing values
// ═════════════════════════════════════════════════════════════════════════

proptest! {
    #[test]
    fn resize_grow_preserves(values in small_values(30), extra in 1usize..=20) {
        let mut ft = FenwickTree::from_values(&values);
        let old_n = values.len();
        ft.resize(old_n + extra);

        prop_assert_eq!(ft.len(), old_n + extra);
        for (i, &v) in values.iter().enumerate() {
            prop_assert_eq!(
                ft.get(i), v,
                "resize grow changed value at index {}",
                i
            );
        }
        for i in old_n..old_n + extra {
            prop_assert_eq!(ft.get(i), 0, "new element at {} should be 0", i);
        }
    }

    #[test]
    fn resize_shrink_preserves(values in small_values(30)) {
        if values.len() <= 1 {
            return Ok(());
        }
        let new_n = values.len() / 2;
        let mut ft = FenwickTree::from_values(&values);
        ft.resize(new_n);

        prop_assert_eq!(ft.len(), new_n);
        for (i, &v) in values.iter().enumerate().take(new_n) {
            prop_assert_eq!(
                ft.get(i), v,
                "resize shrink changed value at index {}",
                i
            );
        }
    }
}

// ═════════════════════════════════════════════════════════════════════════
// 11. find_prefix returns valid index
// ═════════════════════════════════════════════════════════════════════════

proptest! {
    #[test]
    fn find_prefix_valid(values in small_values(50), target in any::<u32>()) {
        let ft = FenwickTree::from_values(&values);
        match ft.find_prefix(target) {
            Some(i) => {
                prop_assert!(i < values.len(),
                    "find_prefix returned {} >= len {}", i, values.len());
                // prefix(i) <= target
                let psum = ft.prefix(i);
                prop_assert!(
                    psum <= target,
                    "find_prefix({}) returned i={} but prefix({})={} > {}",
                    target, i, i, psum, target
                );
            }
            None => {
                // All prefix sums > target, meaning even values[0] > target
                if !values.is_empty() {
                    prop_assert!(
                        values[0] > target,
                        "find_prefix returned None but values[0]={} <= target={}",
                        values[0], target
                    );
                }
            }
        }
    }
}

// ═════════════════════════════════════════════════════════════════════════
// 12. Determinism
// ═════════════════════════════════════════════════════════════════════════

proptest! {
    #[test]
    fn fenwick_deterministic(values in small_values(50)) {
        let ft1 = FenwickTree::from_values(&values);
        let ft2 = FenwickTree::from_values(&values);

        for i in 0..values.len() {
            prop_assert_eq!(ft1.get(i), ft2.get(i));
            prop_assert_eq!(ft1.prefix(i), ft2.prefix(i));
        }
        prop_assert_eq!(ft1.total(), ft2.total());
    }
}

// ═════════════════════════════════════════════════════════════════════════
// 13. total agrees with naive sum
// ═════════════════════════════════════════════════════════════════════════

proptest! {
    #[test]
    fn total_agrees_with_naive(values in small_values(100)) {
        let ft = FenwickTree::from_values(&values);
        let expected = naive_total(&values);
        prop_assert_eq!(
            ft.total(), expected,
            "total() = {} but naive sum = {}",
            ft.total(), expected
        );
    }
}

// ═════════════════════════════════════════════════════════════════════════
// 14. update preserves total correctly
// ═════════════════════════════════════════════════════════════════════════

proptest! {
    #[test]
    fn update_adjusts_total(
        values in small_values(50),
        idx_frac in 0.0f64..1.0,
        delta in 0i32..=500,
    ) {
        let mut ft = FenwickTree::from_values(&values);
        let i = (idx_frac * values.len() as f64) as usize;
        let i = i.min(values.len() - 1);

        let old_total = ft.total();
        ft.update(i, delta);
        let new_total = ft.total();

        prop_assert_eq!(
            new_total,
            old_total.wrapping_add(delta as u32),
            "total after update({}, {}) should be {} + {} = {}",
            i, delta, old_total, delta, old_total.wrapping_add(delta as u32)
        );
    }
}

// ═════════════════════════════════════════════════════════════════════════
// 15. range sum telescoping: range(a,b) + range(b+1,c) == range(a,c)
// ═════════════════════════════════════════════════════════════════════════

proptest! {
    #[test]
    fn range_telescoping(
        values in small_values(50),
        a_frac in 0.0f64..1.0,
        b_frac in 0.0f64..1.0,
        c_frac in 0.0f64..1.0,
    ) {
        let n = values.len();
        let mut indices: Vec<usize> = [a_frac, b_frac, c_frac]
            .iter()
            .map(|f| (f * n as f64) as usize)
            .map(|i| i.min(n - 1))
            .collect();
        indices.sort();
        let (a, b, c) = (indices[0], indices[1], indices[2]);

        if b < c {
            let ft = FenwickTree::from_values(&values);
            let left = ft.range(a, b);
            let right = ft.range(b + 1, c);
            let full = ft.range(a, c);

            prop_assert_eq!(
                left.wrapping_add(right), full,
                "range({},{}) + range({},{}) != range({},{})",
                a, b, b + 1, c, a, c
            );
        }
    }
}

// ═════════════════════════════════════════════════════════════════════════
// 16. No panics on valid operations
// ═════════════════════════════════════════════════════════════════════════

proptest! {
    #[test]
    fn no_panic_operations(values in small_values(50)) {
        let mut ft = FenwickTree::from_values(&values);
        let n = values.len();

        // All basic ops on every index
        for i in 0..n {
            let _ = ft.get(i);
            let _ = ft.prefix(i);
            let _ = ft.range(i, n - 1);
        }
        let _ = ft.total();
        let _ = ft.find_prefix(0);
        let _ = ft.find_prefix(u32::MAX);
        let _ = ft.len();
        let _ = ft.is_empty();

        ft.update(0, 1);
        ft.set(0, 42);
        ft.rebuild(&values);
    }
}
