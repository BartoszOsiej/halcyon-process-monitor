//! Benchmarks for tick strategy I/O reduction (G.2).
//!
//! Measures tick count reduction for each strategy compared to the baseline
//! (all screens ticked every frame). Uses a mock model with 15 screens.
//!
//! Run with: cargo bench -p ftui-runtime --bench tick_strategy_bench
//!
//! Expected results (1000 frames, 15 screens):
//!
//! | Strategy           | Total Ticks | Reduction |
//! |--------------------|-------------|-----------|
//! | None (baseline)    | 15,000      | 0%        |
//! | ActiveOnly         | 1,000       | 93%       |
//! | Uniform(5)         | ~3,800      | ~75%      |
//! | ActivePlusAdjacent | ~5,400      | ~64%      |
//! | Predictive (warm)  | ~2,500-4,000| ~73-83%   |

use criterion::{Criterion, criterion_group, criterion_main};
use std::hint::black_box;

use ftui_runtime::{
    ActiveOnly, ActivePlusAdjacent, Predictive, PredictiveStrategyConfig, TickDecision,
    TickStrategy, Uniform,
};

const NUM_SCREENS: usize = 15;
const FRAMES: u64 = 1000;

fn screen_names() -> Vec<String> {
    (0..NUM_SCREENS).map(|i| format!("Screen{i}")).collect()
}

/// Count total ticks dispatched across all screens over `frames` frames.
fn count_ticks(strategy: &mut dyn TickStrategy, screens: &[String], frames: u64) -> u64 {
    let active = &screens[0];
    let mut total = 0u64;

    for tick in 0..frames {
        // Active screen always ticks
        total += 1;

        // Inactive screens consult the strategy
        for screen in &screens[1..] {
            if strategy.should_tick(screen, tick, active) == TickDecision::Tick {
                total += 1;
            }
        }

        strategy.maintenance_tick(tick);
    }

    total
}

/// Baseline: all screens tick every frame (no strategy).
fn count_baseline(screens: &[String], frames: u64) -> u64 {
    screens.len() as u64 * frames
}

// =============================================================================
// Benchmarks
// =============================================================================

fn bench_tick_strategies(c: &mut Criterion) {
    let mut group = c.benchmark_group("tick_strategy/io_reduction");
    let screens = screen_names();

    // Baseline measurement (for comparison)
    group.bench_function("baseline_all_tick", |b| {
        b.iter(|| black_box(count_baseline(&screens, FRAMES)))
    });

    // ActiveOnly
    group.bench_function("active_only", |b| {
        b.iter(|| {
            let mut strategy = ActiveOnly;
            black_box(count_ticks(&mut strategy, &screens, FRAMES))
        })
    });

    // Uniform(5)
    group.bench_function("uniform_5", |b| {
        b.iter(|| {
            let mut strategy = Uniform::new(5);
            black_box(count_ticks(&mut strategy, &screens, FRAMES))
        })
    });

    // ActivePlusAdjacent (tab order, background_divisor=5)
    group.bench_function("adjacent_tab_order", |b| {
        let screen_refs: Vec<&str> = screens.iter().map(|s| s.as_str()).collect();
        b.iter(|| {
            let mut strategy = ActivePlusAdjacent::from_tab_order(&screen_refs, 5);
            black_box(count_ticks(&mut strategy, &screens, FRAMES))
        })
    });

    // Predictive (cold start)
    group.bench_function("predictive_cold", |b| {
        b.iter(|| {
            let config = PredictiveStrategyConfig {
                fallback_divisor: 5,
                min_observations: 50,
                decay_interval: 10_000,
                ..PredictiveStrategyConfig::default()
            };
            let mut strategy = Predictive::new(config);
            black_box(count_ticks(&mut strategy, &screens, FRAMES))
        })
    });

    // Predictive (warm — pre-trained)
    group.bench_function("predictive_warm", |b| {
        b.iter(|| {
            let config = PredictiveStrategyConfig {
                fallback_divisor: 10,
                min_observations: 5,
                decay_interval: 10_000,
                ..PredictiveStrategyConfig::default()
            };
            let mut strategy = Predictive::new(config);

            // Train: Screen0 → Screen1 (50%), Screen0 → Screen2 (30%),
            // Screen0 → Screen3 (15%), rest 5% spread
            for _ in 0..50 {
                strategy.on_screen_transition("Screen0", "Screen1");
            }
            for _ in 0..30 {
                strategy.on_screen_transition("Screen0", "Screen2");
            }
            for _ in 0..15 {
                strategy.on_screen_transition("Screen0", "Screen3");
            }
            for i in 4..NUM_SCREENS {
                strategy.on_screen_transition("Screen0", &format!("Screen{i}"));
            }

            black_box(count_ticks(&mut strategy, &screens, FRAMES))
        })
    });

    group.finish();
}

fn bench_io_reduction_report(c: &mut Criterion) {
    let mut group = c.benchmark_group("tick_strategy/io_reduction_report");
    let screens = screen_names();

    // This benchmark just prints the reduction report for human consumption.
    group.bench_function("report", |b| {
        b.iter(|| {
            let baseline = count_baseline(&screens, FRAMES);

            let mut active_only = ActiveOnly;
            let active_only_ticks = count_ticks(&mut active_only, &screens, FRAMES);

            let mut uniform5 = Uniform::new(5);
            let uniform5_ticks = count_ticks(&mut uniform5, &screens, FRAMES);

            let screen_refs: Vec<&str> = screens.iter().map(|s| s.as_str()).collect();
            let mut adjacent = ActivePlusAdjacent::from_tab_order(&screen_refs, 5);
            let adjacent_ticks = count_ticks(&mut adjacent, &screens, FRAMES);

            let config = PredictiveStrategyConfig {
                fallback_divisor: 10,
                min_observations: 5,
                decay_interval: 10_000,
                ..PredictiveStrategyConfig::default()
            };
            let mut predictive = Predictive::new(config);
            for _ in 0..50 {
                predictive.on_screen_transition("Screen0", "Screen1");
            }
            for _ in 0..30 {
                predictive.on_screen_transition("Screen0", "Screen2");
            }
            for _ in 0..15 {
                predictive.on_screen_transition("Screen0", "Screen3");
            }
            let predictive_ticks = count_ticks(&mut predictive, &screens, FRAMES);

            black_box((
                baseline,
                active_only_ticks,
                uniform5_ticks,
                adjacent_ticks,
                predictive_ticks,
            ))
        })
    });

    group.finish();

    // Print the report once outside the benchmark loop.
    let screens = screen_names();
    let baseline = count_baseline(&screens, FRAMES);

    let mut active_only = ActiveOnly;
    let active_only_ticks = count_ticks(&mut active_only, &screens, FRAMES);

    let mut uniform5 = Uniform::new(5);
    let uniform5_ticks = count_ticks(&mut uniform5, &screens, FRAMES);

    let screen_refs: Vec<&str> = screens.iter().map(|s| s.as_str()).collect();
    let mut adjacent = ActivePlusAdjacent::from_tab_order(&screen_refs, 5);
    let adjacent_ticks = count_ticks(&mut adjacent, &screens, FRAMES);

    let config = PredictiveStrategyConfig {
        fallback_divisor: 10,
        min_observations: 5,
        decay_interval: 10_000,
        ..PredictiveStrategyConfig::default()
    };
    let mut predictive = Predictive::new(config);
    for _ in 0..50 {
        predictive.on_screen_transition("Screen0", "Screen1");
    }
    for _ in 0..30 {
        predictive.on_screen_transition("Screen0", "Screen2");
    }
    for _ in 0..15 {
        predictive.on_screen_transition("Screen0", "Screen3");
    }
    let predictive_ticks = count_ticks(&mut predictive, &screens, FRAMES);

    let reduction = |ticks: u64| -> f64 { (1.0 - ticks as f64 / baseline as f64) * 100.0 };

    eprintln!("\n=== Tick Strategy I/O Reduction Report ===");
    eprintln!(
        "  Baseline (no strategy):  {baseline:>6} ticks ({:.0}% reduction)",
        reduction(baseline)
    );
    eprintln!(
        "  ActiveOnly:              {active_only_ticks:>6} ticks ({:.0}% reduction)",
        reduction(active_only_ticks)
    );
    eprintln!(
        "  Uniform(5):              {uniform5_ticks:>6} ticks ({:.0}% reduction)",
        reduction(uniform5_ticks)
    );
    eprintln!(
        "  ActivePlusAdjacent:      {adjacent_ticks:>6} ticks ({:.0}% reduction)",
        reduction(adjacent_ticks)
    );
    eprintln!(
        "  Predictive (warm):       {predictive_ticks:>6} ticks ({:.0}% reduction)",
        reduction(predictive_ticks)
    );
    eprintln!("==========================================\n");
}

criterion_group!(benches, bench_tick_strategies, bench_io_reduction_report);
criterion_main!(benches);
