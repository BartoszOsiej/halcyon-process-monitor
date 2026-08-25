#![forbid(unsafe_code)]

//! UX and Accessibility Review Tests for Virtualized List + Fuzzy Search (bd-2zbk.6)
//!
//! This module validates the UX/a11y surface for the virtualized search screen:
//!
//! # Keybindings Review
//! | Key | Action |
//! |-----|--------|
//! | / | Focus search input |
//! | Esc | Clear search / unfocus |
//! | j/↓ | Next item |
//! | k/↑ | Previous item |
//! | g/G | First/Last item |
//! | PgUp/Dn | Page scroll |
//!
//! # Focus Order Invariants
//! 1. **Keyboard-first**: focusing search and clearing works via / and Esc.
//! 2. **Auto-focus**: typing from list focus moves to search and records query.
//! 3. **Navigation stable**: j/k navigation stays in bounds and is deterministic.
//!
//! # Contrast/Legibility Standards
//! - Search bar title includes explicit key hints.
//! - Filtered state is announced via Results (...) match text.
//! - Stats panel includes a numeric Top score line (text-first, not color-only).
//!
//! # Invariants (Alien Artifact)
//! 1. **Focus change logged**: focus transitions emit diagnostic events.
//! 2. **Query change logged**: typing updates the query in diagnostics.
//! 3. **Selection monotone**: j then k returns to original index.
//!
//! Run: `cargo test -p ftui-demo-showcase --test virtualized_search_ux_a11y`

use ftui_core::event::{Event, KeyCode, KeyEvent, KeyEventKind, Modifiers};
use ftui_core::geometry::Rect;
use ftui_demo_showcase::screens::Screen;
use ftui_demo_showcase::screens::virtualized_search::{DiagnosticEventKind, VirtualizedSearch};
use ftui_render::frame::Frame;
use ftui_render::grapheme_pool::GraphemePool;

// =============================================================================
// Test Utilities
// =============================================================================

fn log_jsonl(test: &str, check: &str, passed: bool, notes: &str) {
    eprintln!(
        "{{\"test\":\"{test}\",\"check\":\"{check}\",\"passed\":{passed},\"notes\":\"{notes}\"}}"
    );
}

fn key_press(code: KeyCode) -> Event {
    Event::Key(KeyEvent {
        code,
        modifiers: Modifiers::empty(),
        kind: KeyEventKind::Press,
    })
}

fn render_lines(screen: &VirtualizedSearch, width: u16, height: u16) -> Vec<String> {
    let mut pool = GraphemePool::new();
    let mut frame = Frame::new(width, height, &mut pool);
    screen.view(&mut frame, Rect::new(0, 0, width, height));

    let mut lines = Vec::with_capacity(height as usize);
    for y in 0..height {
        let mut line = String::new();
        for x in 0..width {
            if let Some(cell) = frame.buffer.get(x, y)
                && let Some(ch) = cell.content.as_char()
            {
                line.push(ch);
            } else {
                line.push(' ');
            }
        }
        lines.push(line);
    }
    lines
}

fn find_line<'a>(lines: &'a [String], needle: &str) -> Option<&'a String> {
    lines.iter().find(|line| line.contains(needle))
}

// =============================================================================
// Keybinding Tests
// =============================================================================

#[test]
fn keybindings_documented() {
    let screen = VirtualizedSearch::new();
    let bindings = screen.keybindings();

    let keys: Vec<_> = bindings.iter().map(|h| (h.key, h.action)).collect();
    log_jsonl(
        "keybindings",
        "count",
        !keys.is_empty(),
        &format!("bindings={}", keys.len()),
    );

    assert!(
        keys.iter()
            .any(|(k, a)| *k == "/" && a.contains("Focus search")),
        "/ should be documented for focus"
    );
    assert!(
        keys.iter()
            .any(|(k, a)| *k == "Esc" && a.contains("Clear search")),
        "Esc should be documented for clearing"
    );
    assert!(
        keys.iter().any(|(k, a)| *k == "j/↓" && a.contains("Next")),
        "j/↓ should be documented for next item"
    );
    assert!(
        keys.iter()
            .any(|(k, a)| *k == "k/↑" && a.contains("Previous")),
        "k/↑ should be documented for previous item"
    );
    assert!(
        keys.iter()
            .any(|(k, a)| *k == "g/G" && a.contains("First/Last")),
        "g/G should be documented for jump to edges"
    );
    assert!(
        keys.iter()
            .any(|(k, a)| *k == "PgUp/Dn" && a.contains("Page scroll")),
        "PgUp/Dn should be documented for page scroll"
    );
}

#[test]
fn keybinding_slash_focuses_search_and_escape_unfocuses() {
    let mut screen = VirtualizedSearch::new().with_diagnostics();

    let _ = screen.update(&key_press(KeyCode::Char('/')));
    let focus_entries = screen
        .diagnostic_log()
        .expect("diagnostic log")
        .entries_of_kind(DiagnosticEventKind::FocusChange);
    let focused = focus_entries.last().and_then(|e| e.focus_search);
    log_jsonl(
        "focus",
        "slash_focus",
        focused == Some(true),
        "Slash should focus search",
    );
    assert_eq!(focused, Some(true));

    let _ = screen.update(&key_press(KeyCode::Escape));
    let focus_entries = screen
        .diagnostic_log()
        .expect("diagnostic log")
        .entries_of_kind(DiagnosticEventKind::FocusChange);
    let unfocused = focus_entries.last().and_then(|e| e.focus_search);
    log_jsonl(
        "focus",
        "escape_unfocus",
        unfocused == Some(false),
        "Escape should return focus to list",
    );
    assert_eq!(unfocused, Some(false));
}

#[test]
fn keybinding_typing_auto_focuses_search_and_logs_query() {
    let mut screen = VirtualizedSearch::new().with_diagnostics();

    let _ = screen.update(&key_press(KeyCode::Char('a')));

    let focus_entries = screen
        .diagnostic_log()
        .expect("diagnostic log")
        .entries_of_kind(DiagnosticEventKind::FocusChange);
    let focused = focus_entries.last().and_then(|e| e.focus_search);

    let query_entries = screen
        .diagnostic_log()
        .expect("diagnostic log")
        .entries_of_kind(DiagnosticEventKind::QueryChange);
    let query = query_entries.last().and_then(|e| e.query.clone());

    log_jsonl(
        "focus",
        "auto_focus",
        focused == Some(true) && query.as_deref() == Some("a"),
        "Typing should auto-focus search and record query",
    );

    assert_eq!(focused, Some(true));
    assert_eq!(query.as_deref(), Some("a"));
}

// =============================================================================
// Focus / Navigation Tests
// =============================================================================

#[test]
fn navigation_jk_moves_selection() {
    let mut screen = VirtualizedSearch::new();
    assert_eq!(screen.selected_index(), 0);

    let _ = screen.update(&key_press(KeyCode::Char('j')));
    assert_eq!(screen.selected_index(), 1);

    let _ = screen.update(&key_press(KeyCode::Char('k')));
    assert_eq!(screen.selected_index(), 0);

    log_jsonl(
        "navigation",
        "jk_roundtrip",
        screen.selected_index() == 0,
        "j then k returns to original selection",
    );
}

// =============================================================================
// Legibility Tests
// =============================================================================

#[test]
fn legibility_search_title_and_results_text() {
    let mut screen = VirtualizedSearch::new();

    let lines = render_lines(&screen, 120, 40);
    let has_title = find_line(&lines, "Search (/ to focus, Esc to clear)").is_some();
    log_jsonl(
        "legibility",
        "search_title",
        has_title,
        "Search title should include key hints",
    );
    assert!(has_title);

    let _ = screen.update(&key_press(KeyCode::Char('/')));
    let _ = screen.update(&key_press(KeyCode::Char('a')));
    let lines = render_lines(&screen, 120, 40);
    let has_results = lines
        .iter()
        .any(|line| line.contains("Results (") && line.contains("match"));
    let has_score = lines.iter().any(|line| line.contains("Top score:"));

    log_jsonl(
        "legibility",
        "results_text",
        has_results,
        "Filtered state should announce results",
    );
    log_jsonl(
        "legibility",
        "top_score",
        has_score,
        "Stats panel should show numeric Top score",
    );

    assert!(has_results);
    assert!(has_score);
}
