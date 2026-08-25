#![forbid(unsafe_code)]

//! Render pipeline smoke tests.
//!
//! NOTE: Full flicker/tear integration tests live in
//! `crates/ftui-harness/tests/render_flicker_integration.rs`.
//! They were moved to break the ftui-render ↔ ftui-harness dev-dep cycle (bd-3abcg).

use ftui_render::buffer::Buffer;
use ftui_render::cell::Cell;
use ftui_render::diff::BufferDiff;
use ftui_render::presenter::{Presenter, TerminalCapabilities};

fn present_to_bytes(buffer: &Buffer, diff: &BufferDiff) -> Vec<u8> {
    let mut sink = Vec::new();
    let mut presenter = Presenter::new(&mut sink, TerminalCapabilities::basic());
    presenter.present(buffer, diff).expect("presenter failed");
    drop(presenter);
    sink
}

#[test]
fn presenter_emits_bytes_for_simple_frame() {
    let mut buffer = Buffer::new(20, 4);
    buffer.set_raw(0, 0, Cell::from_char('X'));
    let blank = Buffer::new(20, 4);
    let diff = BufferDiff::compute(&blank, &buffer);
    let output = present_to_bytes(&buffer, &diff);
    assert!(!output.is_empty(), "presenter output should not be empty");
    assert!(
        output.contains(&b'X'),
        "output should contain rendered content"
    );
}

#[test]
fn diff_roundtrip_is_consistent() {
    let mut before = Buffer::new(10, 3);
    let mut after = Buffer::new(10, 3);
    before.set_raw(1, 1, Cell::from_char('A'));
    after.set_raw(1, 1, Cell::from_char('B'));

    let diff = BufferDiff::compute(&before, &after);
    let output = present_to_bytes(&after, &diff);
    assert!(
        output.contains(&b'B'),
        "output should reflect updated content"
    );
}
