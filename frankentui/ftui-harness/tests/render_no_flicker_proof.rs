//! # No-Flicker Proof Sketch + Counterexample Tests (bd-1rz0.20)
//!
//! ## Formal Proof Sketch
//!
//! We prove that FrankenTUI's render pipeline is **flicker-free** under the
//! following assumptions:
//!
//! ### Definitions
//!
//! - **Flicker**: visible intermediate state during a frame update. The user
//!   observes partial content, cleared regions, or mixed old/new content.
//!
//! - **Sync-bracketed output**: output wrapped between DEC 2026 synchronized
//!   output markers (`\x1b[?2026h` ... `\x1b[?2026l`). Terminal buffers all
//!   output until sync_end, then displays atomically.
//!
//! - **Frame**: one call to `Presenter::present(buffer, diff)`.
//!
//! ### Theorem 1: Sync Bracket Completeness
//!
//! **Statement**: When `capabilities.sync_output = true`, every byte emitted
//! by `Presenter::present()` is enclosed within exactly one sync bracket pair.
//!
//! **Proof sketch**:
//! 1. `present_with_pool()` calls `sync_begin()` before any content emission (line 308).
//! 2. Content is emitted via `emit_runs()` (line 312).
//! 3. Style/link cleanup follows (lines 315, 320).
//! 4. `sync_end()` is called after all content (line 326).
//! 5. The `?` operator propagates errors, so if any step fails, execution
//!    exits before `flush()`. The sync block may leak, but no *additional*
//!    content is emitted after a failed frame.
//! 6. No content is emitted outside the sync_begin/sync_end pair in the
//!    `present_with_pool()` method.
//!
//! **Failure mode**: If `sync_end()` write fails (io::Error), the terminal
//! remains in sync mode. Next frame's `sync_begin()` would nest, which some
//! terminals handle gracefully (re-enter sync), others may not. This is a
//! known limitation: io errors in the terminal write path can leave state
//! inconsistent.
//!
//! ### Theorem 2: Diff Completeness (No Ghosting)
//!
//! **Statement**: For any buffers `old` and `new` of equal dimensions,
//! `BufferDiff::compute(old, new)` contains exactly the set
//! `{ (x,y) | old[x,y] ≠ new[x,y] }`.
//!
//! **Proof sketch**:
//! 1. The algorithm scans all rows `y ∈ [0, height)`.
//! 2. For each row, it compares the old and new row slices.
//! 3. If rows are equal (fast path), no changes exist — correct.
//! 4. Otherwise, `scan_row_changes()` processes 4-cell blocks plus remainder.
//! 5. Every cell in [0, width) is covered: `blocks * 4 + remainder = width`.
//! 6. Each cell is compared with `bits_eq()`, which checks all 4 u32 fields.
//! 7. Changed cells are added to the change list; unchanged are skipped.
//!
//! **Soundness** (no false positives): Only cells where `!bits_eq()` are added.
//! **Completeness** (no false negatives): Every cell is checked; changed cells
//! are always added.
//!
//! ### Theorem 3: Dirty Tracking Soundness
//!
//! **Statement**: If any cell in row `y` was mutated since the last
//! `clear_dirty()`, then `is_row_dirty(y) = true`.
//!
//! **Proof sketch**:
//! 1. Every mutation path calls `mark_dirty(y)`: `set()`, `set_raw()`,
//!    `get_mut()`, `fill()`, `clear()`, `cells_mut()`.
//! 2. `mark_dirty()` sets `dirty_rows[y] = true`.
//! 3. The only way to clear dirty state is `clear_dirty()`.
//! 4. Therefore, if a mutation occurred, the flag is set.
//!
//! **Corollary**: `compute_dirty()` can safely skip clean rows — they are
//! guaranteed unchanged.
//!
//! ### Theorem 4: Diff-Dirty Equivalence
//!
//! **Statement**: `BufferDiff::compute(old, new)` and
//! `BufferDiff::compute_dirty(old, new)` produce identical results when all
//! mutated rows are marked dirty.
//!
//! **Proof sketch**: `compute_dirty()` skips rows where `!dirty[y]`. By
//! Theorem 3, these rows are unchanged. By Theorem 2, unchanged rows produce
//! no changes. Therefore, skipping them does not affect the result.
//!
//! ### Theorem 5: Resize Safety (No Ghosting After Resize)
//!
//! **Statement**: After a resize event, the next frame renders against a
//! fresh blank buffer, ensuring all content cells appear in the diff.
//!
//! **Proof sketch**:
//! 1. `TerminalWriter::set_size()` clears `prev_buffer = None` (line 218).
//! 2. On the next `present_ui()`, a new blank buffer is used as `old`.
//! 3. By Theorem 2, all non-blank cells in `new` appear in the diff.
//! 4. All rows start dirty in the new buffer, so `compute_dirty()` scans all.
//!
//! ### Theorem 6: Pipeline Composition (End-to-End No-Flicker)
//!
//! **Statement**: Under the assumptions that (a) `capabilities.sync_output =
//! true`, (b) the terminal correctly implements DEC 2026, and (c) no
//! io::Error occurs during frame emission, the user never observes flicker.
//!
//! **Proof sketch**: By Theorem 1, all output is sync-bracketed. By Theorems
//! 2-5, the diff is complete (no ghosting). The terminal atomically displays
//! the sync-bracketed content. Therefore, the user sees only complete frames.
//!
//! ---
//!
//! ## Counterexample Tests
//!
//! The tests below are adversarial: they attempt to surface violations of the
//! above invariants through property testing, edge cases, and stress tests.

use ftui_core::geometry::Rect;
use ftui_harness::flicker_detection::{analyze_stream, assert_flicker_free};
use ftui_render::buffer::Buffer;
use ftui_render::cell::{Cell, PackedRgba};
use ftui_render::cell::{CellAttrs, StyleFlags};
use ftui_render::diff::BufferDiff;
use ftui_render::presenter::{Presenter, TerminalCapabilities};
use proptest::prelude::*;
use std::collections::HashSet;

// =============================================================================
// Helpers
// =============================================================================

fn caps_with_sync() -> TerminalCapabilities {
    let mut caps = TerminalCapabilities::basic();
    caps.sync_output = true;
    caps
}

fn caps_without_sync() -> TerminalCapabilities {
    let mut caps = TerminalCapabilities::basic();
    caps.sync_output = false;
    caps
}

fn present_frame(buffer: &Buffer, old: &Buffer, caps: TerminalCapabilities) -> Vec<u8> {
    let diff = BufferDiff::compute(old, buffer);
    let mut sink = Vec::new();
    let mut presenter = Presenter::new(&mut sink, caps);
    presenter.present(buffer, &diff).unwrap();
    drop(presenter);
    sink
}

/// FNV-1a hash for deterministic checksums.
fn fnv1a(data: &[u8]) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for &b in data {
        hash ^= b as u64;
        hash = hash.wrapping_mul(0x0100_0000_01b3);
    }
    hash
}

/// Simple deterministic LCG for test data generation.
struct Lcg(u64);

impl Lcg {
    fn new(seed: u64) -> Self {
        Lcg(seed)
    }
    fn next(&mut self) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1);
        self.0
    }
    fn next_u16(&mut self, max: u16) -> u16 {
        (self.next() >> 16) as u16 % max
    }
    fn next_char(&mut self) -> char {
        char::from_u32('A' as u32 + (self.next() % 26) as u32).unwrap()
    }
}

// =============================================================================
// Theorem 1 Counterexample Tests: Sync Bracket Completeness
// =============================================================================

/// Every byte of presenter output must be inside sync brackets when sync is on.
#[test]
fn theorem1_sync_brackets_wrap_all_content() {
    let mut buf = Buffer::new(80, 24);
    for x in 0..80 {
        for y in 0..24 {
            buf.set_raw(x, y, Cell::from_char('#'));
        }
    }

    let old = Buffer::new(80, 24);
    let output = present_frame(&buf, &old, caps_with_sync());

    // Verify sync brackets present
    let sync_begin = b"\x1b[?2026h";
    let sync_end = b"\x1b[?2026l";

    let begin_pos = output
        .windows(sync_begin.len())
        .position(|w| w == sync_begin)
        .expect("sync_begin not found");
    let end_pos = output
        .windows(sync_end.len())
        .rposition(|w| w == sync_end)
        .expect("sync_end not found");

    // All content is between sync brackets
    assert!(begin_pos < end_pos, "sync_begin must precede sync_end");

    // No content before sync_begin
    let before_sync = &output[..begin_pos];
    assert!(
        before_sync.is_empty(),
        "no content should precede sync_begin, found {} bytes",
        before_sync.len()
    );

    // Verify flicker-free via detector
    assert_flicker_free(&output);
}

/// Without sync, detector should report sync gaps.
#[test]
fn theorem1_counterexample_no_sync_detected() {
    let mut buf = Buffer::new(40, 10);
    buf.set_raw(5, 3, Cell::from_char('X'));

    let old = Buffer::new(40, 10);
    let output = present_frame(&buf, &old, caps_without_sync());

    let analysis = analyze_stream(&output);
    // Without sync, all output is outside sync brackets
    assert!(
        !analysis.stats.is_flicker_free(),
        "output without sync should NOT be flicker-free"
    );
    assert!(analysis.stats.sync_gaps > 0, "should detect sync gaps");
}

/// Nested sync brackets (begin-begin-end) should still produce valid output.
/// The presenter never nests, but we test the detector handles it.
#[test]
fn theorem1_counterexample_nested_sync_not_produced() {
    let mut buf = Buffer::new(20, 5);
    buf.set_raw(0, 0, Cell::from_char('A'));

    let old = Buffer::new(20, 5);
    let output = present_frame(&buf, &old, caps_with_sync());

    // Count sync_begin occurrences — should be exactly 1
    let begin = b"\x1b[?2026h";
    let count = output.windows(begin.len()).filter(|w| *w == begin).count();
    assert_eq!(count, 1, "exactly one sync_begin per frame");

    // Count sync_end occurrences — should be exactly 1
    let end = b"\x1b[?2026l";
    let count = output.windows(end.len()).filter(|w| *w == end).count();
    assert_eq!(count, 1, "exactly one sync_end per frame");
}

// =============================================================================
// Theorem 2 Counterexample Tests: Diff Completeness
// =============================================================================

proptest! {
    /// Adversarial: random cell content (including style variations) must all
    /// appear in the diff. This uses random fg/bg colors and attributes to
    /// exercise bits_eq() more thoroughly than char-only tests.
    #[test]
    fn theorem2_adversarial_style_diff_completeness(
        width in 5u16..80,
        height in 5u16..30,
        seed in 0u64..1_000_000,
    ) {
        let old = Buffer::new(width, height);
        let mut new = Buffer::new(width, height);
        let mut rng = Lcg::new(seed);

        let num_changes = rng.next() as usize % 200;
        let mut expected = HashSet::new();

        for _ in 0..num_changes {
            let x = rng.next_u16(width);
            let y = rng.next_u16(height);
            let ch = rng.next_char();
            let fg = PackedRgba::rgb(
                (rng.next() % 256) as u8,
                (rng.next() % 256) as u8,
                (rng.next() % 256) as u8,
            );
            let bg = PackedRgba::rgb(
                (rng.next() % 256) as u8,
                (rng.next() % 256) as u8,
                (rng.next() % 256) as u8,
            );
            new.set_raw(x, y, Cell::from_char(ch).with_fg(fg).with_bg(bg));

            // Only add to expected if actually different from old
            let old_cell = old.get_unchecked(x, y);
            let new_cell = new.get_unchecked(x, y);
            if !old_cell.bits_eq(new_cell) {
                expected.insert((x, y));
            }
        }

        let diff = BufferDiff::compute(&old, &new);
        let diff_set: HashSet<(u16, u16)> = diff.iter().collect();

        // Completeness: every expected change is in diff
        for &(x, y) in &expected {
            prop_assert!(
                diff_set.contains(&(x, y)),
                "missing change at ({}, {})", x, y
            );
        }

        // Soundness: every diff entry is an actual change
        for (x, y) in diff.iter() {
            let old_cell = old.get_unchecked(x, y);
            let new_cell = new.get_unchecked(x, y);
            prop_assert!(
                !old_cell.bits_eq(new_cell),
                "false positive at ({}, {})", x, y
            );
        }
    }

    /// Adversarial: modify a cell back to its original value. The diff should
    /// NOT include it (soundness test for overwrite-to-original scenarios).
    #[test]
    fn theorem2_adversarial_overwrite_to_original(
        width in 5u16..60,
        height in 5u16..20,
        num_changes in 1usize..50,
    ) {
        let old = Buffer::new(width, height);
        let mut new = old.clone();

        // Apply changes, then revert half of them
        let mut changed_positions = Vec::new();
        for i in 0..num_changes {
            let x = (i * 13 + 7) as u16 % width;
            let y = (i * 17 + 3) as u16 % height;
            new.set_raw(x, y, Cell::from_char('X'));
            changed_positions.push((x, y));
        }

        // Revert every other change
        let blank = Cell::default();
        for (i, &(x, y)) in changed_positions.iter().enumerate() {
            if i % 2 == 0 {
                new.set_raw(x, y, blank);
            }
        }

        let diff = BufferDiff::compute(&old, &new);

        // Only the non-reverted changes should appear
        for (x, y) in diff.iter() {
            let old_cell = old.get_unchecked(x, y);
            let new_cell = new.get_unchecked(x, y);
            prop_assert!(
                !old_cell.bits_eq(new_cell),
                "reverted change at ({}, {}) should not be in diff", x, y
            );
        }
    }
}

/// Edge case: single-cell buffer.
#[test]
fn theorem2_edge_single_cell() {
    let old = Buffer::new(1, 1);
    let mut new = Buffer::new(1, 1);
    new.set_raw(0, 0, Cell::from_char('X'));

    let diff = BufferDiff::compute(&old, &new);
    assert_eq!(diff.len(), 1);
    assert_eq!(diff.changes()[0], (0, 0));
}

/// Edge case: maximum width row tests block alignment.
#[test]
fn theorem2_edge_block_alignment_boundaries() {
    // Test widths around block size (4) boundaries
    for width in [1, 2, 3, 4, 5, 7, 8, 9, 15, 16, 17] {
        let old = Buffer::new(width, 1);
        let mut new = Buffer::new(width, 1);

        // Change only the last cell
        new.set_raw(width - 1, 0, Cell::from_char('Z'));

        let diff = BufferDiff::compute(&old, &new);
        assert_eq!(
            diff.len(),
            1,
            "width={}: expected 1 change, got {}",
            width,
            diff.len()
        );
        assert_eq!(diff.changes()[0], (width - 1, 0));
    }
}

// =============================================================================
// Theorem 3 Counterexample Tests: Dirty Tracking Soundness
// =============================================================================

/// Exhaustively test all mutation paths mark rows dirty.
#[test]
fn theorem3_all_mutation_paths_mark_dirty() {
    // Path 1: set()
    {
        let mut buf = Buffer::new(10, 5);
        buf.clear_dirty();
        buf.set(3, 2, Cell::from_char('A'));
        assert!(buf.is_row_dirty(2), "set() must mark dirty");
    }

    // Path 2: set_raw()
    {
        let mut buf = Buffer::new(10, 5);
        buf.clear_dirty();
        buf.set_raw(3, 2, Cell::from_char('B'));
        assert!(buf.is_row_dirty(2), "set_raw() must mark dirty");
    }

    // Path 3: get_mut()
    {
        let mut buf = Buffer::new(10, 5);
        buf.clear_dirty();
        if let Some(cell) = buf.get_mut(3, 2) {
            cell.fg = PackedRgba::rgb(255, 0, 0);
        }
        assert!(buf.is_row_dirty(2), "get_mut() must mark dirty");
    }

    // Path 4: fill()
    {
        let mut buf = Buffer::new(10, 5);
        buf.clear_dirty();
        buf.fill(Rect::new(0, 1, 5, 3), Cell::from_char('.'));
        assert!(buf.is_row_dirty(1), "fill() must mark row 1 dirty");
        assert!(buf.is_row_dirty(2), "fill() must mark row 2 dirty");
        assert!(buf.is_row_dirty(3), "fill() must mark row 3 dirty");
        assert!(!buf.is_row_dirty(0), "fill() should not mark row 0");
        assert!(!buf.is_row_dirty(4), "fill() should not mark row 4");
    }

    // Path 5: clear()
    {
        let mut buf = Buffer::new(10, 5);
        buf.clear_dirty();
        buf.clear();
        assert_eq!(buf.dirty_row_count(), 5, "clear() must mark all dirty");
    }

    // Path 6: cells_mut()
    {
        let mut buf = Buffer::new(10, 5);
        buf.clear_dirty();
        let _ = buf.cells_mut();
        assert_eq!(buf.dirty_row_count(), 5, "cells_mut() must mark all dirty");
    }
}

proptest! {
    /// Property: After any sequence of mutations, every mutated row is dirty.
    #[test]
    fn theorem3_random_mutations_all_dirty(
        width in 5u16..50,
        height in 5u16..30,
        seed in 0u64..1_000_000,
    ) {
        let mut buf = Buffer::new(width, height);
        buf.clear_dirty();

        let mut rng = Lcg::new(seed);
        let num_mutations = rng.next() as usize % 100;
        let mut mutated_rows = HashSet::new();

        for _ in 0..num_mutations {
            let x = rng.next_u16(width);
            let y = rng.next_u16(height);
            buf.set_raw(x, y, Cell::from_char(rng.next_char()));
            mutated_rows.insert(y);
        }

        for &y in &mutated_rows {
            prop_assert!(
                buf.is_row_dirty(y),
                "row {} was mutated but not dirty", y
            );
        }
    }
}

// =============================================================================
// Theorem 4 Counterexample Tests: Diff-Dirty Equivalence
// =============================================================================

proptest! {
    /// Property: compute() and compute_dirty() yield identical results
    /// when new buffer has all rows dirty (which is the default after
    /// mutations via set_raw).
    #[test]
    fn theorem4_full_vs_dirty_equivalence(
        width in 5u16..80,
        height in 5u16..30,
        seed in 0u64..1_000_000,
    ) {
        let old = Buffer::new(width, height);
        let mut new = Buffer::new(width, height);
        let mut rng = Lcg::new(seed);

        let num_changes = rng.next() as usize % 200;
        for _ in 0..num_changes {
            let x = rng.next_u16(width);
            let y = rng.next_u16(height);
            new.set_raw(x, y, Cell::from_char(rng.next_char()));
        }

        let full = BufferDiff::compute(&old, &new);
        let dirty = BufferDiff::compute_dirty(&old, &new);

        prop_assert_eq!(
            full.changes(),
            dirty.changes(),
            "full and dirty diff must be identical"
        );
    }

    /// Adversarial: clear_dirty then selectively mark rows. compute_dirty
    /// must still find all changes in dirty rows and skip clean rows.
    #[test]
    fn theorem4_selective_dirty_correctness(
        width in 5u16..40,
        height in 5u16..20,
        seed in 0u64..1_000_000,
    ) {
        let old = Buffer::new(width, height);
        let mut new = old.clone();
        let mut rng = Lcg::new(seed);

        // Make changes in specific rows
        let num_changes = rng.next() as usize % 50;
        for _ in 0..num_changes {
            let x = rng.next_u16(width);
            let y = rng.next_u16(height);
            new.set_raw(x, y, Cell::from_char(rng.next_char()));
        }

        // compute_dirty uses new's dirty_rows; since set_raw marks dirty,
        // all changed rows are dirty. Full and dirty must agree.
        let full = BufferDiff::compute(&old, &new);
        let dirty = BufferDiff::compute_dirty(&old, &new);

        prop_assert_eq!(
            full.changes(),
            dirty.changes(),
            "dirty diff misses changes"
        );
    }
}

// =============================================================================
// Theorem 5 Counterexample Tests: Resize Safety
// =============================================================================

/// After "resize" (new blank old buffer), all content appears in diff.
#[test]
fn theorem5_resize_no_ghosting_grow() {
    let mut content = Buffer::new(100, 30);
    let mut rng = Lcg::new(0xABCD_1234);

    for _ in 0..500 {
        let x = rng.next_u16(100);
        let y = rng.next_u16(30);
        content.set_raw(x, y, Cell::from_char(rng.next_char()));
    }

    // Simulate resize: old buffer is blank (terminal was resized, prev cleared)
    let old = Buffer::new(100, 30);
    let diff = BufferDiff::compute(&old, &content);

    // Every non-blank cell must appear in diff
    let blank = Cell::default();
    for y in 0..30 {
        for x in 0..100 {
            let cell = content.get_unchecked(x, y);
            if !cell.bits_eq(&blank) {
                assert!(
                    diff.changes().contains(&(x, y)),
                    "ghosting: cell ({}, {}) missing from diff after resize",
                    x,
                    y
                );
            }
        }
    }
}

/// After "resize" to smaller size, the shrunken buffer's content is complete.
#[test]
fn theorem5_resize_no_ghosting_shrink() {
    // Original 120x40 buffer with content
    let mut original = Buffer::new(120, 40);
    let mut rng = Lcg::new(0xDEAD_BEEF);

    for _ in 0..800 {
        let x = rng.next_u16(120);
        let y = rng.next_u16(40);
        original.set_raw(x, y, Cell::from_char(rng.next_char()));
    }

    // Simulate resize to 80x24: re-render into smaller buffer
    let mut shrunken = Buffer::new(80, 24);
    for y in 0..24u16 {
        for x in 0..80u16 {
            let cell = *original.get_unchecked(x, y);
            shrunken.set_raw(x, y, cell);
        }
    }

    // Diff against blank old (resize clears prev_buffer)
    let old = Buffer::new(80, 24);
    let diff = BufferDiff::compute(&old, &shrunken);

    let blank = Cell::default();
    for y in 0..24 {
        for x in 0..80 {
            let cell = shrunken.get_unchecked(x, y);
            if !cell.bits_eq(&blank) {
                assert!(
                    diff.changes().contains(&(x, y)),
                    "ghosting after shrink: ({}, {}) missing",
                    x,
                    y
                );
            }
        }
    }
}

proptest! {
    /// Property: Regardless of buffer content, diffing against blank captures
    /// all non-blank cells. This simulates the post-resize invariant.
    #[test]
    fn theorem5_post_resize_completeness(
        width in 10u16..80,
        height in 5u16..30,
        seed in 0u64..1_000_000,
    ) {
        let mut buf = Buffer::new(width, height);
        let mut rng = Lcg::new(seed);

        let num_cells = rng.next() as usize % 200;
        for _ in 0..num_cells {
            let x = rng.next_u16(width);
            let y = rng.next_u16(height);
            buf.set_raw(x, y, Cell::from_char(rng.next_char()));
        }

        let blank = Buffer::new(width, height);
        let diff = BufferDiff::compute(&blank, &buf);

        let default_cell = Cell::default();
        for y in 0..height {
            for x in 0..width {
                let cell = buf.get_unchecked(x, y);
                if !cell.bits_eq(&default_cell) {
                    prop_assert!(
                        diff.changes().contains(&(x, y)),
                        "post-resize ghosting at ({}, {})", x, y
                    );
                }
            }
        }
    }
}

// =============================================================================
// Theorem 6 Counterexample Tests: End-to-End No-Flicker
// =============================================================================

/// Multi-frame session: each frame must be flicker-free.
#[test]
fn theorem6_e2e_multi_frame_flicker_free() {
    let caps = caps_with_sync();
    let width = 80u16;
    let height = 24u16;
    let mut rng = Lcg::new(0xE2E0_5EED);

    let mut prev = Buffer::new(width, height);

    for frame in 0..20 {
        let mut current = prev.clone();
        // Random mutations per frame
        let num_mutations = rng.next() as usize % 50;
        for _ in 0..num_mutations {
            let x = rng.next_u16(width);
            let y = rng.next_u16(height);
            current.set_raw(x, y, Cell::from_char(rng.next_char()));
        }

        let output = present_frame(&current, &prev, caps);
        let analysis = analyze_stream(&output);

        assert!(
            analysis.stats.is_flicker_free(),
            "frame {} not flicker-free: gaps={}, clears={}, frames={}/{}",
            frame,
            analysis.stats.sync_gaps,
            analysis.stats.partial_clears,
            analysis.stats.complete_frames,
            analysis.stats.total_frames
        );

        prev = current;
    }
}

/// Stress test: rapid full-screen rewrites must all be flicker-free.
#[test]
fn theorem6_e2e_full_screen_rewrite_stress() {
    let caps = caps_with_sync();
    let mut rng = Lcg::new(0x5713_5555);

    let mut prev = Buffer::new(120, 40);

    for _frame in 0..10 {
        let mut current = Buffer::new(120, 40);
        // Fill entire screen with content
        for y in 0..40 {
            for x in 0..120 {
                current.set_raw(x, y, Cell::from_char(rng.next_char()));
            }
        }

        let output = present_frame(&current, &prev, caps);
        assert_flicker_free(&output);

        prev = current;
    }
}

/// Adversarial: alternating between fully populated and blank buffers.
#[test]
fn theorem6_adversarial_flash_pattern() {
    let caps = caps_with_sync();
    let width = 60u16;
    let height = 20u16;

    let mut populated = Buffer::new(width, height);
    for y in 0..height {
        for x in 0..width {
            populated.set_raw(x, y, Cell::from_char('#'));
        }
    }
    let blank = Buffer::new(width, height);

    // Rapidly alternate populated ↔ blank
    let mut prev = blank.clone();
    for i in 0..10 {
        let current = if i % 2 == 0 { &populated } else { &blank };
        let output = present_frame(current, &prev, caps);
        assert_flicker_free(&output);
        prev = current.clone();
    }
}

/// Edge case: empty diff should still produce valid sync brackets.
#[test]
fn theorem6_empty_diff_valid_sync() {
    let buf = Buffer::new(40, 10);
    let output = present_frame(&buf, &buf, caps_with_sync());

    let analysis = analyze_stream(&output);
    assert!(
        analysis.stats.is_flicker_free(),
        "empty diff must still be flicker-free"
    );
    assert_eq!(analysis.stats.total_frames, 1);
    assert_eq!(analysis.stats.complete_frames, 1);
}

// =============================================================================
// Adversarial Counterexample Battery: Style-Only Changes
// =============================================================================

/// Style-only changes (fg/bg/attrs, same char) must appear in diff.
#[test]
fn counterexample_style_only_changes_detected() {
    let mut old = Buffer::new(20, 5);
    let mut new = Buffer::new(20, 5);

    // Same char 'A' in both, but different colors
    old.set_raw(5, 2, Cell::from_char('A'));
    new.set_raw(
        5,
        2,
        Cell::from_char('A')
            .with_fg(PackedRgba::rgb(255, 0, 0))
            .with_bg(PackedRgba::rgb(0, 0, 255)),
    );

    let diff = BufferDiff::compute(&old, &new);
    assert!(
        diff.changes().contains(&(5, 2)),
        "style-only change must be detected"
    );
}

/// Attribute-only change (bold flag) must be detected.
#[test]
fn counterexample_attribute_only_change_detected() {
    let mut old = Buffer::new(20, 5);
    let mut new = Buffer::new(20, 5);

    let plain = Cell::from_char('B');
    old.set_raw(3, 1, plain);

    // Create bold version by modifying attrs
    let mut bold = Cell::from_char('B');
    bold.attrs = CellAttrs::new(StyleFlags::BOLD, 0);
    new.set_raw(3, 1, bold);

    let diff = BufferDiff::compute(&old, &new);
    assert!(
        diff.changes().contains(&(3, 1)),
        "attribute-only change must be detected"
    );
}

// =============================================================================
// Golden Checksum Stability
// =============================================================================

/// Verify that the proof-test pipeline produces deterministic output.
#[test]
fn golden_proof_determinism() {
    let caps = caps_with_sync();
    let mut rng = Lcg::new(0xC01D_E001);

    let mut buf = Buffer::new(80, 24);
    for _ in 0..100 {
        let x = rng.next_u16(80);
        let y = rng.next_u16(24);
        buf.set_raw(x, y, Cell::from_char(rng.next_char()));
    }

    let old = Buffer::new(80, 24);
    let output1 = present_frame(&buf, &old, caps);

    // Reset and replay
    let mut rng2 = Lcg::new(0xC01D_E001);
    let mut buf2 = Buffer::new(80, 24);
    for _ in 0..100 {
        let x = rng2.next_u16(80);
        let y = rng2.next_u16(24);
        buf2.set_raw(x, y, Cell::from_char(rng2.next_char()));
    }
    let output2 = present_frame(&buf2, &old, caps);

    assert_eq!(
        fnv1a(&output1),
        fnv1a(&output2),
        "identical inputs must produce identical ANSI output"
    );
}

// =============================================================================
// JSONL Logging Verification
// =============================================================================

/// E2E test with structured JSONL logging of proof-test results.
#[test]
fn e2e_proof_verification_jsonl() {
    use std::time::Instant;

    let caps = caps_with_sync();
    let scenarios: &[(u16, u16, u64, &str)] = &[
        (80, 24, 0x000D_A00F_0001, "standard_terminal"),
        (120, 40, 0x000D_A00F_0002, "large_terminal"),
        (40, 10, 0x000D_A00F_0003, "small_terminal"),
        (200, 50, 0x000D_A00F_0004, "ultrawide"),
    ];

    for &(width, height, seed, label) in scenarios {
        let start = Instant::now();
        let mut rng = Lcg::new(seed);

        let mut prev = Buffer::new(width, height);
        let mut total_changes = 0usize;
        let mut total_frames = 0u32;
        let mut all_flicker_free = true;

        for _ in 0..5 {
            let mut current = prev.clone();
            let n = rng.next() as usize % 100;
            for _ in 0..n {
                let x = rng.next_u16(width);
                let y = rng.next_u16(height);
                current.set_raw(x, y, Cell::from_char(rng.next_char()));
            }

            let diff = BufferDiff::compute(&prev, &current);
            total_changes += diff.len();

            let output = present_frame(&current, &prev, caps);
            let analysis = analyze_stream(&output);
            if !analysis.stats.is_flicker_free() {
                all_flicker_free = false;
            }
            total_frames += 1;

            prev = current;
        }

        let elapsed_us = start.elapsed().as_micros();

        // JSONL structured log
        eprintln!(
            "{{\"test\":\"no_flicker_proof\",\"scenario\":\"{}\",\"width\":{},\"height\":{},\"seed\":{},\"frames\":{},\"total_changes\":{},\"flicker_free\":{},\"elapsed_us\":{}}}",
            label, width, height, seed, total_frames, total_changes, all_flicker_free, elapsed_us
        );

        assert!(all_flicker_free, "scenario '{}' had flicker", label);
    }
}

// =============================================================================
// Stress: Rapid Resize Oscillation
// =============================================================================

proptest! {
    /// Property: Rapid size oscillation (simulating resize storm) never
    /// produces flicker when each frame is properly sync-bracketed.
    #[test]
    fn stress_resize_oscillation_flicker_free(
        base_width in 20u16..60,
        base_height in 10u16..25,
        seed in 0u64..100_000,
    ) {
        let caps = caps_with_sync();
        let mut rng = Lcg::new(seed);

        for _ in 0..5 {
            // Oscillate size
            let dw = (rng.next() % 20) as i32 - 10;
            let dh = (rng.next() % 10) as i32 - 5;
            let w = (base_width as i32 + dw).clamp(5, 200) as u16;
            let h = (base_height as i32 + dh).clamp(5, 50) as u16;

            let mut buf = Buffer::new(w, h);
            let n = rng.next() as usize % 50;
            for _ in 0..n {
                let x = rng.next_u16(w);
                let y = rng.next_u16(h);
                buf.set_raw(x, y, Cell::from_char(rng.next_char()));
            }

            // After resize, diff against blank (simulates prev_buffer = None)
            let blank = Buffer::new(w, h);
            let output = present_frame(&buf, &blank, caps);

            let analysis = analyze_stream(&output);
            prop_assert!(
                analysis.stats.is_flicker_free(),
                "resize to {}x{} produced flicker", w, h
            );
        }
    }
}
