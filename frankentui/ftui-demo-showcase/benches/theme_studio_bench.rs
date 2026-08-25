//! Benchmarks for Theme Studio screen (bd-vu0o.2)
//!
//! Performance regression tests for theme switching and palette operations.
//!
//! Run with: cargo bench -p ftui-demo-showcase --bench theme_studio_bench
//!
//! Performance budgets (per bd-vu0o.2):
//! - Theme cycle: < 10µs (no allocations on hot path)
//! - Preset apply: < 50µs
//! - View render (80x24): < 500µs
//! - View render (120x40): < 1ms
//! - Contrast ratio calculation: < 1µs per pair
//! - Export JSON: < 100µs

use criterion::{Criterion, Throughput, criterion_group, criterion_main};
use ftui_core::event::{Event, KeyCode, KeyEvent, KeyEventKind, Modifiers};
use ftui_core::geometry::Rect;
use ftui_demo_showcase::screens::Screen;
use ftui_demo_showcase::screens::theme_studio::ThemeStudioDemo;
use ftui_demo_showcase::theme::{self, ThemeId};
use ftui_render::cell::PackedRgba;
use ftui_render::frame::Frame;
use ftui_render::grapheme_pool::GraphemePool;
use std::hint::black_box;

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

fn ctrl_press(code: KeyCode) -> Event {
    Event::Key(KeyEvent {
        code,
        modifiers: Modifiers::CTRL,
        kind: KeyEventKind::Press,
    })
}

// =============================================================================
// Theme Cycling Benchmarks
// =============================================================================

fn bench_theme_cycle(c: &mut Criterion) {
    let mut group = c.benchmark_group("theme_studio/theme_cycle");

    // Reset to known state
    theme::set_theme(ThemeId::CyberpunkAurora);

    // Benchmark raw theme cycling (the core operation)
    group.bench_function("cycle_theme_raw", |b| {
        b.iter(|| {
            let next = black_box(theme::cycle_theme());
            black_box(next)
        })
    });

    // Benchmark theme switching via set_theme
    group.bench_function("set_theme", |b| {
        let themes = ThemeId::ALL;
        let mut idx = 0;
        b.iter(|| {
            theme::set_theme(themes[idx]);
            idx = (idx + 1) % themes.len();
            black_box(theme::current_theme())
        })
    });

    // Benchmark theme cycling through ThemeStudioDemo (includes state update)
    group.bench_function("cycle_via_ctrl_t", |b| {
        theme::set_theme(ThemeId::CyberpunkAurora);
        let mut demo = ThemeStudioDemo::new();
        b.iter(|| {
            demo.update(&ctrl_press(KeyCode::Char('t')));
            black_box(demo.preset_index)
        })
    });

    group.finish();
}

// =============================================================================
// Preset Application Benchmarks
// =============================================================================

fn bench_preset_apply(c: &mut Criterion) {
    let mut group = c.benchmark_group("theme_studio/preset_apply");

    // Navigate to preset and apply
    group.bench_function("navigate_and_apply", |b| {
        theme::set_theme(ThemeId::CyberpunkAurora);
        let mut demo = ThemeStudioDemo::new();
        b.iter(|| {
            // Navigate down
            demo.update(&press(KeyCode::Down));
            // Apply with Enter
            demo.update(&press(KeyCode::Enter));
            black_box(demo.preset_index)
        })
    });

    // Just the apply operation (Enter on current selection)
    group.bench_function("apply_enter_key", |b| {
        theme::set_theme(ThemeId::CyberpunkAurora);
        let mut demo = ThemeStudioDemo::new();
        b.iter(|| {
            demo.update(&press(KeyCode::Enter));
            black_box(demo.preset_index)
        })
    });

    group.finish();
}

// =============================================================================
// View Rendering Benchmarks
// =============================================================================

fn bench_view_render(c: &mut Criterion) {
    let mut group = c.benchmark_group("theme_studio/render");

    // 80x24 (standard terminal)
    group.bench_function("80x24", |b| {
        theme::set_theme(ThemeId::CyberpunkAurora);
        let demo = ThemeStudioDemo::new();
        let mut pool = GraphemePool::new();
        let area = Rect::new(0, 0, 80, 24);

        b.iter(|| {
            let mut frame = Frame::new(80, 24, &mut pool);
            demo.view(&mut frame, area);
            black_box(&frame);
        })
    });

    // 120x40 (larger terminal)
    group.bench_function("120x40", |b| {
        theme::set_theme(ThemeId::CyberpunkAurora);
        let demo = ThemeStudioDemo::new();
        let mut pool = GraphemePool::new();
        let area = Rect::new(0, 0, 120, 40);

        b.iter(|| {
            let mut frame = Frame::new(120, 40, &mut pool);
            demo.view(&mut frame, area);
            black_box(&frame);
        })
    });

    // 200x50 (wide terminal)
    group.bench_function("200x50", |b| {
        theme::set_theme(ThemeId::CyberpunkAurora);
        let demo = ThemeStudioDemo::new();
        let mut pool = GraphemePool::new();
        let area = Rect::new(0, 0, 200, 50);

        b.iter(|| {
            let mut frame = Frame::new(200, 50, &mut pool);
            demo.view(&mut frame, area);
            black_box(&frame);
        })
    });

    // 40x10 (tiny terminal)
    group.bench_function("40x10", |b| {
        theme::set_theme(ThemeId::CyberpunkAurora);
        let demo = ThemeStudioDemo::new();
        let mut pool = GraphemePool::new();
        let area = Rect::new(0, 0, 40, 10);

        b.iter(|| {
            let mut frame = Frame::new(40, 10, &mut pool);
            demo.view(&mut frame, area);
            black_box(&frame);
        })
    });

    group.finish();
}

// =============================================================================
// Contrast Ratio Calculation Benchmarks
// =============================================================================

fn bench_contrast_calculation(c: &mut Criterion) {
    let mut group = c.benchmark_group("theme_studio/contrast");

    // Single contrast calculation
    group.bench_function("single_pair", |b| {
        let fg = PackedRgba::rgb(255, 255, 255);
        let bg = PackedRgba::rgb(0, 0, 0);
        b.iter(|| {
            let ratio = ThemeStudioDemo::contrast_ratio(black_box(fg), black_box(bg));
            black_box(ratio)
        })
    });

    // Batch contrast calculations (simulating token list render)
    group.throughput(Throughput::Elements(20));
    group.bench_function("batch_20_pairs", |b| {
        let colors: Vec<(PackedRgba, PackedRgba)> = (0..20)
            .map(|i| {
                let v = (i * 12) as u8;
                (PackedRgba::rgb(v, v, v), PackedRgba::rgb(0, 0, 0))
            })
            .collect();

        b.iter(|| {
            let ratios: Vec<f32> = colors
                .iter()
                .map(|(fg, bg)| ThemeStudioDemo::contrast_ratio(*fg, *bg))
                .collect();
            black_box(ratios)
        })
    });

    // WCAG rating lookup
    group.bench_function("wcag_rating", |b| {
        b.iter(|| {
            let rating = ThemeStudioDemo::wcag_rating(black_box(4.5));
            black_box(rating)
        })
    });

    group.finish();
}

// =============================================================================
// Export Benchmarks
// =============================================================================

fn bench_export(c: &mut Criterion) {
    let mut group = c.benchmark_group("theme_studio/export");

    // JSON export
    group.bench_function("export_json", |b| {
        theme::set_theme(ThemeId::CyberpunkAurora);
        let demo = ThemeStudioDemo::new();
        b.iter(|| {
            let json = demo.export_json();
            black_box(json)
        })
    });

    // Color hex formatting
    group.throughput(Throughput::Elements(20));
    group.bench_function("color_hex_batch", |b| {
        let colors: Vec<PackedRgba> = (0..20)
            .map(|i| PackedRgba::rgb((i * 12) as u8, (i * 8) as u8, (i * 4) as u8))
            .collect();

        b.iter(|| {
            let hexes: Vec<String> = colors
                .iter()
                .map(|c| ThemeStudioDemo::color_hex(*c))
                .collect();
            black_box(hexes)
        })
    });

    group.finish();
}

// =============================================================================
// Navigation Benchmarks
// =============================================================================

fn bench_navigation(c: &mut Criterion) {
    let mut group = c.benchmark_group("theme_studio/navigation");

    // Panel toggle (Tab)
    group.bench_function("tab_toggle", |b| {
        let mut demo = ThemeStudioDemo::new();
        b.iter(|| {
            demo.update(&press(KeyCode::Tab));
            black_box(demo.focus)
        })
    });

    // Arrow navigation
    group.bench_function("arrow_navigation", |b| {
        let mut demo = ThemeStudioDemo::new();
        demo.preset_index = 2; // Start in middle
        b.iter(|| {
            demo.update(&press(KeyCode::Down));
            demo.update(&press(KeyCode::Up));
            black_box(demo.preset_index)
        })
    });

    // Vim navigation
    group.bench_function("vim_navigation", |b| {
        let mut demo = ThemeStudioDemo::new();
        demo.preset_index = 2;
        b.iter(|| {
            demo.update(&press(KeyCode::Char('j')));
            demo.update(&press(KeyCode::Char('k')));
            black_box(demo.preset_index)
        })
    });

    group.finish();
}

// =============================================================================
// Initialization Benchmarks
// =============================================================================

fn bench_initialization(c: &mut Criterion) {
    let mut group = c.benchmark_group("theme_studio/init");

    // ThemeStudioDemo::new()
    group.bench_function("new", |b| {
        theme::set_theme(ThemeId::CyberpunkAurora);
        b.iter(|| {
            let demo = ThemeStudioDemo::new();
            black_box(demo)
        })
    });

    // Token list building is included in new(), so we just measure initialization
    // and access a public field to verify the demo was constructed
    group.bench_function("new_with_tokens", |b| {
        b.iter(|| {
            let demo = ThemeStudioDemo::new();
            // Access public field to confirm tokens were built
            black_box(demo.tick_count)
        })
    });

    group.finish();
}

// =============================================================================
// Criterion Configuration
// =============================================================================

criterion_group!(
    benches,
    bench_theme_cycle,
    bench_preset_apply,
    bench_view_render,
    bench_contrast_calculation,
    bench_export,
    bench_navigation,
    bench_initialization,
);

criterion_main!(benches);
