use ftui_core::geometry::Rect;
use ftui_render::buffer::Buffer;
use ftui_render::cell::Cell;

#[test]
fn copy_from_slices_wide_char_start() {
    let mut src = Buffer::new(5, 1);
    // Write wide char at 0 (takes 0 and 1)
    src.set(0, 0, Cell::from_char('中'));

    let mut dst = Buffer::new(5, 1);
    // Copy from x=1 (the tail/continuation)
    dst.copy_from(&src, Rect::new(1, 0, 4, 1), 0, 0);

    // dst[0] should be EMPTY, not CONTINUATION
    // because we didn't copy the head.
    let cell = dst.get(0, 0).unwrap();
    assert!(
        !cell.is_continuation(),
        "Orphan continuation created in dst!"
    );
    assert!(cell.is_empty(), "Should be empty (cleaned up)");
}

#[test]
fn copy_from_slices_wide_char_end() {
    let mut src = Buffer::new(5, 1);
    src.set(0, 0, Cell::from_char('中'));

    let mut dst = Buffer::new(5, 1);
    // Copy only the head (x=0, width=1) - tail doesn't fit in copy region
    dst.copy_from(&src, Rect::new(0, 0, 1, 1), 0, 0);

    // With atomic rejection: wide char head should NOT be written if tail is clipped.
    // This prevents orphan continuations and partial wide chars.
    assert!(
        dst.get(0, 0).unwrap().is_empty(),
        "Wide char head should not be written if tail is clipped"
    );
    assert!(
        dst.get(1, 0).unwrap().is_empty(),
        "No continuation should exist since head was rejected"
    );
}
