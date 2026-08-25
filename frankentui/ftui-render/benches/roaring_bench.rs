//! Benchmarks: Roaring Bitmap vs Vec<bool> for cell-level dirty tracking (bd-22wk8.2).
//!
//! Measures construction, union, iteration, and memory at terminal sizes:
//! 80x24, 120x40, 200x60, 400x100.
//!
//! Run with:
//! `cargo bench -p ftui-render --bench roaring_bench`

use criterion::{BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use ftui_render::roaring_bitmap::RoaringBitmap;
use std::hint::black_box;

/// Screen sizes from the bead spec.
const SIZES: &[(u32, u32)] = &[(80, 24), (120, 40), (200, 60), (400, 100)];

/// Fraction of cells marked dirty in sparse/dense scenarios.
const SPARSE_FRACTION: f64 = 0.05; // 5%
const DENSE_FRACTION: f64 = 0.50; // 50%

/// Deterministic dirty cell indices for a given fraction.
fn dirty_cells(width: u32, height: u32, fraction: f64) -> Vec<u32> {
    let total = (width * height) as usize;
    let count = ((total as f64) * fraction).ceil() as usize;
    // Use a simple LCG to spread cells across the screen deterministically.
    let mut indices = Vec::with_capacity(count);
    let mut state: u64 = 0xDEAD_BEEF;
    while indices.len() < count {
        state = state.wrapping_mul(6364136223846793005).wrapping_add(1);
        let idx = ((state >> 33) as u32) % (width * height);
        indices.push(idx);
    }
    indices
}

// ============================================================================
// Construction (insert dirty cells)
// ============================================================================

fn bench_construction(c: &mut Criterion) {
    for &(w, h) in SIZES {
        let total = w * h;
        let label = format!("{w}x{h}");

        // --- Sparse ---
        let cells = dirty_cells(w, h, SPARSE_FRACTION);
        let mut group = c.benchmark_group(format!("dirty_construct/sparse_{label}"));
        group.throughput(Throughput::Elements(cells.len() as u64));

        group.bench_function(BenchmarkId::new("roaring", &label), |b| {
            b.iter(|| {
                let mut bm = RoaringBitmap::new();
                for &idx in &cells {
                    bm.insert(idx);
                }
                black_box(bm.cardinality())
            });
        });

        group.bench_function(BenchmarkId::new("vec_bool", &label), |b| {
            b.iter(|| {
                let mut v = vec![false; total as usize];
                for &idx in &cells {
                    v[idx as usize] = true;
                }
                black_box(v.iter().filter(|&&x| x).count())
            });
        });

        group.finish();

        // --- Dense ---
        let cells = dirty_cells(w, h, DENSE_FRACTION);
        let mut group = c.benchmark_group(format!("dirty_construct/dense_{label}"));
        group.throughput(Throughput::Elements(cells.len() as u64));

        group.bench_function(BenchmarkId::new("roaring", &label), |b| {
            b.iter(|| {
                let mut bm = RoaringBitmap::new();
                for &idx in &cells {
                    bm.insert(idx);
                }
                black_box(bm.cardinality())
            });
        });

        group.bench_function(BenchmarkId::new("vec_bool", &label), |b| {
            b.iter(|| {
                let mut v = vec![false; total as usize];
                for &idx in &cells {
                    v[idx as usize] = true;
                }
                black_box(v.iter().filter(|&&x| x).count())
            });
        });

        group.finish();
    }
}

// ============================================================================
// Union (merge two dirty sets)
// ============================================================================

fn bench_union(c: &mut Criterion) {
    for &(w, h) in SIZES {
        let total = w * h;
        let label = format!("{w}x{h}");

        let cells_a = dirty_cells(w, h, SPARSE_FRACTION);
        // Use offset seed for second set.
        let cells_b: Vec<u32> = {
            let mut state: u64 = 0xCAFE_BABE;
            let count = cells_a.len();
            let mut v = Vec::with_capacity(count);
            while v.len() < count {
                state = state.wrapping_mul(6364136223846793005).wrapping_add(1);
                let idx = ((state >> 33) as u32) % (w * h);
                v.push(idx);
            }
            v
        };

        let mut group = c.benchmark_group(format!("dirty_union/sparse_{label}"));
        group.throughput(Throughput::Elements((cells_a.len() + cells_b.len()) as u64));

        group.bench_function(BenchmarkId::new("roaring", &label), |b| {
            let mut bm_a = RoaringBitmap::new();
            for &idx in &cells_a {
                bm_a.insert(idx);
            }
            let mut bm_b = RoaringBitmap::new();
            for &idx in &cells_b {
                bm_b.insert(idx);
            }
            b.iter(|| {
                let result = bm_a.union(&bm_b);
                black_box(result.cardinality())
            });
        });

        group.bench_function(BenchmarkId::new("vec_bool", &label), |b| {
            let mut va = vec![false; total as usize];
            for &idx in &cells_a {
                va[idx as usize] = true;
            }
            let mut vb = vec![false; total as usize];
            for &idx in &cells_b {
                vb[idx as usize] = true;
            }
            b.iter(|| {
                let result: Vec<bool> = va.iter().zip(vb.iter()).map(|(&a, &b)| a | b).collect();
                black_box(result.iter().filter(|&&x| x).count())
            });
        });

        group.finish();
    }
}

// ============================================================================
// Iteration (enumerate all dirty cells)
// ============================================================================

fn bench_iteration(c: &mut Criterion) {
    for &(w, h) in SIZES {
        let total = w * h;
        let label = format!("{w}x{h}");

        for &(frac, frac_name) in &[(SPARSE_FRACTION, "sparse"), (DENSE_FRACTION, "dense")] {
            let cells = dirty_cells(w, h, frac);

            let mut bm = RoaringBitmap::new();
            for &idx in &cells {
                bm.insert(idx);
            }
            let mut v = vec![false; total as usize];
            for &idx in &cells {
                v[idx as usize] = true;
            }

            let mut group = c.benchmark_group(format!("dirty_iter/{frac_name}_{label}"));
            group.throughput(Throughput::Elements(bm.cardinality() as u64));

            group.bench_function(BenchmarkId::new("roaring", &label), |b| {
                b.iter(|| {
                    let mut sum: u64 = 0;
                    for idx in bm.iter() {
                        sum = sum.wrapping_add(idx as u64);
                    }
                    black_box(sum)
                });
            });

            group.bench_function(BenchmarkId::new("vec_bool", &label), |b| {
                b.iter(|| {
                    let mut sum: u64 = 0;
                    for (i, &dirty) in v.iter().enumerate() {
                        if dirty {
                            sum = sum.wrapping_add(i as u64);
                        }
                    }
                    black_box(sum)
                });
            });

            group.finish();
        }
    }
}

// ============================================================================
// Memory usage (approximate)
// ============================================================================

fn bench_memory(c: &mut Criterion) {
    let mut group = c.benchmark_group("dirty_memory");

    for &(w, h) in SIZES {
        let total = w * h;
        let label = format!("{w}x{h}");

        for &(frac, frac_name) in &[(SPARSE_FRACTION, "sparse"), (DENSE_FRACTION, "dense")] {
            let cells = dirty_cells(w, h, frac);

            group.bench_function(
                BenchmarkId::new(format!("roaring/{frac_name}"), &label),
                |b| {
                    b.iter(|| {
                        let mut bm = RoaringBitmap::new();
                        for &idx in &cells {
                            bm.insert(idx);
                        }
                        // Approximate heap size: containers * (key + container overhead).
                        // For array: key(2) + Vec overhead(24) + values(2*len).
                        // For bitmap: key(2) + words(8192) + count(8).
                        black_box(bm.cardinality())
                    });
                },
            );

            group.bench_function(
                BenchmarkId::new(format!("vec_bool/{frac_name}"), &label),
                |b| {
                    b.iter(|| {
                        let mut v = vec![false; total as usize];
                        for &idx in &cells {
                            v[idx as usize] = true;
                        }
                        // Vec<bool> heap size = total bytes.
                        black_box(v.len())
                    });
                },
            );
        }
    }

    group.finish();
}

criterion_group!(
    benches,
    bench_construction,
    bench_union,
    bench_iteration,
    bench_memory,
);
criterion_main!(benches);
