use ftui_text::editor::Editor;
use ftui_text::rope::Rope;
use unicode_segmentation::UnicodeSegmentation;

#[test]
fn rope_grapheme_count_matches_unicode_segmentation() {
    let text =
        "a\u{0301}e\u{0301}o\u{0301} \u{1F468}\u{200D}\u{1F469}\u{200D}\u{1F467}\u{200D}\u{1F466}";
    let rope = Rope::from(text);

    let expected_count = text.graphemes(true).count();
    assert_eq!(rope.grapheme_count(), expected_count);
}

#[test]
fn editor_cursor_movement_respects_graphemes() {
    let text = "a\u{0301}bc"; // á b c (3 graphemes)
    let mut editor = Editor::with_text(text);

    editor.move_to_document_start();
    assert_eq!(editor.cursor().grapheme, 0);

    editor.move_right();
    // Should skip 'a' and combining accent (2 chars, 1 grapheme)
    assert_eq!(editor.cursor().grapheme, 1);

    editor.move_right();
    assert_eq!(editor.cursor().grapheme, 2);

    editor.move_right();
    assert_eq!(editor.cursor().grapheme, 3);

    // Boundary check
    editor.move_right();
    assert_eq!(editor.cursor().grapheme, 3);
}

#[test]
fn editor_delete_respects_graphemes() {
    // woman + zwj + rocket = woman astronaut (1 grapheme)
    let text = "Start \u{1F469}\u{200D}\u{1F680} End";
    let mut editor = Editor::with_text(text);

    // Move after emoji
    editor.move_to_document_start();
    for _ in 0..7 {
        editor.move_right();
    }

    // Delete the whole emoji sequence
    editor.delete_backward();

    assert_eq!(editor.text(), "Start  End"); // double space remains
}
