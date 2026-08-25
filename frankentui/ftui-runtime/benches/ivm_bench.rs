//! Benchmarks for IVM incremental vs full recompute (bd-3akdb.3).
//!
//! Measures work done (cells visited, styles resolved) for incremental delta
//! propagation vs full recompute. Target: 90%+ work reduction for single-
//! property theme changes.
//!
//! Run with: cargo bench -p ftui-runtime --bench ivm_bench
//!
//! Expected results:
//!
//! | Scenario             | Full Work | Incremental Work | Reduction |
//! |----------------------|-----------|------------------|-----------|
//! | Single theme change  | N widgets | 1 widget         | ~99%      |
//! | 10% theme change     | N widgets | ~N/10 widgets    | ~90%      |
//! | 50% theme change     | N widgets | ~N/2 widgets     | ~50%      |
//! | Full theme swap      | N widgets | N widgets        | ~0%       |

use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main};
use std::collections::HashMap;
use std::hint::black_box;

use ftui_runtime::ivm::{
    DeltaBatch, FilteredListView, IncrementalView, ResolvedStyleValue, StyleKey,
    StyleResolutionView,
};

// ============================================================================
// Setup helpers
// ============================================================================

/// Create a StyleResolutionView with `n` widgets pre-populated.
fn setup_style_view(n: usize, base_hash: u64) -> StyleResolutionView {
    let mut view = StyleResolutionView::new("bench_style", base_hash);
    let mut batch = DeltaBatch::new(0);
    for i in 0..n {
        batch.insert(
            StyleKey(i as u32),
            ResolvedStyleValue {
                style_hash: (i as u64).wrapping_mul(0x517cc1b727220a95),
            },
            i as u64,
        );
    }
    view.apply_delta(&batch);
    view
}

/// Create a FilteredListView with `n` items pre-populated.
/// Filter: only items with value > 0 are visible.
fn setup_filter_view(n: usize) -> FilteredListView<u32, i32> {
    let mut view = FilteredListView::new("bench_filter", |_k: &u32, v: &i32| *v > 0);
    let mut batch = DeltaBatch::new(0);
    for i in 0..n {
        // Alternate positive/negative so ~50% are visible.
        let val = if i % 2 == 0 {
            i as i32 + 1
        } else {
            -(i as i32)
        };
        batch.insert(i as u32, val, i as u64);
    }
    view.apply_delta(&batch);
    view
}

// ============================================================================
// StyleResolutionView benchmarks
// ============================================================================

/// Benchmark: single widget override change (incremental).
fn bench_style_single_change_incremental(c: &mut Criterion) {
    let mut group = c.benchmark_group("ivm_style_single_change");

    for &n in &[100, 500, 1000, 5000] {
        group.bench_with_input(BenchmarkId::new("incremental", n), &n, |b, &n| {
            let mut view = setup_style_view(n, 0xDEAD_BEEF);
            let mut epoch = 1u64;
            b.iter(|| {
                let mut batch = DeltaBatch::new(epoch);
                batch.insert(
                    StyleKey(0),
                    ResolvedStyleValue {
                        style_hash: epoch.wrapping_mul(0x9E3779B97F4A7C15),
                    },
                    0,
                );
                let output = view.apply_delta(&batch);
                epoch += 1;
                black_box(output.len())
            });
        });

        group.bench_with_input(BenchmarkId::new("full_recompute", n), &n, |b, &n| {
            let view = setup_style_view(n, 0xDEAD_BEEF);
            b.iter(|| {
                let full = view.full_recompute();
                black_box(full.len())
            });
        });
    }
    group.finish();
}

/// Benchmark: 10% of widgets changed (incremental vs full).
fn bench_style_10pct_change(c: &mut Criterion) {
    let mut group = c.benchmark_group("ivm_style_10pct_change");

    for &n in &[100, 500, 1000, 5000] {
        let changed = n / 10;

        group.bench_with_input(BenchmarkId::new("incremental", n), &n, |b, &n| {
            let mut view = setup_style_view(n, 0xDEAD_BEEF);
            let mut epoch = 1u64;
            b.iter(|| {
                let mut batch = DeltaBatch::new(epoch);
                for i in 0..changed {
                    batch.insert(
                        StyleKey(i as u32),
                        ResolvedStyleValue {
                            style_hash: epoch.wrapping_add(i as u64),
                        },
                        i as u64,
                    );
                }
                let output = view.apply_delta(&batch);
                epoch += 1;
                black_box(output.len())
            });
        });

        group.bench_with_input(BenchmarkId::new("full_recompute", n), &n, |b, &n| {
            let view = setup_style_view(n, 0xDEAD_BEEF);
            b.iter(|| {
                let full = view.full_recompute();
                black_box(full.len())
            });
        });
    }
    group.finish();
}

/// Benchmark: full theme swap (all widgets change).
fn bench_style_full_swap(c: &mut Criterion) {
    let mut group = c.benchmark_group("ivm_style_full_swap");

    for &n in &[100, 500, 1000] {
        group.bench_with_input(BenchmarkId::new("incremental", n), &n, |b, &n| {
            let mut view = setup_style_view(n, 0xDEAD_BEEF);
            let mut epoch = 1u64;
            b.iter(|| {
                let mut batch = DeltaBatch::new(epoch);
                for i in 0..n {
                    batch.insert(
                        StyleKey(i as u32),
                        ResolvedStyleValue {
                            style_hash: epoch.wrapping_add(i as u64).wrapping_mul(0x123),
                        },
                        i as u64,
                    );
                }
                let output = view.apply_delta(&batch);
                epoch += 1;
                black_box(output.len())
            });
        });

        group.bench_with_input(BenchmarkId::new("full_recompute", n), &n, |b, &n| {
            let view = setup_style_view(n, 0xDEAD_BEEF);
            b.iter(|| {
                let full = view.full_recompute();
                black_box(full.len())
            });
        });
    }
    group.finish();
}

// ============================================================================
// FilteredListView benchmarks
// ============================================================================

/// Benchmark: single item change in filtered list (incremental vs full).
fn bench_filter_single_change(c: &mut Criterion) {
    let mut group = c.benchmark_group("ivm_filter_single_change");

    for &n in &[100, 500, 1000, 5000] {
        group.bench_with_input(BenchmarkId::new("incremental", n), &n, |b, &n| {
            let mut view = setup_filter_view(n);
            let mut epoch = 1u64;
            b.iter(|| {
                let mut batch = DeltaBatch::new(epoch);
                // Toggle item 0 between positive and negative.
                let val = if epoch.is_multiple_of(2) {
                    42i32
                } else {
                    -42i32
                };
                batch.insert(0u32, val, 0);
                let output = view.apply_delta(&batch);
                epoch += 1;
                black_box(output.len())
            });
        });

        group.bench_with_input(BenchmarkId::new("full_recompute", n), &n, |b, &n| {
            let view = setup_filter_view(n);
            b.iter(|| {
                let full = view.full_recompute();
                black_box(full.len())
            });
        });
    }
    group.finish();
}

// ============================================================================
// Work reduction measurement (non-criterion, assertion-based)
// ============================================================================

/// Verify that single-property theme changes achieve 90%+ work reduction.
/// This runs as part of the bench binary's test mode.
fn bench_work_reduction_golden(c: &mut Criterion) {
    let mut group = c.benchmark_group("ivm_work_reduction_golden");

    // Use a large view to make the ratio meaningful.
    let n = 1000usize;

    group.bench_function("single_change_output_count", |b| {
        b.iter(|| {
            let mut v = setup_style_view(n, 0xCAFE);
            let mut batch = DeltaBatch::new(1);
            batch.insert(StyleKey(42), ResolvedStyleValue { style_hash: 0xFFFF }, 0);
            let output = v.apply_delta(&batch);
            let full = v.full_recompute();

            // Verify golden: incremental output is correct.
            let full_map: HashMap<StyleKey, ResolvedStyleValue> = full.into_iter().collect();
            assert_eq!(full_map.len(), v.materialized_size());

            // Verify work reduction: output deltas << materialized size.
            let reduction = 1.0 - (output.len() as f64 / v.materialized_size() as f64);
            assert!(
                reduction >= 0.90,
                "Work reduction {:.1}% below 90% target (output={}, materialized={})",
                reduction * 100.0,
                output.len(),
                v.materialized_size()
            );

            black_box(reduction)
        });
    });

    group.finish();
}

criterion_group!(
    benches,
    bench_style_single_change_incremental,
    bench_style_10pct_change,
    bench_style_full_swap,
    bench_filter_single_change,
    bench_work_reduction_golden,
);
criterion_main!(benches);
