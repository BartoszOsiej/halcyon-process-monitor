use ftui_core::geometry::Rect;
use ftui_render::frame::Frame;
use ftui_render::grapheme_pool::GraphemePool;
use ftui_widgets::StatefulWidget;
use ftui_widgets::list::{List, ListItem, ListState};

#[test]
fn list_panic_on_empty_items_with_selection() {
    let items: Vec<ListItem> = vec![];
    let list = List::new(items);
    let area = Rect::new(0, 0, 10, 10);
    let mut pool = GraphemePool::new();
    let mut frame = Frame::new(10, 10, &mut pool);

    let mut state = ListState::default();
    state.select(Some(0)); // Select index 0 in an empty list

    // This should NOT panic. The state should be updated to handle empty list (selected -> None).
    StatefulWidget::render(&list, area, &mut frame, &mut state);

    assert_eq!(state.selected(), None);
}
