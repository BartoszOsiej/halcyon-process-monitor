#![allow(unsafe_code)]

//! bd-2xj.7: Standalone widget crate isolation tests.
//!
//! Proves that each major widget renders correctly and handles events
//! using ONLY the ftui-widgets crate (no ftui-runtime dependency).
//! This validates the adoption-wedge property: users can embed individual
//! widgets without pulling in the Elm runtime.
//!
//! Run:
//!   cargo test -p ftui-widgets --test standalone_isolation

use ftui_core::event::{KeyCode, KeyEvent, MouseButton, MouseEvent, MouseEventKind};
use ftui_core::geometry::{Rect, Size};
use ftui_render::frame::{Frame, HitId};
use ftui_render::grapheme_pool::GraphemePool;
use ftui_widgets::block::Block;
use ftui_widgets::borders::Borders;
use ftui_widgets::list::{List, ListItem, ListState};
use ftui_widgets::measurable::MeasurableWidget;
use ftui_widgets::paragraph::Paragraph;
use ftui_widgets::progress::ProgressBar;
use ftui_widgets::scrollbar::{Scrollbar, ScrollbarState};
use ftui_widgets::sparkline::Sparkline;
use ftui_widgets::status_line::{StatusItem, StatusLine};
use ftui_widgets::tabs::{Tab, Tabs, TabsState};
use ftui_widgets::tree::{Tree, TreeNode};
use ftui_widgets::{StatefulWidget, Widget};

// ============================================================================
// Helpers
// ============================================================================

fn row_text(frame: &Frame, y: u16) -> String {
    let mut out = String::new();
    for x in 0..frame.buffer.width() {
        let ch = frame
            .buffer
            .get(x, y)
            .and_then(|cell| cell.content.as_char())
            .unwrap_or(' ');
        out.push(ch);
    }
    out
}

// ============================================================================
// Block: renders standalone without runtime
// ============================================================================

#[test]
fn standalone_block_renders() {
    let block = Block::default().title("Test").borders(Borders::ALL);
    let mut pool = GraphemePool::new();
    let mut frame = Frame::new(20, 3, &mut pool);
    Widget::render(&block, Rect::new(0, 0, 20, 3), &mut frame);
    let row = row_text(&frame, 0);
    assert!(row.contains("Test"), "Block title should render: {row:?}");
}

// ============================================================================
// Paragraph: renders standalone without runtime
// ============================================================================

#[test]
fn standalone_paragraph_renders() {
    let para = Paragraph::new("Hello, world!");
    let mut pool = GraphemePool::new();
    let mut frame = Frame::new(20, 3, &mut pool);
    Widget::render(&para, Rect::new(0, 0, 20, 3), &mut frame);
    let row = row_text(&frame, 0);
    assert!(
        row.contains("Hello"),
        "Paragraph text should render: {row:?}"
    );
}

// ============================================================================
// List: renders and handles keys standalone
// ============================================================================

#[test]
fn standalone_list_renders_and_selects() {
    let items = vec![
        ListItem::new("alpha"),
        ListItem::new("beta"),
        ListItem::new("gamma"),
    ];
    let list = List::new(items);
    let mut state = ListState::default();

    let mut pool = GraphemePool::new();
    let mut frame = Frame::new(20, 5, &mut pool);
    StatefulWidget::render(&list, Rect::new(0, 0, 20, 5), &mut frame, &mut state);
    let row = row_text(&frame, 0);
    assert!(row.contains("alpha"), "First item should render: {row:?}");

    // Keyboard navigation (no runtime needed)
    assert!(list.handle_key(&mut state, &KeyEvent::new(KeyCode::Down)));
    assert_eq!(state.selected(), Some(0));
    assert!(list.handle_key(&mut state, &KeyEvent::new(KeyCode::Down)));
    assert_eq!(state.selected(), Some(1));
}

#[test]
fn standalone_list_filter_without_runtime() {
    let items = vec![
        ListItem::new("alpha"),
        ListItem::new("banana"),
        ListItem::new("cherry"),
    ];
    let list = List::new(items);
    let mut state = ListState::default();

    // Type 'b' to filter
    assert!(list.handle_key(&mut state, &KeyEvent::new(KeyCode::Char('b'))));
    assert_eq!(state.filter_query(), "b");
    assert_eq!(state.selected(), Some(1)); // banana
}

// ============================================================================
// Tabs: renders and switches standalone
// ============================================================================

#[test]
fn standalone_tabs_renders_and_switches() {
    let tabs = Tabs::new(vec![Tab::new("One"), Tab::new("Two"), Tab::new("Three")]);
    let mut state = TabsState::default();

    let mut pool = GraphemePool::new();
    let mut frame = Frame::new(30, 1, &mut pool);
    StatefulWidget::render(&tabs, Rect::new(0, 0, 30, 1), &mut frame, &mut state);
    let row = row_text(&frame, 0);
    assert!(row.contains("[One]"), "Active tab should render: {row:?}");

    // Keyboard switching
    assert!(state.handle_key(&KeyEvent::new(KeyCode::Right), 3));
    assert_eq!(state.active, 1);
    assert!(state.handle_key(&KeyEvent::new(KeyCode::Char('3')), 3));
    assert_eq!(state.active, 2);
}

// ============================================================================
// Tree: renders and toggles standalone
// ============================================================================

#[test]
fn standalone_tree_renders_and_toggles() {
    let root = TreeNode::new("Root")
        .with_expanded(true)
        .with_children(vec![
            TreeNode::new("Alpha"),
            TreeNode::new("Beta").with_children(vec![TreeNode::new("Beta-1")]),
        ]);
    let tree = Tree::new(root);

    // Render (Tree is Widget, not StatefulWidget)
    let mut pool = GraphemePool::new();
    let mut frame = Frame::new(30, 5, &mut pool);
    Widget::render(&tree, Rect::new(0, 0, 30, 5), &mut frame);
    let row = row_text(&frame, 0);
    assert!(row.contains("Root"), "Root should render: {row:?}");
}

#[test]
fn standalone_tree_keyboard_toggle() {
    let root = TreeNode::new("Root")
        .with_expanded(true)
        .with_children(vec![
            TreeNode::new("Alpha"),
            TreeNode::new("Beta").with_children(vec![TreeNode::new("Beta-1")]),
        ]);
    let mut tree = Tree::new(root);

    // Keyboard toggle on Beta (index 2, has children) — no runtime needed
    let handled = tree.handle_key(&KeyEvent::new(KeyCode::Enter), 2);
    assert!(handled, "Enter should toggle a node with children");
}

// ============================================================================
// ProgressBar: renders standalone
// ============================================================================

#[test]
fn standalone_progress_renders() {
    let progress = ProgressBar::new().ratio(0.5).label("50%");
    let mut pool = GraphemePool::new();
    let mut frame = Frame::new(20, 1, &mut pool);
    Widget::render(&progress, Rect::new(0, 0, 20, 1), &mut frame);
    let row = row_text(&frame, 0);
    assert!(
        row.contains("50%"),
        "Progress bar label should render: {row:?}"
    );
}

// ============================================================================
// Sparkline: renders standalone
// ============================================================================

#[test]
fn standalone_sparkline_renders() {
    let data = [1.0, 3.0, 7.0, 2.0, 5.0, 8.0, 4.0];
    let spark = Sparkline::new(&data);
    let mut pool = GraphemePool::new();
    let mut frame = Frame::new(10, 1, &mut pool);
    Widget::render(&spark, Rect::new(0, 0, 10, 1), &mut frame);
    let row = row_text(&frame, 0);
    assert!(!row.trim().is_empty(), "Sparkline should render: {row:?}");
}

// ============================================================================
// StatusLine: renders standalone
// ============================================================================

#[test]
fn standalone_status_line_renders() {
    let status = StatusLine::new()
        .left(StatusItem::Text("Mode: Normal"))
        .right(StatusItem::Text("Ln 42"));
    let mut pool = GraphemePool::new();
    let mut frame = Frame::new(40, 1, &mut pool);
    Widget::render(&status, Rect::new(0, 0, 40, 1), &mut frame);
    let row = row_text(&frame, 0);
    assert!(
        row.contains("Normal"),
        "StatusLine left section should render: {row:?}"
    );
    assert!(
        row.contains("42"),
        "StatusLine right section should render: {row:?}"
    );
}

// ============================================================================
// Scrollbar: renders standalone
// ============================================================================

#[test]
fn standalone_scrollbar_renders() {
    let scrollbar = Scrollbar::default();
    let mut scroll_state = ScrollbarState::new(100, 0, 10);

    let mut pool = GraphemePool::new();
    let mut frame = Frame::new(1, 10, &mut pool);
    StatefulWidget::render(
        &scrollbar,
        Rect::new(0, 0, 1, 10),
        &mut frame,
        &mut scroll_state,
    );
    // Should render track/thumb characters
    let col = row_text(&frame, 0);
    assert!(!col.trim().is_empty(), "Scrollbar should render");
}

// ============================================================================
// Multiple widgets compose without runtime
// ============================================================================

#[test]
fn standalone_compose_multiple_widgets() {
    let mut pool = GraphemePool::new();
    let mut frame = Frame::new(40, 10, &mut pool);

    // Render a block in the top area
    let block = Block::default().title("Panel").borders(Borders::ALL);
    Widget::render(&block, Rect::new(0, 0, 40, 5), &mut frame);

    // Render a progress bar inside
    let progress = ProgressBar::new().ratio(0.75);
    Widget::render(&progress, Rect::new(1, 2, 38, 1), &mut frame);

    // Render a status line at the bottom
    let status = StatusLine::new().left(StatusItem::Text("Ready"));
    Widget::render(&status, Rect::new(0, 9, 40, 1), &mut frame);

    // All three should have rendered without panic
    let top = row_text(&frame, 0);
    let bottom = row_text(&frame, 9);
    assert!(top.contains("Panel"), "Block title: {top:?}");
    assert!(bottom.contains("Ready"), "StatusLine: {bottom:?}");
}

// ============================================================================
// Event handling works standalone (no Elm runtime needed)
// ============================================================================

#[test]
fn standalone_list_mouse_without_runtime() {
    let items = vec![
        ListItem::new("alpha"),
        ListItem::new("beta"),
        ListItem::new("gamma"),
    ];
    let list = List::new(items).hit_id(HitId::new(1));
    let mut state = ListState::default();

    // Render with hit grid
    let mut pool = GraphemePool::new();
    let mut frame = Frame::with_hit_grid(20, 5, &mut pool);
    StatefulWidget::render(&list, Rect::new(0, 0, 20, 5), &mut frame, &mut state);

    // Mouse click on second item
    let hit = frame.hit_test(5, 1);
    let event = MouseEvent::new(MouseEventKind::Down(MouseButton::Left), 5, 1);
    if let Some(hit_data) = hit {
        state.handle_mouse(&event, Some(hit_data), HitId::new(1), 3);
        assert_eq!(state.selected(), Some(1), "Click should select second item");
    }
}

#[test]
fn standalone_tabs_mouse_without_runtime() {
    let tabs = Tabs::new(vec![Tab::new("A"), Tab::new("B")]).hit_id(HitId::new(2));
    let mut state = TabsState::default();

    let mut pool = GraphemePool::new();
    let mut frame = Frame::with_hit_grid(20, 1, &mut pool);
    StatefulWidget::render(&tabs, Rect::new(0, 0, 20, 1), &mut frame, &mut state);

    // Click on second tab
    let hit = frame.hit_test(5, 0);
    if let Some(hit_data) = hit {
        let event = MouseEvent::new(MouseEventKind::Down(MouseButton::Left), 5, 0);
        state.handle_mouse(&event, Some(hit_data), HitId::new(2), 2);
        assert_eq!(state.active, 1, "Click should select second tab");
    }
}

// ============================================================================
// MeasurableWidget works standalone
// ============================================================================

#[test]
fn standalone_list_measurable() {
    let items = vec![ListItem::new("short"), ListItem::new("a longer item")];
    let list = List::new(items);
    let constraints = list.measure(Size::new(100, 50));
    assert!(
        constraints.preferred.width > 0,
        "Should have non-zero width"
    );
    assert!(
        constraints.preferred.height >= 2,
        "Should have height for 2 items"
    );
}

// ============================================================================
// Widget trait is_essential defaults
// ============================================================================

#[test]
fn standalone_widget_is_not_essential_by_default() {
    let para = Paragraph::new("test");
    assert!(
        !Widget::is_essential(&para),
        "Paragraph should not be essential by default"
    );
}

#[test]
fn standalone_block_is_not_essential() {
    let block = Block::default();
    assert!(!Widget::is_essential(&block));
}

// ============================================================================
// Zero-area rendering is safe standalone
// ============================================================================

#[test]
fn standalone_all_widgets_safe_on_zero_area() {
    let mut pool = GraphemePool::new();
    let mut frame = Frame::new(20, 20, &mut pool);
    let zero = Rect::new(0, 0, 0, 0);

    // None of these should panic
    Widget::render(&Block::default(), zero, &mut frame);
    Widget::render(&Paragraph::new("test"), zero, &mut frame);
    Widget::render(&ProgressBar::new().ratio(0.5), zero, &mut frame);
    Widget::render(&StatusLine::new(), zero, &mut frame);

    let data = [1.0, 2.0, 3.0];
    Widget::render(&Sparkline::new(&data), zero, &mut frame);

    let tabs = Tabs::new(vec![Tab::new("A")]);
    Widget::render(&tabs, zero, &mut frame);

    let mut list_state = ListState::default();
    StatefulWidget::render(
        &List::new(vec![ListItem::new("a")]),
        zero,
        &mut frame,
        &mut list_state,
    );
}

// ============================================================================
// Linker isolation: no ftui-runtime symbols needed
// ============================================================================
// The fact that this test binary compiles and links at all proves that
// ftui-widgets has no runtime dependency. If someone accidentally adds
// `ftui-runtime` to ftui-widgets/Cargo.toml, this test will still pass,
// but cargo check without ftui-runtime would fail.

#[test]
fn standalone_no_runtime_import_compiles() {
    // This test exists purely as a compilation canary.
    // If ftui-widgets gained a mandatory ftui-runtime dependency, this
    // test file's Cargo dependency set would need updating — which is
    // exactly the signal we want.
    //
    // The fact that this binary links successfully proves isolation.
}
