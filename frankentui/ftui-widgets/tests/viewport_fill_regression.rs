#![forbid(unsafe_code)]

use ftui_core::geometry::Rect;
use ftui_layout::Constraint;
use ftui_render::buffer::Buffer;
use ftui_render::frame::Frame;
use ftui_render::grapheme_pool::GraphemePool;
use ftui_widgets::StatefulWidget;
use ftui_widgets::list::{List, ListItem, ListState};
use ftui_widgets::table::{Row, Table, TableState};

fn assert_buffer_content(buf: &Buffer, y: u16, expected: &str) {
    let width = buf.width();
    let mut actual = String::new();
    for x in 0..width {
        if let Some(cell) = buf.get(x, y) {
            if let Some(c) = cell.content.as_char() {
                actual.push(c);
            } else {
                actual.push(' ');
            }
        }
    }
    let actual = actual.trim_end();
    assert!(
        actual.starts_with(expected),
        "Row {y} mismatch: expected prefix {expected:?}, actual {actual:?}"
    );
}

#[test]
fn list_fills_viewport_on_resize() {
    // Setup: List with 10 items
    let items: Vec<ListItem> = (0..10)
        .map(|i| ListItem::new(format!("Item {}", i)))
        .collect();
    let list = List::new(items);

    let mut pool = GraphemePool::new();
    let mut state = ListState::default();

    // 1. Render with small height, scrolled to bottom
    // Height 3. Items 0..9.
    // We want to show 7, 8, 9. Offset should be 7.
    state.offset = 7;

    let area = Rect::new(0, 0, 10, 3);
    let mut frame = Frame::new(10, 3, &mut pool);

    StatefulWidget::render(&list, area, &mut frame, &mut state);

    // Verify we see 7, 8, 9
    assert_eq!(state.offset, 7);
    assert_buffer_content(&frame.buffer, 0, "Item 7");
    assert_buffer_content(&frame.buffer, 2, "Item 9");

    // 2. Resize to height 5.
    // If we keep offset 7, we show 7, 8, 9, empty, empty.
    // We SHOULD auto-adjust offset to 5, showing 5, 6, 7, 8, 9.

    let area_large = Rect::new(0, 0, 10, 5);
    let mut frame_large = Frame::new(10, 5, &mut pool);

    StatefulWidget::render(&list, area_large, &mut frame_large, &mut state);

    // Verify offset was adjusted
    assert_eq!(state.offset, 5, "Offset should adjust to fill viewport");
    assert_buffer_content(&frame_large.buffer, 0, "Item 5");
    assert_buffer_content(&frame_large.buffer, 4, "Item 9");
}

#[test]
fn table_fills_viewport_on_resize() {
    // Setup: Table with 10 rows
    let rows: Vec<Row> = (0..10)
        .map(|i| Row::new(vec![format!("Row {}", i)]))
        .collect();
    let table = Table::new(rows, vec![Constraint::Min(10)]);

    let mut pool = GraphemePool::new();
    let mut state = TableState::default();

    // 1. Render with small height (3 rows), scrolled to bottom
    // We want to show 7, 8, 9. Offset 7.
    state.offset = 7;

    let area = Rect::new(0, 0, 10, 3);
    let mut frame = Frame::new(10, 3, &mut pool);

    StatefulWidget::render(&table, area, &mut frame, &mut state);

    assert_eq!(state.offset, 7);
    assert_buffer_content(&frame.buffer, 0, "Row 7");

    // 2. Resize to height 5.
    // Should adjust offset to 5.

    let area_large = Rect::new(0, 0, 10, 5);
    let mut frame_large = Frame::new(10, 5, &mut pool);

    StatefulWidget::render(&table, area_large, &mut frame_large, &mut state);

    assert_eq!(state.offset, 5, "Offset should adjust to fill viewport");
    assert_buffer_content(&frame_large.buffer, 0, "Row 5");
    assert_buffer_content(&frame_large.buffer, 4, "Row 9");
}

#[test]
fn table_fills_viewport_with_variable_heights() {
    // Row 0..8: height 1
    // Row 9: height 5
    // Total rows: 10.
    // If we scroll to show Row 9 at bottom.
    // View height 10.
    // Row 9 takes 5.
    // Rows 8, 7, 6, 5, 4 take 1 each.
    // Total 10 lines.
    // Visible rows: 4, 5, 6, 7, 8, 9.
    // Offset should be 4.

    let mut rows: Vec<Row> = (0..9)
        .map(|i| Row::new(vec![format!("Row {}", i)]))
        .collect();
    rows.push(Row::new(vec!["Row 9"]).height(5));

    let table = Table::new(rows, vec![Constraint::Min(10)]);

    let mut pool = GraphemePool::new();
    let mut state = TableState::default();

    // Start with offset at end (9)
    state.offset = 9;

    let area = Rect::new(0, 0, 10, 10);
    let mut frame = Frame::new(10, 10, &mut pool);

    StatefulWidget::render(&table, area, &mut frame, &mut state);

    // Offset 9 means we show Row 9 (5 lines). 5 lines empty.
    // Fix should pull offset back to 4.
    assert_eq!(state.offset, 4, "Offset should adjust to show more context");
    assert_buffer_content(&frame.buffer, 0, "Row 4");
}
