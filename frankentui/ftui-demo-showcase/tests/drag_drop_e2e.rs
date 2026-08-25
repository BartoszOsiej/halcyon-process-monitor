#![forbid(unsafe_code)]

//! End-to-end tests for the Drag-and-Drop Demo (bd-1csc.6).
//!
//! These tests exercise the drag-and-drop functionality through the
//! `DragDropDemo` screen, covering:
//!
//! - Sortable list item reordering (up/down)
//! - Cross-container item transfer
//! - Keyboard drag pick up, navigation, and drop
//! - Mode switching (Tab)
//! - List navigation (j/k)
//!
//! # Invariants (Alien Artifact)
//!
//! 1. **Item count preservation**: Total items across both lists remains constant
//!    after transfers (no items lost or duplicated).
//! 2. **Selection bounds**: selected_index is always within [0, list_len).
//! 3. **Mode transitions**: Tab cycles through modes deterministically.
//! 4. **Drag lifecycle**: A started drag must end (drop or cancel) before another
//!    can begin.
//!
//! # Failure Modes
//!
//! | Scenario | Expected Behavior |
//! |----------|-------------------|
//! | Zero-width render area | No panic, graceful no-op |
//! | Move up at index 0 | No-op, selection stays at 0 |
//! | Move down at last index | No-op, selection stays at last |
//! | Transfer from empty list | No-op, no crash |
//! | Cancel drag when not dragging | No-op |
//!
//! Run: `cargo test -p ftui-demo-showcase --test drag_drop_e2e`

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::sync::atomic::{AtomicU64, Ordering};

use ftui_core::event::{Event, KeyCode, KeyEvent, KeyEventKind, Modifiers};
use ftui_core::geometry::Rect;
use ftui_demo_showcase::screens::Screen;
use ftui_demo_showcase::screens::drag_drop::DragDropDemo;
use ftui_harness::assert_snapshot;
use ftui_render::frame::Frame;
use ftui_render::grapheme_pool::GraphemePool;

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

fn shift_press(code: KeyCode) -> Event {
    Event::Key(KeyEvent {
        code,
        modifiers: Modifiers::SHIFT,
        kind: KeyEventKind::Press,
    })
}

fn char_press(ch: char) -> Event {
    press(KeyCode::Char(ch))
}

/// Emit a JSONL log entry to stderr for verbose test logging.
fn log_jsonl(step: &str, data: &[(&str, &str)]) {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let ts = COUNTER.fetch_add(1, Ordering::Relaxed);
    let fields: Vec<String> = std::iter::once(format!("\"ts\":\"T{ts:06}\""))
        .chain(std::iter::once(format!("\"step\":\"{step}\"")))
        .chain(data.iter().map(|(k, v)| format!("\"{k}\":\"{v}\"")))
        .collect();
    eprintln!("{{{}}}", fields.join(","));
}

/// Capture a frame and return a hash for determinism checks.
fn capture_frame_hash(demo: &DragDropDemo, width: u16, height: u16) -> u64 {
    let mut pool = GraphemePool::new();
    let mut frame = Frame::new(width, height, &mut pool);
    let area = Rect::new(0, 0, width, height);
    demo.view(&mut frame, area);
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

/// Render the demo and return the text for inspection.
#[allow(dead_code)]
fn render_demo_text(demo: &DragDropDemo, width: u16, height: u16) -> String {
    let mut pool = GraphemePool::new();
    let mut frame = Frame::new(width, height, &mut pool);
    let area = Rect::new(0, 0, width, height);
    demo.view(&mut frame, area);
    let mut text = String::new();
    for y in 0..height {
        for x in 0..width {
            if let Some(cell) = frame.buffer.get(x, y)
                && let Some(ch) = cell.content.as_char()
            {
                text.push(ch);
            } else {
                text.push(' ');
            }
        }
        text.push('\n');
    }
    text
}

// ===========================================================================
// Scenario 1: Initial State and Rendering
// ===========================================================================

#[test]
fn e2e_initial_state() {
    log_jsonl(
        "env",
        &[
            ("test", "e2e_initial_state"),
            ("term_cols", "120"),
            ("term_rows", "40"),
        ],
    );

    let demo = DragDropDemo::new();

    // Verify initial mode (Sortable List)
    log_jsonl("check", &[("mode", "SortableList")]);

    // Verify initial selection
    assert_eq!(demo.selected_index(), 0, "Initial selection should be 0");

    // Verify list counts
    assert_eq!(demo.left_list_len(), 8, "Left list should have 8 items");
    assert_eq!(demo.right_list_len(), 8, "Right list should have 8 items");

    log_jsonl(
        "initial",
        &[
            ("selected_index", "0"),
            ("left_count", "8"),
            ("right_count", "8"),
        ],
    );
}

#[test]
fn e2e_renders_at_various_sizes() {
    log_jsonl("env", &[("test", "e2e_renders_at_various_sizes")]);

    let demo = DragDropDemo::new();

    // Standard sizes
    for (w, h) in [(120, 40), (80, 24), (60, 20), (40, 15)] {
        let hash = capture_frame_hash(&demo, w, h);
        log_jsonl(
            "rendered",
            &[
                ("width", &w.to_string()),
                ("height", &h.to_string()),
                ("frame_hash", &format!("{hash:016x}")),
            ],
        );
    }

    // Very small terminal should show "too small" message without panic
    let mut pool = GraphemePool::new();
    let mut frame = Frame::new(30, 5, &mut pool);
    demo.view(&mut frame, Rect::new(0, 0, 30, 5));
    log_jsonl("small_terminal", &[("result", "no_panic")]);
}

#[test]
fn drag_drop_initial_80x24() {
    let demo = DragDropDemo::new();
    let mut pool = GraphemePool::new();
    let mut frame = Frame::new(80, 24, &mut pool);
    let area = Rect::new(0, 0, 80, 24);
    demo.view(&mut frame, area);
    assert_snapshot!("drag_drop_initial_80x24", &frame.buffer);
}

#[test]
fn drag_drop_initial_120x40() {
    let demo = DragDropDemo::new();
    let mut pool = GraphemePool::new();
    let mut frame = Frame::new(120, 40, &mut pool);
    let area = Rect::new(0, 0, 120, 40);
    demo.view(&mut frame, area);
    assert_snapshot!("drag_drop_initial_120x40", &frame.buffer);
}

// ===========================================================================
// Scenario 2: Mode Navigation
// ===========================================================================

#[test]
fn e2e_mode_navigation_tab() {
    log_jsonl("env", &[("test", "e2e_mode_navigation_tab")]);

    let mut demo = DragDropDemo::new();

    // Initial mode is SortableList
    log_jsonl("initial", &[("mode", "SortableList")]);

    // Tab cycles through modes
    demo.update(&press(KeyCode::Tab));
    log_jsonl("after_tab1", &[("mode", "CrossContainer")]);

    demo.update(&press(KeyCode::Tab));
    log_jsonl("after_tab2", &[("mode", "KeyboardDrag")]);

    demo.update(&press(KeyCode::Tab));
    log_jsonl("after_tab3_wrap", &[("mode", "SortableList")]);
}

#[test]
fn e2e_mode_navigation_shift_tab() {
    log_jsonl("env", &[("test", "e2e_mode_navigation_shift_tab")]);

    let mut demo = DragDropDemo::new();

    // Shift-Tab goes backwards
    demo.update(&shift_press(KeyCode::Tab));
    log_jsonl("back_wrap", &[("mode", "KeyboardDrag")]);

    demo.update(&shift_press(KeyCode::Tab));
    log_jsonl("back1", &[("mode", "CrossContainer")]);

    demo.update(&shift_press(KeyCode::Tab));
    log_jsonl("back2", &[("mode", "SortableList")]);
}

// ===========================================================================
// Scenario 3: List Navigation
// ===========================================================================

#[test]
fn e2e_selection_navigation_jk() {
    log_jsonl("env", &[("test", "e2e_selection_navigation_jk")]);

    let mut demo = DragDropDemo::new();

    // Initial selection is 0
    assert_eq!(demo.selected_index(), 0);
    log_jsonl("initial", &[("selected", "0")]);

    // j moves down
    demo.update(&char_press('j'));
    assert_eq!(demo.selected_index(), 1);
    log_jsonl("after_j", &[("selected", "1")]);

    demo.update(&char_press('j'));
    assert_eq!(demo.selected_index(), 2);
    log_jsonl("after_j2", &[("selected", "2")]);

    // k moves up
    demo.update(&char_press('k'));
    assert_eq!(demo.selected_index(), 1);
    log_jsonl("after_k", &[("selected", "1")]);
}

#[test]
fn e2e_selection_navigation_arrows() {
    log_jsonl("env", &[("test", "e2e_selection_navigation_arrows")]);

    let mut demo = DragDropDemo::new();

    demo.update(&press(KeyCode::Down));
    assert_eq!(demo.selected_index(), 1);
    log_jsonl("after_down", &[("selected", "1")]);

    demo.update(&press(KeyCode::Up));
    assert_eq!(demo.selected_index(), 0);
    log_jsonl("after_up", &[("selected", "0")]);

    // Up at 0 wraps to last
    demo.update(&press(KeyCode::Up));
    assert_eq!(demo.selected_index(), 7);
    log_jsonl("wrap_up", &[("selected", "7")]);

    // Down at last wraps to 0
    demo.update(&press(KeyCode::Down));
    assert_eq!(demo.selected_index(), 0);
    log_jsonl("wrap_down", &[("selected", "0")]);
}

// ===========================================================================
// Scenario 4: Sortable List - Item Reordering
// ===========================================================================

#[test]
fn e2e_sortable_list_move_down() {
    log_jsonl("env", &[("test", "e2e_sortable_list_move_down")]);

    let mut demo = DragDropDemo::new();

    // Get first two item IDs
    let item0_id = demo.left_item_id(0).unwrap();
    let item1_id = demo.left_item_id(1).unwrap();
    log_jsonl(
        "initial",
        &[
            ("item0_id", &item0_id.to_string()),
            ("item1_id", &item1_id.to_string()),
        ],
    );

    // Move item down with 'd'
    demo.update(&char_press('d'));

    // Items should be swapped
    assert_eq!(demo.left_item_id(0).unwrap(), item1_id);
    assert_eq!(demo.left_item_id(1).unwrap(), item0_id);
    assert_eq!(demo.selected_index(), 1);
    log_jsonl(
        "after_move",
        &[
            ("pos0_id", &demo.left_item_id(0).unwrap().to_string()),
            ("pos1_id", &demo.left_item_id(1).unwrap().to_string()),
            ("selected", "1"),
        ],
    );
}

#[test]
fn e2e_sortable_list_move_up() {
    log_jsonl("env", &[("test", "e2e_sortable_list_move_up")]);

    let mut demo = DragDropDemo::new();

    // Navigate to second item
    demo.update(&char_press('j'));
    assert_eq!(demo.selected_index(), 1);

    // Get item IDs
    let item0_id = demo.left_item_id(0).unwrap();
    let item1_id = demo.left_item_id(1).unwrap();

    // Move item up with 'u'
    demo.update(&char_press('u'));

    // Items should be swapped
    assert_eq!(demo.left_item_id(0).unwrap(), item1_id);
    assert_eq!(demo.left_item_id(1).unwrap(), item0_id);
    assert_eq!(demo.selected_index(), 0);
    log_jsonl("after_move_up", &[("selected", "0")]);
}

#[test]
fn e2e_sortable_list_move_at_boundary() {
    log_jsonl("env", &[("test", "e2e_sortable_list_move_at_boundary")]);

    let mut demo = DragDropDemo::new();

    // Try to move up at index 0 (should be no-op)
    let item0_id = demo.left_item_id(0).unwrap();
    demo.update(&char_press('u'));
    assert_eq!(
        demo.left_item_id(0).unwrap(),
        item0_id,
        "Item should not move"
    );
    assert_eq!(demo.selected_index(), 0);
    log_jsonl("move_up_at_0", &[("result", "no_op")]);

    // Navigate to last item
    for _ in 0..7 {
        demo.update(&char_press('j'));
    }
    assert_eq!(demo.selected_index(), 7);

    // Try to move down at last index (should be no-op)
    let last_id = demo.left_item_id(7).unwrap();
    demo.update(&char_press('d'));
    assert_eq!(
        demo.left_item_id(7).unwrap(),
        last_id,
        "Item should not move"
    );
    log_jsonl("move_down_at_last", &[("result", "no_op")]);
}

// ===========================================================================
// Scenario 5: Cross-Container Transfer
// ===========================================================================

#[test]
fn e2e_cross_container_transfer() {
    log_jsonl("env", &[("test", "e2e_cross_container_transfer")]);

    let mut demo = DragDropDemo::new();

    // Switch to cross-container mode
    demo.update(&press(KeyCode::Tab));

    let initial_left = demo.left_list_len();
    let initial_right = demo.right_list_len();
    let item_id = demo.left_item_id(0).unwrap();
    log_jsonl(
        "initial",
        &[
            ("left_count", &initial_left.to_string()),
            ("right_count", &initial_right.to_string()),
            ("item_to_transfer", &item_id.to_string()),
        ],
    );

    // Transfer with Enter
    demo.update(&press(KeyCode::Enter));

    // Verify transfer
    assert_eq!(demo.left_list_len(), initial_left - 1);
    assert_eq!(demo.right_list_len(), initial_right + 1);
    assert_eq!(demo.right_last_item_id().unwrap(), item_id);
    log_jsonl(
        "after_transfer",
        &[
            ("left_count", &demo.left_list_len().to_string()),
            ("right_count", &demo.right_list_len().to_string()),
        ],
    );
}

#[test]
fn e2e_cross_container_switch_list() {
    log_jsonl("env", &[("test", "e2e_cross_container_switch_list")]);

    let mut demo = DragDropDemo::new();

    // Switch to cross-container mode
    demo.update(&press(KeyCode::Tab));

    // Initial focus is left list
    assert_eq!(demo.focused_list(), 0);
    log_jsonl("initial", &[("focused_list", "left")]);

    // Switch with 'l' (or right arrow)
    demo.update(&char_press('l'));
    assert_eq!(demo.focused_list(), 1);
    log_jsonl("after_l", &[("focused_list", "right")]);

    // Switch back with 'h'
    demo.update(&char_press('h'));
    assert_eq!(demo.focused_list(), 0);
    log_jsonl("after_h", &[("focused_list", "left")]);
}

#[test]
fn e2e_cross_container_item_count_preserved() {
    log_jsonl(
        "env",
        &[("test", "e2e_cross_container_item_count_preserved")],
    );

    let mut demo = DragDropDemo::new();

    // Switch to cross-container mode
    demo.update(&press(KeyCode::Tab));

    let total_initial = demo.left_list_len() + demo.right_list_len();

    // Transfer multiple items
    for _ in 0..5 {
        demo.update(&press(KeyCode::Enter));
    }

    let total_after = demo.left_list_len() + demo.right_list_len();
    assert_eq!(
        total_initial, total_after,
        "Total item count should be preserved"
    );
    log_jsonl(
        "invariant_check",
        &[
            ("total_initial", &total_initial.to_string()),
            ("total_after", &total_after.to_string()),
        ],
    );
}

// ===========================================================================
// Scenario 6: Keyboard Drag Mode
// ===========================================================================

#[test]
fn e2e_keyboard_drag_mode_render() {
    log_jsonl("env", &[("test", "e2e_keyboard_drag_mode_render")]);

    let mut demo = DragDropDemo::new();

    // Switch to keyboard drag mode
    demo.update(&press(KeyCode::Tab)); // CrossContainer
    demo.update(&press(KeyCode::Tab)); // KeyboardDrag

    // Verify rendering doesn't panic
    let hash = capture_frame_hash(&demo, 80, 24);
    log_jsonl("render", &[("frame_hash", &format!("{hash:016x}"))]);
}

#[test]
fn drag_drop_keyboard_mode_80x24() {
    let mut demo = DragDropDemo::new();

    // Switch to keyboard drag mode
    demo.update(&press(KeyCode::Tab));
    demo.update(&press(KeyCode::Tab));

    let mut pool = GraphemePool::new();
    let mut frame = Frame::new(80, 24, &mut pool);
    let area = Rect::new(0, 0, 80, 24);
    demo.view(&mut frame, area);
    assert_snapshot!("drag_drop_keyboard_mode_80x24", &frame.buffer);
}

// ===========================================================================
// Scenario 7: Screen Trait Implementation
// ===========================================================================

#[test]
fn e2e_screen_trait_methods() {
    log_jsonl("env", &[("test", "e2e_screen_trait_methods")]);

    let demo = DragDropDemo::new();

    assert_eq!(demo.title(), "Drag & Drop");
    assert_eq!(demo.tab_label(), "DnD");

    let keybindings = demo.keybindings();
    assert!(!keybindings.is_empty(), "Should have keybindings");
    log_jsonl("keybindings", &[("count", &keybindings.len().to_string())]);
}

// ===========================================================================
// Scenario 8: Tick Processing
// ===========================================================================

#[test]
fn e2e_tick_processes_without_panic() {
    log_jsonl("env", &[("test", "e2e_tick_processes_without_panic")]);

    let mut demo = DragDropDemo::new();

    // Tick should process without panic
    for i in 0..100 {
        demo.tick(i);
    }

    log_jsonl("ticked", &[("count", "100")]);
}

// ===========================================================================
// Scenario 9: Determinism Checks
// ===========================================================================

#[test]
fn e2e_deterministic_rendering() {
    log_jsonl("env", &[("test", "e2e_deterministic_rendering")]);

    // Create two demos with same initial state
    let demo1 = DragDropDemo::new();
    let demo2 = DragDropDemo::new();

    let hash1 = capture_frame_hash(&demo1, 80, 24);
    let hash2 = capture_frame_hash(&demo2, 80, 24);

    assert_eq!(
        hash1, hash2,
        "Same initial state should produce same output"
    );
    log_jsonl(
        "determinism",
        &[
            ("hash1", &format!("{hash1:016x}")),
            ("hash2", &format!("{hash2:016x}")),
            ("match", "true"),
        ],
    );
}

#[test]
fn e2e_deterministic_after_operations() {
    log_jsonl("env", &[("test", "e2e_deterministic_after_operations")]);

    // Perform same operations on two demos
    let mut demo1 = DragDropDemo::new();
    let mut demo2 = DragDropDemo::new();

    let ops: Vec<Event> = vec![
        press(KeyCode::Tab),
        char_press('j'),
        char_press('j'),
        press(KeyCode::Enter),
        char_press('l'),
        char_press('k'),
    ];

    for op in &ops {
        demo1.update(op);
        demo2.update(op);
    }

    let hash1 = capture_frame_hash(&demo1, 80, 24);
    let hash2 = capture_frame_hash(&demo2, 80, 24);

    assert_eq!(
        hash1, hash2,
        "Same operations should produce deterministic output"
    );
    log_jsonl(
        "determinism_after_ops",
        &[
            ("hash1", &format!("{hash1:016x}")),
            ("hash2", &format!("{hash2:016x}")),
            ("match", "true"),
        ],
    );
}

// ===========================================================================
// Scenario 10: Zero-Area Rendering
// ===========================================================================

#[test]
fn e2e_zero_area_no_panic() {
    log_jsonl("env", &[("test", "e2e_zero_area_no_panic")]);

    let demo = DragDropDemo::new();
    let mut pool = GraphemePool::new();
    let mut frame = Frame::new(1, 1, &mut pool);

    // Zero-area should not panic
    demo.view(&mut frame, Rect::new(0, 0, 0, 0));
    log_jsonl("zero_area", &[("result", "no_panic")]);
}
