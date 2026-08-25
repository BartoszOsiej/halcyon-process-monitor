//! Benchmarks: succinct (Elias-Fano, LOUDS) vs dense (Vec, pointer tree) (bd-1wevm.3)
//!
//! Run with: cargo bench -p ftui-widgets --bench succinct_bench

use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main};
use ftui_widgets::elias_fano::EliasFano;
use ftui_widgets::louds::LoudsTree;
use std::hint::black_box;

// ── Helpers ──────────────────────────────────────────────────────────

/// Build monotone prefix sums simulating row heights.
fn make_prefix_sums(n: usize) -> Vec<u64> {
    let mut sums = Vec::with_capacity(n);
    let mut acc = 0u64;
    for i in 0..n {
        acc += 18 + (i as u64 % 7); // avg ~21px per row, slight variation
        sums.push(acc);
    }
    sums
}

/// Build a complete binary tree degree sequence (n internal + n+1 leaves).
fn make_binary_tree_degrees(internal_nodes: usize) -> Vec<usize> {
    let leaves = internal_nodes + 1;
    let total = internal_nodes + leaves;
    let mut degrees = vec![0usize; total];
    for d in degrees.iter_mut().take(internal_nodes) {
        *d = 2;
    }
    degrees
}

/// Simple pointer-based tree for dense comparison.
struct DenseTree {
    children: Vec<Vec<usize>>,
}

impl DenseTree {
    fn from_degrees(degrees: &[usize]) -> Self {
        let n = degrees.len();
        let mut children = vec![vec![]; n];
        let mut next_child = 1;
        for (v, &d) in degrees.iter().enumerate() {
            for _ in 0..d {
                if next_child >= n {
                    break;
                }
                children[v].push(next_child);
                next_child += 1;
            }
        }
        Self { children }
    }

    fn first_child(&self, v: usize) -> Option<usize> {
        self.children[v].first().copied()
    }

    fn is_leaf(&self, v: usize) -> bool {
        self.children[v].is_empty()
    }
}

// ── Elias-Fano vs Dense: Build ──────────────────────────────────────

fn bench_ef_build(c: &mut Criterion) {
    let mut group = c.benchmark_group("succinct/ef_build");

    for &n in &[100, 1_000, 10_000, 100_000] {
        let sums = make_prefix_sums(n);

        group.bench_with_input(BenchmarkId::new("elias_fano", n), &sums, |b, sums| {
            b.iter(|| black_box(EliasFano::encode(black_box(sums))));
        });
    }

    group.finish();
}

// ── Elias-Fano vs Dense: Random Access ──────────────────────────────

fn bench_ef_access(c: &mut Criterion) {
    let mut group = c.benchmark_group("succinct/ef_access");

    for &n in &[100, 1_000, 10_000, 100_000] {
        let sums = make_prefix_sums(n);
        let ef = EliasFano::encode(&sums);

        // Precompute random indices (deterministic)
        let indices: Vec<usize> = (0..1000).map(|i| (i * 7919) % n).collect();

        group.bench_with_input(BenchmarkId::new("elias_fano", n), &indices, |b, idx| {
            b.iter(|| {
                let mut sum = 0u64;
                for &i in idx {
                    sum = sum.wrapping_add(ef.access(i));
                }
                black_box(sum)
            });
        });

        group.bench_with_input(BenchmarkId::new("dense_vec", n), &indices, |b, idx| {
            b.iter(|| {
                let mut sum = 0u64;
                for &i in idx {
                    sum = sum.wrapping_add(sums[i]);
                }
                black_box(sum)
            });
        });
    }

    group.finish();
}

// ── Elias-Fano vs Dense: Rank (count elements <= v) ─────────────────

fn bench_ef_rank(c: &mut Criterion) {
    let mut group = c.benchmark_group("succinct/ef_rank");

    for &n in &[100, 1_000, 10_000, 100_000] {
        let sums = make_prefix_sums(n);
        let ef = EliasFano::encode(&sums);
        let max_val = *sums.last().unwrap();

        // Deterministic query points
        let queries: Vec<u64> = (0..1000).map(|i| (i as u64 * 6971) % max_val).collect();

        group.bench_with_input(BenchmarkId::new("elias_fano", n), &queries, |b, qs| {
            b.iter(|| {
                let mut sum = 0usize;
                for &q in qs {
                    sum = sum.wrapping_add(ef.rank(q));
                }
                black_box(sum)
            });
        });

        group.bench_with_input(BenchmarkId::new("dense_bsearch", n), &queries, |b, qs| {
            b.iter(|| {
                let mut sum = 0usize;
                for &q in qs {
                    sum = sum.wrapping_add(sums.partition_point(|&x| x <= q));
                }
                black_box(sum)
            });
        });
    }

    group.finish();
}

// ── Elias-Fano vs Dense: next_geq (first element >= v) ─────────────

fn bench_ef_next_geq(c: &mut Criterion) {
    let mut group = c.benchmark_group("succinct/ef_next_geq");

    for &n in &[100, 1_000, 10_000, 100_000] {
        let sums = make_prefix_sums(n);
        let ef = EliasFano::encode(&sums);
        let max_val = *sums.last().unwrap();

        let queries: Vec<u64> = (0..1000).map(|i| (i as u64 * 6971) % max_val).collect();

        group.bench_with_input(BenchmarkId::new("elias_fano", n), &queries, |b, qs| {
            b.iter(|| {
                let mut sum = 0u64;
                for &q in qs {
                    if let Some((_, v)) = ef.next_geq(q) {
                        sum = sum.wrapping_add(v);
                    }
                }
                black_box(sum)
            });
        });

        group.bench_with_input(BenchmarkId::new("dense_bsearch", n), &queries, |b, qs| {
            b.iter(|| {
                let mut sum = 0u64;
                for &q in qs {
                    let idx = sums.partition_point(|&x| x < q);
                    if idx < sums.len() {
                        sum = sum.wrapping_add(sums[idx]);
                    }
                }
                black_box(sum)
            });
        });
    }

    group.finish();
}

// ── Elias-Fano vs Dense: Sequential Scan ────────────────────────────

fn bench_ef_sequential(c: &mut Criterion) {
    let mut group = c.benchmark_group("succinct/ef_sequential");

    for &n in &[100, 1_000, 10_000, 100_000] {
        let sums = make_prefix_sums(n);
        let ef = EliasFano::encode(&sums);

        group.bench_with_input(BenchmarkId::new("elias_fano", n), &n, |b, &n| {
            b.iter(|| {
                let mut sum = 0u64;
                for i in 0..n {
                    sum = sum.wrapping_add(ef.access(i));
                }
                black_box(sum)
            });
        });

        group.bench_with_input(BenchmarkId::new("dense_vec", n), &sums, |b, sums| {
            b.iter(|| {
                let mut sum = 0u64;
                for &v in sums {
                    sum = sum.wrapping_add(v);
                }
                black_box(sum)
            });
        });
    }

    group.finish();
}

// ── Elias-Fano: Memory Usage Report ─────────────────────────────────

fn bench_ef_memory(c: &mut Criterion) {
    let mut group = c.benchmark_group("succinct/ef_memory_report");

    for &n in &[100, 1_000, 10_000, 100_000, 1_000_000] {
        let sums = make_prefix_sums(n);
        let ef = EliasFano::encode(&sums);

        let ef_bytes = ef.size_in_bytes();
        let dense_bytes = n * 8;
        let ratio = ef_bytes as f64 / dense_bytes as f64;

        // Use throughput to report memory in the benchmark output
        group.bench_with_input(
            BenchmarkId::new(
                format!("ef={ef_bytes}B_dense={dense_bytes}B_ratio={ratio:.2}"),
                n,
            ),
            &ef,
            |b, ef| {
                b.iter(|| black_box(ef.size_in_bytes()));
            },
        );
    }

    group.finish();
}

// ── LOUDS vs Dense: Build ───────────────────────────────────────────

fn bench_louds_build(c: &mut Criterion) {
    let mut group = c.benchmark_group("succinct/louds_build");

    for &internal in &[50, 500, 5_000, 50_000] {
        let degrees = make_binary_tree_degrees(internal);
        let total = degrees.len();

        group.bench_with_input(BenchmarkId::new("louds", total), &degrees, |b, deg| {
            b.iter(|| black_box(LoudsTree::from_degrees(black_box(deg))));
        });
    }

    group.finish();
}

// ── LOUDS vs Dense: Navigation ──────────────────────────────────────

fn bench_louds_navigation(c: &mut Criterion) {
    let mut group = c.benchmark_group("succinct/louds_nav");

    for &internal in &[50, 500, 5_000] {
        let degrees = make_binary_tree_degrees(internal);
        let total = degrees.len();
        let louds = LoudsTree::from_degrees(&degrees);
        let dense = DenseTree::from_degrees(&degrees);

        // Random node indices
        let nodes: Vec<usize> = (0..1000).map(|i| (i * 7919) % total).collect();

        group.bench_with_input(
            BenchmarkId::new("louds_first_child", total),
            &nodes,
            |b, ns| {
                b.iter(|| {
                    let mut count = 0usize;
                    for &v in ns {
                        if louds.first_child(v).is_some() {
                            count += 1;
                        }
                    }
                    black_box(count)
                });
            },
        );

        group.bench_with_input(
            BenchmarkId::new("dense_first_child", total),
            &nodes,
            |b, ns| {
                b.iter(|| {
                    let mut count = 0usize;
                    for &v in ns {
                        if dense.first_child(v).is_some() {
                            count += 1;
                        }
                    }
                    black_box(count)
                });
            },
        );

        group.bench_with_input(BenchmarkId::new("louds_is_leaf", total), &nodes, |b, ns| {
            b.iter(|| {
                let mut count = 0usize;
                for &v in ns {
                    if louds.is_leaf(v) {
                        count += 1;
                    }
                }
                black_box(count)
            });
        });

        group.bench_with_input(BenchmarkId::new("dense_is_leaf", total), &nodes, |b, ns| {
            b.iter(|| {
                let mut count = 0usize;
                for &v in ns {
                    if dense.is_leaf(v) {
                        count += 1;
                    }
                }
                black_box(count)
            });
        });
    }

    group.finish();
}

// ── LOUDS: Memory Usage Report ──────────────────────────────────────

fn bench_louds_memory(c: &mut Criterion) {
    let mut group = c.benchmark_group("succinct/louds_memory_report");

    for &internal in &[50, 500, 5_000, 50_000] {
        let degrees = make_binary_tree_degrees(internal);
        let total = degrees.len();
        let louds = LoudsTree::from_degrees(&degrees);

        let louds_bytes = louds.size_in_bytes();
        // Dense: 3 pointers per node (parent, first_child, next_sibling) + children vec overhead
        let dense_bytes = total * 3 * 8;
        let ratio = louds_bytes as f64 / dense_bytes as f64;

        group.bench_with_input(
            BenchmarkId::new(
                format!("louds={louds_bytes}B_dense={dense_bytes}B_ratio={ratio:.3}"),
                total,
            ),
            &louds,
            |b, louds| {
                b.iter(|| black_box(louds.size_in_bytes()));
            },
        );
    }

    group.finish();
}

criterion_group!(
    benches,
    bench_ef_build,
    bench_ef_access,
    bench_ef_rank,
    bench_ef_next_geq,
    bench_ef_sequential,
    bench_ef_memory,
    bench_louds_build,
    bench_louds_navigation,
    bench_louds_memory,
);
criterion_main!(benches);
