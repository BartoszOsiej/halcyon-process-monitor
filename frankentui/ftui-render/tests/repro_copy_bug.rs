#[cfg(test)]
mod tests {
    use ftui_core::geometry::Rect;
    use ftui_render::buffer::Buffer;
    use ftui_render::cell::Cell;

    #[test]
    fn copy_from_leaves_artifacts_on_atomic_rejection() {
        let mut src = Buffer::new(10, 1);
        // Wide char at x=0 (width 2)
        src.set(0, 0, Cell::from_char('中'));

        let mut dst = Buffer::new(10, 1);
        // Pre-fill dst with 'X'
        dst.set(0, 0, Cell::from_char('X'));

        // Copy only the first column (x=0, width=1) from src to dst at (0,0)
        // This includes the head of '中' but EXCLUDES the tail.
        // The write should be rejected atomically.
        dst.copy_from(&src, Rect::new(0, 0, 1, 1), 0, 0);

        // Expectation: 'X' should be overwritten with empty cell (or the wide char if it fit, but it doesn't).
        // Actual behavior suspected: 'X' remains because `set` rejected the write and `copy_from` didn't clear.
        let cell = dst.get(0, 0).unwrap();

        // If the cell is 'X', the bug is present.
        // If the cell is empty (default), the bug is fixed/not present.
        assert!(
            cell.is_empty(),
            "Expected empty cell (cleared artifact), found {:?}",
            cell.content.as_char()
        );
    }
}
