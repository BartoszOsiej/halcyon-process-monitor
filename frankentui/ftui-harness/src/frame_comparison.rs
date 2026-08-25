#![forbid(unsafe_code)]

//! Golden frame comparison for deterministic replay verification (bd-3mjjt.3).
//!
//! Compares rendered frame buffers cell-by-cell against golden references,
//! reporting per-frame PASS/FAIL with detailed mismatch information.
//!
//! # Design
//!
//! - Cell-by-cell comparison using [`Cell::bits_eq`] for cache-efficient checks.
//! - Terminal size mismatch detection with graceful clipped comparison.
//! - BLAKE3 checksums for fast equality checks before cell enumeration.
//! - JSONL-compatible structured output for CI and debugging.
//!
//! # Example
//!
//! ```ignore
//! use ftui_harness::frame_comparison::{FrameComparator, ComparisonResult};
//! use ftui_render::buffer::Buffer;
//!
//! let expected = Buffer::new(80, 24);
//! let actual = Buffer::new(80, 24);
//! let result = FrameComparator::compare(&expected, &actual);
//! assert!(result.pass);
//! ```

use ftui_render::buffer::Buffer;
use ftui_render::cell::Cell;

use crate::golden::compute_buffer_checksum;

// ---------------------------------------------------------------------------
// Result types
// ---------------------------------------------------------------------------

/// A single cell mismatch between expected and actual buffers.
#[derive(Debug, Clone)]
pub struct CellMismatch {
    /// Column (0-indexed).
    pub x: u16,
    /// Row (0-indexed).
    pub y: u16,
    /// Expected cell content.
    pub expected: CellSnapshot,
    /// Actual cell content.
    pub actual: CellSnapshot,
}

/// Snapshot of a cell's visual state for mismatch reporting.
#[derive(Debug, Clone)]
pub struct CellSnapshot {
    /// Raw content value (char or grapheme index).
    pub content_raw: u32,
    /// Character representation, if available.
    pub char_repr: Option<char>,
    /// Foreground color (packed RGBA).
    pub fg: u32,
    /// Background color (packed RGBA).
    pub bg: u32,
    /// Attribute flags byte.
    pub attrs: u8,
}

impl CellSnapshot {
    /// Create a snapshot from a [`Cell`].
    #[must_use]
    pub fn from_cell(cell: &Cell) -> Self {
        Self {
            content_raw: cell.content.raw(),
            char_repr: cell.content.as_char(),
            fg: cell.fg.0,
            bg: cell.bg.0,
            attrs: cell.attrs.flags().bits(),
        }
    }
}

/// Size mismatch information.
#[derive(Debug, Clone, Copy)]
pub struct SizeMismatch {
    pub expected_width: u16,
    pub expected_height: u16,
    pub actual_width: u16,
    pub actual_height: u16,
}

/// Result of comparing a single frame.
#[derive(Debug, Clone)]
pub struct ComparisonResult {
    /// Frame identifier (caller-provided).
    pub frame_id: u32,
    /// Whether the frame matches.
    pub pass: bool,
    /// BLAKE3 checksum of the expected buffer.
    pub expected_checksum: String,
    /// BLAKE3 checksum of the actual buffer.
    pub actual_checksum: String,
    /// Size mismatch, if any.
    pub size_mismatch: Option<SizeMismatch>,
    /// Number of mismatched cells.
    pub mismatch_count: usize,
    /// Detailed mismatches (capped at `max_mismatches`).
    pub mismatches: Vec<CellMismatch>,
    /// Total cells compared.
    pub cells_compared: usize,
}

impl ComparisonResult {
    /// Format a human-readable summary.
    #[must_use]
    pub fn summary(&self) -> String {
        if self.pass {
            format!(
                "Frame {}: PASS ({} cells, checksum {})",
                self.frame_id, self.cells_compared, self.actual_checksum
            )
        } else if let Some(sz) = self.size_mismatch {
            format!(
                "Frame {}: FAIL (size mismatch: expected {}x{}, actual {}x{}, {} cell mismatches in clipped region)",
                self.frame_id,
                sz.expected_width,
                sz.expected_height,
                sz.actual_width,
                sz.actual_height,
                self.mismatch_count
            )
        } else {
            format!(
                "Frame {}: FAIL ({} mismatches out of {} cells)",
                self.frame_id, self.mismatch_count, self.cells_compared
            )
        }
    }

    /// Format detailed mismatch report for debugging.
    #[must_use]
    pub fn detail_report(&self) -> String {
        let mut out = self.summary();
        if !self.mismatches.is_empty() {
            out.push('\n');
            for m in &self.mismatches {
                let exp_char = m
                    .expected
                    .char_repr
                    .map_or("?".to_string(), |c| format!("'{c}'"));
                let act_char = m
                    .actual
                    .char_repr
                    .map_or("?".to_string(), |c| format!("'{c}'"));
                out.push_str(&format!(
                    "  [{},{}] expected: {} (fg={:#010x} bg={:#010x} a={:#04x}) actual: {} (fg={:#010x} bg={:#010x} a={:#04x})\n",
                    m.x, m.y,
                    exp_char, m.expected.fg, m.expected.bg, m.expected.attrs,
                    act_char, m.actual.fg, m.actual.bg, m.actual.attrs,
                ));
            }
            if self.mismatch_count > self.mismatches.len() {
                out.push_str(&format!(
                    "  ... and {} more mismatches\n",
                    self.mismatch_count - self.mismatches.len()
                ));
            }
        }
        out
    }
}

// ---------------------------------------------------------------------------
// Comparator
// ---------------------------------------------------------------------------

/// Compares frame buffers for golden-frame verification.
pub struct FrameComparator {
    /// Maximum number of cell mismatches to capture in detail.
    pub max_mismatches: usize,
}

impl Default for FrameComparator {
    fn default() -> Self {
        Self { max_mismatches: 50 }
    }
}

impl FrameComparator {
    /// Create a comparator with the default mismatch limit (50).
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Set the maximum number of cell mismatches to capture.
    #[must_use]
    pub fn with_max_mismatches(mut self, max: usize) -> Self {
        self.max_mismatches = max;
        self
    }

    /// Compare two buffers, returning a detailed result.
    #[must_use]
    pub fn compare_buffers(
        &self,
        frame_id: u32,
        expected: &Buffer,
        actual: &Buffer,
    ) -> ComparisonResult {
        let expected_checksum = compute_buffer_checksum(expected);
        let actual_checksum = compute_buffer_checksum(actual);

        // Fast path: checksums match means identical.
        if expected_checksum == actual_checksum {
            let cells = expected.width() as usize * expected.height() as usize;
            return ComparisonResult {
                frame_id,
                pass: true,
                expected_checksum,
                actual_checksum,
                size_mismatch: None,
                mismatch_count: 0,
                mismatches: Vec::new(),
                cells_compared: cells,
            };
        }

        // Check for size mismatch.
        let size_mismatch =
            if expected.width() != actual.width() || expected.height() != actual.height() {
                Some(SizeMismatch {
                    expected_width: expected.width(),
                    expected_height: expected.height(),
                    actual_width: actual.width(),
                    actual_height: actual.height(),
                })
            } else {
                None
            };

        // Cell-by-cell comparison over the overlapping region.
        let cmp_width = expected.width().min(actual.width());
        let cmp_height = expected.height().min(actual.height());
        let mut mismatches = Vec::new();
        let mut mismatch_count = 0;

        for y in 0..cmp_height {
            for x in 0..cmp_width {
                let exp_cell = expected.get(x, y);
                let act_cell = actual.get(x, y);

                let equal = match (exp_cell, act_cell) {
                    (Some(e), Some(a)) => e.bits_eq(a),
                    (None, None) => true,
                    _ => false,
                };

                if !equal {
                    mismatch_count += 1;
                    if mismatches.len() < self.max_mismatches {
                        let default_cell = Cell::default();
                        mismatches.push(CellMismatch {
                            x,
                            y,
                            expected: CellSnapshot::from_cell(exp_cell.unwrap_or(&default_cell)),
                            actual: CellSnapshot::from_cell(act_cell.unwrap_or(&default_cell)),
                        });
                    }
                }
            }
        }

        // If sizes differ, count cells outside the overlap as mismatches.
        if size_mismatch.is_some() {
            let exp_cells = expected.width() as usize * expected.height() as usize;
            let act_cells = actual.width() as usize * actual.height() as usize;
            let overlap_cells = cmp_width as usize * cmp_height as usize;
            // Extra cells in whichever buffer is larger.
            mismatch_count += exp_cells.max(act_cells) - overlap_cells;
        }

        let cells_compared = cmp_width as usize * cmp_height as usize;
        let pass = mismatch_count == 0 && size_mismatch.is_none();

        ComparisonResult {
            frame_id,
            pass,
            expected_checksum,
            actual_checksum,
            size_mismatch,
            mismatch_count,
            mismatches,
            cells_compared,
        }
    }

    /// Convenience: compare with frame_id=0.
    #[must_use]
    pub fn compare(&self, expected: &Buffer, actual: &Buffer) -> ComparisonResult {
        self.compare_buffers(0, expected, actual)
    }
}

// ---------------------------------------------------------------------------
// Multi-frame comparison
// ---------------------------------------------------------------------------

/// Result of comparing a sequence of frames.
#[derive(Debug, Clone)]
pub struct SequenceResult {
    /// Per-frame results.
    pub frames: Vec<ComparisonResult>,
    /// Number of passing frames.
    pub passed: usize,
    /// Number of failing frames.
    pub failed: usize,
}

impl SequenceResult {
    /// Overall pass (all frames match).
    #[must_use]
    pub fn all_pass(&self) -> bool {
        self.failed == 0
    }

    /// Format a summary for all frames.
    #[must_use]
    pub fn summary(&self) -> String {
        let mut out = format!(
            "Sequence: {} frames ({} pass, {} fail)\n",
            self.frames.len(),
            self.passed,
            self.failed
        );
        for result in &self.frames {
            out.push_str(&format!("  {}\n", result.summary()));
        }
        out
    }
}

/// Compare a sequence of (expected, actual) buffer pairs.
#[must_use]
pub fn compare_sequence(pairs: &[(&Buffer, &Buffer)], max_mismatches: usize) -> SequenceResult {
    let comparator = FrameComparator::new().with_max_mismatches(max_mismatches);
    let mut frames = Vec::with_capacity(pairs.len());
    let mut passed = 0;
    let mut failed = 0;

    for (i, (expected, actual)) in pairs.iter().enumerate() {
        let result = comparator.compare_buffers(i as u32, expected, actual);
        if result.pass {
            passed += 1;
        } else {
            failed += 1;
        }
        frames.push(result);
    }

    SequenceResult {
        frames,
        passed,
        failed,
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use ftui_render::buffer::Buffer;
    use ftui_render::cell::CellContent;
    use ftui_render::cell::PackedRgba;

    fn make_buffer_with_char(w: u16, h: u16, ch: char) -> Buffer {
        let mut buf = Buffer::new(w, h);
        for y in 0..h {
            for x in 0..w {
                if let Some(cell) = buf.get_mut(x, y) {
                    cell.content = CellContent::from_char(ch);
                }
            }
        }
        buf
    }

    #[test]
    fn identical_buffers_pass() {
        let a = make_buffer_with_char(10, 5, 'x');
        let b = make_buffer_with_char(10, 5, 'x');
        let result = FrameComparator::new().compare(&a, &b);
        assert!(result.pass);
        assert_eq!(result.mismatch_count, 0);
        assert_eq!(result.cells_compared, 50);
        assert_eq!(result.expected_checksum, result.actual_checksum);
    }

    #[test]
    fn empty_buffers_pass() {
        let a = Buffer::new(10, 5);
        let b = Buffer::new(10, 5);
        let result = FrameComparator::new().compare(&a, &b);
        assert!(result.pass);
    }

    #[test]
    fn single_cell_mismatch_detected() {
        let a = Buffer::new(10, 5);
        let mut b = Buffer::new(10, 5);
        if let Some(cell) = b.get_mut(3, 2) {
            cell.content = CellContent::from_char('Z');
        }
        let result = FrameComparator::new().compare(&a, &b);
        assert!(!result.pass);
        assert_eq!(result.mismatch_count, 1);
        assert_eq!(result.mismatches.len(), 1);
        assert_eq!(result.mismatches[0].x, 3);
        assert_eq!(result.mismatches[0].y, 2);
    }

    #[test]
    fn multiple_mismatches_capped_by_max() {
        let a = Buffer::new(10, 5);
        let b = make_buffer_with_char(10, 5, 'Z');
        let result = FrameComparator::new()
            .with_max_mismatches(5)
            .compare(&a, &b);
        assert!(!result.pass);
        assert_eq!(result.mismatch_count, 50);
        assert_eq!(result.mismatches.len(), 5); // capped
    }

    #[test]
    fn size_mismatch_detected() {
        let a = Buffer::new(10, 5);
        let b = Buffer::new(20, 10);
        let result = FrameComparator::new().compare(&a, &b);
        assert!(!result.pass);
        assert!(result.size_mismatch.is_some());
        let sz = result.size_mismatch.unwrap();
        assert_eq!(sz.expected_width, 10);
        assert_eq!(sz.expected_height, 5);
        assert_eq!(sz.actual_width, 20);
        assert_eq!(sz.actual_height, 10);
    }

    #[test]
    fn size_mismatch_with_identical_overlap() {
        // Same content in overlapping region but different sizes.
        let a = Buffer::new(5, 5);
        let b = Buffer::new(10, 5);
        let result = FrameComparator::new().compare(&a, &b);
        assert!(!result.pass);
        assert!(result.size_mismatch.is_some());
        // Overlap is 5x5=25, extra cells in b: 10*5 - 5*5 = 25.
        assert_eq!(result.mismatch_count, 25);
    }

    #[test]
    fn fg_color_mismatch() {
        let a = Buffer::new(10, 5);
        let mut b = Buffer::new(10, 5);
        if let Some(cell) = b.get_mut(0, 0) {
            cell.fg = PackedRgba::rgb(255, 0, 0);
        }
        let result = FrameComparator::new().compare(&a, &b);
        assert!(!result.pass);
        assert_eq!(result.mismatch_count, 1);
        assert_eq!(result.mismatches[0].x, 0);
        assert_eq!(result.mismatches[0].y, 0);
    }

    #[test]
    fn bg_color_mismatch() {
        let a = Buffer::new(5, 3);
        let mut b = Buffer::new(5, 3);
        if let Some(cell) = b.get_mut(2, 1) {
            cell.bg = PackedRgba::rgb(0, 0, 255);
        }
        let result = FrameComparator::new().compare(&a, &b);
        assert!(!result.pass);
        assert_eq!(result.mismatch_count, 1);
    }

    #[test]
    fn compare_buffers_with_frame_id() {
        let a = Buffer::new(5, 5);
        let b = Buffer::new(5, 5);
        let result = FrameComparator::new().compare_buffers(42, &a, &b);
        assert_eq!(result.frame_id, 42);
        assert!(result.pass);
    }

    #[test]
    fn summary_pass() {
        let a = Buffer::new(5, 5);
        let result = FrameComparator::new().compare(&a, &a);
        let summary = result.summary();
        assert!(summary.contains("PASS"));
        assert!(summary.contains("blake3:"));
    }

    #[test]
    fn summary_fail() {
        let a = Buffer::new(5, 5);
        let b = make_buffer_with_char(5, 5, 'Q');
        let result = FrameComparator::new().compare(&a, &b);
        let summary = result.summary();
        assert!(summary.contains("FAIL"));
        assert!(summary.contains("mismatches"));
    }

    #[test]
    fn detail_report_shows_mismatches() {
        let a = Buffer::new(5, 3);
        let mut b = Buffer::new(5, 3);
        if let Some(cell) = b.get_mut(1, 0) {
            cell.content = CellContent::from_char('A');
        }
        let result = FrameComparator::new().compare(&a, &b);
        let detail = result.detail_report();
        assert!(detail.contains("[1,0]"));
        assert!(detail.contains("'A'"));
    }

    #[test]
    fn sequence_all_pass() {
        let a = Buffer::new(10, 5);
        let b = Buffer::new(10, 5);
        let pairs: Vec<(&Buffer, &Buffer)> = vec![(&a, &b), (&a, &b), (&a, &b)];
        let result = compare_sequence(&pairs, 10);
        assert!(result.all_pass());
        assert_eq!(result.passed, 3);
        assert_eq!(result.failed, 0);
    }

    #[test]
    fn sequence_with_failure() {
        let a = Buffer::new(10, 5);
        let b = make_buffer_with_char(10, 5, 'Z');
        let pairs: Vec<(&Buffer, &Buffer)> = vec![(&a, &a), (&a, &b), (&a, &a)];
        let result = compare_sequence(&pairs, 10);
        assert!(!result.all_pass());
        assert_eq!(result.passed, 2);
        assert_eq!(result.failed, 1);
        assert_eq!(result.frames[1].frame_id, 1);
    }

    #[test]
    fn sequence_summary_format() {
        let a = Buffer::new(5, 5);
        let pairs: Vec<(&Buffer, &Buffer)> = vec![(&a, &a)];
        let result = compare_sequence(&pairs, 10);
        let summary = result.summary();
        assert!(summary.contains("1 frames"));
        assert!(summary.contains("1 pass"));
        assert!(summary.contains("0 fail"));
    }

    #[test]
    fn checksum_shortcircuit_on_match() {
        // Identical buffers should match by checksum without cell-by-cell.
        let a = make_buffer_with_char(80, 24, 'x');
        let b = make_buffer_with_char(80, 24, 'x');
        let result = FrameComparator::new().compare(&a, &b);
        assert!(result.pass);
        assert_eq!(result.mismatch_count, 0);
        assert_eq!(result.cells_compared, 80 * 24);
    }

    #[test]
    fn minimal_buffers_pass() {
        let a = Buffer::new(1, 1);
        let b = Buffer::new(1, 1);
        let result = FrameComparator::new().compare(&a, &b);
        assert!(result.pass);
        assert_eq!(result.cells_compared, 1);
    }
}
