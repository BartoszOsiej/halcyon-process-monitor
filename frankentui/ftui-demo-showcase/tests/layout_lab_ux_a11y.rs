#![forbid(unsafe_code)]

//! UX and Accessibility Review Tests for Layout Composer / Layout Lab (bd-32my.6)
//!
//! This suite validates the UX/a11y surface for the Layout Laboratory screen:
//!
//! # Keybindings Review
//! | Key | Action |
//! |-----|--------|
//! | 1-5 | Switch preset |
//! | d | Toggle direction |
//! | a | Cycle alignment |
//! | +/- | Adjust gap |
//! | m/M | Adjust margin |
//! | p/P | Adjust padding |
//! | Tab | Select constraint |
//! | Left/Right | Adjust constraint |
//! | l | Cycle align pos |
//! | D | Toggle debug |
//!
//! # Focus Order Invariants
//! 1. **Constraint selection**: Tab cycles the active constraint marker.
//! 2. **Direction/alignment**: toggles update visible labels deterministically.
//!
//! # Contrast/Legibility Standards
//! - Controls panel shows explicit key hints (e.g., (d), (a), (+/-), (m/M), (p/P)).
//! - Selected constraint is indicated with a textual marker (>) not just color.
//!
//! Run: `cargo test -p ftui-demo-showcase --test layout_lab_ux_a11y`

use ftui_core::event::{Event, KeyCode, KeyEvent, KeyEventKind, Modifiers};
use ftui_core::geometry::Rect;
use ftui_demo_showcase::screens::Screen;
use ftui_demo_showcase::screens::layout_lab::LayoutLab;
use ftui_render::frame::Frame;
use ftui_render::grapheme_pool::GraphemePool;

// =============================================================================
// Test Utilities
// =============================================================================

fn log_jsonl(test: &str, check: &str, passed: bool, notes: &str) {
    eprintln!(r#"{{"test":"{test}","check":"{check}","passed":{passed},"notes":"{notes}"}}"#);
}

fn key_press(code: KeyCode) -> Event {
    Event::Key(KeyEvent {
        code,
        modifiers: Modifiers::empty(),
        kind: KeyEventKind::Press,
    })
}

fn render_lines(screen: &LayoutLab, width: u16, height: u16) -> Vec<String> {
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
    let screen = LayoutLab::new();
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
            .any(|(k, a)| *k == "1-5" && a.contains("Switch preset")),
        "1-5 should be documented for preset switching"
    );
    assert!(
        keys.iter()
            .any(|(k, a)| *k == "d" && a.contains("Toggle direction")),
        "d should be documented for direction"
    );
    assert!(
        keys.iter()
            .any(|(k, a)| *k == "a" && a.contains("Cycle alignment")),
        "a should be documented for alignment"
    );
    assert!(
        keys.iter()
            .any(|(k, a)| *k == "+/-" && a.contains("Adjust gap")),
        "+/- should be documented for gap"
    );
    assert!(
        keys.iter()
            .any(|(k, a)| *k == "m/M" && a.contains("Adjust margin")),
        "m/M should be documented for margin"
    );
    assert!(
        keys.iter()
            .any(|(k, a)| *k == "p/P" && a.contains("Adjust padding")),
        "p/P should be documented for padding"
    );
    assert!(
        keys.iter()
            .any(|(k, a)| *k == "Tab" && a.contains("Select constraint")),
        "Tab should be documented for constraint selection"
    );
    assert!(
        keys.iter()
            .any(|(k, a)| *k == "Left/Right" && a.contains("Adjust constraint")),
        "Left/Right should be documented for constraint adjustment"
    );
    assert!(
        keys.iter()
            .any(|(k, a)| *k == "l" && a.contains("Cycle align pos")),
        "l should be documented for align position"
    );
    assert!(
        keys.iter()
            .any(|(k, a)| *k == "D" && a.contains("Toggle debug")),
        "D should be documented for debug overlay"
    );
}

// =============================================================================
// Focus / Selection Tests
// =============================================================================

#[test]
fn tab_cycles_constraint_marker() {
    let mut screen = LayoutLab::new();

    let lines = render_lines(&screen, 120, 40);
    let before = find_line(&lines, "Constraints:")
        .expect("constraints line should render")
        .to_string();

    let _ = screen.update(&key_press(KeyCode::Tab));
    let lines = render_lines(&screen, 120, 40);
    let after = find_line(&lines, "Constraints:")
        .expect("constraints line should render")
        .to_string();

    log_jsonl(
        "focus",
        "tab_cycle",
        before != after,
        "Tab should move the selected constraint marker",
    );

    assert_ne!(before, after, "Tab should move the marker");
}

#[test]
fn direction_toggle_updates_label() {
    let mut screen = LayoutLab::new();

    let lines = render_lines(&screen, 120, 40);
    let before = find_line(&lines, "Direction:")
        .expect("direction line should render")
        .to_string();

    let _ = screen.update(&key_press(KeyCode::Char('d')));
    let lines = render_lines(&screen, 120, 40);
    let after = find_line(&lines, "Direction:")
        .expect("direction line should render")
        .to_string();

    log_jsonl(
        "focus",
        "direction_toggle",
        before != after,
        "Direction label should change after toggle",
    );

    assert_ne!(before, after, "Direction label should change");
}

#[test]
fn alignment_cycle_updates_label() {
    let mut screen = LayoutLab::new();

    let lines = render_lines(&screen, 120, 40);
    let before = find_line(&lines, "Alignment:")
        .expect("alignment line should render")
        .to_string();

    let _ = screen.update(&key_press(KeyCode::Char('a')));
    let lines = render_lines(&screen, 120, 40);
    let after = find_line(&lines, "Alignment:")
        .expect("alignment line should render")
        .to_string();

    log_jsonl(
        "focus",
        "alignment_cycle",
        before != after,
        "Alignment label should change after cycle",
    );

    assert_ne!(before, after, "Alignment label should change");
}

// =============================================================================
// Legibility Tests
// =============================================================================

#[test]
fn controls_panel_shows_key_hints() {
    let screen = LayoutLab::new();
    let lines = render_lines(&screen, 120, 40);

    let has_direction = lines
        .iter()
        .any(|line| line.contains("Direction:") && line.contains("(d)"));
    let has_alignment = lines
        .iter()
        .any(|line| line.contains("Alignment:") && line.contains("(a)"));
    let has_gap = lines
        .iter()
        .any(|line| line.contains("Gap:") && line.contains("(+/-)"));
    let has_margin = lines
        .iter()
        .any(|line| line.contains("Margin:") && line.contains("(m/M)"));
    let has_padding = lines
        .iter()
        .any(|line| line.contains("Padding:") && line.contains("(p/P)"));
    let has_marker = lines
        .iter()
        .any(|line| line.contains("Constraints:") && line.contains(">"));

    log_jsonl(
        "legibility",
        "key_hints",
        has_direction && has_alignment && has_gap && has_margin && has_padding,
        "Controls panel should show key hints",
    );

    assert!(has_direction);
    assert!(has_alignment);
    assert!(has_gap);
    assert!(has_margin);
    assert!(has_padding);
    assert!(has_marker, "Selected constraint should have '>' marker");
}

#[test]
fn debug_overlay_toggle_renders_label() {
    let mut screen = LayoutLab::new();

    let _ = screen.update(&key_press(KeyCode::Char('D')));
    let lines = render_lines(&screen, 120, 40);
    let has_debug = lines.iter().any(|line| line.contains("Debug"));

    log_jsonl(
        "legibility",
        "debug_label",
        has_debug,
        "Debug overlay should render a label",
    );

    assert!(has_debug);
}
