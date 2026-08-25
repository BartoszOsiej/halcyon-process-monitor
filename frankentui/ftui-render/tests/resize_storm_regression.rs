#![forbid(unsafe_code)]

//! Resize storm regression smoke test.
//!
//! NOTE: Full resize storm regression coverage moved to
//! `crates/ftui-harness/tests/render_resize_storm_regression.rs` to avoid
//! the ftui-render ↔ ftui-harness dev-dep cycle (bd-3abcg).

use ftui_render::buffer::AdaptiveDoubleBuffer;

fn burst_sizes(count: usize, base: (u16, u16)) -> Vec<(u16, u16)> {
    (0..count)
        .map(|i| {
            let w = base.0.saturating_add((i % 5) as u16);
            let h = base.1.saturating_add((i % 3) as u16);
            (w, h)
        })
        .collect()
}

#[test]
fn adaptive_double_buffer_survives_burst_resizes() {
    let mut buffer = AdaptiveDoubleBuffer::new(80, 24);
    let sizes = burst_sizes(64, (80, 24));
    for (w, h) in sizes {
        buffer.resize(w, h);
    }
    let ratio = buffer.stats().avoidance_ratio();
    assert!(
        (0.0..=1.0).contains(&ratio),
        "avoidance ratio should be sane"
    );
}
