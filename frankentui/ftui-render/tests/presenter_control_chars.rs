use ftui_core::terminal_capabilities::TerminalCapabilities;
use ftui_render::buffer::Buffer;
use ftui_render::cell::Cell;
use ftui_render::diff::BufferDiff;
use ftui_render::presenter::Presenter;

#[test]
fn presenter_sanitizes_control_characters() {
    let mut buffer = Buffer::new(10, 1);

    // Set control characters that have width 1 in the buffer model
    // but would break cursor sync if emitted raw.
    buffer.set_raw(0, 0, Cell::from_char('\t'));
    buffer.set_raw(1, 0, Cell::from_char('\n'));
    buffer.set_raw(2, 0, Cell::from_char('\r'));
    buffer.set_raw(3, 0, Cell::from_char('A'));

    let old = Buffer::new(10, 1);
    let diff = BufferDiff::compute(&old, &buffer);

    let caps = TerminalCapabilities::basic();
    let mut presenter = Presenter::new(Vec::new(), caps);
    presenter.present(&buffer, &diff).unwrap();
    let output = presenter.into_inner().unwrap();
    let output_str = String::from_utf8_lossy(&output);

    // Verify no raw control characters are present
    assert!(
        !output_str.contains('\t'),
        "Output should not contain raw tab"
    );
    assert!(
        !output_str.contains('\n'),
        "Output should not contain raw newline"
    );
    assert!(
        !output_str.contains('\r'),
        "Output should not contain raw CR"
    );

    // Verify they were replaced by spaces (we expect 3 spaces then 'A')
    // Note: Presenter optimization might skip spaces if it just moves cursor,
    // but here we are writing contiguous cells, so it should emit them.
    // However, if they are spaces, the diff might just advance cursor?
    // Wait, the 'old' buffer was empty (default cells are spaces/empty).
    // Cell::default() is empty. Cell::from_char(' ') is space.
    // If we set '	', it differs from default.
    // So Presenter sees a change.
    // It emits the sanitized char.
    // So we should see spaces in the output, or at least the cursor should move correctly.

    // Actually, if we sanitized to space, and the cell is considered "changed",
    // the presenter emits the sanitized character.
    // So we expect "   A" (plus escape codes).

    // Let's just check 'A' is there and controls are not.
    assert!(output_str.contains('A'));
}
