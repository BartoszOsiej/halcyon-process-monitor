#![forbid(unsafe_code)]

//! UX and Accessibility Review Tests for Snapshot/Time Travel Player (bd-3sa7.6)
//!
//! This suite validates the UX/a11y surface for Snapshot Player:
//!
//! # Keybindings Review
//! | Key | Action |
//! |-----|--------|
//! | Space | Play/Pause |
//! | Left/Right or h/l | Step frame |
//! | Home/End or g/G | First/Last |
//! | M | Toggle marker |
//! | R | Toggle record |
//! | C | Clear all |
//! | D | Diagnostics |
//!
//! # Focus/Visibility Invariants
//! 1. Status line shows Playing/Paused explicitly.
//! 2. Marker state is textual (Marked: Yes/No) and not color-only.
//! 3. Empty state renders a clear message ("No frames recorded").
//!
//! Run: `cargo test -p ftui-demo-showcase --test snapshot_player_ux_a11y`

use ftui_core::event::{Event, KeyCode, KeyEvent, KeyEventKind, Modifiers};
use ftui_core::geometry::Rect;
use ftui_demo_showcase::screens::Screen;
use ftui_demo_showcase::screens::snapshot_player::{SnapshotPlayer, SnapshotPlayerConfig};
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

fn render_lines(screen: &SnapshotPlayer, width: u16, height: u16) -> Vec<String> {
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
    let screen = SnapshotPlayer::new();
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
            .any(|(k, a)| *k == "Space" && a.contains("Play/Pause")),
        "Space should be documented for Play/Pause"
    );
    assert!(
        keys.iter()
            .any(|(k, a)| k.contains("h/l") && a.contains("Step")),
        "Step frame should be documented for Left/Right or h/l"
    );
    assert!(
        keys.iter()
            .any(|(k, a)| *k == "Home/End or g/G" && a.contains("First/Last")),
        "Home/End or g/G should be documented for First/Last"
    );
    assert!(
        keys.iter()
            .any(|(k, a)| *k == "M" && a.contains("Toggle marker")),
        "M should be documented for marker toggle"
    );
    assert!(
        keys.iter()
            .any(|(k, a)| *k == "R" && a.contains("Toggle record")),
        "R should be documented for recording"
    );
    assert!(
        keys.iter().any(|(k, a)| *k == "C" && a.contains("Clear")),
        "C should be documented for clear"
    );
    assert!(
        keys.iter()
            .any(|(k, a)| *k == "D" && a.contains("Diagnostics")),
        "D should be documented for diagnostics"
    );
}

// =============================================================================
// Visibility + Empty State Tests
// =============================================================================

#[test]
fn empty_state_message_visible() {
    let config = SnapshotPlayerConfig {
        auto_generate_demo: false,
        ..Default::default()
    };
    let screen = SnapshotPlayer::with_config(config);
    let lines = render_lines(&screen, 120, 40);

    let empty_line = find_line(&lines, "No frames recorded");
    log_jsonl(
        "empty_state",
        "message_visible",
        empty_line.is_some(),
        "expected No frames recorded",
    );
    assert!(empty_line.is_some(), "Empty state message should render");
}

#[test]
fn controls_panel_includes_key_hints() {
    let screen = SnapshotPlayer::new();
    let lines = render_lines(&screen, 120, 40);

    let has_controls = find_line(&lines, "Controls").is_some();
    let has_play_pause = find_line(&lines, "Space: Play/Pause").is_some();
    let has_step = find_line(&lines, "h/l: Step").is_some();
    let has_first_last = find_line(&lines, "First/Last").is_some();
    let has_marker = find_line(&lines, "Toggle marker").is_some();
    let has_record = find_line(&lines, "Toggle record").is_some();
    let has_clear = find_line(&lines, "C: Clear").is_some();
    let has_diag = find_line(&lines, "D: Diagnostics").is_some();

    log_jsonl(
        "controls",
        "controls_header",
        has_controls,
        "Controls header",
    );
    log_jsonl(
        "controls",
        "play_pause",
        has_play_pause,
        "Space: Play/Pause",
    );
    log_jsonl("controls", "step", has_step, "Step frame hint");
    log_jsonl("controls", "first_last", has_first_last, "First/Last hint");
    log_jsonl("controls", "marker", has_marker, "Toggle marker hint");
    log_jsonl("controls", "record", has_record, "Toggle record hint");
    log_jsonl("controls", "clear", has_clear, "Clear hint");
    log_jsonl("controls", "diagnostics", has_diag, "Diagnostics hint");

    assert!(has_controls, "Controls header should render");
    assert!(has_play_pause, "Play/Pause hint should render");
    assert!(has_step, "Step frame hint should render (h/l present)");
    assert!(has_first_last, "First/Last hint should render");
    assert!(has_marker, "Toggle marker hint should render");
    assert!(has_record, "Toggle record hint should render");
    assert!(has_clear, "Clear hint should render");
    assert!(has_diag, "Diagnostics hint should render");
}

#[test]
fn status_and_marker_text_are_explicit() {
    let mut screen = SnapshotPlayer::new();

    let lines = render_lines(&screen, 120, 40);
    let status_line = find_line(&lines, "Status:").cloned();
    let marker_line = find_line(&lines, "Marked:").cloned();

    log_jsonl(
        "status_marker",
        "initial_lines_present",
        status_line.is_some() && marker_line.is_some(),
        "Status/Marked lines should render",
    );

    assert!(status_line.is_some(), "Status line should render");
    assert!(marker_line.is_some(), "Marked line should render");
    assert!(
        status_line.unwrap().contains("Paused"),
        "Initial status should be Paused"
    );
    assert!(
        marker_line.unwrap().contains("No"),
        "Initial marker state should be No"
    );

    screen.update(&key_press(KeyCode::Char('M')));
    let lines_marked = render_lines(&screen, 120, 40);
    let marker_line =
        find_line(&lines_marked, "Marked:").expect("Marked line should render after toggle");

    log_jsonl(
        "status_marker",
        "marker_toggle",
        marker_line.contains("Yes"),
        "Marker toggle should show Yes",
    );
    assert!(marker_line.contains("Yes"), "Marker should display Yes");
}

#[test]
fn status_updates_on_playback() {
    let mut screen = SnapshotPlayer::new();

    let lines = render_lines(&screen, 120, 40);
    let status_line = find_line(&lines, "Status:").expect("Status line should render");
    assert!(
        status_line.contains("Paused"),
        "Initial status should be Paused"
    );

    screen.update(&key_press(KeyCode::Char(' ')));
    let lines_playing = render_lines(&screen, 120, 40);
    let status_line =
        find_line(&lines_playing, "Status:").expect("Status line should render after toggle");

    log_jsonl(
        "status",
        "playback_toggle",
        status_line.contains("Playing"),
        "Status should change to Playing",
    );
    assert!(
        status_line.contains("Playing"),
        "Status should show Playing"
    );
}

#[test]
fn clear_action_shows_empty_state() {
    let mut screen = SnapshotPlayer::new();
    screen.update(&key_press(KeyCode::Char('C')));

    let lines = render_lines(&screen, 120, 40);
    let empty_line = find_line(&lines, "No frames recorded");

    log_jsonl(
        "clear",
        "empty_state",
        empty_line.is_some(),
        "Clear should show empty state",
    );
    assert!(
        empty_line.is_some(),
        "Clear should show empty state message"
    );
}
