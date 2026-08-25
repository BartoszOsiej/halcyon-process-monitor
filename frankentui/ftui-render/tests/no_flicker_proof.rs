#![forbid(unsafe_code)]

//! Render correctness smoke tests.
//!
//! NOTE: Extensive flicker/ghosting proofs moved to
//! `crates/ftui-harness/tests/render_no_flicker_proof.rs` to avoid
//! the ftui-render ↔ ftui-harness dev-dep cycle (bd-3abcg).

use ftui_render::buffer::Buffer;
use ftui_render::cell::{Cell, PackedRgba};
use ftui_render::diff::BufferDiff;
use ftui_render::presenter::{Presenter, TerminalCapabilities};

#[test]
fn diff_is_empty_for_identical_buffers() {
    let buffer = Buffer::new(12, 4);
    let diff = BufferDiff::compute(&buffer, &buffer);
    assert!(
        diff.is_empty(),
        "diff should be empty for identical buffers"
    );
}

#[test]
fn presenter_handles_colored_cells() {
    let mut buffer = Buffer::new(8, 2);
    buffer.set_raw(
        0,
        0,
        Cell::from_char('C').with_fg(PackedRgba::rgb(200, 50, 50)),
    );
    let blank = Buffer::new(8, 2);
    let diff = BufferDiff::compute(&blank, &buffer);

    let mut sink = Vec::new();
    let mut presenter = Presenter::new(&mut sink, TerminalCapabilities::basic());
    presenter.present(&buffer, &diff).expect("presenter failed");
    drop(presenter);

    assert!(!sink.is_empty(), "presenter should emit bytes");
    assert!(
        sink.contains(&b'C'),
        "rendered content should appear in output"
    );
}
