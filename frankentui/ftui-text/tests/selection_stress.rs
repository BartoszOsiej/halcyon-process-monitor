//! Selection resilience stress tests under churn (bd-2vr05.4.5).
//!
//! Validates that selection invariants hold under rapid insert/delete cycles,
//! multiline mutations, interleaved undo/redo, and content replacement.

use ftui_text::cursor::CursorPosition;
use ftui_text::editor::Editor;

// ── Helpers ────────────────────────────────────────────────────────

fn assert_selection_invariants(ed: &Editor, ctx: &str) {
    let cursor = ed.cursor();
    let lines = ed.line_count();
    assert!(
        cursor.line < lines || (cursor.line == 0 && lines == 1),
        "{ctx}: cursor line {line} out of bounds (lines={lines})",
        line = cursor.line,
    );

    if let Some(sel) = ed.selection() {
        let nav = ftui_text::cursor::CursorNavigator::new(ed.rope());
        let (start, end) = sel.byte_range(&nav);
        assert!(
            start <= end,
            "{ctx}: byte_range out of order: {start} > {end}"
        );
        assert!(
            end <= ed.rope().len_bytes(),
            "{ctx}: byte_range end {end} exceeds rope len {}",
            ed.rope().len_bytes(),
        );
    }
}

fn multiline_content(lines: usize, line_len: usize) -> String {
    (0..lines)
        .map(|i| {
            let base = format!("L{i:04}:");
            let pad: String = (0..(line_len.saturating_sub(base.len())))
                .map(|j| char::from(b'a' + (j % 26) as u8))
                .collect();
            format!("{base}{pad}")
        })
        .collect::<Vec<_>>()
        .join("\n")
}

// ── Stress: rapid insert/delete with active selection ──────────────

#[test]
fn selection_survives_rapid_insert_delete_churn() {
    let mut ed = Editor::with_text("The quick brown fox jumps over the lazy dog");
    ed.set_cursor(CursorPosition::new(0, 4, 4));

    for i in 0..200 {
        match i % 7 {
            0 => ed.select_right(),
            1 => ed.select_right(),
            2 => {
                ed.insert_char(char::from(b'A' + (i % 26) as u8));
            }
            3 => ed.select_left(),
            4 => {
                ed.delete_backward();
            }
            5 => ed.select_word_right(),
            6 => ed.clear_selection(),
            _ => unreachable!(),
        }
        assert_selection_invariants(&ed, &format!("rapid_churn iter={i}"));
    }
}

// ── Stress: selection across multiline content with inserts ────────

#[test]
fn selection_stable_during_multiline_insert_churn() {
    let content = multiline_content(50, 40);
    let mut ed = Editor::with_text(&content);

    // Place cursor in the middle
    ed.set_cursor(CursorPosition::new(25, 10, 10));

    // Select downward across multiple lines
    for _ in 0..10 {
        ed.select_down();
    }
    assert_selection_invariants(&ed, "after multi-line select down");

    let sel_text = ed.selected_text();
    assert!(
        sel_text.is_some(),
        "should have selected text spanning lines"
    );
    let sel_len = sel_text.unwrap().len();
    assert!(
        sel_len > 40,
        "selected text should span multiple lines, got {sel_len}"
    );

    // Now insert text (replaces selection)
    ed.insert_text("REPLACED_BLOCK");
    assert_selection_invariants(&ed, "after replace multiline selection");
    assert!(
        ed.selection().is_none(),
        "selection should be cleared after insert"
    );

    // Undo should restore the original selected text
    ed.undo();
    assert_selection_invariants(&ed, "after undo multiline replace");
    assert_eq!(ed.text(), content);
}

// ── Stress: interleaved select + undo/redo ─────────────────────────

#[test]
fn selection_consistent_through_undo_redo_interleaving() {
    let mut ed = Editor::with_text("alpha beta gamma delta epsilon");

    let ops: Vec<u8> = (0..300)
        .map(|i| (i * 7 + 3) % 11)
        .map(|v: usize| v as u8)
        .collect();

    for (i, &op) in ops.iter().enumerate() {
        match op {
            0 => ed.select_right(),
            1 => ed.select_left(),
            2 => ed.select_word_right(),
            3 => ed.select_word_left(),
            4 => ed.clear_selection(),
            5 => {
                ed.insert_char('x');
            }
            6 => {
                ed.delete_backward();
            }
            7 => {
                ed.undo();
            }
            8 => {
                ed.redo();
            }
            9 => ed.move_right(),
            10 => ed.move_left(),
            _ => {}
        }
        assert_selection_invariants(&ed, &format!("undo_redo_interleave iter={i} op={op}"));
    }
}

// ── Stress: selection after set_text content replacement ───────────

#[test]
fn selection_cleared_and_valid_after_set_text() {
    let mut ed = Editor::with_text("original content here");
    ed.set_cursor(CursorPosition::new(0, 5, 5));
    ed.select_right();
    ed.select_right();
    ed.select_right();
    assert!(ed.selection().is_some());

    // Replace with shorter content
    ed.set_text("hi");
    assert!(ed.selection().is_none(), "set_text should clear selection");
    assert_selection_invariants(&ed, "after set_text shorter");

    // Replace with longer content
    ed.set_text(&"x".repeat(5000));
    assert_selection_invariants(&ed, "after set_text longer");

    // Replace with empty
    ed.set_text("");
    assert_selection_invariants(&ed, "after set_text empty");
    assert!(ed.is_empty());
}

// ── Stress: select_all + operations cycle ──────────────────────────

#[test]
fn select_all_replace_cycle_stress() {
    let mut ed = Editor::new();

    for i in 0..100 {
        let content = format!("cycle_{i}_content_with_some_padding_text");
        ed.insert_text(&content);
        ed.select_all();
        assert_selection_invariants(&ed, &format!("select_all cycle={i}"));

        let sel = ed.selected_text().unwrap_or_default();
        assert!(!sel.is_empty(), "cycle {i}: select_all should select text");

        // Replace with new content
        ed.insert_text(&format!("replaced_{i}"));
        assert!(ed.selection().is_none());
        assert_selection_invariants(&ed, &format!("after_replace cycle={i}"));
    }
}

// ── Stress: unicode selection under churn ──────────────────────────

#[test]
fn unicode_selection_resilience() {
    let content = "Hello 🌍 世界 café 👩‍💻 résumé 🇺🇸 naïve";
    let mut ed = Editor::with_text(content);

    ed.set_cursor(CursorPosition::new(0, 0, 0));

    // Select through mixed-width characters
    for i in 0..30 {
        ed.select_right();
        assert_selection_invariants(&ed, &format!("unicode_select_right iter={i}"));
    }

    let sel = ed.selected_text();
    assert!(sel.is_some(), "should have unicode selection");

    // Now delete and verify
    ed.delete_backward();
    assert_selection_invariants(&ed, "after unicode selection delete");

    // Undo should restore
    ed.undo();
    assert_selection_invariants(&ed, "after unicode undo");
    assert_eq!(ed.text(), content);
}

// ── Stress: selection with very long lines ─────────────────────────

#[test]
fn selection_on_very_long_lines() {
    let long_line = "a".repeat(10_000);
    let content = format!("{long_line}\n{long_line}\n{long_line}");
    let mut ed = Editor::with_text(&content);

    // Select from middle of line 1 across to line 2
    ed.set_cursor(CursorPosition::new(0, 5000, 5000));
    for _ in 0..20 {
        ed.select_right();
    }
    ed.select_down();
    assert_selection_invariants(&ed, "long line cross-line selection");

    let sel = ed.selected_text();
    assert!(sel.is_some());
    assert!(sel.unwrap().len() > 5000, "should select across long lines");

    // Replace and verify
    ed.insert_text("SHORT");
    assert_selection_invariants(&ed, "after long line replace");
    assert!(
        ed.text().len() < content.len(),
        "text should be shorter after replace"
    );
}

// ── Stress: rapid cursor movement doesn't corrupt selection ────────

#[test]
fn rapid_movement_no_selection_corruption() {
    let content = multiline_content(20, 30);
    let mut ed = Editor::with_text(&content);

    ed.set_cursor(CursorPosition::new(10, 15, 15));

    // Rapid directional movements
    for i in 0..500 {
        match i % 8 {
            0 => ed.move_left(),
            1 => ed.move_right(),
            2 => ed.move_up(),
            3 => ed.move_down(),
            4 => ed.move_word_left(),
            5 => ed.move_word_right(),
            6 => ed.move_to_line_start(),
            7 => ed.move_to_line_end(),
            _ => unreachable!(),
        }
        assert!(
            ed.selection().is_none(),
            "movement should not create selection at iter={i}"
        );
        assert_selection_invariants(&ed, &format!("rapid_move iter={i}"));
    }
}

// ── Stress: extend_selection_to with extreme positions ─────────────

#[test]
fn extend_selection_to_extreme_positions() {
    let mut ed = Editor::with_text("line one\nline two\nline three");
    ed.set_cursor(CursorPosition::new(1, 4, 4));

    // Extend to far beyond document bounds — should clamp
    ed.extend_selection_to(CursorPosition::new(100, 100, 100));
    assert_selection_invariants(&ed, "extend to extreme right");
    assert!(ed.selection().is_some());

    // Extend back to origin — anchor was at (1,4), head goes to (0,0)
    ed.extend_selection_to(CursorPosition::new(0, 0, 0));
    assert_selection_invariants(&ed, "extend to origin");

    let sel = ed.selected_text().unwrap();
    // Selection spans from anchor (1,4) to head (0,0) — byte_range orders them
    // so we get from start of doc to anchor position
    assert!(
        sel.contains("line one\nline"),
        "expected selection spanning across line boundary, got: {sel:?}"
    );
}

// ── Stress: word selection exhaustive scan ─────────────────────────

#[test]
fn word_selection_exhaustive_scan() {
    let content = "fn main() { let x = 42; println!(\"hello world\"); }";
    let mut ed = Editor::with_text(content);
    ed.set_cursor(CursorPosition::new(0, 0, 0));

    let total_graphemes = content.chars().count();
    let mut selections_made = 0;

    // Walk through every position and select word right
    for pos in 0..total_graphemes {
        ed.set_cursor(CursorPosition::new(0, pos, pos));
        ed.select_word_right();

        if let Some(sel) = ed.selection() {
            let nav = ftui_text::cursor::CursorNavigator::new(ed.rope());
            let (start, end) = sel.byte_range(&nav);
            assert!(
                start <= end,
                "word selection at pos={pos}: range order violated"
            );
            selections_made += 1;
        }
        assert_selection_invariants(&ed, &format!("word_select pos={pos}"));
    }

    assert!(
        selections_made > 0,
        "should have made at least one word selection"
    );
}

// ── Stress: concurrent-style insert/select/delete/undo ─────────────

#[test]
fn heavy_mixed_operation_stress() {
    let mut ed = Editor::with_text("initial text for stress testing");

    // Deterministic pseudo-random operation sequence
    let mut state: u32 = 0xDEAD_BEEF;
    for i in 0..1000 {
        // Simple LCG for deterministic pseudo-random
        state = state.wrapping_mul(1103515245).wrapping_add(12345);
        let op = (state >> 16) % 12;

        match op {
            0 => ed.insert_char(char::from(b'a' + (state % 26) as u8)),
            1 => {
                ed.insert_text("chunk");
            }
            2 => {
                ed.delete_backward();
            }
            3 => {
                ed.delete_forward();
            }
            4 => ed.select_right(),
            5 => ed.select_left(),
            6 => ed.select_word_right(),
            7 => ed.clear_selection(),
            8 => {
                ed.undo();
            }
            9 => {
                ed.redo();
            }
            10 => ed.move_right(),
            11 => ed.move_left(),
            _ => {}
        }
        assert_selection_invariants(&ed, &format!("heavy_mixed iter={i} op={op}"));
    }
}

// ── Stress: selection after clear() ────────────────────────────────

#[test]
fn selection_after_clear() {
    let mut ed = Editor::with_text("some content here");
    ed.set_cursor(CursorPosition::new(0, 5, 5));
    ed.select_right();
    ed.select_right();
    assert!(ed.selection().is_some());

    ed.clear();
    assert!(ed.selection().is_none(), "clear should remove selection");
    assert!(ed.is_empty());
    assert_selection_invariants(&ed, "after clear");

    // Operations after clear should work
    ed.insert_text("new content");
    ed.select_all();
    assert!(ed.selection().is_some());
    assert_selection_invariants(&ed, "select_all after clear+insert");
}

// ── Stress: delete_word with selection spanning word boundaries ─────

#[test]
fn delete_word_with_selection_across_boundaries() {
    let mut ed = Editor::with_text("word1 word2 word3 word4 word5");

    for round in 0..20 {
        if ed.is_empty() {
            ed.insert_text("word1 word2 word3 word4 word5");
        }

        // Move to a position and select across word boundaries
        ed.move_to_document_start();
        for _ in 0..(round % 15) {
            ed.move_right();
        }

        // Select a few chars across a word boundary
        ed.select_right();
        ed.select_right();
        ed.select_right();

        // Delete word operations should handle selection first
        ed.delete_word_backward();
        assert_selection_invariants(&ed, &format!("delete_word round={round}"));
    }
}
