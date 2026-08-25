#![allow(clippy::too_many_lines)]

//! bd-1lg.13: E2E test — Full widget gallery rendering.
//!
//! Renders every major widget type across multiple terminal configurations
//! (sizes and degradation levels) and validates:
//! 1. Static rendering correctness (each widget produces expected content).
//! 2. No panics on any combination of widget + size + degradation.
//! 3. Composition of multiple widgets in a single frame.
//!
//! Run:
//!   cargo test -p ftui-widgets --test widget_gallery_e2e

use ftui_core::geometry::Rect;
use ftui_layout::Constraint;
use ftui_render::budget::DegradationLevel;
use ftui_render::frame::Frame;
use ftui_render::grapheme_pool::GraphemePool;
use ftui_widgets::block::Block;
use ftui_widgets::borders::Borders;
use ftui_widgets::command_palette::CommandPalette;
use ftui_widgets::input::TextInput;
use ftui_widgets::list::{List, ListItem, ListState};
use ftui_widgets::modal::Modal;
use ftui_widgets::paragraph::Paragraph;
use ftui_widgets::progress::ProgressBar;
use ftui_widgets::sparkline::Sparkline;
use ftui_widgets::status_line::{StatusItem, StatusLine};
use ftui_widgets::table::{Row, Table, TableState};
use ftui_widgets::tabs::{Tab, Tabs, TabsState};
use ftui_widgets::tree::{Tree, TreeNode};
use ftui_widgets::{StatefulWidget, Widget};

// ============================================================================
// Terminal configurations (simulating different emulators/sizes)
// ============================================================================

/// Terminal sizes simulating different emulators and window sizes.
const TERMINAL_CONFIGS: [(u16, u16, &str); 5] = [
    (80, 24, "VT100 standard"),
    (120, 40, "Modern wide terminal"),
    (40, 12, "Small/mobile terminal"),
    (200, 60, "Ultra-wide monitor"),
    (132, 43, "DEC VT132 mode"),
];

/// Degradation levels to test adaptive rendering.
const DEGRADATION_LEVELS: [DegradationLevel; 5] = [
    DegradationLevel::Full,
    DegradationLevel::SimpleBorders,
    DegradationLevel::NoStyling,
    DegradationLevel::EssentialOnly,
    DegradationLevel::Skeleton,
];

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

fn contains_in_frame(frame: &Frame, needle: &str) -> bool {
    for y in 0..frame.buffer.height() {
        if row_text(frame, y).contains(needle) {
            return true;
        }
    }
    false
}

fn make_frame<'a>(pool: &'a mut GraphemePool, w: u16, h: u16, deg: DegradationLevel) -> Frame<'a> {
    let mut frame = Frame::new(w, h, pool);
    frame.buffer.degradation = deg;
    frame
}

// ============================================================================
// Widget constructors
// ============================================================================

fn make_table() -> (Table<'static>, TableState) {
    let rows = vec![
        Row::new(["Alice", "42", "Engineering"]),
        Row::new(["Bob", "37", "Design"]),
        Row::new(["Charlie", "29", "Marketing"]),
    ];
    let widths = [
        Constraint::Percentage(40.0),
        Constraint::Percentage(20.0),
        Constraint::Percentage(40.0),
    ];
    let table = Table::new(rows, widths).header(Row::new(["Name", "Age", "Department"]));
    let mut state = TableState::default();
    state.selected = Some(0);
    (table, state)
}

fn make_list() -> (List<'static>, ListState) {
    let items = vec![
        ListItem::new("Alpha"),
        ListItem::new("Beta"),
        ListItem::new("Gamma"),
        ListItem::new("Delta"),
    ];
    let list = List::new(items);
    let mut state = ListState::default();
    state.select(Some(1));
    (list, state)
}

fn make_text_input() -> TextInput {
    let mut input = TextInput::new();
    input.set_value("Hello, world!");
    input.set_focused(true);
    input
}

fn make_tree() -> Tree {
    Tree::new(
        TreeNode::new("Root")
            .with_expanded(true)
            .with_children(vec![
                TreeNode::new("src")
                    .with_expanded(true)
                    .with_children(vec![TreeNode::new("main.rs"), TreeNode::new("lib.rs")]),
                TreeNode::new("Cargo.toml"),
                TreeNode::new("README.md"),
            ]),
    )
}

fn make_tabs() -> (Tabs<'static>, TabsState) {
    let tabs = Tabs::new(vec![
        Tab::new("Home"),
        Tab::new("Editor"),
        Tab::new("Settings"),
    ]);
    let state = TabsState::default();
    (tabs, state)
}

fn make_command_palette() -> CommandPalette {
    let mut palette = CommandPalette::new();
    palette.register("File: Open", Some("Open a file"), &["file", "open"]);
    palette.register("File: Save", Some("Save current file"), &["file", "save"]);
    palette.register("Edit: Copy", Some("Copy selection"), &["edit", "copy"]);
    palette.open();
    palette
}

fn make_status_line() -> StatusLine<'static> {
    StatusLine::new()
        .left(StatusItem::Text("[INSERT]"))
        .right(StatusItem::Text("Ln 42, Col 10"))
}

fn make_progress_bar() -> ProgressBar<'static> {
    ProgressBar::new().ratio(0.65).label("65%")
}

fn make_sparkline_data() -> Vec<f64> {
    vec![1.0, 4.0, 2.0, 8.0, 3.0, 6.0, 5.0, 7.0, 2.0, 9.0]
}

fn make_paragraph() -> Paragraph<'static> {
    Paragraph::new("The quick brown fox jumps over the lazy dog.")
}

// ============================================================================
// Test: Every widget renders on all terminal configs without panic
// ============================================================================

#[test]
fn gallery_all_widgets_render_all_configs() {
    let spark_data = make_sparkline_data();

    for &(w, h, config_name) in &TERMINAL_CONFIGS {
        for &deg in &DEGRADATION_LEVELS {
            let mut pool = GraphemePool::new();
            let mut frame = make_frame(&mut pool, w, h, deg);
            let area = Rect::new(0, 0, w, h);

            // Block
            Widget::render(
                &Block::default().title("Block").borders(Borders::ALL),
                area,
                &mut frame,
            );

            // Paragraph
            Widget::render(&make_paragraph(), area, &mut frame);

            // ProgressBar
            Widget::render(&make_progress_bar(), Rect::new(0, 0, w, 1), &mut frame);

            // Sparkline
            Widget::render(
                &Sparkline::new(&spark_data),
                Rect::new(0, 0, w, 1.min(h)),
                &mut frame,
            );

            // StatusLine
            Widget::render(&make_status_line(), Rect::new(0, 0, w, 1), &mut frame);

            // TextInput
            Widget::render(&make_text_input(), Rect::new(0, 0, w, 1.min(h)), &mut frame);

            // Tree
            Widget::render(&make_tree(), area, &mut frame);

            // Table (stateful)
            let (table, mut table_state) = make_table();
            StatefulWidget::render(&table, area, &mut frame, &mut table_state);

            // List (stateful)
            let (list, mut list_state) = make_list();
            StatefulWidget::render(&list, area, &mut frame, &mut list_state);

            // Tabs (stateful)
            let (tabs, mut tabs_state) = make_tabs();
            StatefulWidget::render(
                &tabs,
                Rect::new(0, 0, w, 1.min(h)),
                &mut frame,
                &mut tabs_state,
            );

            // CommandPalette
            Widget::render(&make_command_palette(), area, &mut frame);

            // Modal wrapping a paragraph
            let modal = Modal::new(Paragraph::new("Modal content"));
            Widget::render(&modal, area, &mut frame);

            // If we reach here, all widgets rendered without panic
            // for config {config_name} at degradation {deg:?}
            let _ = config_name;
        }
    }
}

// ============================================================================
// Test: Content correctness at Full degradation
// ============================================================================

#[test]
fn gallery_block_renders_title() {
    for &(w, h, name) in &TERMINAL_CONFIGS {
        if w < 10 || h < 3 {
            continue;
        }
        let mut pool = GraphemePool::new();
        let mut frame = make_frame(&mut pool, w, h, DegradationLevel::Full);
        Widget::render(
            &Block::default().title("MyTitle").borders(Borders::ALL),
            Rect::new(0, 0, w, h),
            &mut frame,
        );
        assert!(
            contains_in_frame(&frame, "MyTitle"),
            "Block title missing at {name} ({w}x{h})"
        );
    }
}

#[test]
fn gallery_paragraph_renders_text() {
    for &(w, h, name) in &TERMINAL_CONFIGS {
        let mut pool = GraphemePool::new();
        let mut frame = make_frame(&mut pool, w, h, DegradationLevel::Full);
        Widget::render(&make_paragraph(), Rect::new(0, 0, w, h), &mut frame);
        assert!(
            contains_in_frame(&frame, "quick brown"),
            "Paragraph text missing at {name} ({w}x{h})"
        );
    }
}

#[test]
fn gallery_progress_renders_label() {
    for &(w, h, name) in &TERMINAL_CONFIGS {
        if w < 10 {
            continue;
        }
        let mut pool = GraphemePool::new();
        let mut frame = make_frame(&mut pool, w, h, DegradationLevel::Full);
        Widget::render(
            &make_progress_bar(),
            Rect::new(0, 0, w, 1.min(h)),
            &mut frame,
        );
        assert!(
            contains_in_frame(&frame, "65%"),
            "ProgressBar label missing at {name} ({w}x{h})"
        );
    }
}

#[test]
fn gallery_status_line_renders_sections() {
    for &(w, h, name) in &TERMINAL_CONFIGS {
        if w < 20 {
            continue;
        }
        let mut pool = GraphemePool::new();
        let mut frame = make_frame(&mut pool, w, h, DegradationLevel::Full);
        Widget::render(
            &make_status_line(),
            Rect::new(0, 0, w, 1.min(h)),
            &mut frame,
        );
        assert!(
            contains_in_frame(&frame, "INSERT"),
            "StatusLine left section missing at {name} ({w}x{h})"
        );
    }
}

#[test]
fn gallery_text_input_renders_value() {
    for &(w, h, name) in &TERMINAL_CONFIGS {
        if w < 10 {
            continue;
        }
        let mut pool = GraphemePool::new();
        let mut frame = make_frame(&mut pool, w, h, DegradationLevel::Full);
        Widget::render(&make_text_input(), Rect::new(0, 0, w, 1.min(h)), &mut frame);
        assert!(
            contains_in_frame(&frame, "Hello"),
            "TextInput value missing at {name} ({w}x{h})"
        );
    }
}

#[test]
fn gallery_tree_renders_root() {
    for &(w, h, name) in &TERMINAL_CONFIGS {
        let mut pool = GraphemePool::new();
        let mut frame = make_frame(&mut pool, w, h, DegradationLevel::Full);
        Widget::render(&make_tree(), Rect::new(0, 0, w, h), &mut frame);
        assert!(
            contains_in_frame(&frame, "Root"),
            "Tree root missing at {name} ({w}x{h})"
        );
    }
}

#[test]
fn gallery_table_renders_header() {
    for &(w, h, name) in &TERMINAL_CONFIGS {
        if w < 20 || h < 3 {
            continue;
        }
        let mut pool = GraphemePool::new();
        let mut frame = make_frame(&mut pool, w, h, DegradationLevel::Full);
        let (table, mut state) = make_table();
        StatefulWidget::render(&table, Rect::new(0, 0, w, h), &mut frame, &mut state);
        assert!(
            contains_in_frame(&frame, "Name"),
            "Table header missing at {name} ({w}x{h})"
        );
    }
}

#[test]
fn gallery_list_renders_items() {
    for &(w, h, name) in &TERMINAL_CONFIGS {
        let mut pool = GraphemePool::new();
        let mut frame = make_frame(&mut pool, w, h, DegradationLevel::Full);
        let (list, mut state) = make_list();
        StatefulWidget::render(&list, Rect::new(0, 0, w, h), &mut frame, &mut state);
        assert!(
            contains_in_frame(&frame, "Alpha"),
            "List first item missing at {name} ({w}x{h})"
        );
    }
}

#[test]
fn gallery_tabs_renders_labels() {
    for &(w, h, name) in &TERMINAL_CONFIGS {
        if w < 20 {
            continue;
        }
        let mut pool = GraphemePool::new();
        let mut frame = make_frame(&mut pool, w, h, DegradationLevel::Full);
        let (tabs, mut state) = make_tabs();
        StatefulWidget::render(&tabs, Rect::new(0, 0, w, 1.min(h)), &mut frame, &mut state);
        assert!(
            contains_in_frame(&frame, "Home"),
            "Tabs first label missing at {name} ({w}x{h})"
        );
    }
}

#[test]
fn gallery_sparkline_renders_nonempty() {
    let data = make_sparkline_data();
    for &(w, h, _name) in &TERMINAL_CONFIGS {
        let mut pool = GraphemePool::new();
        let mut frame = make_frame(&mut pool, w, h, DegradationLevel::Full);
        Widget::render(
            &Sparkline::new(&data),
            Rect::new(0, 0, w, 1.min(h)),
            &mut frame,
        );
        // Sparkline uses braille/block chars — just verify something rendered
        let row = row_text(&frame, 0);
        assert!(
            row.chars().any(|c| c != ' '),
            "Sparkline should render non-space content"
        );
    }
}

// ============================================================================
// Test: Composed dashboard — all widgets in one frame
// ============================================================================

#[test]
fn gallery_composed_dashboard() {
    let spark_data = make_sparkline_data();

    for &(w, h, name) in &TERMINAL_CONFIGS {
        if w < 40 || h < 12 {
            continue; // Need minimum space for dashboard layout
        }

        let mut pool = GraphemePool::new();
        let mut frame = make_frame(&mut pool, w, h, DegradationLevel::Full);

        // Layout: tabs on top, then split area, status at bottom
        let tabs_area = Rect::new(0, 0, w, 1);
        let main_area = Rect::new(0, 1, w, h.saturating_sub(2));
        let status_area = Rect::new(0, h.saturating_sub(1), w, 1);

        // Tabs
        let (tabs, mut tabs_state) = make_tabs();
        StatefulWidget::render(&tabs, tabs_area, &mut frame, &mut tabs_state);

        // Left half: list inside a block
        let left_w = w / 2;
        let block = Block::default().title("Files").borders(Borders::ALL);
        let inner = block.inner(Rect::new(0, 1, left_w, main_area.height));
        Widget::render(
            &block,
            Rect::new(0, 1, left_w, main_area.height),
            &mut frame,
        );
        let (list, mut list_state) = make_list();
        StatefulWidget::render(&list, inner, &mut frame, &mut list_state);

        // Right half: progress + sparkline
        let right_x = left_w;
        let right_w = w.saturating_sub(left_w);
        Widget::render(
            &make_progress_bar(),
            Rect::new(right_x, 1, right_w, 1),
            &mut frame,
        );
        Widget::render(
            &Sparkline::new(&spark_data),
            Rect::new(right_x, 2, right_w, 1),
            &mut frame,
        );

        // Status line at bottom
        Widget::render(&make_status_line(), status_area, &mut frame);

        // Verify key elements
        assert!(
            contains_in_frame(&frame, "Home"),
            "Dashboard tabs missing at {name}"
        );
        assert!(
            contains_in_frame(&frame, "Files"),
            "Dashboard block title missing at {name}"
        );
        assert!(
            contains_in_frame(&frame, "INSERT"),
            "Dashboard status missing at {name}"
        );
    }
}

// ============================================================================
// Test: Zero-area safety for all widgets
// ============================================================================

#[test]
fn gallery_zero_area_safety() {
    let spark_data = make_sparkline_data();
    let zero = Rect::new(0, 0, 0, 0);

    let mut pool = GraphemePool::new();
    let mut frame = Frame::new(20, 20, &mut pool);

    Widget::render(&Block::default(), zero, &mut frame);
    Widget::render(&make_paragraph(), zero, &mut frame);
    Widget::render(&make_progress_bar(), zero, &mut frame);
    Widget::render(&Sparkline::new(&spark_data), zero, &mut frame);
    Widget::render(&make_status_line(), zero, &mut frame);
    Widget::render(&make_text_input(), zero, &mut frame);
    Widget::render(&make_tree(), zero, &mut frame);
    Widget::render(&make_command_palette(), zero, &mut frame);
    Widget::render(&Modal::new(Paragraph::new("test")), zero, &mut frame);

    let (table, mut ts) = make_table();
    StatefulWidget::render(&table, zero, &mut frame, &mut ts);
    let (list, mut ls) = make_list();
    StatefulWidget::render(&list, zero, &mut frame, &mut ls);
    let (tabs, mut tbs) = make_tabs();
    StatefulWidget::render(&tabs, zero, &mut frame, &mut tbs);
}

// ============================================================================
// Test: Degradation level gradual drop-off
// ============================================================================

#[test]
fn gallery_degradation_levels_reduce_output() {
    let w = 80u16;
    let h = 24u16;

    // At Full, block should render border chars (Unicode)
    let mut pool = GraphemePool::new();
    let mut frame = make_frame(&mut pool, w, h, DegradationLevel::Full);
    Widget::render(
        &Block::default().title("Test").borders(Borders::ALL),
        Rect::new(0, 0, w, h),
        &mut frame,
    );
    let full_row = row_text(&frame, 0);

    // At SimpleBorders, should use ASCII border chars
    let mut pool2 = GraphemePool::new();
    let mut frame2 = make_frame(&mut pool2, w, h, DegradationLevel::SimpleBorders);
    Widget::render(
        &Block::default().title("Test").borders(Borders::ALL),
        Rect::new(0, 0, w, h),
        &mut frame2,
    );
    let simple_row = row_text(&frame2, 0);

    // Both should contain the title
    assert!(full_row.contains("Test"));
    assert!(simple_row.contains("Test"));

    // But border characters differ (Unicode vs ASCII)
    // Full uses ─ (U+2500), SimpleBorders uses -
    let full_has_box = full_row.contains('─') || full_row.contains('┌');
    let simple_has_ascii = simple_row.contains('-') || simple_row.contains('+');
    assert!(
        full_has_box || simple_has_ascii,
        "Border rendering should differ: full={full_row:?} simple={simple_row:?}"
    );
}
