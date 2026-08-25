use ftui_core::geometry::Rect;
use ftui_render::cell::PackedRgba;
use ftui_render::frame::Frame;
use ftui_render::grapheme_pool::GraphemePool;
use ftui_style::Style;
use ftui_widgets::Widget;
use ftui_widgets::textarea::TextArea;

#[test]
fn test_wide_char_scroll_gap() {
    // 1. Setup TextArea with a wide character at the start
    // U+3000 is Ideographic Space (width 2)
    let wide_char = "\u{3000}";
    let mut ta = TextArea::new().with_text(wide_char);

    // 2. Set style to have a background color so we can see the gap
    let bg_color = PackedRgba::rgb(0, 0, 255); // Blue
    ta = ta.with_style(Style::default().bg(bg_color));

    // 3. Scroll 1 column to the right
    // We can't directly set scroll_left, but we can force it via cursor.
    // Viewport width 2.
    // Move cursor to col 2 (after the wide char).
    // Ensure visibility.
    // visual_col is 2.
    // If we have a viewport of width 2...
    // Logic: if visual_col (2) >= scroll_left + width (2) -> scroll_left = 2 - 2 + 1 = 1.

    // We need to access internal state or render to update state.
    // Render first to set up state.
    let mut pool = GraphemePool::new();
    let mut frame = Frame::new(2, 1, &mut pool);
    let area = Rect::new(0, 0, 2, 1);

    ta.move_to_line_end(); // Cursor at (0, 1) grapheme index (visual 2)

    // First render to establish viewport size and update scroll
    // Note: TextArea updates scroll during render based on cursor visibility
    Widget::render(&ta, area, &mut frame);

    // Now check the buffer.
    // With scroll_left = 1:
    // Col 0: Should show right half of wide char.
    // Col 1: Should be empty (past end of line).

    // Check cell at (0, 0)
    let cell = frame.buffer.get(0, 0).unwrap();

    // CURRENT BEHAVIOR (Expected Failure):
    // The wide char starts at visual 0. scroll_left is 1.
    // 0 < 1, so it's skipped.
    // Nothing is drawn at (0,0). Cell remains default (empty, no bg).
    // So bg should be default, not Blue.

    // DESIRED BEHAVIOR:
    // The cell at (0,0) should have the Blue background, even if content is space/empty.

    assert_eq!(
        cell.bg, bg_color,
        "Cell at (0,0) should have blue background from wide char tail"
    );
}
