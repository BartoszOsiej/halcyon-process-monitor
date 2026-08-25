#![forbid(unsafe_code)]

//! Golden snapshot tests for Advanced Text Editor.
//!
//! Tests cover:
//! - Cursor positioning (single cursor, various positions)
//! - Selection states (single line, multi-line, word selection)
//! - Text operations (insert, delete, undo/redo)
//! - Edge cases (empty buffer, long lines, Unicode)
//!
//! # Invariants (Alien Artifact)
//!
//! 1. **Cursor bounds**: Cursor is always within valid buffer bounds
//! 2. **Selection consistency**: anchor and head form valid range
//! 3. **Undo symmetry**: undo(redo(state)) == state
//! 4. **Unicode correctness**: Grapheme clusters handled as atomic units
//!
//! # Running Tests
//!
//! ```sh
//! # Run snapshot tests
//! cargo test --package ftui-harness editor_snapshots
//!
//! # Update golden snapshots (bless mode)
//! BLESS=1 cargo test --package ftui-harness editor_snapshots
//!
//! # Deterministic mode with fixed seed
//! GOLDEN_SEED=42 cargo test --package ftui-harness editor_snapshots
//! ```
//!
//! # JSONL Schema
//!
//! Each test emits structured logs (when run with logging):
//! ```json
//! {"event":"start","case":"editor_cursor_end","env":{...},"seed":0}
//! {"event":"frame","checksum":"sha256:...","timing_ms":1}
//! {"event":"complete","outcome":"pass","total_ms":5}
//! ```

use ftui_core::geometry::Rect;
use ftui_harness::assert_snapshot;
use ftui_render::frame::Frame;
use ftui_render::grapheme_pool::GraphemePool;
use ftui_widgets::StatefulWidget;
use ftui_widgets::textarea::{TextArea, TextAreaState};

// ============================================================================
// Test Helpers
// ============================================================================

/// Create a focused TextArea with the given content.
fn textarea_with_text(text: &str) -> TextArea {
    TextArea::new().with_text(text).with_focus(true)
}

// ============================================================================
// Cursor Position Tests
// ============================================================================

#[test]
fn snapshot_editor_empty() {
    let ta = TextArea::new();
    let area = Rect::new(0, 0, 20, 5);
    let mut pool = GraphemePool::new();
    let mut frame = Frame::new(20, 5, &mut pool);
    let mut state = TextAreaState::default();
    StatefulWidget::render(&ta, area, &mut frame, &mut state);
    assert_snapshot!("editor_empty", &frame.buffer);
}

#[test]
fn snapshot_editor_cursor_at_end() {
    let ta = textarea_with_text("Hello, World!");
    let area = Rect::new(0, 0, 20, 3);
    let mut pool = GraphemePool::new();
    let mut frame = Frame::new(20, 3, &mut pool);
    let mut state = TextAreaState::default();
    StatefulWidget::render(&ta, area, &mut frame, &mut state);
    assert_snapshot!("editor_cursor_end", &frame.buffer);
}

#[test]
fn snapshot_editor_cursor_at_start() {
    let mut ta = textarea_with_text("Hello, World!");
    ta.move_to_document_start();
    let area = Rect::new(0, 0, 20, 3);
    let mut pool = GraphemePool::new();
    let mut frame = Frame::new(20, 3, &mut pool);
    let mut state = TextAreaState::default();
    StatefulWidget::render(&ta, area, &mut frame, &mut state);
    assert_snapshot!("editor_cursor_start", &frame.buffer);
}

#[test]
fn snapshot_editor_cursor_middle() {
    let mut ta = textarea_with_text("Hello, World!");
    ta.move_to_document_start();
    // Move to middle (after "Hello, ")
    for _ in 0..7 {
        ta.move_right();
    }
    let area = Rect::new(0, 0, 20, 3);
    let mut pool = GraphemePool::new();
    let mut frame = Frame::new(20, 3, &mut pool);
    let mut state = TextAreaState::default();
    StatefulWidget::render(&ta, area, &mut frame, &mut state);
    assert_snapshot!("editor_cursor_middle", &frame.buffer);
}

// ============================================================================
// Multi-line Text Tests
// ============================================================================

#[test]
fn snapshot_editor_multiline() {
    let ta = textarea_with_text("Line 1\nLine 2\nLine 3");
    let area = Rect::new(0, 0, 15, 5);
    let mut pool = GraphemePool::new();
    let mut frame = Frame::new(15, 5, &mut pool);
    let mut state = TextAreaState::default();
    StatefulWidget::render(&ta, area, &mut frame, &mut state);
    assert_snapshot!("editor_multiline", &frame.buffer);
}

#[test]
fn snapshot_editor_multiline_cursor_line2() {
    let mut ta = textarea_with_text("Line 1\nLine 2\nLine 3");
    // Move cursor to middle of line 2
    ta.move_to_document_start();
    ta.move_down();
    for _ in 0..3 {
        ta.move_right();
    }
    let area = Rect::new(0, 0, 15, 5);
    let mut pool = GraphemePool::new();
    let mut frame = Frame::new(15, 5, &mut pool);
    let mut state = TextAreaState::default();
    StatefulWidget::render(&ta, area, &mut frame, &mut state);
    assert_snapshot!("editor_multiline_cursor_line2", &frame.buffer);
}

#[test]
fn snapshot_editor_long_lines() {
    let ta = textarea_with_text(
        "This is a very long line that exceeds the viewport width\nShort\nAnother long line here",
    );
    let area = Rect::new(0, 0, 25, 4);
    let mut pool = GraphemePool::new();
    let mut frame = Frame::new(25, 4, &mut pool);
    let mut state = TextAreaState::default();
    StatefulWidget::render(&ta, area, &mut frame, &mut state);
    assert_snapshot!("editor_long_lines", &frame.buffer);
}

// ============================================================================
// Selection Tests
// ============================================================================

#[test]
fn snapshot_editor_selection_word() {
    let mut ta = textarea_with_text("Hello World Rust");
    ta.move_to_document_start();
    // Select "Hello"
    for _ in 0..5 {
        ta.select_right();
    }
    let area = Rect::new(0, 0, 20, 3);
    let mut pool = GraphemePool::new();
    let mut frame = Frame::new(20, 3, &mut pool);
    let mut state = TextAreaState::default();
    StatefulWidget::render(&ta, area, &mut frame, &mut state);
    assert_snapshot!("editor_selection_word", &frame.buffer);
}

#[test]
fn snapshot_editor_selection_all() {
    let mut ta = textarea_with_text("Select All Test");
    ta.select_all();
    let area = Rect::new(0, 0, 20, 3);
    let mut pool = GraphemePool::new();
    let mut frame = Frame::new(20, 3, &mut pool);
    let mut state = TextAreaState::default();
    StatefulWidget::render(&ta, area, &mut frame, &mut state);
    assert_snapshot!("editor_selection_all", &frame.buffer);
}

#[test]
fn snapshot_editor_selection_multiline() {
    let mut ta = textarea_with_text("Line 1\nLine 2\nLine 3");
    ta.move_to_document_start();
    // Select first line + part of second
    for _ in 0..10 {
        ta.select_right();
    }
    let area = Rect::new(0, 0, 15, 5);
    let mut pool = GraphemePool::new();
    let mut frame = Frame::new(15, 5, &mut pool);
    let mut state = TextAreaState::default();
    StatefulWidget::render(&ta, area, &mut frame, &mut state);
    assert_snapshot!("editor_selection_multiline", &frame.buffer);
}

#[test]
fn snapshot_editor_selection_word_boundary() {
    let mut ta = textarea_with_text("one two three");
    ta.move_to_document_start();
    // Move to start of "two" then select the word (3 chars)
    ta.move_word_right();
    // Select "two" by selecting right 3 times
    for _ in 0..3 {
        ta.select_right();
    }
    let area = Rect::new(0, 0, 20, 3);
    let mut pool = GraphemePool::new();
    let mut frame = Frame::new(20, 3, &mut pool);
    let mut state = TextAreaState::default();
    StatefulWidget::render(&ta, area, &mut frame, &mut state);
    assert_snapshot!("editor_selection_word_boundary", &frame.buffer);
}

// ============================================================================
// Edit Operations Tests
// ============================================================================

#[test]
fn snapshot_editor_after_insert() {
    let mut ta = textarea_with_text("Hello");
    ta.insert_text(" World");
    let area = Rect::new(0, 0, 20, 3);
    let mut pool = GraphemePool::new();
    let mut frame = Frame::new(20, 3, &mut pool);
    let mut state = TextAreaState::default();
    StatefulWidget::render(&ta, area, &mut frame, &mut state);
    assert_snapshot!("editor_after_insert", &frame.buffer);
}

#[test]
fn snapshot_editor_after_delete() {
    let mut ta = textarea_with_text("Hello World");
    // Delete last 5 characters (cursor at end)
    for _ in 0..5 {
        ta.delete_backward();
    }
    let area = Rect::new(0, 0, 20, 3);
    let mut pool = GraphemePool::new();
    let mut frame = Frame::new(20, 3, &mut pool);
    let mut state = TextAreaState::default();
    StatefulWidget::render(&ta, area, &mut frame, &mut state);
    assert_snapshot!("editor_after_delete", &frame.buffer);
}

#[test]
fn snapshot_editor_after_newline() {
    let mut ta = textarea_with_text("HelloWorld");
    // Move to middle and insert newline
    ta.move_to_document_start();
    for _ in 0..5 {
        ta.move_right();
    }
    ta.insert_newline();
    let area = Rect::new(0, 0, 15, 4);
    let mut pool = GraphemePool::new();
    let mut frame = Frame::new(15, 4, &mut pool);
    let mut state = TextAreaState::default();
    StatefulWidget::render(&ta, area, &mut frame, &mut state);
    assert_snapshot!("editor_after_newline", &frame.buffer);
}

// ============================================================================
// Undo/Redo Tests
// ============================================================================

#[test]
fn snapshot_editor_undo() {
    let mut ta = textarea_with_text("Hello");
    ta.insert_text(" World");
    ta.undo();
    let area = Rect::new(0, 0, 20, 3);
    let mut pool = GraphemePool::new();
    let mut frame = Frame::new(20, 3, &mut pool);
    let mut state = TextAreaState::default();
    StatefulWidget::render(&ta, area, &mut frame, &mut state);
    assert_snapshot!("editor_undo", &frame.buffer);
}

#[test]
fn snapshot_editor_undo_redo() {
    let mut ta = textarea_with_text("Hello");
    ta.insert_text(" World");
    ta.undo();
    ta.redo();
    let area = Rect::new(0, 0, 20, 3);
    let mut pool = GraphemePool::new();
    let mut frame = Frame::new(20, 3, &mut pool);
    let mut state = TextAreaState::default();
    StatefulWidget::render(&ta, area, &mut frame, &mut state);
    assert_snapshot!("editor_undo_redo", &frame.buffer);
}

#[test]
fn snapshot_editor_multi_undo() {
    let mut ta = TextArea::new().with_focus(true);
    ta.insert_text("One");
    ta.insert_text(" Two");
    ta.insert_text(" Three");
    ta.undo();
    ta.undo();
    let area = Rect::new(0, 0, 20, 3);
    let mut pool = GraphemePool::new();
    let mut frame = Frame::new(20, 3, &mut pool);
    let mut state = TextAreaState::default();
    StatefulWidget::render(&ta, area, &mut frame, &mut state);
    assert_snapshot!("editor_multi_undo", &frame.buffer);
}

// ============================================================================
// Unicode / Edge Case Tests
// ============================================================================

#[test]
fn snapshot_editor_unicode() {
    let ta = textarea_with_text("Hello 世界 🌍 Здравствуй");
    let area = Rect::new(0, 0, 30, 3);
    let mut pool = GraphemePool::new();
    let mut frame = Frame::new(30, 3, &mut pool);
    let mut state = TextAreaState::default();
    StatefulWidget::render(&ta, area, &mut frame, &mut state);
    assert_snapshot!("editor_unicode", &frame.buffer);
}

#[test]
fn snapshot_editor_emoji() {
    let ta = textarea_with_text("👋 Hello 👨‍👩‍👧‍👦 Family 🎉");
    let area = Rect::new(0, 0, 35, 3);
    let mut pool = GraphemePool::new();
    let mut frame = Frame::new(35, 3, &mut pool);
    let mut state = TextAreaState::default();
    StatefulWidget::render(&ta, area, &mut frame, &mut state);
    assert_snapshot!("editor_emoji", &frame.buffer);
}

#[test]
fn snapshot_editor_rtl() {
    // Arabic text (right-to-left)
    let ta = textarea_with_text("مرحبا بالعالم");
    let area = Rect::new(0, 0, 20, 3);
    let mut pool = GraphemePool::new();
    let mut frame = Frame::new(20, 3, &mut pool);
    let mut state = TextAreaState::default();
    StatefulWidget::render(&ta, area, &mut frame, &mut state);
    assert_snapshot!("editor_rtl", &frame.buffer);
}

#[test]
fn snapshot_editor_combining_chars() {
    // Text with combining diacritics
    let ta = textarea_with_text("café naïve résumé");
    let area = Rect::new(0, 0, 25, 3);
    let mut pool = GraphemePool::new();
    let mut frame = Frame::new(25, 3, &mut pool);
    let mut state = TextAreaState::default();
    StatefulWidget::render(&ta, area, &mut frame, &mut state);
    assert_snapshot!("editor_combining", &frame.buffer);
}

// ============================================================================
// Viewport / Scrolling Tests
// ============================================================================

#[test]
fn snapshot_editor_scroll_vertical() {
    let mut ta = TextArea::new().with_focus(true);
    // Insert many lines
    for i in 1..=20 {
        ta.insert_text(&format!("Line {i}\n"));
    }
    // Cursor is at bottom
    let area = Rect::new(0, 0, 15, 5);
    let mut pool = GraphemePool::new();
    let mut frame = Frame::new(15, 5, &mut pool);
    let mut state = TextAreaState::default();
    StatefulWidget::render(&ta, area, &mut frame, &mut state);
    assert_snapshot!("editor_scroll_vertical", &frame.buffer);
}

#[test]
fn snapshot_editor_narrow_viewport() {
    let ta = textarea_with_text("A very long line of text");
    let area = Rect::new(0, 0, 10, 3);
    let mut pool = GraphemePool::new();
    let mut frame = Frame::new(10, 3, &mut pool);
    let mut state = TextAreaState::default();
    StatefulWidget::render(&ta, area, &mut frame, &mut state);
    assert_snapshot!("editor_narrow_viewport", &frame.buffer);
}

// ============================================================================
// Line Numbers Tests
// ============================================================================

#[test]
fn snapshot_editor_line_numbers() {
    let ta = TextArea::new()
        .with_text("Line 1\nLine 2\nLine 3\nLine 4\nLine 5")
        .with_line_numbers(true)
        .with_focus(true);
    let area = Rect::new(0, 0, 20, 7);
    let mut pool = GraphemePool::new();
    let mut frame = Frame::new(20, 7, &mut pool);
    let mut state = TextAreaState::default();
    StatefulWidget::render(&ta, area, &mut frame, &mut state);
    assert_snapshot!("editor_line_numbers", &frame.buffer);
}

// ============================================================================
// Placeholder Tests
// ============================================================================

#[test]
fn snapshot_editor_placeholder() {
    let ta = TextArea::new().with_placeholder("Enter text here...");
    let area = Rect::new(0, 0, 25, 3);
    let mut pool = GraphemePool::new();
    let mut frame = Frame::new(25, 3, &mut pool);
    let mut state = TextAreaState::default();
    StatefulWidget::render(&ta, area, &mut frame, &mut state);
    assert_snapshot!("editor_placeholder", &frame.buffer);
}

// ============================================================================
// Property Tests
// ============================================================================

/// Invariant: undo stack preserves exact state
#[test]
fn property_undo_preserves_state() {
    let mut ta = TextArea::new().with_focus(true);
    ta.insert_text("Initial");
    let initial_text = ta.text();

    ta.insert_text(" Addition");
    assert_ne!(ta.text(), initial_text);

    ta.undo();
    assert_eq!(ta.text(), initial_text);
}

/// Invariant: selection is always within bounds
#[test]
fn property_selection_in_bounds() {
    let mut ta = textarea_with_text("Hello World");
    ta.select_all();

    let sel = ta.selection().expect("should have selection");
    let text = ta.text();
    let char_count = text.chars().count();

    // Selection should cover entire text
    // anchor at start, head at end
    assert_eq!(sel.anchor.grapheme, 0);
    assert_eq!(sel.head.grapheme, char_count);
}

/// Invariant: cursor never exceeds document bounds
#[test]
fn property_cursor_bounds() {
    let mut ta = textarea_with_text("Short");
    let text_len = ta.text().chars().count();

    // Try to move past end
    for _ in 0..100 {
        ta.move_right();
    }
    assert!(ta.cursor().grapheme <= text_len);

    // Try to move before start
    for _ in 0..100 {
        ta.move_left();
    }
    assert_eq!(ta.cursor().grapheme, 0);
}
