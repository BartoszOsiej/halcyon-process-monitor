//! Performance Regression Benchmarks for Resize Storms (bd-1rz0.11)
//!
//! Measures reflow performance under various resize storm patterns and enforces
//! budget thresholds. Outputs JSONL performance logs for regression tracking.
//!
//! ## Performance Budgets
//!
//! | Pattern | p50 Budget | p95 Budget | p99 Budget |
//! |---------|------------|------------|------------|
//! | Burst 50 | < 5ms | < 10ms | < 20ms |
//! | Oscillate 10 | < 2ms | < 5ms | < 10ms |
//! | Sweep 20 | < 3ms | < 7ms | < 15ms |
//! | Pathological 20 | < 10ms | < 20ms | < 40ms |
//!
//! ## JSONL Schema
//!
//! ```json
//! {"event":"perf_run","bench":"resize_storm_burst","pattern":"burst","count":50,"seed":42}
//! {"event":"perf_sample","bench":"resize_storm_burst","iteration":0,"duration_ns":1234567}
//! {"event":"perf_summary","bench":"resize_storm_burst","p50_ns":1000000,"p95_ns":2000000,"p99_ns":3000000,"mean_ns":1500000}
//! ```
//!
//! Run with: cargo bench -p ftui-render --bench resize_storm_bench
//! Flamegraph: cargo flamegraph --bench resize_storm_bench -- --bench

use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main};
use ftui_render::buffer::AdaptiveDoubleBuffer;
use std::hint::black_box;

// =============================================================================
// Performance Budget Constants (nanoseconds)
// =============================================================================

/// Budget thresholds for different patterns (referenced in perf docs/checklists).
#[allow(dead_code)]
mod budgets {
    /// Burst pattern: 50 rapid resizes
    pub mod burst_50 {
        pub const P50_NS: u64 = 5_000_000; // 5ms
        pub const P95_NS: u64 = 10_000_000; // 10ms
        pub const P99_NS: u64 = 20_000_000; // 20ms
    }

    /// Oscillate pattern: 10 cycles between two sizes
    pub mod oscillate_10 {
        pub const P50_NS: u64 = 2_000_000; // 2ms
        pub const P95_NS: u64 = 5_000_000; // 5ms
        pub const P99_NS: u64 = 10_000_000; // 10ms
    }

    /// Sweep pattern: gradual size change over 20 steps
    pub mod sweep_20 {
        pub const P50_NS: u64 = 3_000_000; // 3ms
        pub const P95_NS: u64 = 7_000_000; // 7ms
        pub const P99_NS: u64 = 15_000_000; // 15ms
    }

    /// Pathological pattern: edge cases and extremes
    pub mod pathological_20 {
        pub const P50_NS: u64 = 10_000_000; // 10ms
        pub const P95_NS: u64 = 20_000_000; // 20ms
        pub const P99_NS: u64 = 40_000_000; // 40ms
    }

    /// Single resize operation
    pub mod single_resize {
        pub const P50_NS: u64 = 100_000; // 100us
        pub const P95_NS: u64 = 500_000; // 500us
        pub const P99_NS: u64 = 1_000_000; // 1ms
    }
}

// =============================================================================
// JSONL Performance Logging
// =============================================================================

/// Log a performance run start to JSONL (stderr for benchmark compatibility)
fn log_perf_run(bench: &str, pattern: &str, count: usize, seed: u64) {
    if std::env::var("FTUI_PERF_LOG").is_ok() {
        eprintln!(
            r#"{{"event":"perf_run","bench":"{}","pattern":"{}","count":{},"seed":{}}}"#,
            bench, pattern, count, seed
        );
    }
}

/// Log a performance sample to JSONL
#[allow(dead_code)]
fn log_perf_sample(bench: &str, iteration: usize, duration_ns: u64) {
    if std::env::var("FTUI_PERF_LOG").is_ok() {
        eprintln!(
            r#"{{"event":"perf_sample","bench":"{}","iteration":{},"duration_ns":{}}}"#,
            bench, iteration, duration_ns
        );
    }
}

/// Log performance summary to JSONL
#[allow(dead_code)]
fn log_perf_summary(bench: &str, p50_ns: u64, p95_ns: u64, p99_ns: u64, mean_ns: u64) {
    if std::env::var("FTUI_PERF_LOG").is_ok() {
        eprintln!(
            r#"{{"event":"perf_summary","bench":"{}","p50_ns":{},"p95_ns":{},"p99_ns":{},"mean_ns":{}}}"#,
            bench, p50_ns, p95_ns, p99_ns, mean_ns
        );
    }
}

// =============================================================================
// Local Resize Storm Generator (no ftui-harness dependency)
// =============================================================================

#[derive(Debug, Clone, Copy)]
struct ResizeEvent {
    width: u16,
    height: u16,
}

#[derive(Debug, Clone, Copy)]
struct SizeBounds {
    min_w: u16,
    max_w: u16,
    min_h: u16,
    max_h: u16,
}

impl SizeBounds {
    fn clamp(self, width: u16, height: u16) -> ResizeEvent {
        ResizeEvent {
            width: width.clamp(self.min_w, self.max_w),
            height: height.clamp(self.min_h, self.max_h),
        }
    }
}

#[derive(Debug, Clone)]
struct StormConfig {
    seed: u64,
    pattern: StormPattern,
    initial: (u16, u16),
    bounds: SizeBounds,
}

impl Default for StormConfig {
    fn default() -> Self {
        Self {
            seed: 0,
            pattern: StormPattern::Burst { count: 10 },
            initial: (80, 24),
            bounds: SizeBounds {
                min_w: 20,
                max_w: 240,
                min_h: 8,
                max_h: 80,
            },
        }
    }
}

impl StormConfig {
    fn with_seed(mut self, seed: u64) -> Self {
        self.seed = seed;
        self
    }

    fn with_pattern(mut self, pattern: StormPattern) -> Self {
        self.pattern = pattern;
        self
    }

    fn with_initial_size(mut self, width: u16, height: u16) -> Self {
        self.initial = (width, height);
        self
    }

    fn with_size_bounds(mut self, min_w: u16, max_w: u16, min_h: u16, max_h: u16) -> Self {
        self.bounds = SizeBounds {
            min_w,
            max_w,
            min_h,
            max_h,
        };
        self
    }
}

#[derive(Debug, Clone)]
enum StormPattern {
    Burst {
        count: usize,
    },
    Oscillate {
        size_a: (u16, u16),
        size_b: (u16, u16),
        cycles: usize,
    },
    Sweep {
        start_width: u16,
        start_height: u16,
        end_width: u16,
        end_height: u16,
        steps: usize,
    },
    Pathological {
        count: usize,
    },
    Mixed {
        count: usize,
    },
}

struct ResizeStorm {
    config: StormConfig,
}

impl ResizeStorm {
    fn new(config: StormConfig) -> Self {
        Self { config }
    }

    fn events(&self) -> Vec<ResizeEvent> {
        match self.config.pattern {
            StormPattern::Burst { count } => self.burst_events(count),
            StormPattern::Oscillate {
                size_a,
                size_b,
                cycles,
            } => self.oscillate_events(size_a, size_b, cycles),
            StormPattern::Sweep {
                start_width,
                start_height,
                end_width,
                end_height,
                steps,
            } => self.sweep_events(start_width, start_height, end_width, end_height, steps),
            StormPattern::Pathological { count } => self.pathological_events(count),
            StormPattern::Mixed { count } => self.mixed_events(count),
        }
    }

    fn sequence_checksum(&self) -> u64 {
        let mut hash = 0xcbf29ce484222325u64;
        for event in self.events() {
            let packed = ((event.width as u64) << 32) | event.height as u64;
            hash ^= packed;
            hash = hash.wrapping_mul(0x100000001b3);
        }
        hash
    }

    fn burst_events(&self, count: usize) -> Vec<ResizeEvent> {
        let mut lcg = Lcg::new(self.config.seed);
        (0..count)
            .map(|_| {
                let width = lcg.next_range(self.config.bounds.min_w, self.config.bounds.max_w);
                let height = lcg.next_range(self.config.bounds.min_h, self.config.bounds.max_h);
                self.config.bounds.clamp(width, height)
            })
            .collect()
    }

    fn oscillate_events(
        &self,
        size_a: (u16, u16),
        size_b: (u16, u16),
        cycles: usize,
    ) -> Vec<ResizeEvent> {
        let mut events = Vec::with_capacity(cycles * 2);
        for _ in 0..cycles {
            events.push(self.config.bounds.clamp(size_a.0, size_a.1));
            events.push(self.config.bounds.clamp(size_b.0, size_b.1));
        }
        events
    }

    fn sweep_events(
        &self,
        start_width: u16,
        start_height: u16,
        end_width: u16,
        end_height: u16,
        steps: usize,
    ) -> Vec<ResizeEvent> {
        if steps == 0 {
            return Vec::new();
        }
        let denom = (steps.saturating_sub(1)).max(1) as f32;
        (0..steps)
            .map(|i| {
                let t = i as f32 / denom;
                let width = lerp_u16(start_width, end_width, t);
                let height = lerp_u16(start_height, end_height, t);
                self.config.bounds.clamp(width, height)
            })
            .collect()
    }

    fn pathological_events(&self, count: usize) -> Vec<ResizeEvent> {
        let min = self
            .config
            .bounds
            .clamp(self.config.bounds.min_w, self.config.bounds.min_h);
        let max = self
            .config
            .bounds
            .clamp(self.config.bounds.max_w, self.config.bounds.max_h);
        let base = self
            .config
            .bounds
            .clamp(self.config.initial.0, self.config.initial.1);
        let pattern = [min, max, base];
        (0..count).map(|i| pattern[i % pattern.len()]).collect()
    }

    fn mixed_events(&self, count: usize) -> Vec<ResizeEvent> {
        let mut lcg = Lcg::new(self.config.seed);
        let min = self
            .config
            .bounds
            .clamp(self.config.bounds.min_w, self.config.bounds.min_h);
        let max = self
            .config
            .bounds
            .clamp(self.config.bounds.max_w, self.config.bounds.max_h);
        (0..count)
            .map(|i| {
                if i % 7 == 0 {
                    max
                } else if i % 5 == 0 {
                    min
                } else {
                    let width = lcg.next_range(self.config.bounds.min_w, self.config.bounds.max_w);
                    let height = lcg.next_range(self.config.bounds.min_h, self.config.bounds.max_h);
                    self.config.bounds.clamp(width, height)
                }
            })
            .collect()
    }
}

struct Lcg {
    state: u64,
}

impl Lcg {
    fn new(seed: u64) -> Self {
        let seed = if seed == 0 { 0x9e3779b97f4a7c15 } else { seed };
        Self { state: seed }
    }

    fn next_u32(&mut self) -> u32 {
        self.state = self.state.wrapping_mul(6364136223846793005).wrapping_add(1);
        (self.state >> 32) as u32
    }

    fn next_range(&mut self, min: u16, max: u16) -> u16 {
        if min >= max {
            return min;
        }
        let span = (max - min) as u32;
        let value = self.next_u32() % (span + 1);
        min + value as u16
    }
}

fn lerp_u16(start: u16, end: u16, t: f32) -> u16 {
    let start = start as f32;
    let end = end as f32;
    (start + (end - start) * t)
        .round()
        .clamp(0.0, u16::MAX as f32) as u16
}

// =============================================================================
// Burst Pattern Benchmarks
// =============================================================================

fn bench_burst_pattern(c: &mut Criterion) {
    let mut group = c.benchmark_group("resize_storm/burst");
    let seed = 42u64;

    for count in [10, 25, 50, 100] {
        let config = StormConfig::default()
            .with_seed(seed)
            .with_pattern(StormPattern::Burst { count })
            .with_initial_size(80, 24);

        let storm = ResizeStorm::new(config);
        let events = storm.events();

        log_perf_run(&format!("burst_{}", count), "burst", count, seed);

        group.bench_with_input(
            BenchmarkId::new("adaptive_buffer", count),
            &events,
            |b, events| {
                b.iter(|| {
                    let mut adb = AdaptiveDoubleBuffer::new(80, 24);
                    for event in events.iter() {
                        adb.resize(event.width, event.height);
                    }
                    black_box(adb.stats().avoidance_ratio())
                })
            },
        );
    }

    group.finish();
}

// =============================================================================
// Oscillate Pattern Benchmarks
// =============================================================================

fn bench_oscillate_pattern(c: &mut Criterion) {
    let mut group = c.benchmark_group("resize_storm/oscillate");
    let seed = 42u64;

    for cycles in [5, 10, 20, 50] {
        let config = StormConfig::default()
            .with_seed(seed)
            .with_pattern(StormPattern::Oscillate {
                size_a: (80, 24),
                size_b: (120, 40),
                cycles,
            })
            .with_initial_size(80, 24);

        let storm = ResizeStorm::new(config);
        let events = storm.events();

        log_perf_run(
            &format!("oscillate_{}", cycles),
            "oscillate",
            cycles * 2,
            seed,
        );

        group.bench_with_input(
            BenchmarkId::new("adaptive_buffer", cycles),
            &events,
            |b, events| {
                b.iter(|| {
                    let mut adb = AdaptiveDoubleBuffer::new(80, 24);
                    for event in events.iter() {
                        adb.resize(event.width, event.height);
                    }
                    black_box(adb.stats().avoidance_ratio())
                })
            },
        );
    }

    group.finish();
}

// =============================================================================
// Sweep Pattern Benchmarks
// =============================================================================

fn bench_sweep_pattern(c: &mut Criterion) {
    let mut group = c.benchmark_group("resize_storm/sweep");

    for steps in [10, 20, 50, 100] {
        let config = StormConfig::default()
            .with_pattern(StormPattern::Sweep {
                start_width: 40,
                start_height: 12,
                end_width: 200,
                end_height: 60,
                steps,
            })
            .with_initial_size(40, 12);

        let storm = ResizeStorm::new(config);
        let events = storm.events();

        log_perf_run(&format!("sweep_{}", steps), "sweep", steps, 0);

        group.bench_with_input(
            BenchmarkId::new("adaptive_buffer", steps),
            &events,
            |b, events| {
                b.iter(|| {
                    let mut adb = AdaptiveDoubleBuffer::new(40, 12);
                    for event in events.iter() {
                        adb.resize(event.width, event.height);
                    }
                    black_box(adb.stats().avoidance_ratio())
                })
            },
        );
    }

    group.finish();
}

// =============================================================================
// Pathological Pattern Benchmarks
// =============================================================================

fn bench_pathological_pattern(c: &mut Criterion) {
    let mut group = c.benchmark_group("resize_storm/pathological");
    let seed = 42u64;

    for count in [10, 20, 40] {
        let config = StormConfig::default()
            .with_seed(seed)
            .with_pattern(StormPattern::Pathological { count })
            .with_initial_size(80, 24);

        let storm = ResizeStorm::new(config);
        let events = storm.events();

        log_perf_run(
            &format!("pathological_{}", count),
            "pathological",
            count,
            seed,
        );

        group.bench_with_input(
            BenchmarkId::new("adaptive_buffer", count),
            &events,
            |b, events| {
                b.iter(|| {
                    let mut adb = AdaptiveDoubleBuffer::new(80, 24);
                    for event in events.iter() {
                        adb.resize(event.width, event.height);
                    }
                    black_box(adb.stats().avoidance_ratio())
                })
            },
        );
    }

    group.finish();
}

// =============================================================================
// Mixed Pattern Benchmarks
// =============================================================================

fn bench_mixed_pattern(c: &mut Criterion) {
    let mut group = c.benchmark_group("resize_storm/mixed");
    let seed = 42u64;

    for count in [50, 100, 200] {
        let config = StormConfig::default()
            .with_seed(seed)
            .with_pattern(StormPattern::Mixed { count })
            .with_initial_size(80, 24);

        let storm = ResizeStorm::new(config);
        let events = storm.events();

        log_perf_run(&format!("mixed_{}", count), "mixed", count, seed);

        group.bench_with_input(
            BenchmarkId::new("adaptive_buffer", count),
            &events,
            |b, events| {
                b.iter(|| {
                    let mut adb = AdaptiveDoubleBuffer::new(80, 24);
                    for event in events.iter() {
                        adb.resize(event.width, event.height);
                    }
                    black_box(adb.stats().avoidance_ratio())
                })
            },
        );
    }

    group.finish();
}

// =============================================================================
// Large-Screen Storm Benchmarks (bd-3e1t.4)
// =============================================================================

fn bench_large_screen_storm(c: &mut Criterion) {
    let mut group = c.benchmark_group("resize_storm/large");
    let seed = 4242u64;

    let cases = [
        (
            "burst_50",
            StormPattern::Burst { count: 50 },
            (200u16, 60u16),
        ),
        (
            "oscillate_20",
            StormPattern::Oscillate {
                size_a: (200, 60),
                size_b: (240, 80),
                cycles: 20,
            },
            (200u16, 60u16),
        ),
    ];

    for (label, pattern, initial) in cases {
        let config = StormConfig::default()
            .with_seed(seed)
            .with_pattern(pattern)
            .with_initial_size(initial.0, initial.1)
            .with_size_bounds(160, 320, 50, 120);

        let storm = ResizeStorm::new(config);
        let events = storm.events();

        log_perf_run(label, "large", events.len(), seed);

        group.bench_with_input(
            BenchmarkId::new("adaptive_buffer", label),
            &events,
            |b, events| {
                b.iter(|| {
                    let mut adb = AdaptiveDoubleBuffer::new(initial.0, initial.1);
                    for event in events.iter() {
                        adb.resize(event.width, event.height);
                    }
                    black_box(adb.stats().avoidance_ratio())
                })
            },
        );
    }

    group.finish();
}

// =============================================================================
// Single Resize Operation (Baseline)
// =============================================================================

fn bench_single_resize(c: &mut Criterion) {
    let mut group = c.benchmark_group("resize_storm/single");

    // Small resize (within capacity)
    group.bench_function("within_capacity", |b| {
        let mut adb = AdaptiveDoubleBuffer::new(80, 24);
        b.iter(|| {
            adb.resize(85, 26);
            adb.resize(80, 24);
            black_box(&adb);
        })
    });

    // Large resize (beyond capacity)
    group.bench_function("beyond_capacity", |b| {
        let mut adb = AdaptiveDoubleBuffer::new(80, 24);
        b.iter(|| {
            adb.resize(200, 60);
            adb.resize(80, 24);
            black_box(&adb);
        })
    });

    // Resize to same size (no-op)
    group.bench_function("noop", |b| {
        let mut adb = AdaptiveDoubleBuffer::new(80, 24);
        b.iter(|| {
            adb.resize(80, 24);
            black_box(&adb);
        })
    });

    group.finish();
}

// =============================================================================
// Avoidance Ratio Verification
// =============================================================================

fn bench_avoidance_ratio_tracking(c: &mut Criterion) {
    let mut group = c.benchmark_group("resize_storm/avoidance");

    // Verify avoidance ratio meets budget (>= 80% for oscillate within capacity)
    let config = StormConfig::default()
        .with_seed(42)
        .with_pattern(StormPattern::Oscillate {
            size_a: (80, 24),
            size_b: (90, 28), // Within initial capacity
            cycles: 50,
        })
        .with_initial_size(80, 24);

    let storm = ResizeStorm::new(config);
    let events = storm.events();

    group.bench_function("oscillate_within_capacity", |b| {
        b.iter(|| {
            let mut adb = AdaptiveDoubleBuffer::new(80, 24);
            for event in events.iter() {
                adb.resize(event.width, event.height);
            }
            let ratio = adb.stats().avoidance_ratio();
            // Should achieve high avoidance when staying within capacity
            assert!(
                ratio >= 0.80,
                "Avoidance ratio {:.2}% below budget (80%)",
                ratio * 100.0
            );
            black_box(ratio)
        })
    });

    group.finish();
}

// =============================================================================
// Memory Efficiency Under Storm
// =============================================================================

fn bench_memory_efficiency(c: &mut Criterion) {
    let mut group = c.benchmark_group("resize_storm/memory");

    let config = StormConfig::default()
        .with_seed(42)
        .with_pattern(StormPattern::Burst { count: 100 })
        .with_initial_size(80, 24);

    let storm = ResizeStorm::new(config);
    let events = storm.events();

    group.bench_function("efficiency_after_storm", |b| {
        b.iter(|| {
            let mut adb = AdaptiveDoubleBuffer::new(80, 24);
            for event in events.iter() {
                adb.resize(event.width, event.height);
            }
            let efficiency = adb.memory_efficiency();
            // Memory efficiency should stay above 35% even after storm
            assert!(
                efficiency >= 0.35,
                "Memory efficiency {:.2}% below budget (35%)",
                efficiency * 100.0
            );
            black_box(efficiency)
        })
    });

    group.finish();
}

// =============================================================================
// Deterministic Replay Verification
// =============================================================================

fn bench_deterministic_replay(c: &mut Criterion) {
    let mut group = c.benchmark_group("resize_storm/determinism");

    // Same seed should produce same results
    let seed = 12345u64;
    let config = StormConfig::default()
        .with_seed(seed)
        .with_pattern(StormPattern::Burst { count: 50 })
        .with_initial_size(80, 24);

    let storm1 = ResizeStorm::new(config.clone());
    let storm2 = ResizeStorm::new(config);

    // Verify checksums match
    assert_eq!(
        storm1.sequence_checksum(),
        storm2.sequence_checksum(),
        "Deterministic replay failed: checksums differ"
    );

    group.bench_function("verify_checksum", |b| {
        b.iter(|| {
            let config = StormConfig::default()
                .with_seed(black_box(seed))
                .with_pattern(StormPattern::Burst { count: 50 });
            let storm = ResizeStorm::new(config);
            black_box(storm.sequence_checksum())
        })
    });

    group.finish();
}

// =============================================================================
// Benchmark Group Registration
// =============================================================================

criterion_group!(
    benches,
    bench_burst_pattern,
    bench_oscillate_pattern,
    bench_sweep_pattern,
    bench_pathological_pattern,
    bench_mixed_pattern,
    bench_large_screen_storm,
    bench_single_resize,
    bench_avoidance_ratio_tracking,
    bench_memory_efficiency,
    bench_deterministic_replay,
);
criterion_main!(benches);
