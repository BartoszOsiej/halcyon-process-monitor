//! E2E integration tests: succinct data structures in the widget pipeline (bd-1wevm.5)
//!
//! Verifies end-to-end correctness of Elias-Fano and LOUDS within realistic
//! widget scenarios: scroll-to-offset, viewport resize, dynamic insert/delete,
//! threshold crossover, tree navigation, round-trip, and degenerate inputs.
//!
//! All tests emit structured JSONL evidence to stdout.

use ftui_widgets::elias_fano::EliasFano;
use ftui_widgets::louds::LoudsTree;
use serde::Serialize;
use std::time::Instant;

// ── JSONL Evidence ──────────────────────────────────────────────────

#[derive(Serialize)]
struct Evidence {
    test: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    row_count: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    node_count: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    encoding: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    memory_bytes: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    dense_memory_bytes: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    ratio: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    query_ns: Option<u128>,
    pass: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    detail: Option<String>,
}

impl Evidence {
    fn new(test: &'static str) -> Self {
        Self {
            test,
            row_count: None,
            node_count: None,
            encoding: None,
            memory_bytes: None,
            dense_memory_bytes: None,
            ratio: None,
            query_ns: None,
            pass: true,
            detail: None,
        }
    }

    fn emit(&self) {
        println!("{}", serde_json::to_string(self).unwrap());
    }
}

// ── Helpers ─────────────────────────────────────────────────────────

fn make_prefix_sums(n: usize, avg_height: u64) -> Vec<u64> {
    let mut sums = Vec::with_capacity(n);
    let mut acc = 0u64;
    for i in 0..n {
        acc += avg_height + (i as u64 % 5);
        sums.push(acc);
    }
    sums
}

/// Given pixel offset, find the visible row index using dense binary search.
fn dense_scroll_to_row(sums: &[u64], offset_px: u64) -> usize {
    sums.partition_point(|&s| s <= offset_px)
}

/// Given pixel offset, find the visible row index using Elias-Fano rank.
fn ef_scroll_to_row(ef: &EliasFano, offset_px: u64) -> usize {
    ef.rank(offset_px)
}

// ── Test 1: Scroll-to-offset ────────────────────────────────────────

#[test]
fn e2e_scroll_to_offset() {
    for &n in &[100, 1_000, 10_000, 100_000] {
        let sums = make_prefix_sums(n, 20);
        let ef = EliasFano::encode(&sums);

        // Test at 50 deterministic offsets spread across the range
        let max_px = *sums.last().unwrap();
        let step = max_px / 50;

        let start = Instant::now();
        for i in 0..50 {
            let offset_px = i * step;
            let expected = dense_scroll_to_row(&sums, offset_px);
            let actual = ef_scroll_to_row(&ef, offset_px);
            assert_eq!(
                actual, expected,
                "row mismatch at offset {offset_px}: ef={actual}, dense={expected} (n={n})"
            );
        }
        let elapsed = start.elapsed();

        Evidence {
            test: "scroll_to_offset",
            row_count: Some(n),
            encoding: Some("elias_fano"),
            query_ns: Some(elapsed.as_nanos() / 50),
            memory_bytes: Some(ef.size_in_bytes()),
            dense_memory_bytes: Some(n * 8),
            ratio: Some(ef.size_in_bytes() as f64 / (n * 8) as f64),
            ..Evidence::new("scroll_to_offset")
        }
        .emit();
    }
}

// ── Test 2: Viewport resize ─────────────────────────────────────────

#[test]
fn e2e_viewport_resize() {
    let n = 10_000;
    let sums = make_prefix_sums(n, 20);
    let ef = EliasFano::encode(&sums);

    // Simulate viewport at different heights: which rows are visible?
    for viewport_h in [10u64, 24, 40, 60, 100] {
        // Scroll to middle
        let scroll_px = *sums.last().unwrap() / 2;
        let first_row = ef_scroll_to_row(&ef, scroll_px);
        let expected_first = dense_scroll_to_row(&sums, scroll_px);
        assert_eq!(first_row, expected_first, "viewport_h={viewport_h}");

        // Find last visible row
        let last_visible_px = scroll_px + viewport_h;
        let last_row = ef_scroll_to_row(&ef, last_visible_px);
        let expected_last = dense_scroll_to_row(&sums, last_visible_px);
        assert_eq!(
            last_row, expected_last,
            "last row at viewport_h={viewport_h}"
        );

        let visible_count = last_row.saturating_sub(first_row);

        Evidence {
            test: "viewport_resize",
            row_count: Some(n),
            encoding: Some("elias_fano"),
            detail: Some(format!(
                "viewport_h={viewport_h} first={first_row} last={last_row} visible={visible_count}"
            )),
            ..Evidence::new("viewport_resize")
        }
        .emit();
    }
}

// ── Test 3: Dynamic insert/delete (rebuild) ─────────────────────────

#[test]
fn e2e_dynamic_rebuild() {
    let mut sums = make_prefix_sums(1_000, 20);
    let ef1 = EliasFano::encode(&sums);

    // Record current scroll position
    let scroll_px = sums[500];
    let row_before = ef_scroll_to_row(&ef1, scroll_px);

    // Insert a row at position 250 (height 25)
    let insert_height = 25u64;
    let insert_idx = 250;
    // Shift all sums from insert_idx onward by insert_height
    for s in sums.iter_mut().skip(insert_idx) {
        *s += insert_height;
    }
    // Insert the new cumulative sum
    let new_sum = if insert_idx > 0 {
        sums[insert_idx - 1] + insert_height
    } else {
        insert_height
    };
    sums.insert(insert_idx, new_sum);

    // Rebuild EF
    let ef2 = EliasFano::encode(&sums);

    // Verify scroll position maps to correct row in rebuilt structure
    let adjusted_px = scroll_px + insert_height; // scroll target shifted
    let row_after = ef_scroll_to_row(&ef2, adjusted_px);
    let expected_after = dense_scroll_to_row(&sums, adjusted_px);
    assert_eq!(row_after, expected_after);

    Evidence {
        test: "dynamic_rebuild",
        row_count: Some(sums.len()),
        encoding: Some("elias_fano"),
        detail: Some(format!(
            "row_before={row_before} row_after={row_after} insert_idx={insert_idx}"
        )),
        ..Evidence::new("dynamic_rebuild")
    }
    .emit();
}

// ── Test 4: Threshold crossover ─────────────────────────────────────

#[test]
fn e2e_threshold_crossover() {
    // Below and above a threshold, both should give identical results
    for &n in &[99, 100, 101, 999, 1_000, 1_001] {
        let sums = make_prefix_sums(n, 20);
        let ef = EliasFano::encode(&sums);

        // Verify every access matches
        for (i, &sum) in sums.iter().enumerate() {
            assert_eq!(ef.access(i), sum, "access mismatch at i={i}, n={n}");
        }

        // Verify rank at boundaries
        let max_val = *sums.last().unwrap();
        for q in [0, max_val / 4, max_val / 2, max_val * 3 / 4, max_val] {
            let ef_rank = ef.rank(q);
            let dense_rank = sums.partition_point(|&s| s <= q);
            assert_eq!(ef_rank, dense_rank, "rank mismatch at q={q}, n={n}");
        }

        Evidence {
            test: "threshold_crossover",
            row_count: Some(n),
            encoding: Some("elias_fano"),
            memory_bytes: Some(ef.size_in_bytes()),
            dense_memory_bytes: Some(n * 8),
            ratio: Some(ef.size_in_bytes() as f64 / (n * 8) as f64),
            ..Evidence::new("threshold_crossover")
        }
        .emit();
    }
}

// ── Test 5: LOUDS tree navigation ───────────────────────────────────

#[test]
fn e2e_louds_tree_navigation() {
    // Build a realistic widget hierarchy: root has 5 panels, each has
    // 10 sections, each has 100 items = 5551 nodes
    let mut degrees = Vec::new();
    degrees.push(5); // root
    degrees.extend(std::iter::repeat_n(10, 5)); // panel
    degrees.extend(std::iter::repeat_n(100, 50)); // section
    // All remaining nodes are leaves
    let total_children: usize = degrees.iter().sum();
    let total_nodes = total_children + 1; // root + all children
    degrees.resize(total_nodes, 0);

    let louds = LoudsTree::from_degrees(&degrees);

    // Verify structure
    assert_eq!(louds.node_count(), total_nodes);
    assert_eq!(louds.degree(0), 5); // root has 5 children

    // Verify all leaves are actually leaves
    let leaf_start = 1 + 5 + 50; // root + panels + sections
    for v in leaf_start..total_nodes {
        assert!(louds.is_leaf(v), "node {v} should be leaf");
    }

    // Verify parent chain for a deep node
    let deep_node = leaf_start + 50; // first leaf of second section of first panel
    let mut chain = vec![deep_node];
    let mut cur = deep_node;
    while let Some(p) = louds.parent(cur) {
        chain.push(p);
        cur = p;
    }
    assert_eq!(*chain.last().unwrap(), 0, "chain should end at root");

    let louds_bytes = louds.size_in_bytes();
    let pointer_bytes = total_nodes * 3 * 8;

    Evidence {
        test: "louds_tree_navigation",
        node_count: Some(total_nodes),
        encoding: Some("louds"),
        memory_bytes: Some(louds_bytes),
        dense_memory_bytes: Some(pointer_bytes),
        ratio: Some(louds_bytes as f64 / pointer_bytes as f64),
        detail: Some(format!(
            "depth_chain_len={} root_degree=5 leaf_count={}",
            chain.len(),
            total_nodes - leaf_start
        )),
        ..Evidence::new("louds_tree_navigation")
    }
    .emit();
}

// ── Test 6: Round-trip ──────────────────────────────────────────────

#[test]
fn e2e_roundtrip_bitwise_identical() {
    for &n in &[100, 1_000, 10_000, 100_000] {
        let sums = make_prefix_sums(n, 20);
        let ef = EliasFano::encode(&sums);

        // Reconstruct from EF
        let reconstructed: Vec<u64> = (0..n).map(|i| ef.access(i)).collect();
        assert_eq!(
            &reconstructed,
            &sums,
            "round-trip failed at n={n}: first diff at {:?}",
            reconstructed
                .iter()
                .zip(sums.iter())
                .position(|(a, b)| a != b)
        );

        // Re-encode and verify bitwise identical access
        let ef2 = EliasFano::encode(&reconstructed);
        for i in 0..n {
            assert_eq!(ef.access(i), ef2.access(i), "re-encode mismatch at i={i}");
        }

        Evidence {
            test: "roundtrip",
            row_count: Some(n),
            encoding: Some("elias_fano"),
            ..Evidence::new("roundtrip")
        }
        .emit();
    }
}

// ── Test 7: Degenerate inputs ───────────────────────────────────────

#[test]
fn e2e_degenerate_inputs() {
    // All same heights → constant prefix sums
    let same_sums: Vec<u64> = (1..=1000).map(|i| i * 20).collect();
    let ef_same = EliasFano::encode(&same_sums);
    assert_eq!(ef_same.len(), 1000);
    for (i, &sum) in same_sums.iter().enumerate() {
        assert_eq!(ef_same.access(i), sum);
    }
    Evidence {
        test: "degenerate_same_height",
        row_count: Some(1000),
        encoding: Some("elias_fano"),
        ..Evidence::new("degenerate_same_height")
    }
    .emit();

    // Zero heights → all sums are 0
    let zero_sums = vec![0u64; 100];
    let ef_zero = EliasFano::encode(&zero_sums);
    assert_eq!(ef_zero.len(), 100);
    for i in 0..100 {
        assert_eq!(ef_zero.access(i), 0);
    }
    assert_eq!(ef_zero.rank(0), 100); // all elements ≤ 0
    assert_eq!(ef_zero.next_geq(0), Some((0, 0)));
    assert_eq!(ef_zero.next_geq(1), None);
    Evidence {
        test: "degenerate_zero_height",
        row_count: Some(100),
        encoding: Some("elias_fano"),
        ..Evidence::new("degenerate_zero_height")
    }
    .emit();

    // Maximum heights
    let max_sums = vec![0u64, u64::MAX / 4, u64::MAX / 2];
    let ef_max = EliasFano::encode(&max_sums);
    assert_eq!(ef_max.len(), 3);
    for (i, &sum) in max_sums.iter().enumerate() {
        assert_eq!(ef_max.access(i), sum);
    }
    Evidence {
        test: "degenerate_max_height",
        row_count: Some(3),
        encoding: Some("elias_fano"),
        ..Evidence::new("degenerate_max_height")
    }
    .emit();

    // Single row
    let single = vec![42u64];
    let ef_single = EliasFano::encode(&single);
    assert_eq!(ef_single.access(0), 42);
    assert_eq!(ef_single.rank(42), 1);
    assert_eq!(ef_single.next_geq(42), Some((0, 42)));
    Evidence {
        test: "degenerate_single_row",
        row_count: Some(1),
        encoding: Some("elias_fano"),
        ..Evidence::new("degenerate_single_row")
    }
    .emit();

    // LOUDS single node
    let louds_single = LoudsTree::from_degrees(&[0]);
    assert_eq!(louds_single.node_count(), 1);
    assert!(louds_single.is_leaf(0));
    Evidence {
        test: "degenerate_single_node_tree",
        node_count: Some(1),
        encoding: Some("louds"),
        ..Evidence::new("degenerate_single_node_tree")
    }
    .emit();
}

// ── Test 8: Memory comparison report ────────────────────────────────

#[test]
fn e2e_memory_report() {
    for &n in &[100, 1_000, 10_000, 100_000] {
        let sums = make_prefix_sums(n, 20);
        let ef = EliasFano::encode(&sums);

        let ef_bytes = ef.size_in_bytes();
        let dense_bytes = n * 8;
        let ratio = ef_bytes as f64 / dense_bytes as f64;

        assert!(
            ef_bytes < dense_bytes,
            "EF ({ef_bytes}B) should be smaller than dense ({dense_bytes}B) at n={n}"
        );

        Evidence {
            test: "memory_report",
            row_count: Some(n),
            encoding: Some("elias_fano"),
            memory_bytes: Some(ef_bytes),
            dense_memory_bytes: Some(dense_bytes),
            ratio: Some(ratio),
            ..Evidence::new("memory_report")
        }
        .emit();
    }

    // LOUDS memory report
    for &internal in &[50, 500, 5_000] {
        let total = 2 * internal + 1; // k internal + (k+1) leaves
        let mut degrees = vec![0usize; total];
        for d in degrees.iter_mut().take(internal) {
            *d = 2;
        }
        let louds = LoudsTree::from_degrees(&degrees);
        let louds_bytes = louds.size_in_bytes();
        let pointer_bytes = total * 3 * 8;
        let ratio = louds_bytes as f64 / pointer_bytes as f64;

        assert!(
            louds_bytes < pointer_bytes,
            "LOUDS ({louds_bytes}B) should be smaller than pointers ({pointer_bytes}B) at n={total}"
        );

        Evidence {
            test: "memory_report",
            node_count: Some(total),
            encoding: Some("louds"),
            memory_bytes: Some(louds_bytes),
            dense_memory_bytes: Some(pointer_bytes),
            ratio: Some(ratio),
            ..Evidence::new("memory_report")
        }
        .emit();
    }
}
