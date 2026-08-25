//! Benchmarks for DoubleBuffer O(1) swap (bd-1rz0.4.4)
//! and AdaptiveDoubleBuffer allocation efficiency (bd-1rz0.4.2)
//!
//! ## Performance budgets
//!
//! ### DoubleBuffer
//! - Swap: < 10ns (O(1) index flip)
//! - Clone (baseline): ~70,000ns for 120x40
//! - Clear: ~15,000ns for 120x40
//! - Expected improvement: swap is ~10,000x faster than clone
//!
//! ### AdaptiveDoubleBuffer (bd-1rz0.4.2)
//! - Resize (no realloc): < 20,000ns (clear only)
//! - Resize (realloc): ~120,000ns for 120x40 (new DoubleBuffer allocation)
//! - Resize storm avoidance: > 80% of resizes should avoid reallocation
//! - Memory overhead: < 30% extra capacity
//!
//! Run with: cargo bench -p ftui-render --bench double_buffer_bench

use criterion::{BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use ftui_render::buffer::{AdaptiveDoubleBuffer, Buffer, DoubleBuffer};
use std::hint::black_box;

// =============================================================================
// DoubleBuffer swap vs Buffer clone
// =============================================================================

fn bench_swap_vs_clone(c: &mut Criterion) {
    let mut group = c.benchmark_group("double_buffer/transition");

    for (w, h) in [(80, 24), (120, 40), (200, 60)] {
        let cells = w as u64 * h as u64;
        group.throughput(Throughput::Elements(cells));

        // Baseline: Buffer::clone (the old approach)
        let buf = Buffer::new(w, h);
        group.bench_with_input(
            BenchmarkId::new("clone", format!("{w}x{h}")),
            &buf,
            |b, buf| b.iter(|| black_box(buf.clone())),
        );

        // New approach: DoubleBuffer::swap
        let mut db = DoubleBuffer::new(w, h);
        group.bench_with_input(BenchmarkId::new("swap", format!("{w}x{h}")), &(), |b, _| {
            b.iter(|| {
                db.swap();
                black_box(&db);
            })
        });

        // Clear (still needed after swap)
        let mut clear_buf = Buffer::new(w, h);
        group.bench_with_input(
            BenchmarkId::new("clear", format!("{w}x{h}")),
            &(),
            |b, _| {
                b.iter(|| {
                    clear_buf.clear();
                    black_box(&clear_buf);
                })
            },
        );

        // Combined: swap + clear (actual frame transition cost)
        let mut db_combined = DoubleBuffer::new(w, h);
        group.bench_with_input(
            BenchmarkId::new("swap_and_clear", format!("{w}x{h}")),
            &(),
            |b, _| {
                b.iter(|| {
                    db_combined.swap();
                    db_combined.current_mut().clear();
                    black_box(&db_combined);
                })
            },
        );
    }

    group.finish();
}

// =============================================================================
// DoubleBuffer operations
// =============================================================================

fn bench_double_buffer_operations(c: &mut Criterion) {
    let mut group = c.benchmark_group("double_buffer/ops");

    // New allocation
    for (w, h) in [(80, 24), (120, 40), (200, 60)] {
        let cells = w as u64 * h as u64;
        group.throughput(Throughput::Elements(cells));
        group.bench_with_input(
            BenchmarkId::new("new", format!("{w}x{h}")),
            &(w, h),
            |b, &(w, h)| b.iter(|| black_box(DoubleBuffer::new(w, h))),
        );
    }

    // Resize
    let mut db = DoubleBuffer::new(80, 24);
    group.bench_function("resize_80x24_to_120x40", |b| {
        b.iter(|| {
            db.resize(120, 40);
            db.resize(80, 24); // Reset for next iteration
            black_box(&db);
        })
    });

    // Access patterns
    let db = DoubleBuffer::new(120, 40);
    group.bench_function("current_ref_120x40", |b| b.iter(|| black_box(db.current())));

    group.bench_function("previous_ref_120x40", |b| {
        b.iter(|| black_box(db.previous()))
    });

    let mut db_mut = DoubleBuffer::new(120, 40);
    group.bench_function("current_mut_120x40", |b| {
        b.iter(|| {
            let _ = black_box(db_mut.current_mut());
        })
    });

    // Dimension queries
    let db = DoubleBuffer::new(120, 40);
    group.bench_function("dimensions_match", |b| {
        b.iter(|| black_box(db.dimensions_match(120, 40)))
    });

    group.finish();
}

// =============================================================================
// Frame transition simulation
// =============================================================================

fn bench_frame_transition_simulation(c: &mut Criterion) {
    let mut group = c.benchmark_group("double_buffer/frame_sim");

    for (w, h) in [(80, 24), (120, 40)] {
        let cells = w as u64 * h as u64;
        group.throughput(Throughput::Elements(cells));

        // Old approach: clone buffer each frame
        group.bench_with_input(
            BenchmarkId::new("clone_per_frame", format!("{w}x{h}")),
            &(w, h),
            |b, &(w, h)| {
                let mut current = Buffer::new(w, h);
                b.iter(|| {
                    // Simulate: prev = current.clone(), then render to current
                    let _prev = current.clone();
                    current.clear();
                    black_box(&current);
                })
            },
        );

        // New approach: swap + clear
        group.bench_with_input(
            BenchmarkId::new("swap_per_frame", format!("{w}x{h}")),
            &(w, h),
            |b, &(w, h)| {
                let mut db = DoubleBuffer::new(w, h);
                b.iter(|| {
                    db.swap();
                    db.current_mut().clear();
                    black_box(&db);
                })
            },
        );
    }

    group.finish();
}

// =============================================================================
// AdaptiveDoubleBuffer benchmarks (bd-1rz0.4.2)
// =============================================================================

fn bench_adaptive_resize_storm(c: &mut Criterion) {
    let mut group = c.benchmark_group("adaptive_buffer/resize_storm");

    // Simulate resize storm: rapid consecutive resizes
    for base_size in [(80, 24), (120, 40)] {
        let (w, h) = base_size;
        let cells = w as u64 * h as u64;
        group.throughput(Throughput::Elements(cells));

        // AdaptiveDoubleBuffer: should reuse capacity for small changes
        group.bench_with_input(
            BenchmarkId::new("adaptive", format!("{w}x{h}")),
            &(w, h),
            |b, &(w, h)| {
                let mut adb = AdaptiveDoubleBuffer::new(w, h);
                b.iter(|| {
                    // Simulate resize storm: +1, +2, +3, ..., +10
                    for i in 1u16..=10 {
                        adb.resize(w + i, h + (i / 2));
                    }
                    // Reset to original
                    adb.resize(w, h);
                    black_box(&adb);
                })
            },
        );

        // DoubleBuffer baseline: always reallocates
        group.bench_with_input(
            BenchmarkId::new("regular", format!("{w}x{h}")),
            &(w, h),
            |b, &(w, h)| {
                let mut db = DoubleBuffer::new(w, h);
                b.iter(|| {
                    for i in 1u16..=10 {
                        db.resize(w + i, h + (i / 2));
                    }
                    db.resize(w, h);
                    black_box(&db);
                })
            },
        );
    }

    group.finish();
}

fn bench_adaptive_operations(c: &mut Criterion) {
    let mut group = c.benchmark_group("adaptive_buffer/ops");

    for (w, h) in [(80, 24), (120, 40), (200, 60)] {
        let cells = w as u64 * h as u64;
        group.throughput(Throughput::Elements(cells));

        // New allocation
        group.bench_with_input(
            BenchmarkId::new("new", format!("{w}x{h}")),
            &(w, h),
            |b, &(w, h)| b.iter(|| black_box(AdaptiveDoubleBuffer::new(w, h))),
        );
    }

    // Resize without reallocation (within capacity)
    let mut adb = AdaptiveDoubleBuffer::new(80, 24);
    group.bench_function("resize_within_capacity", |b| {
        b.iter(|| {
            adb.resize(90, 28); // Within 100x30 capacity
            adb.resize(80, 24); // Reset
            black_box(&adb);
        })
    });

    // Resize with reallocation (beyond capacity)
    let mut adb2 = AdaptiveDoubleBuffer::new(80, 24);
    group.bench_function("resize_beyond_capacity", |b| {
        b.iter(|| {
            adb2.resize(150, 60); // Beyond 100x30 capacity
            adb2.resize(80, 24); // Reset and shrink (below threshold)
            black_box(&adb2);
        })
    });

    // Swap operation (should be identical to DoubleBuffer)
    let mut adb3 = AdaptiveDoubleBuffer::new(120, 40);
    group.bench_function("swap_120x40", |b| {
        b.iter(|| {
            adb3.swap();
            black_box(&adb3);
        })
    });

    group.finish();
}

fn bench_adaptive_vs_regular_frame_transition(c: &mut Criterion) {
    let mut group = c.benchmark_group("adaptive_buffer/frame_transition");

    for (w, h) in [(80, 24), (120, 40)] {
        let cells = w as u64 * h as u64;
        group.throughput(Throughput::Elements(cells));

        // Adaptive buffer frame transition
        group.bench_with_input(
            BenchmarkId::new("adaptive_swap_clear", format!("{w}x{h}")),
            &(w, h),
            |b, &(w, h)| {
                let mut adb = AdaptiveDoubleBuffer::new(w, h);
                b.iter(|| {
                    adb.swap();
                    adb.current_mut().clear();
                    black_box(&adb);
                })
            },
        );

        // Regular DoubleBuffer frame transition (for comparison)
        group.bench_with_input(
            BenchmarkId::new("regular_swap_clear", format!("{w}x{h}")),
            &(w, h),
            |b, &(w, h)| {
                let mut db = DoubleBuffer::new(w, h);
                b.iter(|| {
                    db.swap();
                    db.current_mut().clear();
                    black_box(&db);
                })
            },
        );
    }

    group.finish();
}

/// Benchmark to measure allocation avoidance ratio under resize storm.
/// This is an observability check, not a timing benchmark.
fn bench_adaptive_avoidance_ratio(c: &mut Criterion) {
    let mut group = c.benchmark_group("adaptive_buffer/avoidance");

    // Test various resize patterns
    group.bench_function("small_increments_10x", |b| {
        b.iter(|| {
            let mut adb = AdaptiveDoubleBuffer::new(80, 24);
            for i in 1u16..=10 {
                adb.resize(80 + i, 24 + (i / 3));
            }
            let stats = adb.stats().clone();
            black_box((adb.stats().avoidance_ratio(), stats));
        })
    });

    group.bench_function("oscillation_pattern", |b| {
        b.iter(|| {
            let mut adb = AdaptiveDoubleBuffer::new(80, 24);
            // Oscillate between two sizes
            for _ in 0..5 {
                adb.resize(90, 28);
                adb.resize(80, 24);
            }
            black_box(adb.stats().avoidance_ratio());
        })
    });

    group.finish();
}

criterion_group!(
    benches,
    bench_swap_vs_clone,
    bench_double_buffer_operations,
    bench_frame_transition_simulation,
    bench_adaptive_resize_storm,
    bench_adaptive_operations,
    bench_adaptive_vs_regular_frame_transition,
    bench_adaptive_avoidance_ratio,
);
criterion_main!(benches);
