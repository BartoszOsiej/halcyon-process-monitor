//! Benchmarks for FrameArena allocation patterns (bd-2alzw.4).
//!
//! Run with:
//! `cargo bench -p ftui-render --bench arena_bench`

use criterion::{BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use ftui_render::arena::FrameArena;
use std::hint::black_box;

const ALLOCATIONS: usize = 10_000;

fn bench_arena_vs_global(c: &mut Criterion) {
    let mut group = c.benchmark_group("arena/alloc_10k_u64");
    group.throughput(Throughput::Elements(ALLOCATIONS as u64));
    let mut arena = FrameArena::new(256 * 1024);

    group.bench_function(BenchmarkId::new("frame_arena", "u64"), |b| {
        b.iter(|| {
            for i in 0..ALLOCATIONS {
                let value = arena.alloc(i as u64);
                black_box(*value);
            }
            // Per-frame reuse contract: reclaim scratch allocations at frame boundary.
            arena.reset();
            black_box(arena.allocated_bytes_including_metadata());
        });
    });

    group.bench_function(BenchmarkId::new("global_allocator", "u64"), |b| {
        b.iter(|| {
            let mut values = Vec::with_capacity(ALLOCATIONS);
            for i in 0..ALLOCATIONS {
                values.push(Box::new(i as u64));
            }
            for value in &values {
                black_box(**value);
            }
            black_box(values);
        });
    });

    group.finish();
}

criterion_group!(benches, bench_arena_vs_global);
criterion_main!(benches);
