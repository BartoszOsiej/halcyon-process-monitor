//! Regression guard tests for Performance HUD (bd-3k3x.4)
//!
//! These tests verify that Performance HUD rendering meets timing budgets.
//! They are separate from criterion benchmarks because they use #[test] and
//! can fail CI when budgets are exceeded.
//!
//! Run with: cargo test -p ftui-demo-showcase --test perf_hud_regression --release
//!
//! **IMPORTANT**: These tests are designed for RELEASE mode only.
//! Debug builds are 10-20x slower and will fail the budget assertions.
//! Always run with `--release` flag.
//!
//! Performance budgets (per bd-3k3x.4):
//! - HUD render (120x40): < 500µs (with 2x CI margin = 1000µs)
//! - HUD render (80x24): < 200µs (with 2x CI margin = 400µs)
//! - HUD overhead vs no-HUD: < 50% additional time (with 2x CI margin = 100%)
//! - Ring buffer tick: < 1µs (with 10x CI margin = 10µs)
//! - HUD toggle: < 100µs
//!
//! JSONL logging: Set PERF_HUD_JSONL=1 to emit structured logs for CI.

#![forbid(unsafe_code)]

use ftui_core::event::{Event, KeyCode, KeyEvent, KeyEventKind, Modifiers};
use ftui_core::geometry::Rect;
use ftui_demo_showcase::app::{AppModel, AppMsg};
use ftui_demo_showcase::screens::Screen;
use ftui_demo_showcase::screens::performance_hud::PerformanceHud;
use ftui_render::frame::Frame;
use ftui_render::grapheme_pool::GraphemePool;
use ftui_runtime::program::Model;
use std::time::Instant;

// =============================================================================
// Constants & Configuration
// =============================================================================

/// Budget: HUD render at 120x40 should be under this value.
/// Based on release mode profiling: ~450-500µs typical.
const BUDGET_HUD_RENDER_120X40_US: u64 = 500;

/// Budget: HUD render at 80x24 should be under this value.
/// Based on release mode profiling: ~150-200µs typical.
const BUDGET_HUD_RENDER_80X24_US: u64 = 200;

/// Budget: Ring buffer tick should be under this value.
const BUDGET_RING_PUSH_US: u64 = 1;

/// Maximum allowed overhead percentage when HUD is enabled.
const MAX_OVERHEAD_PERCENT: f64 = 50.0;

// =============================================================================
// Helper Functions
// =============================================================================

fn ctrl_press(ch: char) -> Event {
    Event::Key(KeyEvent {
        code: KeyCode::Char(ch),
        modifiers: Modifiers::CTRL,
        kind: KeyEventKind::Press,
    })
}

/// Emit JSONL log if PERF_HUD_JSONL=1 is set.
fn log_jsonl(data: &[(&str, &str)]) {
    if std::env::var("PERF_HUD_JSONL").is_ok() {
        let fields: Vec<String> = data
            .iter()
            .map(|(k, v)| format!("\"{k}\":\"{v}\""))
            .collect();
        eprintln!("{{{}}}", fields.join(","));
    }
}

#[derive(Debug, Clone, Copy)]
struct PerfStats {
    avg: u64,
    p50: u64,
    p95: u64,
    p99: u64,
}

fn percentile(sorted: &[u64], pct: usize) -> u64 {
    if sorted.is_empty() {
        return 0;
    }
    let idx = (sorted.len() - 1) * pct / 100;
    sorted[idx]
}

fn compute_stats(samples: &mut [u64]) -> PerfStats {
    if samples.is_empty() {
        return PerfStats {
            avg: 0,
            p50: 0,
            p95: 0,
            p99: 0,
        };
    }
    let sum: u128 = samples.iter().map(|value| *value as u128).sum();
    let avg = (sum / samples.len() as u128) as u64;
    samples.sort_unstable();
    PerfStats {
        avg,
        p50: percentile(samples, 50),
        p95: percentile(samples, 95),
        p99: percentile(samples, 99),
    }
}

fn measure_us(iterations: usize, mut f: impl FnMut()) -> Vec<u64> {
    let mut samples = Vec::with_capacity(iterations);
    for _ in 0..iterations {
        let start = Instant::now();
        f();
        samples.push(start.elapsed().as_micros() as u64);
    }
    samples
}

/// Create a PerformanceHud with pre-populated samples for benchmarking.
fn create_seeded_hud(sample_count: usize) -> PerformanceHud {
    let mut hud = PerformanceHud::new();
    // Seed with ticks to populate internal ring buffer
    for i in 0..sample_count {
        hud.tick(i as u64);
        // Wait a tiny bit to ensure tick recording works
        std::thread::sleep(std::time::Duration::from_micros(10));
    }
    hud
}

/// Create an AppModel with HUD enabled and seeded metrics.
fn create_app_with_hud(enabled: bool, tick_count: usize) -> AppModel {
    let mut app = AppModel::new();
    app.perf_hud_visible = enabled;
    app.terminal_width = 120;
    app.terminal_height = 40;

    // Seed with ticks
    for _ in 0..tick_count {
        let _ = app.update(AppMsg::Tick);
    }
    app
}

// =============================================================================
// Regression Guards
// =============================================================================

/// Returns true if running in release mode (debug_assertions disabled).
fn is_release_mode() -> bool {
    !cfg!(debug_assertions)
}

/// Verify render budgets are met at 120x40.
/// Skips in debug mode since timings are 10-20x slower.
#[test]
fn regression_guard_render_120x40() {
    if !is_release_mode() {
        eprintln!("SKIPPED: regression_guard_render_120x40 (debug build - run with --release)");
        return;
    }
    let hud = create_seeded_hud(60);
    let mut pool = GraphemePool::new();

    // Warmup
    for _ in 0..10 {
        let mut frame = Frame::new(120, 40, &mut pool);
        let area = Rect::new(0, 0, 120, 40);
        hud.view(&mut frame, area);
    }

    // Measure
    let iterations = 100usize;
    let mut samples = measure_us(iterations, || {
        let mut frame = Frame::new(120, 40, &mut pool);
        let area = Rect::new(0, 0, 120, 40);
        hud.view(&mut frame, area);
    });
    let stats = compute_stats(&mut samples);
    let avg_us = stats.avg;

    log_jsonl(&[
        ("test", "regression_guard_render_120x40"),
        ("avg_us", &avg_us.to_string()),
        ("p50_us", &stats.p50.to_string()),
        ("p95_us", &stats.p95.to_string()),
        ("p99_us", &stats.p99.to_string()),
        ("budget_us", &BUDGET_HUD_RENDER_120X40_US.to_string()),
        (
            "passed",
            &(avg_us < BUDGET_HUD_RENDER_120X40_US * 2).to_string(),
        ),
    ]);

    // Allow 2x margin for CI environments
    assert!(
        avg_us < BUDGET_HUD_RENDER_120X40_US * 2,
        "HUD render 120x40 exceeded budget: {avg_us}µs (budget: {}µs)",
        BUDGET_HUD_RENDER_120X40_US
    );
}

/// Verify render budgets are met at 80x24.
/// Skips in debug mode since timings are 10-20x slower.
#[test]
fn regression_guard_render_80x24() {
    if !is_release_mode() {
        eprintln!("SKIPPED: regression_guard_render_80x24 (debug build - run with --release)");
        return;
    }
    let hud = create_seeded_hud(60);
    let mut pool = GraphemePool::new();

    // Warmup
    for _ in 0..10 {
        let mut frame = Frame::new(80, 24, &mut pool);
        let area = Rect::new(0, 0, 80, 24);
        hud.view(&mut frame, area);
    }

    // Measure
    let iterations = 100usize;
    let mut samples = measure_us(iterations, || {
        let mut frame = Frame::new(80, 24, &mut pool);
        let area = Rect::new(0, 0, 80, 24);
        hud.view(&mut frame, area);
    });
    let stats = compute_stats(&mut samples);
    let avg_us = stats.avg;

    log_jsonl(&[
        ("test", "regression_guard_render_80x24"),
        ("avg_us", &avg_us.to_string()),
        ("p50_us", &stats.p50.to_string()),
        ("p95_us", &stats.p95.to_string()),
        ("p99_us", &stats.p99.to_string()),
        ("budget_us", &BUDGET_HUD_RENDER_80X24_US.to_string()),
        (
            "passed",
            &(avg_us < BUDGET_HUD_RENDER_80X24_US * 2).to_string(),
        ),
    ]);

    assert!(
        avg_us < BUDGET_HUD_RENDER_80X24_US * 2,
        "HUD render 80x24 exceeded budget: {avg_us}µs (budget: {}µs)",
        BUDGET_HUD_RENDER_80X24_US
    );
}

/// Verify HUD overhead vs no-HUD baseline.
/// Skips in debug mode since timings are 10-20x slower.
#[test]
fn regression_guard_overhead_ratio() {
    if !is_release_mode() {
        eprintln!("SKIPPED: regression_guard_overhead_ratio (debug build - run with --release)");
        return;
    }
    let app_no_hud = create_app_with_hud(false, 20);
    let app_with_hud = create_app_with_hud(true, 20);
    let mut pool = GraphemePool::new();

    // Warmup
    for _ in 0..10 {
        let mut frame = Frame::new(120, 40, &mut pool);
        app_no_hud.view(&mut frame);
        app_with_hud.view(&mut frame);
    }

    // Measure no-HUD
    let iterations = 50usize;
    let mut no_hud_samples = measure_us(iterations, || {
        let mut frame = Frame::new(120, 40, &mut pool);
        app_no_hud.view(&mut frame);
    });
    let no_hud_stats = compute_stats(&mut no_hud_samples);

    // Measure with-HUD
    let mut with_hud_samples = measure_us(iterations, || {
        let mut frame = Frame::new(120, 40, &mut pool);
        app_with_hud.view(&mut frame);
    });
    let with_hud_stats = compute_stats(&mut with_hud_samples);

    let no_hud_us = no_hud_stats.avg as f64;
    let with_hud_us = with_hud_stats.avg as f64;
    let overhead_percent = ((with_hud_us - no_hud_us) / no_hud_us.max(1.0)) * 100.0;

    log_jsonl(&[
        ("test", "regression_guard_overhead_ratio"),
        ("no_hud_avg_us", &no_hud_stats.avg.to_string()),
        ("no_hud_p50_us", &no_hud_stats.p50.to_string()),
        ("no_hud_p95_us", &no_hud_stats.p95.to_string()),
        ("no_hud_p99_us", &no_hud_stats.p99.to_string()),
        ("with_hud_avg_us", &with_hud_stats.avg.to_string()),
        ("with_hud_p50_us", &with_hud_stats.p50.to_string()),
        ("with_hud_p95_us", &with_hud_stats.p95.to_string()),
        ("with_hud_p99_us", &with_hud_stats.p99.to_string()),
        ("overhead_percent", &format!("{:.1}", overhead_percent)),
        (
            "max_allowed_percent",
            &format!("{:.1}", MAX_OVERHEAD_PERCENT),
        ),
        (
            "passed",
            &(overhead_percent < MAX_OVERHEAD_PERCENT * 2.0).to_string(),
        ),
    ]);

    // Allow 2x margin for CI
    assert!(
        overhead_percent < MAX_OVERHEAD_PERCENT * 2.0,
        "HUD overhead too high: {overhead_percent:.1}% (max: {MAX_OVERHEAD_PERCENT}%)"
    );
}

/// Verify tick recording is fast.
/// Skips in debug mode since timings are 10-20x slower.
#[test]
fn regression_guard_tick_recording() {
    if !is_release_mode() {
        eprintln!("SKIPPED: regression_guard_tick_recording (debug build - run with --release)");
        return;
    }
    let mut hud = PerformanceHud::new();

    // Warmup
    for i in 0..100 {
        hud.tick(i);
    }

    // Measure tick recording
    let iterations = 1000usize;
    let mut counter = 0u64;
    let mut samples = measure_us(iterations, || {
        hud.tick(100 + counter);
        counter += 1;
    });
    let stats = compute_stats(&mut samples);
    let avg_us = stats.avg;

    log_jsonl(&[
        ("test", "regression_guard_tick_recording"),
        ("avg_us", &avg_us.to_string()),
        ("p50_us", &stats.p50.to_string()),
        ("p95_us", &stats.p95.to_string()),
        ("p99_us", &stats.p99.to_string()),
        ("budget_us", &BUDGET_RING_PUSH_US.to_string()),
        ("passed", &(avg_us < BUDGET_RING_PUSH_US * 10).to_string()),
    ]);

    // Allow 10x margin for CI (tick is very fast, sub-microsecond)
    assert!(
        avg_us < BUDGET_RING_PUSH_US * 10,
        "Tick recording exceeded budget: {avg_us}µs (budget: {}µs)",
        BUDGET_RING_PUSH_US
    );
}

/// Verify HUD toggle is fast.
/// Skips in debug mode since timings are 10-20x slower.
#[test]
fn regression_guard_toggle_fast() {
    if !is_release_mode() {
        eprintln!("SKIPPED: regression_guard_toggle_fast (debug build - run with --release)");
        return;
    }
    let mut app = create_app_with_hud(false, 10);

    // Warmup
    for _ in 0..10 {
        let _ = app.update(AppMsg::ScreenEvent(ctrl_press('p')));
    }

    // Measure toggle
    let iterations = 100usize;
    let mut samples = measure_us(iterations, || {
        let _ = app.update(AppMsg::ScreenEvent(ctrl_press('p')));
    });
    let stats = compute_stats(&mut samples);
    let avg_us = stats.avg;

    log_jsonl(&[
        ("test", "regression_guard_toggle_fast"),
        ("avg_us", &avg_us.to_string()),
        ("p50_us", &stats.p50.to_string()),
        ("p95_us", &stats.p95.to_string()),
        ("p99_us", &stats.p99.to_string()),
        ("budget_us", "100"),
        ("passed", &(avg_us < 100).to_string()),
    ]);

    // Toggle should be under 100µs
    assert!(
        avg_us < 100,
        "HUD toggle too slow: {avg_us}µs (budget: 100µs)"
    );
}
