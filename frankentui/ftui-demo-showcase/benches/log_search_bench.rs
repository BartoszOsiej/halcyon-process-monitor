//! Benchmarks for Log Search Performance (bd-1b5h.5)
//!
//! Performance/Latency Regression Tests for LogViewer search operations.
//!
//! Run with: `cargo bench -p ftui-demo-showcase --bench log_search_bench`
//!
//! # Dataset Sizes
//! - 1k lines: Baseline small dataset
//! - 10k lines: Medium load
//! - 50k lines: Stress test
//!
//! # Performance Budgets
//! - Search query latency: < 10ms at 10k lines
//! - Highlight span generation: < 5ms at 10k lines
//! - Filter application: < 15ms at 10k lines
//! - No >2x regression vs baseline
//!
//! # JSONL Logging
//! Results are written to `target/criterion/log_search_bench/` with:
//! - Raw timing data per iteration
//! - Statistical analysis (mean, stddev, p50/p95/p99)
//! - Comparison to previous runs

use criterion::{BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use ftui_core::event::{Event, KeyCode, KeyEvent, KeyEventKind, Modifiers};
use ftui_core::geometry::Rect;
use ftui_demo_showcase::screens::{Screen, log_search::LogSearch};
use ftui_render::frame::Frame;
use ftui_render::grapheme_pool::GraphemePool;
use ftui_style::Style;
use ftui_text::Text;
use ftui_widgets::log_viewer::{LogViewer, LogViewerState, LogWrapMode, SearchConfig, SearchMode};
use std::hint::black_box;

// =============================================================================
// Test Data Generation
// =============================================================================

/// Generate a single log line matching the LogSearch screen format.
fn generate_log_line(seq: u64) -> Text<'static> {
    let severity_label = match seq % 13 {
        0..=5 => "INFO",
        6..=8 => "DEBUG",
        9..=10 => "WARN",
        11 => "ERROR",
        _ => "TRACE",
    };

    let module = match seq % 9 {
        0 => "server::http",
        1 => "db::pool",
        2 => "auth::jwt",
        3 => "cache::redis",
        4 => "queue::worker",
        5 => "api::handler",
        6 => "core::runtime",
        7 => "metrics::push",
        _ => "config::reload",
    };

    let message = match seq % 11 {
        0 => "Request processed successfully",
        1 => "Connection pool health check passed",
        2 => "Token refresh completed for session",
        3 => "Cache hit ratio: 0.94",
        4 => "Worker picked up job from queue",
        5 => "Rate limit threshold approaching",
        6 => "Garbage collection cycle completed",
        7 => "Metric batch flushed to backend",
        8 => "Configuration hot-reload triggered",
        9 => "Retry attempt 2/3 for downstream call",
        _ => "Scheduled maintenance window check",
    };

    let line = format!(
        "[{:>6}] {:>5} {:<18} {}",
        seq, severity_label, module, message
    );

    Text::styled(line, Style::default())
}

/// Create a LogViewer pre-populated with a given number of lines.
fn create_log_viewer(line_count: usize) -> LogViewer {
    let mut viewer = LogViewer::new(line_count.max(10_000)).wrap_mode(LogWrapMode::NoWrap);

    for i in 0..line_count {
        viewer.push(generate_log_line(i as u64));
    }

    viewer
}

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

fn type_chars(screen: &mut LogSearch, text: &str) {
    for ch in text.chars() {
        screen.update(&press(KeyCode::Char(ch)));
    }
}

// =============================================================================
// Search Query Latency Benchmarks
// =============================================================================

fn bench_search_query_latency(c: &mut Criterion) {
    let mut group = c.benchmark_group("log_search/query_latency");

    for line_count in [1_000, 10_000, 50_000] {
        group.throughput(Throughput::Elements(line_count as u64));

        // Common pattern (many matches)
        group.bench_with_input(
            BenchmarkId::new("common_pattern", line_count),
            &line_count,
            |b, &count| {
                let mut viewer = create_log_viewer(count);
                b.iter(|| {
                    viewer.search("INFO");
                    black_box(&viewer);
                });
            },
        );

        // Rare pattern (few matches)
        group.bench_with_input(
            BenchmarkId::new("rare_pattern", line_count),
            &line_count,
            |b, &count| {
                let mut viewer = create_log_viewer(count);
                b.iter(|| {
                    viewer.search("ERROR");
                    black_box(&viewer);
                });
            },
        );

        // No match
        group.bench_with_input(
            BenchmarkId::new("no_match", line_count),
            &line_count,
            |b, &count| {
                let mut viewer = create_log_viewer(count);
                b.iter(|| {
                    viewer.search("xyzzynotfound");
                    black_box(&viewer);
                });
            },
        );

        // Case-insensitive search
        group.bench_with_input(
            BenchmarkId::new("case_insensitive", line_count),
            &line_count,
            |b, &count| {
                let mut viewer = create_log_viewer(count);
                let config = SearchConfig {
                    mode: SearchMode::Literal,
                    case_sensitive: false,
                    context_lines: 0,
                };
                b.iter(|| {
                    viewer.search_with_config("error", config.clone());
                    black_box(&viewer);
                });
            },
        );
    }

    group.finish();
}

// =============================================================================
// Highlight Span Generation Benchmarks
// =============================================================================

fn bench_highlight_generation(c: &mut Criterion) {
    let mut group = c.benchmark_group("log_search/highlight_spans");

    for line_count in [1_000, 10_000, 50_000] {
        group.throughput(Throughput::Elements(line_count as u64));

        // Search + render (measures highlight span application)
        group.bench_with_input(
            BenchmarkId::new("search_and_render", line_count),
            &line_count,
            |b, &count| {
                let mut viewer = create_log_viewer(count);
                viewer.search("pool");
                let mut state = LogViewerState::default();
                let mut pool = GraphemePool::new();
                let area = Rect::new(0, 0, 120, 40);

                b.iter(|| {
                    let mut frame = Frame::new(120, 40, &mut pool);
                    ftui_widgets::StatefulWidget::render(&viewer, area, &mut frame, &mut state);
                    black_box(&frame);
                });
            },
        );

        // Multiple matches per line
        group.bench_with_input(
            BenchmarkId::new("multi_match_render", line_count),
            &line_count,
            |b, &count| {
                let mut viewer = create_log_viewer(count);
                // Search for common substring that appears in many positions
                viewer.search("e");
                let mut state = LogViewerState::default();
                let mut pool = GraphemePool::new();
                let area = Rect::new(0, 0, 120, 40);

                b.iter(|| {
                    let mut frame = Frame::new(120, 40, &mut pool);
                    ftui_widgets::StatefulWidget::render(&viewer, area, &mut frame, &mut state);
                    black_box(&frame);
                });
            },
        );
    }

    group.finish();
}

// =============================================================================
// Filter Benchmarks
// =============================================================================

fn bench_filter_application(c: &mut Criterion) {
    let mut group = c.benchmark_group("log_search/filter");

    for line_count in [1_000, 10_000, 50_000] {
        group.throughput(Throughput::Elements(line_count as u64));

        // Apply filter
        group.bench_with_input(
            BenchmarkId::new("apply_filter", line_count),
            &line_count,
            |b, &count| {
                let mut viewer = create_log_viewer(count);
                b.iter(|| {
                    viewer.set_filter(Some("ERROR"));
                    black_box(&viewer);
                });
            },
        );

        // Clear filter
        group.bench_with_input(
            BenchmarkId::new("clear_filter", line_count),
            &line_count,
            |b, &count| {
                let mut viewer = create_log_viewer(count);
                viewer.set_filter(Some("ERROR"));
                b.iter(|| {
                    viewer.set_filter(None);
                    black_box(&viewer);
                });
            },
        );

        // Filter + render
        group.bench_with_input(
            BenchmarkId::new("filter_and_render", line_count),
            &line_count,
            |b, &count| {
                let mut viewer = create_log_viewer(count);
                viewer.set_filter(Some("WARN"));
                let mut state = LogViewerState::default();
                let mut pool = GraphemePool::new();
                let area = Rect::new(0, 0, 120, 40);

                b.iter(|| {
                    let mut frame = Frame::new(120, 40, &mut pool);
                    ftui_widgets::StatefulWidget::render(&viewer, area, &mut frame, &mut state);
                    black_box(&frame);
                });
            },
        );
    }

    group.finish();
}

// =============================================================================
// Match Navigation Benchmarks
// =============================================================================

fn bench_match_navigation(c: &mut Criterion) {
    let mut group = c.benchmark_group("log_search/navigation");

    for line_count in [1_000, 10_000, 50_000] {
        // Next match navigation
        group.bench_with_input(
            BenchmarkId::new("next_match", line_count),
            &line_count,
            |b, &count| {
                let mut viewer = create_log_viewer(count);
                viewer.search("pool");

                b.iter(|| {
                    viewer.next_match();
                    black_box(&viewer);
                });
            },
        );

        // Previous match navigation
        group.bench_with_input(
            BenchmarkId::new("prev_match", line_count),
            &line_count,
            |b, &count| {
                let mut viewer = create_log_viewer(count);
                viewer.search("pool");
                // Move to middle first
                for _ in 0..50 {
                    viewer.next_match();
                }

                b.iter(|| {
                    viewer.prev_match();
                    black_box(&viewer);
                });
            },
        );
    }

    group.finish();
}

// =============================================================================
// LogSearch Screen Benchmarks (Integration)
// =============================================================================

fn bench_log_search_screen(c: &mut Criterion) {
    let mut group = c.benchmark_group("log_search/screen");

    // Initial render
    group.bench_function("initial_render_120x40", |b| {
        let screen = LogSearch::new();
        let mut pool = GraphemePool::new();
        let area = Rect::new(0, 0, 120, 40);

        b.iter(|| {
            let mut frame = Frame::new(120, 40, &mut pool);
            screen.view(&mut frame, area);
            black_box(&frame);
        });
    });

    // Enter search mode
    group.bench_function("enter_search_mode", |b| {
        let mut screen = LogSearch::new();
        b.iter(|| {
            screen.update(&press(KeyCode::Char('/')));
            black_box(&screen);
        });
    });

    // Type and search
    group.bench_function("type_and_search", |b| {
        b.iter(|| {
            let mut screen = LogSearch::new();
            screen.update(&press(KeyCode::Char('/')));
            type_chars(&mut screen, "ERROR");
            screen.update(&press(KeyCode::Enter));
            black_box(&screen);
        });
    });

    // Full search workflow + render
    group.bench_function("search_workflow_render", |b| {
        let mut pool = GraphemePool::new();
        let area = Rect::new(0, 0, 120, 40);

        b.iter(|| {
            let mut screen = LogSearch::new();
            screen.update(&press(KeyCode::Char('/')));
            type_chars(&mut screen, "WARN");
            screen.update(&press(KeyCode::Enter));
            // Navigate matches
            screen.update(&press(KeyCode::Char('n')));
            screen.update(&press(KeyCode::Char('n')));
            // Render
            let mut frame = Frame::new(120, 40, &mut pool);
            screen.view(&mut frame, area);
            black_box(&frame);
        });
    });

    group.finish();
}

// =============================================================================
// Streaming Append Benchmarks
// =============================================================================

fn bench_streaming_append(c: &mut Criterion) {
    let mut group = c.benchmark_group("log_search/streaming");

    // Append to log with active search
    group.bench_function("append_with_search_10k", |b| {
        let mut viewer = create_log_viewer(10_000);
        viewer.search("pool");
        let mut seq = 10_000u64;

        b.iter(|| {
            viewer.push(generate_log_line(seq));
            seq += 1;
            black_box(&viewer);
        });
    });

    // Append to log with active filter
    group.bench_function("append_with_filter_10k", |b| {
        let mut viewer = create_log_viewer(10_000);
        viewer.set_filter(Some("INFO"));
        let mut seq = 10_000u64;

        b.iter(|| {
            viewer.push(generate_log_line(seq));
            seq += 1;
            black_box(&viewer);
        });
    });

    // Append without search/filter (baseline)
    group.bench_function("append_baseline_10k", |b| {
        let mut viewer = create_log_viewer(10_000);
        let mut seq = 10_000u64;

        b.iter(|| {
            viewer.push(generate_log_line(seq));
            seq += 1;
            black_box(&viewer);
        });
    });

    group.finish();
}

// =============================================================================
// Context Lines Benchmarks
// =============================================================================

fn bench_context_lines(c: &mut Criterion) {
    let mut group = c.benchmark_group("log_search/context");

    for context_lines in [0, 1, 2, 5] {
        group.bench_with_input(
            BenchmarkId::new("search_with_context", context_lines),
            &context_lines,
            |b, &ctx| {
                let mut viewer = create_log_viewer(10_000);
                let config = SearchConfig {
                    mode: SearchMode::Literal,
                    case_sensitive: true,
                    context_lines: ctx,
                };

                b.iter(|| {
                    viewer.search_with_config("ERROR", config.clone());
                    black_box(&viewer);
                });
            },
        );
    }

    group.finish();
}

// =============================================================================
// Regression Budget Assertions
// =============================================================================

/// Compile-time budget documentation (enforced by CI comparison)
///
/// Performance budgets (no >2x regression):
/// - 1k lines search: < 1ms
/// - 10k lines search: < 10ms
/// - 50k lines search: < 50ms
/// - Highlight render (10k): < 5ms
/// - Filter apply (10k): < 15ms
/// - Match navigation: < 1ms
///
/// See `target/criterion/log_search_bench/` for historical comparison.
#[cfg(test)]
mod regression_tests {
    use super::{Frame, GraphemePool, LogViewerState, Rect};

    /// Sanity check that search completes in reasonable time
    #[test]
    fn search_completes_under_100ms() {
        let mut viewer = super::create_log_viewer(50_000);
        let start = std::time::Instant::now();
        viewer.search("ERROR");
        let elapsed = start.elapsed();
        assert!(
            elapsed.as_millis() < 100,
            "Search took {}ms, budget is 100ms",
            elapsed.as_millis()
        );
    }

    /// Sanity check that filter completes in reasonable time
    #[test]
    fn filter_completes_under_100ms() {
        let mut viewer = super::create_log_viewer(50_000);
        let start = std::time::Instant::now();
        viewer.set_filter(Some("INFO"));
        let elapsed = start.elapsed();
        assert!(
            elapsed.as_millis() < 100,
            "Filter took {}ms, budget is 100ms",
            elapsed.as_millis()
        );
    }

    /// Render with search highlights completes quickly
    #[test]
    fn render_with_highlights_under_50ms() {
        let mut viewer = super::create_log_viewer(10_000);
        viewer.search("pool");
        let mut state = LogViewerState::default();
        let mut pool = GraphemePool::new();
        let area = Rect::new(0, 0, 120, 40);

        let start = std::time::Instant::now();
        for _ in 0..10 {
            let mut frame = Frame::new(120, 40, &mut pool);
            ftui_widgets::StatefulWidget::render(&viewer, area, &mut frame, &mut state);
        }
        let elapsed = start.elapsed();
        let per_render = elapsed.as_micros() / 10;
        assert!(
            per_render < 5_000,
            "Render took {}µs, budget is 5000µs",
            per_render
        );
    }
}

// =============================================================================
// Criterion Configuration
// =============================================================================

criterion_group!(
    benches,
    bench_search_query_latency,
    bench_highlight_generation,
    bench_filter_application,
    bench_match_navigation,
    bench_log_search_screen,
    bench_streaming_append,
    bench_context_lines,
);

criterion_main!(benches);
