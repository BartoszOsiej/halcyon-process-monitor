//! Property tests: succinct data structures match dense counterparts (bd-1wevm.4)
//!
//! Verifies that Elias-Fano and LOUDS produce identical results to
//! naive dense implementations for all operations.

use ftui_widgets::elias_fano::EliasFano;
use ftui_widgets::louds::LoudsTree;
use proptest::prelude::*;

// ── Dense reference: sorted array ────────────────────────────────

/// Dense rank: count values ≤ v in sorted slice (matches EliasFano::rank semantics).
fn dense_rank(sorted: &[u64], v: u64) -> usize {
    sorted.partition_point(|&x| x <= v)
}

/// Dense select: return the r-th element (0-indexed).
fn dense_select(sorted: &[u64], r: usize) -> u64 {
    sorted[r]
}

/// Dense next_geq: first element >= v.
fn dense_next_geq(sorted: &[u64], v: u64) -> Option<(usize, u64)> {
    let idx = sorted.partition_point(|&x| x < v);
    if idx < sorted.len() {
        Some((idx, sorted[idx]))
    } else {
        None
    }
}

// ── Dense reference: pointer-based tree ──────────────────────────

/// A simple pointer-based tree for reference comparison.
struct DenseTree {
    parent: Vec<Option<usize>>,
    children: Vec<Vec<usize>>,
}

impl DenseTree {
    fn from_degrees(degrees: &[usize]) -> Self {
        let n = degrees.len();
        let mut parent = vec![None; n];
        let mut children = vec![vec![]; n];

        // BFS assignment: degrees[0] is root, etc.
        let mut queue = std::collections::VecDeque::new();
        queue.push_back(0);
        let mut next_child = 1;

        for v in 0..n {
            if next_child > n {
                break;
            }
            for _ in 0..degrees[v] {
                if next_child >= n {
                    break;
                }
                parent[next_child] = Some(v);
                children[v].push(next_child);
                queue.push_back(next_child);
                next_child += 1;
            }
        }

        Self { parent, children }
    }

    fn parent(&self, v: usize) -> Option<usize> {
        self.parent[v]
    }

    fn first_child(&self, v: usize) -> Option<usize> {
        self.children[v].first().copied()
    }

    fn next_sibling(&self, v: usize) -> Option<usize> {
        let p = self.parent[v]?;
        let siblings = &self.children[p];
        let idx = siblings.iter().position(|&c| c == v)?;
        siblings.get(idx + 1).copied()
    }

    fn is_leaf(&self, v: usize) -> bool {
        self.children[v].is_empty()
    }

    fn node_count(&self) -> usize {
        self.parent.len()
    }
}

// ── Strategies ───────────────────────────────────────────────────

/// Generate a sorted, deduplicated sequence of u64.
fn arb_sorted_sequence(max_len: usize) -> impl Strategy<Value = Vec<u64>> {
    prop::collection::vec(0u64..100_000, 1..=max_len).prop_map(|mut v| {
        v.sort_unstable();
        v.dedup();
        v
    })
}

/// Generate valid BFS degree sequences for tree construction.
///
/// Ensures: sum(degrees) == n-1 AND cumulative sum after node i >= i+1
/// for all i < n-1 (BFS reachability invariant).
fn arb_degree_sequence(max_nodes: usize) -> impl Strategy<Value = Vec<usize>> {
    (2..=max_nodes).prop_flat_map(|n| {
        let n = n.min(50); // cap for test speed
        prop::collection::vec(0..=4usize, n).prop_map(move |raw| {
            let mut degrees = vec![0usize; n];
            let target = n - 1;
            let mut total = 0usize;

            for i in 0..n {
                let remaining = target - total;
                if remaining == 0 {
                    break;
                }
                // BFS reachability: after assigning degrees[i], cumulative
                // must be >= i+1 (so node i+1 exists as someone's child).
                let min_for_reach = if i < n - 1 {
                    (i + 1).saturating_sub(total)
                } else {
                    remaining
                };
                let max_allowed = remaining.min(4);
                let d = raw[i].clamp(min_for_reach, max_allowed);
                degrees[i] = d;
                total += d;
            }

            // Safety: absorb any remaining deficit into root
            let deficit = target.saturating_sub(total);
            if deficit > 0 {
                degrees[0] += deficit;
            }

            degrees
        })
    })
}

// ── Elias-Fano property tests ────────────────────────────────────

proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]

    #[test]
    fn ef_access_matches_dense(sorted in arb_sorted_sequence(50)) {
        let ef = EliasFano::encode(&sorted);
        for (i, &expected) in sorted.iter().enumerate() {
            prop_assert_eq!(
                ef.access(i), expected,
                "access({}) mismatch: ef={}, dense={}", i, ef.access(i), expected
            );
        }
    }

    #[test]
    fn ef_rank_matches_dense(sorted in arb_sorted_sequence(50)) {
        let ef = EliasFano::encode(&sorted);
        // Test rank at each stored value and at boundaries
        for &v in &sorted {
            prop_assert_eq!(
                ef.rank(v), dense_rank(&sorted, v),
                "rank({}) mismatch", v
            );
            // Also test v+1
            prop_assert_eq!(
                ef.rank(v + 1), dense_rank(&sorted, v + 1),
                "rank({}) mismatch", v + 1,
            );
        }
        // Test rank(0)
        prop_assert_eq!(ef.rank(0), dense_rank(&sorted, 0));
    }

    #[test]
    fn ef_select_matches_dense(sorted in arb_sorted_sequence(50)) {
        let ef = EliasFano::encode(&sorted);
        for r in 0..sorted.len() {
            prop_assert_eq!(
                ef.select(r), dense_select(&sorted, r),
                "select({}) mismatch", r
            );
        }
    }

    #[test]
    fn ef_next_geq_matches_dense(sorted in arb_sorted_sequence(50)) {
        let ef = EliasFano::encode(&sorted);
        // Test at stored values
        for &v in &sorted {
            prop_assert_eq!(
                ef.next_geq(v), dense_next_geq(&sorted, v),
                "next_geq({}) mismatch", v
            );
        }
        // Test beyond max
        if let Some(&max) = sorted.last() {
            prop_assert_eq!(ef.next_geq(max + 1), dense_next_geq(&sorted, max + 1));
        }
        // Test at 0
        prop_assert_eq!(ef.next_geq(0), dense_next_geq(&sorted, 0));
    }

    #[test]
    fn ef_len_matches(sorted in arb_sorted_sequence(50)) {
        let ef = EliasFano::encode(&sorted);
        prop_assert_eq!(ef.len(), sorted.len());
        prop_assert_eq!(ef.is_empty(), sorted.is_empty());
    }

    #[test]
    fn ef_roundtrip_identical(sorted in arb_sorted_sequence(50)) {
        let ef1 = EliasFano::encode(&sorted);
        // Rebuild from accessed values
        let rebuilt: Vec<u64> = (0..ef1.len()).map(|i| ef1.access(i)).collect();
        prop_assert_eq!(&rebuilt, &sorted);
        // Re-encode should produce same access results
        let ef2 = EliasFano::encode(&rebuilt);
        for i in 0..sorted.len() {
            prop_assert_eq!(ef1.access(i), ef2.access(i));
        }
    }
}

// ── LOUDS property tests ─────────────────────────────────────────

proptest! {
    #![proptest_config(ProptestConfig::with_cases(50))]

    #[test]
    fn louds_parent_matches_dense(degrees in arb_degree_sequence(30)) {
        let n = degrees.len();
        if n < 2 { return Ok(()); }

        let louds = LoudsTree::from_degrees(&degrees);
        let dense = DenseTree::from_degrees(&degrees);

        prop_assert_eq!(louds.node_count(), dense.node_count());

        for v in 0..louds.node_count() {
            prop_assert_eq!(
                louds.parent(v), dense.parent(v),
                "parent({}) mismatch: louds={:?}, dense={:?}", v,
                louds.parent(v), dense.parent(v)
            );
        }
    }

    #[test]
    fn louds_first_child_matches_dense(degrees in arb_degree_sequence(30)) {
        let n = degrees.len();
        if n < 2 { return Ok(()); }

        let louds = LoudsTree::from_degrees(&degrees);
        let dense = DenseTree::from_degrees(&degrees);

        for v in 0..louds.node_count() {
            prop_assert_eq!(
                louds.first_child(v), dense.first_child(v),
                "first_child({}) mismatch", v
            );
        }
    }

    #[test]
    fn louds_next_sibling_matches_dense(degrees in arb_degree_sequence(30)) {
        let n = degrees.len();
        if n < 2 { return Ok(()); }

        let louds = LoudsTree::from_degrees(&degrees);
        let dense = DenseTree::from_degrees(&degrees);

        for v in 0..louds.node_count() {
            prop_assert_eq!(
                louds.next_sibling(v), dense.next_sibling(v),
                "next_sibling({}) mismatch", v
            );
        }
    }

    #[test]
    fn louds_is_leaf_matches_dense(degrees in arb_degree_sequence(30)) {
        let n = degrees.len();
        if n < 2 { return Ok(()); }

        let louds = LoudsTree::from_degrees(&degrees);
        let dense = DenseTree::from_degrees(&degrees);

        for v in 0..louds.node_count() {
            prop_assert_eq!(
                louds.is_leaf(v), dense.is_leaf(v),
                "is_leaf({}) mismatch", v
            );
        }
    }

    #[test]
    fn louds_degree_matches_dense(degrees in arb_degree_sequence(30)) {
        let n = degrees.len();
        if n < 2 { return Ok(()); }

        let louds = LoudsTree::from_degrees(&degrees);
        let dense = DenseTree::from_degrees(&degrees);

        for v in 0..louds.node_count() {
            prop_assert_eq!(
                louds.degree(v), dense.children[v].len(),
                "degree({}) mismatch", v
            );
        }
    }

    #[test]
    fn louds_children_iter_matches_dense(degrees in arb_degree_sequence(30)) {
        let n = degrees.len();
        if n < 2 { return Ok(()); }

        let louds = LoudsTree::from_degrees(&degrees);
        let dense = DenseTree::from_degrees(&degrees);

        for v in 0..louds.node_count() {
            let louds_children: Vec<usize> = louds.children(v).collect();
            prop_assert_eq!(
                &louds_children, &dense.children[v],
                "children({}) mismatch", v
            );
        }
    }

    #[test]
    fn louds_dfs_order_matches_dense(degrees in arb_degree_sequence(20)) {
        let n = degrees.len();
        if n < 2 { return Ok(()); }

        let louds = LoudsTree::from_degrees(&degrees);
        let dense = DenseTree::from_degrees(&degrees);

        // DFS traversal via LOUDS
        let mut louds_dfs = Vec::new();
        let mut stack = vec![0usize];
        while let Some(v) = stack.pop() {
            louds_dfs.push(v);
            // Push children in reverse order so leftmost is popped first
            let children: Vec<usize> = louds.children(v).collect();
            for &c in children.iter().rev() {
                stack.push(c);
            }
        }

        // DFS traversal via dense tree
        let mut dense_dfs = Vec::new();
        let mut stack = vec![0usize];
        while let Some(v) = stack.pop() {
            dense_dfs.push(v);
            for &c in dense.children[v].iter().rev() {
                stack.push(c);
            }
        }

        prop_assert_eq!(&louds_dfs, &dense_dfs, "DFS order mismatch");
    }

    #[test]
    fn louds_roundtrip_from_degrees(degrees in arb_degree_sequence(30)) {
        let n = degrees.len();
        if n < 2 { return Ok(()); }

        let louds = LoudsTree::from_degrees(&degrees);
        // Verify node_count matches
        prop_assert_eq!(louds.node_count(), n);
        // Verify core navigation operations don't panic
        for v in 0..n {
            let _ = louds.parent(v);
            let _ = louds.first_child(v);
            let _ = louds.next_sibling(v);
            let _ = louds.is_leaf(v);
            let _ = louds.degree(v);
            let _: Vec<_> = louds.children(v).collect();
        }
        // depth/subtree_size are O(n) per call — spot-check root only
        prop_assert_eq!(louds.depth(0), 0);
        prop_assert_eq!(louds.subtree_size(0), n);
    }
}

// ── Edge case tests ──────────────────────────────────────────────

#[test]
fn ef_single_element() {
    let sorted = vec![42u64];
    let ef = EliasFano::encode(&sorted);
    assert_eq!(ef.len(), 1);
    assert_eq!(ef.access(0), 42);
    assert_eq!(ef.rank(42), 1);
    assert_eq!(ef.rank(43), 1);
    assert_eq!(ef.select(0), 42);
    assert_eq!(ef.next_geq(0), Some((0, 42)));
    assert_eq!(ef.next_geq(42), Some((0, 42)));
    assert_eq!(ef.next_geq(43), None);
}

#[test]
fn ef_all_same_after_dedup() {
    // After dedup, [5,5,5] becomes [5]
    let sorted = vec![5u64];
    let ef = EliasFano::encode(&sorted);
    assert_eq!(ef.len(), 1);
    assert_eq!(ef.access(0), 5);
}

#[test]
fn ef_maximum_values() {
    let sorted = vec![0u64, u64::MAX / 2, u64::MAX - 1];
    let ef = EliasFano::encode(&sorted);
    assert_eq!(ef.len(), 3);
    for (i, &v) in sorted.iter().enumerate() {
        assert_eq!(ef.access(i), v);
    }
}

#[test]
fn louds_single_node() {
    let louds = LoudsTree::from_degrees(&[0]);
    assert_eq!(louds.node_count(), 1);
    assert!(louds.is_leaf(0));
    assert_eq!(louds.parent(0), None);
    assert_eq!(louds.first_child(0), None);
    assert_eq!(louds.degree(0), 0);
}

#[test]
fn louds_linear_chain() {
    // A → B → C → D (each node has 1 child except leaf)
    let louds = LoudsTree::from_degrees(&[1, 1, 1, 0]);
    assert_eq!(louds.node_count(), 4);
    assert_eq!(louds.parent(0), None);
    assert_eq!(louds.parent(1), Some(0));
    assert_eq!(louds.parent(2), Some(1));
    assert_eq!(louds.parent(3), Some(2));
    assert!(louds.is_leaf(3));
    assert!(!louds.is_leaf(0));
}

#[test]
fn louds_wide_tree() {
    // Root with 10 children, all leaves
    let mut degrees = vec![10];
    degrees.extend(vec![0; 10]);
    let louds = LoudsTree::from_degrees(&degrees);
    assert_eq!(louds.node_count(), 11);
    assert_eq!(louds.degree(0), 10);
    for i in 1..=10 {
        assert!(louds.is_leaf(i));
        assert_eq!(louds.parent(i), Some(0));
    }
}
