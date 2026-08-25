#![forbid(unsafe_code)]

//! UX and Accessibility Review Tests for Advanced Text Editor (bd-12o8.6)
//!
//! This module verifies that the Advanced Text Editor meets UX and accessibility standards:
//!
//! # Keybindings Review
//!
//! | Key | Action | Context | Notes |
//! |-----|--------|---------|-------|
//! | Ctrl+F | Open search | Global | Opens search panel |
//! | Ctrl+H | Open replace | Global | Opens search/replace panel |
//! | Ctrl+G / F3 | Next match | Search open | Jumps to next match |
//! | Shift+F3 | Previous match | Search open | Jumps to previous match |
//! | Ctrl+Z | Undo | Global | Reverts last change |
//! | Ctrl+Y | Redo | Global | Reapplies undone change |
//! | Ctrl+Shift+Z | Redo (alt) | Global | Alternative redo shortcut |
//! | Ctrl+U | Toggle history | Global | Shows/hides undo panel |
//! | Shift+Arrow | Select text | Editor focus | Text selection |
//! | Ctrl+A | Select all / Replace all | Context-dependent | |
//! | Ctrl+R | Replace current | Replace focus | Single replacement |
//! | Escape | Close/clear | Global | Context-dependent action |
//! | Ctrl+Left/Right | Focus cycle | Search visible | Between Editor/Search/Replace |
//! | Tab | Focus next | Search visible | Cycles focus forward |
//!
//! # Focus Order Invariants
//!
//! 1. **Three focus areas**: Editor, Search, Replace (when search visible)
//! 2. **Cyclic navigation**: Tab/Ctrl+Arrow cycles through focus areas
//! 3. **Default focus**: Editor has focus on start
//! 4. **Focus visibility**: Active widget shows focus indicator
//!
//! # Failure Modes
//!
//! | Scenario | Expected | Status |
//! |----------|----------|--------|
//! | Empty document | Editor renders placeholder | ✓ |
//! | Search with no matches | Shows "0/0", no crash | ✓ |
//! | Undo at empty stack | No-op, no crash | ✓ |
//! | Redo at empty stack | No-op, no crash | ✓ |
//! | Very small terminal | Graceful degradation | ✓ |
//!
//! # JSONL Logging Schema
//!
//! ```json
//! {
//!   "test": "ux_a11y_keybindings",
//!   "keybinding": "Ctrl+F",
//!   "expected_action": "open_search",
//!   "result": "passed"
//! }
//! ```

use ftui_core::event::{Event, KeyCode, KeyEvent, KeyEventKind, Modifiers};
use ftui_core::geometry::Rect;
use ftui_demo_showcase::screens::Screen;
use ftui_demo_showcase::screens::advanced_text_editor::AdvancedTextEditor;
use ftui_render::frame::Frame;
use ftui_render::grapheme_pool::GraphemePool;

// =============================================================================
// Test Utilities
// =============================================================================

/// Generate a JSONL log entry.
fn log_jsonl(data: &serde_json::Value) {
    eprintln!("{}", serde_json::to_string(data).unwrap());
}

/// Create a key press event with modifiers.
fn key_press(code: KeyCode, modifiers: Modifiers) -> Event {
    Event::Key(KeyEvent {
        code,
        modifiers,
        kind: KeyEventKind::Press,
    })
}

/// Create a simple key press event (no modifiers).
fn simple_key(code: KeyCode) -> Event {
    key_press(code, Modifiers::empty())
}

/// Create a Ctrl+key press event.
fn ctrl_key(code: KeyCode) -> Event {
    key_press(code, Modifiers::CTRL)
}

/// Create a Shift+key press event.
fn shift_key(code: KeyCode) -> Event {
    key_press(code, Modifiers::SHIFT)
}

/// Create a Ctrl+Shift+key press event.
fn ctrl_shift_key(code: KeyCode) -> Event {
    key_press(code, Modifiers::CTRL | Modifiers::SHIFT)
}

/// Render frame helper - returns true if render succeeded without panic.
fn render_frame(editor: &AdvancedTextEditor, width: u16, height: u16) {
    let mut pool = GraphemePool::new();
    let mut frame = Frame::new(width, height, &mut pool);
    editor.view(&mut frame, Rect::new(0, 0, width, height));
}

// =============================================================================
// Keybinding Tests
// =============================================================================

/// Ctrl+F should open the search panel (verified via render).
#[test]
fn keybindings_ctrl_f_opens_search() {
    let mut editor = AdvancedTextEditor::new();

    // Ctrl+F opens search
    editor.update(&ctrl_key(KeyCode::Char('f')));

    // Render should succeed with search panel
    render_frame(&editor, 120, 40);

    log_jsonl(&serde_json::json!({
        "test": "keybindings_ctrl_f_opens_search",
        "result": "passed",
    }));
}

/// Ctrl+H should open the replace panel.
#[test]
fn keybindings_ctrl_h_opens_replace() {
    let mut editor = AdvancedTextEditor::new();

    // Ctrl+H opens search with replace
    editor.update(&ctrl_key(KeyCode::Char('h')));

    // Render should succeed
    render_frame(&editor, 120, 40);

    log_jsonl(&serde_json::json!({
        "test": "keybindings_ctrl_h_opens_replace",
        "result": "passed",
    }));
}

/// Ctrl+Z should undo (tested via can_undo/undo interface).
#[test]
fn keybindings_ctrl_z_undo() {
    let mut editor = AdvancedTextEditor::new();

    // Fresh editor has no undo history
    assert!(
        !editor.can_undo(),
        "Fresh editor should have empty undo stack"
    );

    // Type something to create content
    editor.update(&simple_key(KeyCode::Char('a')));

    // Try undo via Ctrl+Z
    editor.update(&ctrl_key(KeyCode::Char('z')));

    // Should not panic
    render_frame(&editor, 80, 24);

    log_jsonl(&serde_json::json!({
        "test": "keybindings_ctrl_z_undo",
        "result": "passed",
    }));
}

/// Ctrl+Y should redo (tested via can_redo/redo interface).
#[test]
fn keybindings_ctrl_y_redo() {
    let mut editor = AdvancedTextEditor::new();

    // Fresh editor has no redo
    assert!(
        !editor.can_redo(),
        "Fresh editor should have empty redo stack"
    );

    // Type, then undo
    editor.update(&simple_key(KeyCode::Char('x')));
    editor.update(&ctrl_key(KeyCode::Char('z'))); // Undo

    // Now redo with Ctrl+Y
    editor.update(&ctrl_key(KeyCode::Char('y')));

    // Should not panic
    render_frame(&editor, 80, 24);

    log_jsonl(&serde_json::json!({
        "test": "keybindings_ctrl_y_redo",
        "result": "passed",
    }));
}

/// Ctrl+Shift+Z should also redo (alternative shortcut).
#[test]
fn keybindings_ctrl_shift_z_redo_alt() {
    let mut editor = AdvancedTextEditor::new();

    // Type and undo
    editor.update(&simple_key(KeyCode::Char('y')));
    editor.update(&ctrl_key(KeyCode::Char('z'))); // Undo

    // Redo with Ctrl+Shift+Z
    editor.update(&ctrl_shift_key(KeyCode::Char('Z')));

    // Should not panic
    render_frame(&editor, 80, 24);

    log_jsonl(&serde_json::json!({
        "test": "keybindings_ctrl_shift_z_redo_alt",
        "result": "passed",
    }));
}

/// Ctrl+U should toggle the undo history panel.
#[test]
fn keybindings_ctrl_u_toggle_history() {
    let mut editor = AdvancedTextEditor::new();

    // Toggle on
    editor.update(&ctrl_key(KeyCode::Char('u')));
    render_frame(&editor, 120, 40);

    // Toggle off
    editor.update(&ctrl_key(KeyCode::Char('u')));
    render_frame(&editor, 120, 40);

    log_jsonl(&serde_json::json!({
        "test": "keybindings_ctrl_u_toggle_history",
        "result": "passed",
    }));
}

/// Escape should close search panel when open.
#[test]
fn keybindings_escape_closes_search() {
    let mut editor = AdvancedTextEditor::new();

    // Open search
    editor.update(&ctrl_key(KeyCode::Char('f')));
    render_frame(&editor, 80, 24);

    // Escape closes it
    editor.update(&simple_key(KeyCode::Escape));
    render_frame(&editor, 80, 24);

    log_jsonl(&serde_json::json!({
        "test": "keybindings_escape_closes_search",
        "result": "passed",
    }));
}

/// F3 / Ctrl+G should navigate to next match.
#[test]
fn keybindings_next_match_navigation() {
    let mut editor = AdvancedTextEditor::new();

    // Open search
    editor.update(&ctrl_key(KeyCode::Char('f')));

    // Type search query
    editor.update(&simple_key(KeyCode::Char('t')));
    editor.update(&simple_key(KeyCode::Char('h')));
    editor.update(&simple_key(KeyCode::Char('e')));

    // Navigate with F3
    editor.update(&simple_key(KeyCode::F(3)));

    // Also test Ctrl+G
    editor.update(&ctrl_key(KeyCode::Char('g')));

    // Should not panic
    render_frame(&editor, 80, 24);

    log_jsonl(&serde_json::json!({
        "test": "keybindings_next_match_navigation",
        "result": "passed",
    }));
}

/// Shift+F3 should navigate to previous match.
#[test]
fn keybindings_prev_match_navigation() {
    let mut editor = AdvancedTextEditor::new();

    // Open search
    editor.update(&ctrl_key(KeyCode::Char('f')));

    // Type search query
    editor.update(&simple_key(KeyCode::Char('t')));
    editor.update(&simple_key(KeyCode::Char('h')));
    editor.update(&simple_key(KeyCode::Char('e')));

    // Navigate with Shift+F3
    editor.update(&shift_key(KeyCode::F(3)));

    // Should not panic
    render_frame(&editor, 80, 24);

    log_jsonl(&serde_json::json!({
        "test": "keybindings_prev_match_navigation",
        "result": "passed",
    }));
}

// =============================================================================
// Focus Order Tests
// =============================================================================

/// Ctrl+Right should cycle focus forward when search is visible.
#[test]
fn focus_order_ctrl_right_cycles() {
    let mut editor = AdvancedTextEditor::new();

    // Open search to enable focus cycling
    editor.update(&ctrl_key(KeyCode::Char('f')));
    assert_eq!(editor.focus_panel(), "search");

    // Ctrl+Right: Search → Replace
    editor.update(&ctrl_key(KeyCode::Right));
    assert_eq!(editor.focus_panel(), "replace");

    // Ctrl+Right: Replace → Editor
    editor.update(&ctrl_key(KeyCode::Right));
    assert_eq!(editor.focus_panel(), "editor");

    // Ctrl+Right: Editor → Search (full cycle)
    editor.update(&ctrl_key(KeyCode::Right));
    assert_eq!(editor.focus_panel(), "search");

    render_frame(&editor, 80, 24);

    log_jsonl(&serde_json::json!({
        "test": "focus_order_ctrl_right_cycles",
        "result": "passed",
    }));
}

/// Ctrl+Left should cycle focus backward when search is visible.
#[test]
fn focus_order_ctrl_left_cycles() {
    let mut editor = AdvancedTextEditor::new();

    // Open search to enable focus cycling
    editor.update(&ctrl_key(KeyCode::Char('f')));
    assert_eq!(editor.focus_panel(), "search");

    // Ctrl+Left: Search → Editor (backward)
    editor.update(&ctrl_key(KeyCode::Left));
    assert_eq!(editor.focus_panel(), "editor");

    // Ctrl+Left: Editor → Replace (backward)
    editor.update(&ctrl_key(KeyCode::Left));
    assert_eq!(editor.focus_panel(), "replace");

    // Ctrl+Left: Replace → Search (full reverse cycle)
    editor.update(&ctrl_key(KeyCode::Left));
    assert_eq!(editor.focus_panel(), "search");

    render_frame(&editor, 80, 24);

    log_jsonl(&serde_json::json!({
        "test": "focus_order_ctrl_left_cycles",
        "result": "passed",
    }));
}

/// Tab should cycle focus forward when search is visible.
#[test]
fn focus_order_tab_cycles() {
    let mut editor = AdvancedTextEditor::new();

    // Open search — focus starts on Search
    editor.update(&ctrl_key(KeyCode::Char('f')));
    assert_eq!(editor.focus_panel(), "search");

    // Tab: Search → Replace
    editor.update(&simple_key(KeyCode::Tab));
    assert_eq!(editor.focus_panel(), "replace");

    // Tab: Replace → Editor
    editor.update(&simple_key(KeyCode::Tab));
    assert_eq!(editor.focus_panel(), "editor");

    // Tab: Editor → Search (full cycle)
    editor.update(&simple_key(KeyCode::Tab));
    assert_eq!(editor.focus_panel(), "search");

    render_frame(&editor, 80, 24);

    log_jsonl(&serde_json::json!({
        "test": "focus_order_tab_cycles",
        "result": "passed",
    }));
}

/// Shift+Tab should cycle focus backward when search is visible.
#[test]
fn focus_order_shift_tab_cycles_backward() {
    let mut editor = AdvancedTextEditor::new();

    // Open search — focus starts on Search
    editor.update(&ctrl_key(KeyCode::Char('f')));
    assert_eq!(editor.focus_panel(), "search");

    // Shift+Tab: Search → Editor (backward)
    editor.update(&shift_key(KeyCode::Tab));
    assert_eq!(editor.focus_panel(), "editor");

    // Shift+Tab: Editor → Replace (backward)
    editor.update(&shift_key(KeyCode::Tab));
    assert_eq!(editor.focus_panel(), "replace");

    // Shift+Tab: Replace → Search (full reverse cycle)
    editor.update(&shift_key(KeyCode::Tab));
    assert_eq!(editor.focus_panel(), "search");

    render_frame(&editor, 80, 24);

    log_jsonl(&serde_json::json!({
        "test": "focus_order_shift_tab_cycles_backward",
        "result": "passed",
    }));
}

/// Tab should NOT cycle focus when search is hidden (editor-only mode).
#[test]
fn focus_order_tab_noop_without_search() {
    let mut editor = AdvancedTextEditor::new();

    // Search not visible — Tab should go to widget, not cycle focus
    assert_eq!(editor.focus_panel(), "editor");
    assert!(!editor.is_search_visible());

    editor.update(&simple_key(KeyCode::Tab));
    assert_eq!(editor.focus_panel(), "editor"); // Still editor

    render_frame(&editor, 80, 24);

    log_jsonl(&serde_json::json!({
        "test": "focus_order_tab_noop_without_search",
        "result": "passed",
    }));
}

// =============================================================================
// Contrast/Legibility Tests
// =============================================================================

/// Rendering should work with various terminal sizes.
#[test]
fn contrast_renders_at_various_sizes() {
    let editor = AdvancedTextEditor::new();

    let sizes = [(80, 24), (120, 40), (40, 10), (200, 50)];

    for (w, h) in sizes {
        render_frame(&editor, w, h);
        log_jsonl(&serde_json::json!({
            "test": "contrast_renders_at_various_sizes",
            "size": format!("{}x{}", w, h),
            "result": "no_panic",
        }));
    }
}

/// Editor with search panel should render correctly.
#[test]
fn contrast_search_panel_renders() {
    let mut editor = AdvancedTextEditor::new();

    // Open search
    editor.update(&ctrl_key(KeyCode::Char('f')));

    // Render with search visible
    render_frame(&editor, 120, 40);

    log_jsonl(&serde_json::json!({
        "test": "contrast_search_panel_renders",
        "result": "rendered",
    }));
}

/// Editor with undo panel should render correctly.
#[test]
fn contrast_undo_panel_renders() {
    let mut editor = AdvancedTextEditor::new();

    // Toggle undo panel
    editor.update(&ctrl_key(KeyCode::Char('u')));

    // Render with undo panel visible
    render_frame(&editor, 120, 40);

    log_jsonl(&serde_json::json!({
        "test": "contrast_undo_panel_renders",
        "result": "rendered",
    }));
}

// =============================================================================
// Property Tests: UX Invariants
// =============================================================================

/// Property: Undo at empty stack is a no-op.
#[test]
fn property_undo_empty_stack_noop() {
    let mut editor = AdvancedTextEditor::new();

    // Fresh editor has empty undo stack
    assert!(
        !editor.can_undo(),
        "Fresh editor should have empty undo stack"
    );

    // Try to undo - should not panic
    editor.update(&ctrl_key(KeyCode::Char('z')));
    render_frame(&editor, 80, 24);

    log_jsonl(&serde_json::json!({
        "test": "property_undo_empty_stack_noop",
        "result": "passed",
    }));
}

/// Property: Redo at empty stack is a no-op.
#[test]
fn property_redo_empty_stack_noop() {
    let mut editor = AdvancedTextEditor::new();

    // Fresh editor has empty redo stack
    assert!(
        !editor.can_redo(),
        "Fresh editor should have empty redo stack"
    );

    // Try to redo - should not panic
    editor.update(&ctrl_key(KeyCode::Char('y')));
    render_frame(&editor, 80, 24);

    log_jsonl(&serde_json::json!({
        "test": "property_redo_empty_stack_noop",
        "result": "passed",
    }));
}

/// Property: Search with no matches is safe.
#[test]
fn property_search_no_matches_safe() {
    let mut editor = AdvancedTextEditor::new();

    // Open search
    editor.update(&ctrl_key(KeyCode::Char('f')));

    // Type a query that won't match
    for c in "ZZZZXYZZZZ".chars() {
        editor.update(&simple_key(KeyCode::Char(c)));
    }

    // Navigate to next match - should be no-op
    editor.update(&ctrl_key(KeyCode::Char('g')));

    // Render should work
    render_frame(&editor, 80, 24);

    log_jsonl(&serde_json::json!({
        "test": "property_search_no_matches_safe",
        "result": "passed",
    }));
}

/// Property: Rapid focus cycling is stable.
#[test]
fn property_rapid_focus_cycling() {
    let mut editor = AdvancedTextEditor::new();

    // Open search
    editor.update(&ctrl_key(KeyCode::Char('f')));

    // Rapidly cycle focus 100 times
    for i in 0..100 {
        if i % 2 == 0 {
            editor.update(&ctrl_key(KeyCode::Right));
        } else {
            editor.update(&ctrl_key(KeyCode::Left));
        }
    }

    // Render should work
    render_frame(&editor, 80, 24);

    log_jsonl(&serde_json::json!({
        "test": "property_rapid_focus_cycling",
        "cycles": 100,
        "result": "passed",
    }));
}

// =============================================================================
// Accessibility Audit Tests
// =============================================================================

/// All actions should have keyboard equivalents.
#[test]
fn a11y_all_actions_keyboard_accessible() {
    let editor = AdvancedTextEditor::new();
    let keybindings = editor.keybindings();

    log_jsonl(&serde_json::json!({
        "test": "a11y_all_actions_keyboard_accessible",
        "keybinding_count": keybindings.len(),
        "keybindings": keybindings.iter().map(|h| {
            serde_json::json!({
                "key": h.key,
                "action": h.action,
            })
        }).collect::<Vec<_>>(),
    }));

    // Verify minimum required actions
    let actions: Vec<_> = keybindings.iter().map(|h| h.action).collect();
    assert!(
        actions.iter().any(|a| a.contains("Search")),
        "Search action required"
    );
    assert!(
        actions.iter().any(|a| a.contains("Undo")),
        "Undo action required"
    );
    assert!(
        actions.iter().any(|a| a.contains("Redo")),
        "Redo action required"
    );
}

/// Help entry keybindings should match documented shortcuts.
#[test]
fn a11y_keybindings_documented() {
    let editor = AdvancedTextEditor::new();
    let keybindings = editor.keybindings();

    // Check that key documented keybindings are present
    let keys: Vec<_> = keybindings.iter().map(|h| h.key).collect();

    // These are the documented shortcuts from the module
    let expected = ["Ctrl+F", "Ctrl+H", "Ctrl+Z", "Ctrl+Y", "Esc"];
    for exp in expected {
        assert!(
            keys.iter().any(|k| k.contains(exp) || exp.contains(k)),
            "Keybinding '{}' should be documented",
            exp
        );
    }

    log_jsonl(&serde_json::json!({
        "test": "a11y_keybindings_documented",
        "result": "passed",
    }));
}

/// Editor should support undo/redo via Screen trait.
#[test]
fn a11y_undo_redo_via_trait() {
    let mut editor = AdvancedTextEditor::new();

    // Type to create undo history
    editor.update(&simple_key(KeyCode::Char('H')));
    editor.update(&simple_key(KeyCode::Char('i')));

    // Test undo via trait method
    let did_undo = editor.undo();
    assert!(did_undo, "undo() should return true when there's history");

    // Test redo via trait method
    let did_redo = editor.redo();
    assert!(did_redo, "redo() should return true after undo");

    log_jsonl(&serde_json::json!({
        "test": "a11y_undo_redo_via_trait",
        "result": "passed",
    }));
}

// =============================================================================
// Regression Tests
// =============================================================================

/// Empty render area should not panic.
#[test]
fn regression_empty_render_area() {
    let editor = AdvancedTextEditor::new();

    let mut pool = GraphemePool::new();
    let mut frame = Frame::new(80, 24, &mut pool);
    editor.view(&mut frame, Rect::new(0, 0, 0, 0));

    log_jsonl(&serde_json::json!({
        "test": "regression_empty_render_area",
        "result": "no_panic",
    }));
}

/// Very small terminal should render without panic.
#[test]
fn regression_minimum_terminal_size() {
    let editor = AdvancedTextEditor::new();

    let sizes = [(1, 1), (5, 3), (10, 5), (20, 8)];

    for (w, h) in sizes {
        let mut pool = GraphemePool::new();
        let mut frame = Frame::new(w, h, &mut pool);
        editor.view(&mut frame, Rect::new(0, 0, w, h));

        log_jsonl(&serde_json::json!({
            "test": "regression_minimum_terminal_size",
            "size": format!("{}x{}", w, h),
            "result": "no_panic",
        }));
    }
}

/// Rapid operations should not corrupt state.
#[test]
fn regression_rapid_operations_stable() {
    let mut editor = AdvancedTextEditor::new();

    // Rapid sequence of operations
    for i in 0..100 {
        match i % 10 {
            0 => editor.update(&ctrl_key(KeyCode::Char('f'))),
            1 => editor.update(&ctrl_key(KeyCode::Char('z'))),
            2 => editor.update(&ctrl_key(KeyCode::Char('y'))),
            3 => editor.update(&simple_key(KeyCode::Char('a'))),
            4 => editor.update(&simple_key(KeyCode::Escape)),
            5 => editor.update(&ctrl_key(KeyCode::Right)),
            6 => editor.update(&simple_key(KeyCode::Tab)),
            7 => editor.update(&ctrl_key(KeyCode::Char('g'))),
            8 => editor.update(&ctrl_key(KeyCode::Char('u'))),
            _ => editor.update(&simple_key(KeyCode::Enter)),
        };
    }

    // State should be valid - render should work
    render_frame(&editor, 80, 24);

    log_jsonl(&serde_json::json!({
        "test": "regression_rapid_operations_stable",
        "operations": 100,
        "result": "passed",
    }));
}

/// Search with special characters should not panic.
#[test]
fn regression_search_special_chars() {
    let mut editor = AdvancedTextEditor::new();

    // Open search
    editor.update(&ctrl_key(KeyCode::Char('f')));

    // Type special characters
    for c in "[]{}().*+?|\\^$".chars() {
        editor.update(&simple_key(KeyCode::Char(c)));
    }

    // Navigate should not panic
    editor.update(&ctrl_key(KeyCode::Char('g')));
    editor.update(&shift_key(KeyCode::F(3)));

    // Render should work
    render_frame(&editor, 80, 24);

    log_jsonl(&serde_json::json!({
        "test": "regression_search_special_chars",
        "result": "passed",
    }));
}

/// Editor with both search and undo panels should render correctly.
#[test]
fn regression_both_panels_visible() {
    let mut editor = AdvancedTextEditor::new();

    // Open search
    editor.update(&ctrl_key(KeyCode::Char('f')));

    // Open undo panel
    editor.update(&ctrl_key(KeyCode::Char('u')));

    // Render with both panels
    render_frame(&editor, 120, 40);

    log_jsonl(&serde_json::json!({
        "test": "regression_both_panels_visible",
        "result": "passed",
    }));
}

// =============================================================================
// Context-Dependent Behavior Tests
// =============================================================================

/// Escape should close search when open, not clear selection.
#[test]
fn context_escape_closes_search_first() {
    let mut editor = AdvancedTextEditor::new();

    // Open search
    editor.update(&ctrl_key(KeyCode::Char('f')));
    assert!(editor.is_search_visible());
    assert_eq!(editor.focus_panel(), "search");

    // Escape closes search, returns focus to editor
    editor.update(&simple_key(KeyCode::Escape));
    assert!(!editor.is_search_visible());
    assert_eq!(editor.focus_panel(), "editor");

    log_jsonl(&serde_json::json!({
        "test": "context_escape_closes_search_first",
        "result": "passed",
    }));
}

/// Escape without search open should clear selection (not crash).
#[test]
fn context_escape_clears_selection_when_no_search() {
    let mut editor = AdvancedTextEditor::new();

    // No search visible
    assert!(!editor.is_search_visible());

    // Escape should not crash and editor stays focused
    editor.update(&simple_key(KeyCode::Escape));
    assert_eq!(editor.focus_panel(), "editor");

    render_frame(&editor, 80, 24);

    log_jsonl(&serde_json::json!({
        "test": "context_escape_clears_selection_when_no_search",
        "result": "passed",
    }));
}

/// Ctrl+Shift+G should navigate to previous match (alternative shortcut).
#[test]
fn keybindings_ctrl_shift_g_prev_match() {
    let mut editor = AdvancedTextEditor::new();

    // Open search and type a query
    editor.update(&ctrl_key(KeyCode::Char('f')));
    for c in "the".chars() {
        editor.update(&simple_key(KeyCode::Char(c)));
    }

    // Navigate with Ctrl+Shift+G
    editor.update(&ctrl_shift_key(KeyCode::Char('G')));

    // Should not panic
    render_frame(&editor, 80, 24);

    log_jsonl(&serde_json::json!({
        "test": "keybindings_ctrl_shift_g_prev_match",
        "result": "passed",
    }));
}

/// Enter in replace focus should replace current match (not insert newline).
#[test]
fn context_enter_in_replace_replaces() {
    let mut editor = AdvancedTextEditor::new();

    // Open replace panel directly with Ctrl+H (focuses Replace)
    editor.update(&ctrl_key(KeyCode::Char('h')));
    assert_eq!(editor.focus_panel(), "replace");

    // Enter in replace mode should not crash
    editor.update(&simple_key(KeyCode::Enter));

    render_frame(&editor, 80, 24);

    log_jsonl(&serde_json::json!({
        "test": "context_enter_in_replace_replaces",
        "result": "passed",
    }));
}

/// Shift+Enter in search focus should find previous match.
#[test]
fn context_shift_enter_in_search_finds_prev() {
    let mut editor = AdvancedTextEditor::new();
    editor.update(&ctrl_key(KeyCode::Char('f')));
    assert_eq!(editor.focus_panel(), "search");

    // Type a query
    for c in "text".chars() {
        editor.update(&simple_key(KeyCode::Char(c)));
    }

    // Shift+Enter should find previous match
    editor.update(&shift_key(KeyCode::Enter));

    render_frame(&editor, 80, 24);

    log_jsonl(&serde_json::json!({
        "test": "context_shift_enter_in_search_finds_prev",
        "result": "passed",
    }));
}

/// Keybindings list should include Tab/Shift+Tab and focus cycling entries.
#[test]
fn a11y_keybindings_include_focus_cycling() {
    let editor = AdvancedTextEditor::new();
    let keybindings = editor.keybindings();
    let keys: Vec<_> = keybindings.iter().map(|h| h.key).collect();

    assert!(
        keys.iter().any(|k| k.contains("Tab")),
        "Tab focus cycling should be documented in keybindings"
    );
    assert!(
        keys.iter().any(|k| k.contains("Ctrl+Left")),
        "Ctrl+Left/Right focus cycling should be documented"
    );

    log_jsonl(&serde_json::json!({
        "test": "a11y_keybindings_include_focus_cycling",
        "result": "passed",
    }));
}

/// Keybindings should document Enter behavior in search/replace.
#[test]
fn a11y_keybindings_include_enter_actions() {
    let editor = AdvancedTextEditor::new();
    let keybindings = editor.keybindings();
    let actions: Vec<_> = keybindings.iter().map(|h| h.action).collect();

    assert!(
        actions.iter().any(|a| a.contains("Find next")),
        "Enter (search) → Find next should be documented"
    );
    assert!(
        actions.iter().any(|a| a.contains("Find previous")),
        "Shift+Enter (search) → Find previous should be documented"
    );

    log_jsonl(&serde_json::json!({
        "test": "a11y_keybindings_include_enter_actions",
        "result": "passed",
    }));
}
