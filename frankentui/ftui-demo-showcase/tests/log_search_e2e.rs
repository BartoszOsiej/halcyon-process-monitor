#![forbid(unsafe_code)]

//! End-to-end tests for the Log Search demo (bd-1b5h.10).
//!
//! Exercises keyboard-only interactions:
//! - Open search bar, type query, submit
//! - Toggle case sensitivity + context lines
//! - Navigate matches
//! - Open filter bar, apply filter, clear filter
//! - Pause/resume log stream
//!
//! Run: `cargo test -p ftui-demo-showcase --test log_search_e2e`

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::sync::OnceLock;
use std::time::Instant;

use ftui_core::event::{Event, KeyCode, KeyEvent, KeyEventKind, Modifiers};
use ftui_demo_showcase::app::{AppModel, AppMsg, ScreenId};
use ftui_harness::determinism::{JsonValue, TestJsonlLogger};
use ftui_render::frame::Frame;
use ftui_render::grapheme_pool::GraphemePool;
use ftui_runtime::Model;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

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

fn type_chars(app: &mut AppModel, text: &str) {
    for ch in text.chars() {
        app.update(AppMsg::ScreenEvent(press(KeyCode::Char(ch))));
    }
}

fn capture_frame_hash(app: &mut AppModel, width: u16, height: u16) -> u64 {
    let mut pool = GraphemePool::new();
    let mut frame = Frame::new(width, height, &mut pool);
    app.view(&mut frame);
    let mut hasher = DefaultHasher::new();
    for y in 0..height {
        for x in 0..width {
            if let Some(cell) = frame.buffer.get(x, y)
                && let Some(ch) = cell.content.as_char()
            {
                ch.hash(&mut hasher);
            }
        }
    }
    hasher.finish()
}

fn log_jsonl(event: &str, data: &[(&str, JsonValue)]) {
    logger().log(event, data);
}

fn logger() -> &'static TestJsonlLogger {
    static LOGGER: OnceLock<TestJsonlLogger> = OnceLock::new();
    LOGGER.get_or_init(|| {
        let mut logger = TestJsonlLogger::new("log_search_e2e", 42);
        logger.add_context_str("suite", "log_search_e2e");
        logger
    })
}

// ===========================================================================
// Scenario: Keyboard-only Log Search interaction
// ===========================================================================

#[test]
fn e2e_keyboard_only_log_search() {
    let start = Instant::now();

    log_jsonl(
        "env",
        &[
            ("test", JsonValue::str("e2e_keyboard_only_log_search")),
            ("term_cols", JsonValue::u64(120)),
            ("term_rows", JsonValue::u64(40)),
        ],
    );

    let mut app = AppModel::new();
    app.update(AppMsg::Resize {
        width: 120,
        height: 40,
    });

    // Navigate to LogSearch screen (setup only).
    app.current_screen = ScreenId::LogSearch;
    assert_eq!(app.current_screen, ScreenId::LogSearch);

    // Open search bar and type query.
    log_jsonl("step", &[("action", JsonValue::str("open_search"))]);
    app.update(AppMsg::ScreenEvent(press(KeyCode::Char('/'))));
    type_chars(&mut app, "ERROR");

    // Toggle case sensitivity + context lines.
    log_jsonl("step", &[("action", JsonValue::str("toggle_search_opts"))]);
    app.update(AppMsg::ScreenEvent(ctrl_press(KeyCode::Char('c'))));
    app.update(AppMsg::ScreenEvent(ctrl_press(KeyCode::Char('x'))));

    // Submit search and navigate matches.
    log_jsonl("step", &[("action", JsonValue::str("submit_search"))]);
    app.update(AppMsg::ScreenEvent(press(KeyCode::Enter)));
    app.update(AppMsg::ScreenEvent(press(KeyCode::Char('n'))));
    app.update(AppMsg::ScreenEvent(press(KeyCode::Char('N'))));

    let search_hash = capture_frame_hash(&mut app, 120, 40);
    log_jsonl(
        "search_view",
        &[("frame_hash", JsonValue::str(format!("{search_hash:016x}")))],
    );

    // Open filter bar and apply filter.
    log_jsonl("step", &[("action", JsonValue::str("open_filter"))]);
    app.update(AppMsg::ScreenEvent(press(KeyCode::Char('f'))));
    type_chars(&mut app, "WARN");
    app.update(AppMsg::ScreenEvent(press(KeyCode::Enter)));

    let filter_hash = capture_frame_hash(&mut app, 120, 40);
    log_jsonl(
        "filter_view",
        &[("frame_hash", JsonValue::str(format!("{filter_hash:016x}")))],
    );

    // Clear filter and pause/resume stream.
    log_jsonl("step", &[("action", JsonValue::str("clear_filter_pause"))]);
    app.update(AppMsg::ScreenEvent(press(KeyCode::Char('F'))));
    app.update(AppMsg::ScreenEvent(press(KeyCode::Char(' '))));
    app.update(AppMsg::ScreenEvent(press(KeyCode::Char(' '))));

    let final_hash = capture_frame_hash(&mut app, 120, 40);
    let elapsed = start.elapsed();
    log_jsonl(
        "completed",
        &[
            ("elapsed_us", JsonValue::u64(elapsed.as_micros() as u64)),
            ("frame_hash", JsonValue::str(format!("{final_hash:016x}"))),
        ],
    );
}

// ===========================================================================
// Performance Regression Tests (bd-1b5h.5)
// ===========================================================================

/// Tests that search operations complete within performance budgets.
/// Budget: < 100ms for full workflow at default log size.
#[test]
fn e2e_search_performance_budget() {
    log_jsonl(
        "env",
        &[
            ("test", JsonValue::str("e2e_search_performance_budget")),
            ("term_cols", JsonValue::u64(120)),
            ("term_rows", JsonValue::u64(40)),
        ],
    );

    let mut app = AppModel::new();
    app.update(AppMsg::Resize {
        width: 120,
        height: 40,
    });
    app.current_screen = ScreenId::LogSearch;

    // Measure search workflow latency
    let start = Instant::now();

    app.update(AppMsg::ScreenEvent(press(KeyCode::Char('/'))));
    type_chars(&mut app, "ERROR");
    app.update(AppMsg::ScreenEvent(press(KeyCode::Enter)));

    // Navigate through matches
    for _ in 0..10 {
        app.update(AppMsg::ScreenEvent(press(KeyCode::Char('n'))));
    }

    // Render frame
    let _hash = capture_frame_hash(&mut app, 120, 40);

    let elapsed = start.elapsed();
    let elapsed_us = elapsed.as_micros();

    log_jsonl(
        "perf_result",
        &[
            ("elapsed_us", JsonValue::u64(elapsed_us as u64)),
            ("budget_us", JsonValue::u64(100_000)),
            ("pass", JsonValue::bool(elapsed_us < 100_000)),
        ],
    );

    assert!(
        elapsed_us < 100_000,
        "Search workflow took {}µs, budget is 100000µs",
        elapsed_us
    );
}

/// Tests filter application latency.
/// Budget: < 50ms for filter apply + render.
#[test]
fn e2e_filter_performance_budget() {
    log_jsonl(
        "env",
        &[
            ("test", JsonValue::str("e2e_filter_performance_budget")),
            ("term_cols", JsonValue::u64(120)),
            ("term_rows", JsonValue::u64(40)),
        ],
    );

    let mut app = AppModel::new();
    app.update(AppMsg::Resize {
        width: 120,
        height: 40,
    });
    app.current_screen = ScreenId::LogSearch;

    // Measure filter workflow latency
    let start = Instant::now();

    app.update(AppMsg::ScreenEvent(press(KeyCode::Char('f'))));
    type_chars(&mut app, "INFO");
    app.update(AppMsg::ScreenEvent(press(KeyCode::Enter)));

    // Render frame with filter active
    let _hash = capture_frame_hash(&mut app, 120, 40);

    let elapsed = start.elapsed();
    let elapsed_us = elapsed.as_micros();

    log_jsonl(
        "perf_result",
        &[
            ("elapsed_us", JsonValue::u64(elapsed_us as u64)),
            ("budget_us", JsonValue::u64(50_000)),
            ("pass", JsonValue::bool(elapsed_us < 50_000)),
        ],
    );

    assert!(
        elapsed_us < 50_000,
        "Filter workflow took {}µs, budget is 50000µs",
        elapsed_us
    );
}

/// Tests render latency with active search highlights.
/// Budget: < 10ms per render at 120x40.
#[test]
fn e2e_render_with_highlights_budget() {
    log_jsonl(
        "env",
        &[
            ("test", JsonValue::str("e2e_render_with_highlights_budget")),
            ("term_cols", JsonValue::u64(120)),
            ("term_rows", JsonValue::u64(40)),
        ],
    );

    let mut app = AppModel::new();
    app.update(AppMsg::Resize {
        width: 120,
        height: 40,
    });
    app.current_screen = ScreenId::LogSearch;

    // Set up search
    app.update(AppMsg::ScreenEvent(press(KeyCode::Char('/'))));
    type_chars(&mut app, "pool");
    app.update(AppMsg::ScreenEvent(press(KeyCode::Enter)));

    // Measure repeated render latency (10 renders)
    let start = Instant::now();
    for _ in 0..10 {
        let _hash = capture_frame_hash(&mut app, 120, 40);
    }
    let elapsed = start.elapsed();
    let per_render_us = elapsed.as_micros() / 10;

    log_jsonl(
        "perf_result",
        &[
            ("total_us", JsonValue::u64(elapsed.as_micros() as u64)),
            ("per_render_us", JsonValue::u64(per_render_us as u64)),
            ("budget_us", JsonValue::u64(10_000)),
            ("pass", JsonValue::bool(per_render_us < 10_000)),
        ],
    );

    assert!(
        per_render_us < 10_000,
        "Render with highlights took {}µs, budget is 10000µs",
        per_render_us
    );
}

/// Tests log streaming with active search.
/// Verifies that log append + search maintenance doesn't cause lag.
#[test]
fn e2e_streaming_with_search_latency() {
    log_jsonl(
        "env",
        &[
            ("test", JsonValue::str("e2e_streaming_with_search_latency")),
            ("term_cols", JsonValue::u64(120)),
            ("term_rows", JsonValue::u64(40)),
        ],
    );

    let mut app = AppModel::new();
    app.update(AppMsg::Resize {
        width: 120,
        height: 40,
    });
    app.current_screen = ScreenId::LogSearch;

    // Set up search
    app.update(AppMsg::ScreenEvent(press(KeyCode::Char('/'))));
    type_chars(&mut app, "ERROR");
    app.update(AppMsg::ScreenEvent(press(KeyCode::Enter)));

    // Simulate streaming with ticks
    let start = Instant::now();
    for tick in 0..100 {
        app.update(AppMsg::Tick);
        if tick % 10 == 0 {
            let _hash = capture_frame_hash(&mut app, 120, 40);
        }
    }
    let elapsed = start.elapsed();

    log_jsonl(
        "perf_result",
        &[
            ("ticks", JsonValue::u64(100)),
            ("elapsed_us", JsonValue::u64(elapsed.as_micros() as u64)),
            (
                "avg_per_tick_us",
                JsonValue::u64((elapsed.as_micros() / 100) as u64),
            ),
        ],
    );

    // Budget: < 10ms average per tick (including periodic renders)
    // Note: Budget is loose to account for debug mode overhead
    assert!(
        elapsed.as_micros() < 1_000_000,
        "100 ticks took {}µs, budget is 1000000µs",
        elapsed.as_micros()
    );
}
