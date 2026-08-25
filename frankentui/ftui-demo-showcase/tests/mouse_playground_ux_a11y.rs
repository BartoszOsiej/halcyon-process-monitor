#![forbid(unsafe_code)]

//! UX and Accessibility Review Tests for Mouse/Hit-Test Playground (bd-bksf.6)
//!
//! This suite validates the Mouse Playground UX surface:
//!
//! # Keybindings Review
//!
//! | Key | Action |
//! |-----|--------|
//! | Tab | Cycle focus |
//! | ↑↓←→ / hjkl | Navigate targets |
//! | Space/Enter | Click target |
//! | Home/g | First target |
//! | End/G | Last target |
//! | O | Toggle overlay |
//! | J | Toggle jitter stats |
//! | C | Clear log |
//!
//! # Focus Order Invariants
//!
//! 1. **Deterministic cycle**: Tab cycles Targets -> Event Log -> Stats -> Targets.
//! 2. **Backwards cycle**: Shift+Tab / BackTab cycles in reverse.
//! 3. **Focus gating**: Target navigation keys only affect focus in Targets panel.
//!
//! # Contrast/Legibility Standards
//!
//! - Panel titles are rendered as text (Hit-Test Targets / Event Log / Stats).
//! - Stats panel shows Overlay and Jitter state text (ON/OFF).
//! - Empty log renders a readable prompt.
//!
//! # Invariants (Alien Artifact)
//!
//! 1. **Navigation round-trip**: Right then Left returns to original target.
//! 2. **Toggle idempotence**: Overlay + Jitter toggles are involutions.
//! 3. **Log clear visibility**: Clear log shrinks event log to zero and records a diagnostic.
//!
//! Run: `cargo test -p ftui-demo-showcase --test mouse_playground_ux_a11y`

use ftui_core::event::{Event, KeyCode, KeyEvent, KeyEventKind, Modifiers};
use ftui_core::geometry::Rect;
use ftui_demo_showcase::screens::Screen;
use ftui_demo_showcase::screens::mouse_playground::{
    DiagnosticEventKind, Focus, MousePlayground, entries_of_kind,
};
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

fn key_press_shift(code: KeyCode) -> Event {
    Event::Key(KeyEvent {
        code,
        modifiers: Modifiers::SHIFT,
        kind: KeyEventKind::Press,
    })
}

fn render_lines(screen: &MousePlayground, width: u16, height: u16) -> Vec<String> {
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
    let screen = MousePlayground::new();
    let bindings = screen.keybindings();

    let keys: Vec<_> = bindings.iter().map(|h| (h.key, h.action)).collect();
    log_jsonl(
        "keybindings",
        "count",
        !keys.is_empty(),
        &format!("bindings={}", keys.len()),
    );

    assert!(keys.iter().any(|(k, a)| *k == "Tab" && a.contains("Cycle")));
    assert!(
        keys.iter()
            .any(|(k, a)| *k == "↑↓←→/hjkl" && a.contains("Navigate")),
        "Arrow + hjkl navigation should be documented"
    );
    assert!(
        keys.iter()
            .any(|(k, a)| *k == "Space/Enter" && a.contains("Click")),
        "Space/Enter should be documented for target click"
    );
    assert!(
        keys.iter()
            .any(|(k, a)| *k == "Home/g" && a.contains("First")),
        "Home/g should be documented for first target"
    );
    assert!(
        keys.iter()
            .any(|(k, a)| *k == "End/G" && a.contains("Last")),
        "End/G should be documented for last target"
    );
    assert!(
        keys.iter().any(|(k, a)| *k == "O" && a.contains("overlay")),
        "O should be documented for overlay"
    );
    assert!(
        keys.iter().any(|(k, a)| *k == "J" && a.contains("jitter")),
        "J should be documented for jitter stats"
    );
    assert!(
        keys.iter().any(|(k, a)| *k == "C" && a.contains("Clear")),
        "C should be documented for log clear"
    );
}

// =============================================================================
// Focus Order Tests
// =============================================================================

#[test]
fn focus_cycles_forward_and_backward() {
    let mut screen = MousePlayground::new();
    assert_eq!(screen.current_focus(), Focus::Targets);

    let _ = screen.update(&key_press(KeyCode::Tab));
    log_jsonl(
        "focus",
        "tab_event_log",
        screen.current_focus() == Focus::EventLog,
        "Tab should move focus to Event Log",
    );
    assert_eq!(screen.current_focus(), Focus::EventLog);

    let _ = screen.update(&key_press(KeyCode::Tab));
    log_jsonl(
        "focus",
        "tab_stats",
        screen.current_focus() == Focus::Stats,
        "Tab should move focus to Stats",
    );
    assert_eq!(screen.current_focus(), Focus::Stats);

    let _ = screen.update(&key_press(KeyCode::Tab));
    log_jsonl(
        "focus",
        "tab_targets",
        screen.current_focus() == Focus::Targets,
        "Tab should wrap back to Targets",
    );
    assert_eq!(screen.current_focus(), Focus::Targets);

    let _ = screen.update(&key_press(KeyCode::BackTab));
    log_jsonl(
        "focus",
        "backtab_stats",
        screen.current_focus() == Focus::Stats,
        "BackTab should cycle backwards to Stats",
    );
    assert_eq!(screen.current_focus(), Focus::Stats);

    let _ = screen.update(&key_press_shift(KeyCode::Tab));
    log_jsonl(
        "focus",
        "shift_tab_event_log",
        screen.current_focus() == Focus::EventLog,
        "Shift+Tab should also cycle backwards",
    );
    assert_eq!(screen.current_focus(), Focus::EventLog);
}

#[test]
fn focus_gates_target_navigation() {
    let mut screen = MousePlayground::new();
    let initial = screen.focused_target_index();

    let _ = screen.update(&key_press(KeyCode::Tab));
    assert_eq!(screen.current_focus(), Focus::EventLog);

    let _ = screen.update(&key_press(KeyCode::Right));
    log_jsonl(
        "focus",
        "nav_blocked",
        screen.focused_target_index() == initial,
        "Navigation should be inert outside Targets focus",
    );
    assert_eq!(screen.focused_target_index(), initial);

    let _ = screen.update(&key_press(KeyCode::BackTab));
    assert_eq!(screen.current_focus(), Focus::Targets);

    let _ = screen.update(&key_press(KeyCode::Right));
    log_jsonl(
        "focus",
        "nav_active",
        screen.focused_target_index() != initial,
        "Navigation should update focus in Targets panel",
    );
    assert_ne!(screen.focused_target_index(), initial);
}

// =============================================================================
// Navigation Tests
// =============================================================================

#[test]
fn navigation_round_trips() {
    let mut screen = MousePlayground::new();
    let origin = screen.focused_target_index();

    let _ = screen.update(&key_press(KeyCode::Right));
    let after_right = screen.focused_target_index();
    log_jsonl(
        "navigation",
        "right",
        after_right != origin,
        "Right should move focus",
    );
    assert_ne!(after_right, origin);

    let _ = screen.update(&key_press(KeyCode::Left));
    log_jsonl(
        "navigation",
        "left_back",
        screen.focused_target_index() == origin,
        "Left should return to origin",
    );
    assert_eq!(screen.focused_target_index(), origin);

    let _ = screen.update(&key_press(KeyCode::Down));
    let after_down = screen.focused_target_index();
    log_jsonl(
        "navigation",
        "down",
        after_down != origin,
        "Down should move focus",
    );
    assert_ne!(after_down, origin);

    let _ = screen.update(&key_press(KeyCode::Up));
    log_jsonl(
        "navigation",
        "up_back",
        screen.focused_target_index() == origin,
        "Up should return to origin",
    );
    assert_eq!(screen.focused_target_index(), origin);

    let _ = screen.update(&key_press(KeyCode::End));
    let end_index = screen.focused_target_index();
    log_jsonl(
        "navigation",
        "end",
        end_index != origin,
        "End should jump away from origin",
    );
    assert_ne!(end_index, origin);

    let _ = screen.update(&key_press(KeyCode::Home));
    log_jsonl(
        "navigation",
        "home_back",
        screen.focused_target_index() == origin,
        "Home should return to origin",
    );
    assert_eq!(screen.focused_target_index(), origin);
}

// =============================================================================
// Toggle and Log Tests
// =============================================================================

#[test]
fn toggles_record_diagnostics() {
    let mut screen = MousePlayground::new().with_diagnostics();

    let _ = screen.update(&key_press(KeyCode::Char('o')));
    log_jsonl(
        "toggle",
        "overlay_on",
        screen.overlay_enabled(),
        "Overlay should toggle on",
    );
    assert!(screen.overlay_enabled());

    let overlay_entries = entries_of_kind(
        screen.diagnostic_log().expect("diagnostic log"),
        DiagnosticEventKind::OverlayToggle,
    );
    assert!(!overlay_entries.is_empty(), "Overlay toggle should log");

    let _ = screen.update(&key_press(KeyCode::Char('J')));
    log_jsonl(
        "toggle",
        "jitter_on",
        screen.jitter_stats_enabled(),
        "Jitter stats should toggle on",
    );
    assert!(screen.jitter_stats_enabled());

    let jitter_entries = entries_of_kind(
        screen.diagnostic_log().expect("diagnostic log"),
        DiagnosticEventKind::JitterStatsToggle,
    );
    assert!(!jitter_entries.is_empty(), "Jitter toggle should log");

    let _ = screen.update(&key_press(KeyCode::Char('o')));
    assert!(!screen.overlay_enabled());
    let _ = screen.update(&key_press(KeyCode::Char('J')));
    assert!(!screen.jitter_stats_enabled());
}

#[test]
fn clear_log_via_keybinding() {
    let mut screen = MousePlayground::new().with_diagnostics();
    screen.push_test_event("Test", 1, 1);
    screen.push_test_event("Test2", 2, 2);
    assert!(screen.event_log_len() > 0);

    let _ = screen.update(&key_press(KeyCode::Char('C')));
    log_jsonl(
        "log",
        "cleared",
        screen.event_log_len() == 0,
        "Clear log should empty the event log",
    );
    assert_eq!(screen.event_log_len(), 0);

    let clear_entries = entries_of_kind(
        screen.diagnostic_log().expect("diagnostic log"),
        DiagnosticEventKind::LogClear,
    );
    assert!(!clear_entries.is_empty(), "Log clear should log");
}

// =============================================================================
// Legibility Tests
// =============================================================================

#[test]
fn legibility_titles_and_states_render() {
    let screen = MousePlayground::new();
    let lines = render_lines(&screen, 120, 40);

    assert!(
        find_line(&lines, "Hit-Test Targets").is_some(),
        "Hit-Test Targets title should render"
    );
    assert!(
        find_line(&lines, "Event Log").is_some(),
        "Event Log title should render"
    );
    assert!(
        find_line(&lines, "Stats").is_some(),
        "Stats title should render"
    );
    assert!(
        find_line(&lines, "Overlay:").is_some(),
        "Overlay state should be text-visible"
    );
    assert!(
        find_line(&lines, "Jitter Stats:").is_some(),
        "Jitter stats state should be text-visible"
    );
    assert!(
        find_line(&lines, "No events yet").is_some(),
        "Empty log should prompt the user"
    );
}
