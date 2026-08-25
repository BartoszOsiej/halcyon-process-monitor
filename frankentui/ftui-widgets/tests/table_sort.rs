use ftui_core::geometry::Rect;
use ftui_layout::Constraint;
use ftui_render::buffer::Buffer;
use ftui_render::frame::Frame;
use ftui_render::grapheme_pool::GraphemePool;
use ftui_widgets::StatefulWidget;
use ftui_widgets::table::{Row, Table, TableState};

fn cell_text(buf: &Buffer, x: u16, y: u16) -> Option<char> {
    buf.get(x, y).and_then(|c| c.content.as_char())
}

#[test]
fn test_table_sort_ascending() {
    let rows = vec![
        Row::new(vec!["Bravo"]),
        Row::new(vec!["Alpha"]),
        Row::new(vec!["Charlie"]),
    ];
    let table = Table::new(rows, vec![Constraint::Fixed(10)]);

    let mut pool = GraphemePool::new();
    let mut frame = Frame::new(10, 3, &mut pool);
    let mut state = TableState::default();

    // Sort by column 0 ascending
    state.set_sort(Some(0), true);

    StatefulWidget::render(&table, Rect::new(0, 0, 10, 3), &mut frame, &mut state);

    // Expected: Alpha, Bravo, Charlie
    assert_eq!(cell_text(&frame.buffer, 0, 0), Some('A'));
    assert_eq!(cell_text(&frame.buffer, 0, 1), Some('B'));
    assert_eq!(cell_text(&frame.buffer, 0, 2), Some('C'));
}

#[test]
fn test_table_sort_descending() {
    let rows = vec![
        Row::new(vec!["Bravo"]),
        Row::new(vec!["Alpha"]),
        Row::new(vec!["Charlie"]),
    ];
    let table = Table::new(rows, vec![Constraint::Fixed(10)]);

    let mut pool = GraphemePool::new();
    let mut frame = Frame::new(10, 3, &mut pool);
    let mut state = TableState::default();

    // Sort by column 0 descending
    state.set_sort(Some(0), false);

    StatefulWidget::render(&table, Rect::new(0, 0, 10, 3), &mut frame, &mut state);

    // Expected: Charlie, Bravo, Alpha
    assert_eq!(cell_text(&frame.buffer, 0, 0), Some('C'));
    assert_eq!(cell_text(&frame.buffer, 0, 1), Some('B'));
    assert_eq!(cell_text(&frame.buffer, 0, 2), Some('A'));
}
