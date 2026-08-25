use ftui_render::buffer::Buffer;
use ftui_render::cell::Cell;
use ftui_render::diff::BufferDiff;
use ftui_render::presenter::{Presenter, TerminalCapabilities};
use std::time::Instant;

#[test]
fn regression_plan_row_quadratic_perf() {
    // Regression test for O(N^2) blowup in cost_model::plan_row.
    // See bd-4kq0.2.2 optimization.

    // Large width to create many runs.
    // 20,000 width => 10,000 runs.
    // Naive DP is O(runs^2). 10,000^2 = 100,000,000 iterations (~200-500ms).
    // Optimized O(N) should be < 50ms.
    let width = 20000u16;
    let height = 1u16;
    let mut buffer = Buffer::new(width, height);

    // Checkerboard pattern: X . X . X . ...
    for x in (0..width).step_by(2) {
        buffer.set_raw(x, 0, Cell::from_char('X'));
    }

    let old = Buffer::new(width, height);
    let diff = BufferDiff::compute(&old, &buffer);

    let caps = TerminalCapabilities::basic();
    let mut presenter = Presenter::new(Vec::new(), caps);

    let start = Instant::now();
    presenter.present(&buffer, &diff).unwrap();
    let elapsed = start.elapsed();

    println!("Elapsed: {:?}", elapsed);

    // Assert fast execution (linear time)
    // 100ms is a very generous budget for 20k cells if O(N).
    assert!(
        elapsed.as_millis() < 100,
        "Presenter took too long: {:?} (quadratic blowup?)",
        elapsed
    );
}
