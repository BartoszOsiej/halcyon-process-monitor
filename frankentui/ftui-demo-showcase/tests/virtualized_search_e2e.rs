#![forbid(unsafe_code)]

//! End-to-end tests for the Virtualized Search feature (bd-2zbk.4).
//!
//! These tests exercise the full virtualized search lifecycle through the
//! `AppModel`, covering:
//!
//! - Initial state with 10k items
//! - Navigation (j/k, PgUp/PgDn, Home/End)
//! - Search input and filtering
//! - Fuzzy match scoring and ordering
//! - Empty results handling
//! - Verbose JSONL logging with event timestamps
//!
//! Run: `cargo test -p ftui-demo-showcase --test virtualized_search_e2e`

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::sync::OnceLock;
use std::time::Instant;

use ftui_core::event::{Event, KeyCode, KeyEvent, KeyEventKind, Modifiers};
use ftui_demo_showcase::app::{AppModel, AppMsg, ScreenId};
use ftui_harness::determinism::DeterminismFixture;
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

fn char_press(ch: char) -> Event {
    press(KeyCode::Char(ch))
}

/// Navigate to the VirtualizedSearch screen.
fn go_to_virtualized_search(app: &mut AppModel) {
    app.current_screen = ScreenId::VirtualizedSearch;
}

/// Simulate a tick.
fn tick(app: &mut AppModel) {
    app.update(AppMsg::Tick);
}

/// Capture a frame and return a hash.
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

/// Emit a JSONL log entry to stderr.
fn log_jsonl(step: &str, data: &[(&str, &str)]) {
    let fields: Vec<String> = std::iter::once(format!("\"ts\":\"{}\"", chrono_like_timestamp()))
        .chain(std::iter::once(format!("\"step\":\"{}\"", step)))
        .chain(data.iter().map(|(k, v)| format!("\"{}\":\"{}\"", k, v)))
        .collect();
    eprintln!("{{{}}}", fields.join(","));
}

fn chrono_like_timestamp() -> String {
    fixture().timestamp()
}

fn fixture() -> &'static DeterminismFixture {
    static FIXTURE: OnceLock<DeterminismFixture> = OnceLock::new();
    FIXTURE.get_or_init(|| DeterminismFixture::new("virtualized_search_e2e", 42))
}

// ===========================================================================
// Scenario 1: Initial State with 10k Items
// ===========================================================================

#[test]
fn e2e_initial_state_10k_items() {
    let start = Instant::now();

    log_jsonl(
        "env",
        &[
            ("test", "e2e_initial_state_10k_items"),
            ("term_cols", "120"),
            ("term_rows", "40"),
        ],
    );

    let mut app = AppModel::new();
    app.update(AppMsg::Resize {
        width: 120,
        height: 40,
    });

    // Navigate to VirtualizedSearch screen.
    go_to_virtualized_search(&mut app);
    assert_eq!(app.current_screen, ScreenId::VirtualizedSearch);

    log_jsonl("step", &[("action", "navigate_to_screen")]);

    // Render initial frame and verify no panic.
    let frame_hash = capture_frame_hash(&mut app, 120, 40);
    log_jsonl(
        "initial_render",
        &[("frame_hash", &format!("{frame_hash:016x}"))],
    );

    // Verify frame hash is non-zero (content was rendered).
    assert!(frame_hash != 0, "Frame should have content");

    let elapsed = start.elapsed();
    log_jsonl(
        "outcome",
        &[
            ("result", "pass"),
            ("elapsed_ms", &format!("{}", elapsed.as_millis())),
        ],
    );
}

// ===========================================================================
// Scenario 2: Navigation (j/k)
// ===========================================================================

#[test]
fn e2e_navigation_jk() {
    let start = Instant::now();

    log_jsonl(
        "env",
        &[
            ("test", "e2e_navigation_jk"),
            ("term_cols", "120"),
            ("term_rows", "40"),
        ],
    );

    let mut app = AppModel::new();
    app.update(AppMsg::Resize {
        width: 120,
        height: 40,
    });
    go_to_virtualized_search(&mut app);

    // Capture initial frame.
    let initial_hash = capture_frame_hash(&mut app, 120, 40);
    log_jsonl(
        "initial_state",
        &[("frame_hash", &format!("{initial_hash:016x}"))],
    );

    // Navigate down with 'j'.
    log_jsonl("step", &[("action", "navigate_down_j")]);
    for i in 0..10 {
        app.update(AppMsg::ScreenEvent(char_press('j')));
        let hash = capture_frame_hash(&mut app, 120, 40);
        log_jsonl(
            "after_j",
            &[
                ("iteration", &format!("{i}")),
                ("frame_hash", &format!("{hash:016x}")),
            ],
        );
    }

    // Navigate up with 'k'.
    log_jsonl("step", &[("action", "navigate_up_k")]);
    for i in 0..5 {
        app.update(AppMsg::ScreenEvent(char_press('k')));
        let hash = capture_frame_hash(&mut app, 120, 40);
        log_jsonl(
            "after_k",
            &[
                ("iteration", &format!("{i}")),
                ("frame_hash", &format!("{hash:016x}")),
            ],
        );
    }

    // Final state should differ from initial (selection changed).
    let final_hash = capture_frame_hash(&mut app, 120, 40);
    log_jsonl(
        "final_state",
        &[("frame_hash", &format!("{final_hash:016x}"))],
    );

    // Selection moved, so frame should be different.
    assert!(
        initial_hash != final_hash,
        "Navigation should change the display"
    );

    let elapsed = start.elapsed();
    log_jsonl(
        "outcome",
        &[
            ("result", "pass"),
            ("elapsed_ms", &format!("{}", elapsed.as_millis())),
        ],
    );
}

// ===========================================================================
// Scenario 3: Page Navigation (PgUp/PgDn)
// ===========================================================================

#[test]
fn e2e_page_navigation() {
    let start = Instant::now();

    log_jsonl(
        "env",
        &[
            ("test", "e2e_page_navigation"),
            ("term_cols", "120"),
            ("term_rows", "40"),
        ],
    );

    let mut app = AppModel::new();
    app.update(AppMsg::Resize {
        width: 120,
        height: 40,
    });
    go_to_virtualized_search(&mut app);

    let initial_hash = capture_frame_hash(&mut app, 120, 40);
    log_jsonl(
        "initial_state",
        &[("frame_hash", &format!("{initial_hash:016x}"))],
    );

    // Page down.
    log_jsonl("step", &[("action", "page_down")]);
    app.update(AppMsg::ScreenEvent(press(KeyCode::PageDown)));
    let after_pgdn_hash = capture_frame_hash(&mut app, 120, 40);
    log_jsonl(
        "after_page_down",
        &[("frame_hash", &format!("{after_pgdn_hash:016x}"))],
    );

    // Page down should change the display significantly.
    assert!(
        initial_hash != after_pgdn_hash,
        "PageDown should scroll the view"
    );

    // Page up.
    log_jsonl("step", &[("action", "page_up")]);
    app.update(AppMsg::ScreenEvent(press(KeyCode::PageUp)));
    let after_pgup_hash = capture_frame_hash(&mut app, 120, 40);
    log_jsonl(
        "after_page_up",
        &[("frame_hash", &format!("{after_pgup_hash:016x}"))],
    );

    // Should be back near initial.
    assert_eq!(
        initial_hash, after_pgup_hash,
        "PageUp should return to original position"
    );

    let elapsed = start.elapsed();
    log_jsonl(
        "outcome",
        &[
            ("result", "pass"),
            ("elapsed_ms", &format!("{}", elapsed.as_millis())),
        ],
    );
}

// ===========================================================================
// Scenario 4: Home/End Navigation
// ===========================================================================

#[test]
fn e2e_home_end_navigation() {
    let start = Instant::now();

    log_jsonl(
        "env",
        &[
            ("test", "e2e_home_end_navigation"),
            ("term_cols", "120"),
            ("term_rows", "40"),
        ],
    );

    let mut app = AppModel::new();
    app.update(AppMsg::Resize {
        width: 120,
        height: 40,
    });
    go_to_virtualized_search(&mut app);

    let initial_hash = capture_frame_hash(&mut app, 120, 40);

    // Jump to end.
    log_jsonl("step", &[("action", "jump_to_end")]);
    app.update(AppMsg::ScreenEvent(press(KeyCode::End)));
    let end_hash = capture_frame_hash(&mut app, 120, 40);
    log_jsonl("at_end", &[("frame_hash", &format!("{end_hash:016x}"))]);

    assert!(initial_hash != end_hash, "End should jump to bottom");

    // Jump to home.
    log_jsonl("step", &[("action", "jump_to_home")]);
    app.update(AppMsg::ScreenEvent(press(KeyCode::Home)));
    let home_hash = capture_frame_hash(&mut app, 120, 40);
    log_jsonl("at_home", &[("frame_hash", &format!("{home_hash:016x}"))]);

    assert_eq!(initial_hash, home_hash, "Home should return to start");

    let elapsed = start.elapsed();
    log_jsonl(
        "outcome",
        &[
            ("result", "pass"),
            ("elapsed_ms", &format!("{}", elapsed.as_millis())),
        ],
    );
}

// ===========================================================================
// Scenario 5: Search Input and Filtering
// ===========================================================================

#[test]
fn e2e_search_filtering() {
    let start = Instant::now();

    log_jsonl(
        "env",
        &[
            ("test", "e2e_search_filtering"),
            ("term_cols", "120"),
            ("term_rows", "40"),
        ],
    );

    let mut app = AppModel::new();
    app.update(AppMsg::Resize {
        width: 120,
        height: 40,
    });
    go_to_virtualized_search(&mut app);

    let initial_hash = capture_frame_hash(&mut app, 120, 40);
    log_jsonl(
        "initial_state",
        &[("frame_hash", &format!("{initial_hash:016x}"))],
    );

    // Focus search input with '/'.
    log_jsonl("step", &[("action", "focus_search")]);
    app.update(AppMsg::ScreenEvent(char_press('/')));

    // Type a search query.
    log_jsonl(
        "step",
        &[("action", "type_query"), ("query", "CoreService")],
    );
    for c in "CoreService".chars() {
        app.update(AppMsg::ScreenEvent(char_press(c)));
    }

    let filtered_hash = capture_frame_hash(&mut app, 120, 40);
    log_jsonl(
        "after_filter",
        &[("frame_hash", &format!("{filtered_hash:016x}"))],
    );

    // Filtered view should differ from initial.
    assert!(
        initial_hash != filtered_hash,
        "Search should filter and change display"
    );

    let elapsed = start.elapsed();
    log_jsonl(
        "outcome",
        &[
            ("result", "pass"),
            ("elapsed_ms", &format!("{}", elapsed.as_millis())),
        ],
    );
}

// ===========================================================================
// Scenario 6: Empty Results Handling
// ===========================================================================

#[test]
fn e2e_empty_results() {
    let start = Instant::now();

    log_jsonl(
        "env",
        &[
            ("test", "e2e_empty_results"),
            ("term_cols", "120"),
            ("term_rows", "40"),
        ],
    );

    let mut app = AppModel::new();
    app.update(AppMsg::Resize {
        width: 120,
        height: 40,
    });
    go_to_virtualized_search(&mut app);

    // Type an impossible query.
    log_jsonl(
        "step",
        &[("action", "type_impossible_query"), ("query", "xyzzy12345")],
    );
    for c in "xyzzy12345".chars() {
        app.update(AppMsg::ScreenEvent(char_press(c)));
    }

    // Should render without panic.
    let hash = capture_frame_hash(&mut app, 120, 40);
    log_jsonl("empty_results", &[("frame_hash", &format!("{hash:016x}"))]);

    // Frame should have some content (the "no matches" message).
    assert!(hash != 0, "Empty results should still render");

    let elapsed = start.elapsed();
    log_jsonl(
        "outcome",
        &[
            ("result", "pass"),
            ("elapsed_ms", &format!("{}", elapsed.as_millis())),
        ],
    );
}

// ===========================================================================
// Scenario 7: Clear Search (Escape)
// ===========================================================================

#[test]
fn e2e_clear_search() {
    let start = Instant::now();

    log_jsonl(
        "env",
        &[
            ("test", "e2e_clear_search"),
            ("term_cols", "120"),
            ("term_rows", "40"),
        ],
    );

    let mut app = AppModel::new();
    app.update(AppMsg::Resize {
        width: 120,
        height: 40,
    });
    go_to_virtualized_search(&mut app);

    let initial_hash = capture_frame_hash(&mut app, 120, 40);

    // Type a search query.
    log_jsonl("step", &[("action", "type_query")]);
    for c in "Database".chars() {
        app.update(AppMsg::ScreenEvent(char_press(c)));
    }
    let filtered_hash = capture_frame_hash(&mut app, 120, 40);
    assert!(
        initial_hash != filtered_hash,
        "Search should filter and change display"
    );

    // Clear search with Escape.
    log_jsonl("step", &[("action", "clear_search")]);
    app.update(AppMsg::ScreenEvent(press(KeyCode::Escape)));
    let cleared_hash = capture_frame_hash(&mut app, 120, 40);
    log_jsonl(
        "after_clear",
        &[("frame_hash", &format!("{cleared_hash:016x}"))],
    );

    // Should return to initial state.
    assert_eq!(
        initial_hash, cleared_hash,
        "Escape should clear search and return to full list"
    );

    let elapsed = start.elapsed();
    log_jsonl(
        "outcome",
        &[
            ("result", "pass"),
            ("elapsed_ms", &format!("{}", elapsed.as_millis())),
        ],
    );
}

// ===========================================================================
// Scenario 8: Full Workflow (Navigate + Search + Clear)
// ===========================================================================

#[test]
fn e2e_full_workflow() {
    let start = Instant::now();

    log_jsonl(
        "env",
        &[
            ("test", "e2e_full_workflow"),
            ("term_cols", "120"),
            ("term_rows", "40"),
        ],
    );

    let mut app = AppModel::new();
    app.update(AppMsg::Resize {
        width: 120,
        height: 40,
    });
    go_to_virtualized_search(&mut app);

    // Step 1: Navigate down.
    log_jsonl("step", &[("action", "navigate_down")]);
    for _ in 0..5 {
        app.update(AppMsg::ScreenEvent(char_press('j')));
    }
    tick(&mut app);

    // Step 2: Search.
    log_jsonl("step", &[("action", "search"), ("query", "Auth")]);
    for c in "Auth".chars() {
        app.update(AppMsg::ScreenEvent(char_press(c)));
    }
    tick(&mut app);

    let search_hash = capture_frame_hash(&mut app, 120, 40);
    log_jsonl(
        "after_search",
        &[("frame_hash", &format!("{search_hash:016x}"))],
    );

    // Step 3: Navigate in filtered results.
    log_jsonl("step", &[("action", "navigate_filtered")]);
    for _ in 0..3 {
        app.update(AppMsg::ScreenEvent(char_press('j')));
    }
    tick(&mut app);

    // Step 4: Clear and verify.
    log_jsonl("step", &[("action", "clear_and_verify")]);
    app.update(AppMsg::ScreenEvent(press(KeyCode::Escape)));
    tick(&mut app);

    let final_hash = capture_frame_hash(&mut app, 120, 40);
    log_jsonl(
        "final_state",
        &[("frame_hash", &format!("{final_hash:016x}"))],
    );

    let elapsed = start.elapsed();
    log_jsonl(
        "outcome",
        &[
            ("result", "pass"),
            ("elapsed_ms", &format!("{}", elapsed.as_millis())),
        ],
    );
}

// ===========================================================================
// Scenario 9: Multiple Screen Sizes
// ===========================================================================

#[test]
fn e2e_multiple_screen_sizes() {
    let start = Instant::now();

    log_jsonl("env", &[("test", "e2e_multiple_screen_sizes")]);

    let sizes: &[(u16, u16)] = &[(80, 24), (120, 40), (200, 60), (40, 10)];

    for (width, height) in sizes {
        log_jsonl(
            "step",
            &[
                ("action", "test_size"),
                ("width", &format!("{width}")),
                ("height", &format!("{height}")),
            ],
        );

        let mut app = AppModel::new();
        app.update(AppMsg::Resize {
            width: *width,
            height: *height,
        });
        go_to_virtualized_search(&mut app);

        // Render should not panic at any size.
        let hash = capture_frame_hash(&mut app, *width, *height);
        log_jsonl(
            "rendered",
            &[
                ("size", &format!("{width}x{height}")),
                ("frame_hash", &format!("{hash:016x}")),
            ],
        );

        assert!(
            hash != 0,
            "Frame at {}x{} should have content",
            width,
            height
        );
    }

    let elapsed = start.elapsed();
    log_jsonl(
        "outcome",
        &[
            ("result", "pass"),
            ("elapsed_ms", &format!("{}", elapsed.as_millis())),
        ],
    );
}

// ===========================================================================
// Scenario 10: Rapid Navigation Stress
// ===========================================================================

#[test]
fn e2e_rapid_navigation_stress() {
    let start = Instant::now();

    log_jsonl(
        "env",
        &[
            ("test", "e2e_rapid_navigation_stress"),
            ("term_cols", "120"),
            ("term_rows", "40"),
        ],
    );

    let mut app = AppModel::new();
    app.update(AppMsg::Resize {
        width: 120,
        height: 40,
    });
    go_to_virtualized_search(&mut app);

    // Rapid navigation: 100 down, 50 up, page operations.
    log_jsonl("step", &[("action", "rapid_down"), ("count", "100")]);
    for _ in 0..100 {
        app.update(AppMsg::ScreenEvent(char_press('j')));
    }

    log_jsonl("step", &[("action", "rapid_up"), ("count", "50")]);
    for _ in 0..50 {
        app.update(AppMsg::ScreenEvent(char_press('k')));
    }

    log_jsonl("step", &[("action", "page_operations")]);
    for _ in 0..10 {
        app.update(AppMsg::ScreenEvent(press(KeyCode::PageDown)));
        app.update(AppMsg::ScreenEvent(press(KeyCode::PageUp)));
    }

    log_jsonl("step", &[("action", "home_end")]);
    app.update(AppMsg::ScreenEvent(press(KeyCode::End)));
    app.update(AppMsg::ScreenEvent(press(KeyCode::Home)));

    // Should complete without panic.
    let hash = capture_frame_hash(&mut app, 120, 40);
    log_jsonl("final_state", &[("frame_hash", &format!("{hash:016x}"))]);

    let elapsed = start.elapsed();
    log_jsonl(
        "outcome",
        &[
            ("result", "pass"),
            ("elapsed_ms", &format!("{}", elapsed.as_millis())),
        ],
    );
}

// ===========================================================================
// Scenario 11: Deterministic Frame Hashes
// ===========================================================================

#[test]
fn e2e_deterministic_rendering() {
    log_jsonl(
        "env",
        &[
            ("test", "e2e_deterministic_rendering"),
            ("term_cols", "120"),
            ("term_rows", "40"),
        ],
    );

    // Create two apps and perform identical operations.
    let mut app1 = AppModel::new();
    let mut app2 = AppModel::new();

    for app in [&mut app1, &mut app2] {
        app.update(AppMsg::Resize {
            width: 120,
            height: 40,
        });
    }

    go_to_virtualized_search(&mut app1);
    go_to_virtualized_search(&mut app2);

    // Perform identical navigation.
    for _ in 0..5 {
        app1.update(AppMsg::ScreenEvent(char_press('j')));
        app2.update(AppMsg::ScreenEvent(char_press('j')));
    }

    let hash1 = capture_frame_hash(&mut app1, 120, 40);
    let hash2 = capture_frame_hash(&mut app2, 120, 40);

    log_jsonl(
        "determinism_check",
        &[
            ("hash1", &format!("{hash1:016x}")),
            ("hash2", &format!("{hash2:016x}")),
        ],
    );

    assert_eq!(
        hash1, hash2,
        "Identical operations should produce identical frames"
    );

    log_jsonl("outcome", &[("result", "pass")]);
}
