//! Benchmarks for Virtualized Search screen (bd-2zbk, bd-2zbk.2)
//!
//! Performance Regression Tests for large list with fuzzy search.
//!
//! Run with: cargo bench -p ftui-demo-showcase --bench virtualized_search_bench
//!
//! Performance budgets (per bd-2zbk):
//! - Render 10k items (visible only): < 2ms at 120x40
//! - Navigation (j/k): < 50µs per operation
//! - Page navigation (PgUp/Dn): < 100µs per operation
//! - Filter update (10k items): < 10ms
//! - Fuzzy match (single item): < 10µs

use criterion::{BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use ftui_core::event::{Event, KeyCode, KeyEvent, KeyEventKind, Modifiers};
use ftui_core::geometry::Rect;
use ftui_demo_showcase::screens::{Screen, virtualized_search::VirtualizedSearch};
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

// =============================================================================
// Render Benchmarks: Various Terminal Sizes
// =============================================================================

fn bench_virtualized_search_render(c: &mut Criterion) {
    let mut group = c.benchmark_group("virtualized_search/render");

    // Standard terminal (80x24)
    group.bench_function("initial_80x24", |b| {
        let screen = VirtualizedSearch::new();
        let mut pool = GraphemePool::new();
        let area = Rect::new(0, 0, 80, 24);

        b.iter(|| {
            let mut frame = Frame::new(80, 24, &mut pool);
            screen.view(&mut frame, area);
            black_box(&frame);
        })
    });

    // Large terminal (120x40)
    group.bench_function("initial_120x40", |b| {
        let screen = VirtualizedSearch::new();
        let mut pool = GraphemePool::new();
        let area = Rect::new(0, 0, 120, 40);

        b.iter(|| {
            let mut frame = Frame::new(120, 40, &mut pool);
            screen.view(&mut frame, area);
            black_box(&frame);
        })
    });

    // Very large terminal (200x60)
    group.bench_function("initial_200x60", |b| {
        let screen = VirtualizedSearch::new();
        let mut pool = GraphemePool::new();
        let area = Rect::new(0, 0, 200, 60);

        b.iter(|| {
            let mut frame = Frame::new(200, 60, &mut pool);
            screen.view(&mut frame, area);
            black_box(&frame);
        })
    });

    group.finish();
}

// =============================================================================
// Terminal Size Scaling Benchmarks
// =============================================================================

fn bench_virtualized_search_sizes(c: &mut Criterion) {
    let mut group = c.benchmark_group("virtualized_search/terminal_size");

    for (w, h) in [(80, 24), (120, 40), (200, 60), (320, 80)] {
        let cells = w as u64 * h as u64;
        group.throughput(Throughput::Elements(cells));

        group.bench_with_input(
            BenchmarkId::new("render_10k_items", format!("{w}x{h}")),
            &(w, h),
            |b, &(w, h)| {
                let screen = VirtualizedSearch::new();
                let mut pool = GraphemePool::new();
                let area = Rect::new(0, 0, w, h);

                b.iter(|| {
                    let mut frame = Frame::new(w, h, &mut pool);
                    screen.view(&mut frame, area);
                    black_box(&frame);
                })
            },
        );
    }

    group.finish();
}

// =============================================================================
// Navigation Benchmarks
// =============================================================================

fn bench_virtualized_search_navigation(c: &mut Criterion) {
    let mut group = c.benchmark_group("virtualized_search/navigation");

    // Single step down (j key)
    group.bench_function("step_down_j", |b| {
        let mut screen = VirtualizedSearch::new();

        b.iter(|| {
            screen.update(&press(KeyCode::Char('j')));
            black_box(&screen);
        })
    });

    // Single step up (k key)
    group.bench_function("step_up_k", |b| {
        let mut screen = VirtualizedSearch::new();
        // Move down first so we can move up
        for _ in 0..100 {
            screen.update(&press(KeyCode::Char('j')));
        }

        b.iter(|| {
            screen.update(&press(KeyCode::Char('k')));
            black_box(&screen);
        })
    });

    // Page down
    group.bench_function("page_down", |b| {
        let mut screen = VirtualizedSearch::new();

        b.iter(|| {
            screen.update(&press(KeyCode::PageDown));
            black_box(&screen);
        })
    });

    // Jump to end (G)
    group.bench_function("jump_to_end", |b| {
        let mut screen = VirtualizedSearch::new();

        b.iter(|| {
            screen.update(&press(KeyCode::End));
            black_box(&screen);
        })
    });

    // Navigation + render (simulates one frame)
    group.bench_function("step_and_render_120x40", |b| {
        let mut screen = VirtualizedSearch::new();
        let mut pool = GraphemePool::new();
        let area = Rect::new(0, 0, 120, 40);

        b.iter(|| {
            screen.update(&press(KeyCode::Char('j')));
            let mut frame = Frame::new(120, 40, &mut pool);
            screen.view(&mut frame, area);
            black_box(&frame);
        })
    });

    group.finish();
}

// =============================================================================
// Filter/Search Benchmarks
// =============================================================================

fn bench_virtualized_search_filter(c: &mut Criterion) {
    let mut group = c.benchmark_group("virtualized_search/filter");

    // Type single character (incremental filter)
    group.bench_function("type_single_char", |b| {
        let mut screen = VirtualizedSearch::new();
        let chars: Vec<char> = "abcdefghijklmnopqrstuvwxyz".chars().collect();
        let mut idx = 0;

        b.iter(|| {
            screen.update(&press(KeyCode::Char(chars[idx % chars.len()])));
            idx += 1;
            black_box(&screen);
        })
    });

    // Type full query "CoreService"
    group.bench_function("type_full_query", |b| {
        b.iter(|| {
            let mut screen = VirtualizedSearch::new();
            for c in "CoreService".chars() {
                screen.update(&press(KeyCode::Char(c)));
            }
            black_box(&screen);
        })
    });

    // Filter + render
    group.bench_function("filter_and_render_120x40", |b| {
        let mut pool = GraphemePool::new();
        let area = Rect::new(0, 0, 120, 40);

        b.iter(|| {
            let mut screen = VirtualizedSearch::new();
            for c in "init".chars() {
                screen.update(&press(KeyCode::Char(c)));
            }
            let mut frame = Frame::new(120, 40, &mut pool);
            screen.view(&mut frame, area);
            black_box(&frame);
        })
    });

    // Clear search (Escape)
    group.bench_function("clear_search", |b| {
        let mut screen = VirtualizedSearch::new();
        // Set up query first
        for c in "CoreService".chars() {
            screen.update(&press(KeyCode::Char(c)));
        }

        b.iter(|| {
            // Clear and re-populate
            screen.update(&press(KeyCode::Escape));
            for c in "init".chars() {
                screen.update(&press(KeyCode::Char(c)));
            }
            black_box(&screen);
        })
    });

    group.finish();
}

// =============================================================================
// Scroll Position Benchmarks
// =============================================================================

fn bench_virtualized_search_scroll(c: &mut Criterion) {
    let mut group = c.benchmark_group("virtualized_search/scroll");

    // Render at various scroll positions
    for scroll_pct in [0, 25, 50, 75, 100] {
        group.bench_with_input(
            BenchmarkId::new("render_at_scroll", format!("{scroll_pct}%")),
            &scroll_pct,
            |b, &scroll_pct| {
                let mut screen = VirtualizedSearch::new();
                // Navigate to target position
                let target = (10_000 * scroll_pct) / 100;
                for _ in 0..target {
                    screen.update(&press(KeyCode::Char('j')));
                }

                let mut pool = GraphemePool::new();
                let area = Rect::new(0, 0, 120, 40);

                b.iter(|| {
                    let mut frame = Frame::new(120, 40, &mut pool);
                    screen.view(&mut frame, area);
                    black_box(&frame);
                })
            },
        );
    }

    group.finish();
}

// =============================================================================
// Stress Tests
// =============================================================================

fn bench_virtualized_search_stress(c: &mut Criterion) {
    let mut group = c.benchmark_group("virtualized_search/stress");
    group.sample_size(50); // Reduce samples for stress tests

    // Rapid navigation (100 steps)
    group.bench_function("rapid_navigation_100", |b| {
        let mut screen = VirtualizedSearch::new();

        b.iter(|| {
            for _ in 0..100 {
                screen.update(&press(KeyCode::Char('j')));
            }
            black_box(&screen);
        })
    });

    // Full workflow: navigate + search + render
    group.bench_function("full_workflow", |b| {
        let mut pool = GraphemePool::new();
        let area = Rect::new(0, 0, 120, 40);

        b.iter(|| {
            let mut screen = VirtualizedSearch::new();
            // Navigate down
            for _ in 0..20 {
                screen.update(&press(KeyCode::Char('j')));
            }
            // Search
            for c in "Database".chars() {
                screen.update(&press(KeyCode::Char(c)));
            }
            // Render
            let mut frame = Frame::new(120, 40, &mut pool);
            screen.view(&mut frame, area);
            black_box(&frame);
        })
    });

    group.finish();
}

// =============================================================================
// Benchmark Groups
// =============================================================================

criterion_group!(
    benches,
    bench_virtualized_search_render,
    bench_virtualized_search_sizes,
    bench_virtualized_search_navigation,
    bench_virtualized_search_filter,
    bench_virtualized_search_scroll,
    bench_virtualized_search_stress,
);

criterion_main!(benches);
