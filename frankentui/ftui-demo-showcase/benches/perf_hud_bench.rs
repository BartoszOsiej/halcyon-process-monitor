//! Benchmarks for Performance HUD overhead and regression testing (bd-3k3x.4)
//!
//! Measures HUD rendering overhead and guards against performance regressions.
//!
//! Run with: cargo bench -p ftui-demo-showcase --bench perf_hud_bench
//!
//! Performance budgets (per bd-3k3x.4):
//! - HUD render (120x40): < 500µs
//! - HUD render (80x24): < 200µs
//! - HUD overhead vs no-HUD: < 50% additional time
//! - Ring buffer tick: < 1µs
//! - HUD toggle: < 100µs
//!
//! Note: Regression tests in tests/perf_hud_regression.rs use 2x margins for CI.
//!
//! JSONL logging: Set PERF_HUD_JSONL=1 to emit structured logs for CI.

#![forbid(unsafe_code)]

use criterion::{Criterion, Throughput, criterion_group, criterion_main};
use ftui_core::event::{Event, KeyCode, KeyEvent, KeyEventKind, Modifiers};
use ftui_core::geometry::Rect;
use ftui_demo_showcase::app::{AppModel, AppMsg};
use ftui_demo_showcase::screens::Screen;
use ftui_demo_showcase::screens::dashboard::Dashboard;
use ftui_demo_showcase::screens::performance_hud::PerformanceHud;
use ftui_render::frame::Frame;
use ftui_render::grapheme_pool::GraphemePool;
use ftui_runtime::program::Model;
use std::hint::black_box;

// =============================================================================
// Note: Regression guard tests with budget assertions are in:
// tests/perf_hud_regression.rs
// =============================================================================

// =============================================================================
// Helper Functions
// =============================================================================

fn press(code: KeyCode) -> Event {
    Event::Key(KeyEvent {
        code,
        modifiers: Modifiers::NONE,
        kind: KeyEventKind::Press,
    })
}

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
// Benchmark: HUD Render at Various Sizes
// =============================================================================

fn bench_hud_render(c: &mut Criterion) {
    let mut group = c.benchmark_group("perf_hud/render");

    log_jsonl(&[("group", "perf_hud/render"), ("event", "start")]);

    // 120x40 render benchmark
    group.throughput(Throughput::Elements(1));
    group.bench_function("render_120x40", |b| {
        let hud = create_seeded_hud(60);
        let mut pool = GraphemePool::new();

        b.iter(|| {
            let mut frame = Frame::new(120, 40, &mut pool);
            let area = Rect::new(0, 0, 120, 40);
            hud.view(black_box(&mut frame), black_box(area));
            black_box(&frame);
        })
    });

    // 80x24 render benchmark
    group.bench_function("render_80x24", |b| {
        let hud = create_seeded_hud(60);
        let mut pool = GraphemePool::new();

        b.iter(|| {
            let mut frame = Frame::new(80, 24, &mut pool);
            let area = Rect::new(0, 0, 80, 24);
            hud.view(black_box(&mut frame), black_box(area));
            black_box(&frame);
        })
    });

    // Small terminal (graceful degradation)
    group.bench_function("render_40x10", |b| {
        let hud = create_seeded_hud(30);
        let mut pool = GraphemePool::new();

        b.iter(|| {
            let mut frame = Frame::new(40, 10, &mut pool);
            let area = Rect::new(0, 0, 40, 10);
            hud.view(black_box(&mut frame), black_box(area));
            black_box(&frame);
        })
    });

    log_jsonl(&[("group", "perf_hud/render"), ("event", "complete")]);
    group.finish();
}

// =============================================================================
// Benchmark: HUD Overhead vs No-HUD Baseline
// =============================================================================

fn bench_hud_overhead(c: &mut Criterion) {
    let mut group = c.benchmark_group("perf_hud/overhead");

    log_jsonl(&[("group", "perf_hud/overhead"), ("event", "start")]);

    // Baseline: App render without HUD
    group.bench_function("app_no_hud_120x40", |b| {
        let app = create_app_with_hud(false, 20);
        let mut pool = GraphemePool::new();

        b.iter(|| {
            let mut frame = Frame::new(120, 40, &mut pool);
            app.view(black_box(&mut frame));
            black_box(&frame);
        })
    });

    // With HUD enabled
    group.bench_function("app_with_hud_120x40", |b| {
        let app = create_app_with_hud(true, 20);
        let mut pool = GraphemePool::new();

        b.iter(|| {
            let mut frame = Frame::new(120, 40, &mut pool);
            app.view(black_box(&mut frame));
            black_box(&frame);
        })
    });

    // Toggle overhead (Ctrl+P processing)
    group.bench_function("hud_toggle", |b| {
        let mut app = create_app_with_hud(false, 10);

        b.iter(|| {
            let _ = app.update(AppMsg::ScreenEvent(ctrl_press('p')));
            black_box(app.perf_hud_visible);
        })
    });

    log_jsonl(&[("group", "perf_hud/overhead"), ("event", "complete")]);
    group.finish();
}

// =============================================================================
// Benchmark: View Render with Different Sample Counts
// =============================================================================

fn bench_render_by_samples(c: &mut Criterion) {
    let mut group = c.benchmark_group("perf_hud/samples");

    log_jsonl(&[("group", "perf_hud/samples"), ("event", "start")]);

    // Render with 120 samples (full buffer)
    group.bench_function("render_120_samples", |b| {
        let hud = create_seeded_hud(120);
        let mut pool = GraphemePool::new();

        b.iter(|| {
            let mut frame = Frame::new(120, 40, &mut pool);
            let area = Rect::new(0, 0, 120, 40);
            hud.view(black_box(&mut frame), black_box(area));
            black_box(&frame);
        })
    });

    // Render with 30 samples (partial buffer)
    group.bench_function("render_30_samples", |b| {
        let hud = create_seeded_hud(30);
        let mut pool = GraphemePool::new();

        b.iter(|| {
            let mut frame = Frame::new(120, 40, &mut pool);
            let area = Rect::new(0, 0, 120, 40);
            hud.view(black_box(&mut frame), black_box(area));
            black_box(&frame);
        })
    });

    // Render with empty buffer (edge case)
    group.bench_function("render_empty", |b| {
        let hud = PerformanceHud::new();
        let mut pool = GraphemePool::new();

        b.iter(|| {
            let mut frame = Frame::new(120, 40, &mut pool);
            let area = Rect::new(0, 0, 120, 40);
            hud.view(black_box(&mut frame), black_box(area));
            black_box(&frame);
        })
    });

    log_jsonl(&[("group", "perf_hud/samples"), ("event", "complete")]);
    group.finish();
}

// =============================================================================
// Benchmark: Ring Buffer Operations
// =============================================================================

fn bench_ring_buffer(c: &mut Criterion) {
    let mut group = c.benchmark_group("perf_hud/ring_buffer");

    log_jsonl(&[("group", "perf_hud/ring_buffer"), ("event", "start")]);

    // Single tick recording
    group.bench_function("tick_record", |b| {
        let mut hud = PerformanceHud::new();
        let mut tick_count = 0u64;

        b.iter(|| {
            tick_count += 1;
            hud.tick(black_box(tick_count));
        })
    });

    // Reset operation
    group.bench_function("reset", |b| {
        let mut hud = create_seeded_hud(120);

        b.iter(|| {
            hud.update(&press(KeyCode::Char('r')));
            black_box(&hud);
        })
    });

    log_jsonl(&[("group", "perf_hud/ring_buffer"), ("event", "complete")]);
    group.finish();
}

// =============================================================================
// Benchmark: Mode Switching
// =============================================================================

fn bench_mode_switching(c: &mut Criterion) {
    let mut group = c.benchmark_group("perf_hud/mode_switch");

    log_jsonl(&[("group", "perf_hud/mode_switch"), ("event", "start")]);

    // Sparkline mode toggle
    group.bench_function("sparkline_mode_toggle", |b| {
        let mut hud = create_seeded_hud(60);

        b.iter(|| {
            hud.update(&press(KeyCode::Char('m')));
            black_box(&hud);
        })
    });

    // Pause toggle
    group.bench_function("pause_toggle", |b| {
        let mut hud = create_seeded_hud(60);

        b.iter(|| {
            hud.update(&press(KeyCode::Char('p')));
            black_box(&hud);
        })
    });

    log_jsonl(&[("group", "perf_hud/mode_switch"), ("event", "complete")]);
    group.finish();
}

// =============================================================================
// Benchmark: Dashboard Render
// =============================================================================

fn bench_dashboard_render(c: &mut Criterion) {
    let mut group = c.benchmark_group("dashboard/render");

    // 120x40 render benchmark
    group.throughput(Throughput::Elements(1));
    group.bench_function("render_120x40", |b| {
        let mut dashboard = Dashboard::new();
        let mut pool = GraphemePool::new();
        let mut tick = 0u64;

        b.iter(|| {
            tick = tick.wrapping_add(1);
            dashboard.tick(tick);
            let mut frame = Frame::new(120, 40, &mut pool);
            let area = Rect::new(0, 0, 120, 40);
            dashboard.view(black_box(&mut frame), black_box(area));
            black_box(&frame);
        })
    });

    // 80x24 render benchmark
    group.bench_function("render_80x24", |b| {
        let mut dashboard = Dashboard::new();
        let mut pool = GraphemePool::new();
        let mut tick = 0u64;

        b.iter(|| {
            tick = tick.wrapping_add(1);
            dashboard.tick(tick);
            let mut frame = Frame::new(80, 24, &mut pool);
            let area = Rect::new(0, 0, 80, 24);
            dashboard.view(black_box(&mut frame), black_box(area));
            black_box(&frame);
        })
    });

    group.finish();
}

// =============================================================================
// Criterion Configuration
// =============================================================================

criterion_group!(
    benches,
    bench_hud_render,
    bench_hud_overhead,
    bench_render_by_samples,
    bench_ring_buffer,
    bench_mode_switching,
    bench_dashboard_render,
);

criterion_main!(benches);
