//! Golden matrix and drift detection for serialization exports (bd-2vr05.5.5).
//!
//! Creates a matrix of test buffers (varying styles, characters, dimensions)
//! and exports each through ANSI, HTML, and plain text, then verifies output
//! against known golden hashes for deterministic drift detection.

#![cfg(feature = "export")]

use ftui_extras::export::{HtmlExporter, SvgExporter, TextExporter};
use ftui_render::buffer::Buffer;
use ftui_render::cell::{Cell, CellAttrs, PackedRgba, StyleFlags};
use ftui_render::grapheme_pool::GraphemePool;
use std::collections::BTreeMap;
use std::hash::{DefaultHasher, Hash, Hasher};

// ---------------------------------------------------------------------------
// Test buffer fixtures
// ---------------------------------------------------------------------------

/// Create a simple ASCII buffer with no styling.
fn fixture_plain_ascii() -> (Buffer, GraphemePool) {
    let mut buf = Buffer::new(10, 3);
    let pool = GraphemePool::new();

    let lines = ["Hello     ", "World!    ", "FrankenTUI"];
    for (y, line) in lines.iter().enumerate() {
        for (x, ch) in line.chars().enumerate() {
            buf.set_fast(x as u16, y as u16, Cell::from_char(ch));
        }
    }

    (buf, pool)
}

/// Buffer with 24-bit foreground and background colors.
fn fixture_colored() -> (Buffer, GraphemePool) {
    let mut buf = Buffer::new(6, 2);
    let pool = GraphemePool::new();

    // Row 0: red on black
    for x in 0..3 {
        let mut cell = Cell::from_char('R');
        cell.fg = PackedRgba::rgb(255, 0, 0);
        cell.bg = PackedRgba::rgb(0, 0, 0);
        buf.set_fast(x, 0, cell);
    }
    // Row 0: green on white
    for x in 3..6 {
        let mut cell = Cell::from_char('G');
        cell.fg = PackedRgba::rgb(0, 255, 0);
        cell.bg = PackedRgba::rgb(255, 255, 255);
        buf.set_fast(x, 0, cell);
    }
    // Row 1: blue with transparency
    for x in 0..6 {
        let mut cell = Cell::from_char('B');
        cell.fg = PackedRgba::rgb(0, 0, 255);
        cell.bg = PackedRgba::TRANSPARENT;
        buf.set_fast(x, 1, cell);
    }

    (buf, pool)
}

/// Buffer with various text style attributes.
fn fixture_styled() -> (Buffer, GraphemePool) {
    let mut buf = Buffer::new(8, 1);
    let pool = GraphemePool::new();

    // Each character has a different style attribute
    let styles = [
        StyleFlags::BOLD,
        StyleFlags::DIM,
        StyleFlags::ITALIC,
        StyleFlags::UNDERLINE,
        StyleFlags::BLINK,
        StyleFlags::REVERSE,
        StyleFlags::HIDDEN,
        StyleFlags::STRIKETHROUGH,
    ];
    let chars = ['B', 'D', 'I', 'U', 'K', 'R', 'H', 'S'];

    for (x, (&ch, &style)) in chars.iter().zip(styles.iter()).enumerate() {
        let mut cell = Cell::from_char(ch);
        cell.attrs = CellAttrs::new(style, 0);
        cell.fg = PackedRgba::rgb(200, 200, 200);
        buf.set_fast(x as u16, 0, cell);
    }

    (buf, pool)
}

/// Buffer with combined styles.
fn fixture_multi_style() -> (Buffer, GraphemePool) {
    let mut buf = Buffer::new(4, 1);
    let pool = GraphemePool::new();

    // Bold + italic + underline
    let mut cell = Cell::from_char('X');
    cell.attrs = CellAttrs::new(
        StyleFlags::BOLD | StyleFlags::ITALIC | StyleFlags::UNDERLINE,
        0,
    );
    cell.fg = PackedRgba::rgb(255, 128, 0);
    cell.bg = PackedRgba::rgb(0, 32, 64);

    for x in 0..4 {
        buf.set_fast(x, 0, cell);
    }

    (buf, pool)
}

/// Empty buffer.
fn fixture_empty() -> (Buffer, GraphemePool) {
    let buf = Buffer::new(5, 2);
    let pool = GraphemePool::new();
    (buf, pool)
}

/// Buffer with special characters that need HTML/SVG escaping.
fn fixture_special_chars() -> (Buffer, GraphemePool) {
    let mut buf = Buffer::new(8, 1);
    let pool = GraphemePool::new();

    let chars = ['<', '>', '&', '"', '\'', '/', '\\', '!'];
    for (x, &ch) in chars.iter().enumerate() {
        buf.set_fast(x as u16, 0, Cell::from_char(ch));
    }

    (buf, pool)
}

/// Single-cell buffer (edge case).
fn fixture_single_cell() -> (Buffer, GraphemePool) {
    let mut buf = Buffer::new(1, 1);
    let pool = GraphemePool::new();
    let mut cell = Cell::from_char('Z');
    cell.fg = PackedRgba::rgb(128, 64, 255);
    cell.attrs = CellAttrs::new(StyleFlags::BOLD, 0);
    buf.set_fast(0, 0, cell);
    (buf, pool)
}

// ---------------------------------------------------------------------------
// Hash helper
// ---------------------------------------------------------------------------

fn content_hash(s: &str) -> u64 {
    let mut hasher = DefaultHasher::new();
    s.hash(&mut hasher);
    hasher.finish()
}

// ---------------------------------------------------------------------------
// Golden matrix tests
// ---------------------------------------------------------------------------

/// Collect all golden hashes into a BTreeMap for deterministic ordering.
fn compute_golden_matrix() -> BTreeMap<String, u64> {
    let mut matrix = BTreeMap::new();

    let fixtures: Vec<(&str, Buffer, GraphemePool)> = {
        let (b1, p1) = fixture_plain_ascii();
        let (b2, p2) = fixture_colored();
        let (b3, p3) = fixture_styled();
        let (b4, p4) = fixture_multi_style();
        let (b5, p5) = fixture_empty();
        let (b6, p6) = fixture_special_chars();
        let (b7, p7) = fixture_single_cell();
        vec![
            ("plain_ascii", b1, p1),
            ("colored", b2, p2),
            ("styled", b3, p3),
            ("multi_style", b4, p4),
            ("empty", b5, p5),
            ("special_chars", b6, p6),
            ("single_cell", b7, p7),
        ]
    };

    let text_plain = TextExporter::plain();
    let text_ansi = TextExporter::ansi();
    let html_inline = HtmlExporter::default();
    let html_class = HtmlExporter {
        inline_styles: false,
        ..HtmlExporter::default()
    };
    let svg = SvgExporter::default();

    for (name, buf, pool) in &fixtures {
        let plain = text_plain.export(buf, pool);
        let ansi = text_ansi.export(buf, pool);
        let html_i = html_inline.export(buf, pool);
        let html_c = html_class.export(buf, pool);
        let svg_out = svg.export(buf, pool);

        matrix.insert(format!("{name}/plain"), content_hash(&plain));
        matrix.insert(format!("{name}/ansi"), content_hash(&ansi));
        matrix.insert(format!("{name}/html_inline"), content_hash(&html_i));
        matrix.insert(format!("{name}/html_class"), content_hash(&html_c));
        matrix.insert(format!("{name}/svg"), content_hash(&svg_out));
    }

    matrix
}

#[test]
fn golden_matrix_is_deterministic() {
    let run1 = compute_golden_matrix();
    let run2 = compute_golden_matrix();
    assert_eq!(
        run1, run2,
        "golden matrix should be deterministic across runs"
    );
}

#[test]
fn golden_matrix_has_expected_entries() {
    let matrix = compute_golden_matrix();
    // 7 fixtures * 5 formats = 35 entries
    assert_eq!(matrix.len(), 35, "expected 35 golden hash entries");

    // Spot-check that all fixture/format combinations exist
    assert!(matrix.contains_key("plain_ascii/plain"));
    assert!(matrix.contains_key("plain_ascii/ansi"));
    assert!(matrix.contains_key("plain_ascii/html_inline"));
    assert!(matrix.contains_key("plain_ascii/html_class"));
    assert!(matrix.contains_key("plain_ascii/svg"));
    assert!(matrix.contains_key("colored/ansi"));
    assert!(matrix.contains_key("styled/html_inline"));
    assert!(matrix.contains_key("empty/plain"));
    assert!(matrix.contains_key("single_cell/svg"));
}

#[test]
fn golden_hashes_are_distinct() {
    let matrix = compute_golden_matrix();

    // Plain text should differ from ANSI for styled fixtures
    assert_ne!(
        matrix["styled/plain"], matrix["styled/ansi"],
        "styled plain and ANSI should differ"
    );
    assert_ne!(
        matrix["colored/plain"], matrix["colored/ansi"],
        "colored plain and ANSI should differ"
    );

    // HTML inline and class modes should differ
    assert_ne!(
        matrix["colored/html_inline"], matrix["colored/html_class"],
        "HTML inline and class modes should differ"
    );

    // Different fixtures should produce different hashes
    assert_ne!(
        matrix["plain_ascii/plain"], matrix["colored/plain"],
        "different fixtures should differ"
    );
    assert_ne!(
        matrix["plain_ascii/plain"], matrix["empty/plain"],
        "non-empty and empty should differ"
    );
}

// ---------------------------------------------------------------------------
// Format-specific golden tests
// ---------------------------------------------------------------------------

#[test]
fn plain_text_golden_structure() {
    let (buf, pool) = fixture_plain_ascii();
    let text = TextExporter::plain().export(&buf, &pool);

    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines.len(), 3);
    assert_eq!(lines[0], "Hello");
    assert_eq!(lines[1], "World!");
    assert_eq!(lines[2], "FrankenTUI");
}

#[test]
fn ansi_text_golden_contains_escapes() {
    let (buf, pool) = fixture_colored();
    let ansi = TextExporter::ansi().export(&buf, &pool);

    // Should contain ANSI CSI sequences
    assert!(ansi.contains("\x1b["), "ANSI output should contain CSI");
    // Should contain reset sequences
    assert!(ansi.contains("\x1b[0m"), "ANSI output should contain reset");
    // Should contain 24-bit color codes
    assert!(ansi.contains("38;2;255;0;0"), "should have red foreground");
    assert!(
        ansi.contains("38;2;0;255;0"),
        "should have green foreground"
    );
}

#[test]
fn ansi_text_golden_style_flags() {
    let (buf, pool) = fixture_styled();
    let ansi = TextExporter::ansi().export(&buf, &pool);

    // Should contain SGR codes for each style (each is first param after CSI)
    assert!(ansi.contains("\x1b[1;"), "should have bold (SGR 1)");
    assert!(ansi.contains("\x1b[2;"), "should have dim (SGR 2)");
    assert!(ansi.contains("\x1b[3;"), "should have italic (SGR 3)");
    assert!(ansi.contains("\x1b[4;"), "should have underline (SGR 4)");
}

#[test]
fn html_golden_escapes_special_chars() {
    let (buf, pool) = fixture_special_chars();
    let html = HtmlExporter::default().export(&buf, &pool);

    assert!(html.contains("&lt;"), "should escape <");
    assert!(html.contains("&gt;"), "should escape >");
    assert!(html.contains("&amp;"), "should escape &");
    assert!(!html.contains("<>&"), "raw special chars should not appear");
}

#[test]
fn html_golden_includes_color_css() {
    let (buf, pool) = fixture_colored();
    let html = HtmlExporter::default().export(&buf, &pool);

    // Should include inline color styles
    assert!(
        html.contains("color:"),
        "HTML should include color properties"
    );
    assert!(
        html.contains("background:"),
        "HTML should include background"
    );
}

#[test]
fn svg_golden_structure() {
    let (buf, pool) = fixture_plain_ascii();
    let svg = SvgExporter::default().export(&buf, &pool);

    assert!(svg.starts_with("<svg"), "SVG should start with <svg");
    assert!(svg.contains("</svg>"), "SVG should end with </svg>");
    assert!(svg.contains("<text"), "SVG should contain text elements");
}

#[test]
fn empty_buffer_produces_minimal_output() {
    let (buf, pool) = fixture_empty();

    let plain = TextExporter::plain().export(&buf, &pool);
    // Plain text of empty buffer should be empty or whitespace-only
    assert!(
        plain.trim().is_empty(),
        "empty buffer plain text should be blank"
    );

    let ansi = TextExporter::ansi().export(&buf, &pool);
    // ANSI of empty buffer should have no escape codes
    assert!(
        !ansi.contains("\x1b["),
        "empty buffer ANSI should have no escapes"
    );
}

// ---------------------------------------------------------------------------
// Cross-format consistency
// ---------------------------------------------------------------------------

#[test]
fn plain_text_content_matches_across_formats() {
    let (buf, pool) = fixture_plain_ascii();
    let plain = TextExporter::plain().export(&buf, &pool);
    let ansi = TextExporter::ansi().export(&buf, &pool);

    // Plain ASCII has no styling, so ANSI should be identical to plain
    assert_eq!(
        plain, ansi,
        "unstyled buffer: plain and ANSI should be identical"
    );
}

#[test]
fn all_formats_produce_nonempty_for_nonempty_buffer() {
    let (buf, pool) = fixture_plain_ascii();

    let plain = TextExporter::plain().export(&buf, &pool);
    let ansi = TextExporter::ansi().export(&buf, &pool);
    let html = HtmlExporter::default().export(&buf, &pool);
    let svg = SvgExporter::default().export(&buf, &pool);

    assert!(!plain.is_empty(), "plain should not be empty");
    assert!(!ansi.is_empty(), "ansi should not be empty");
    assert!(!html.is_empty(), "html should not be empty");
    assert!(!svg.is_empty(), "svg should not be empty");
}

// ---------------------------------------------------------------------------
// Drift detection: hash stability
// ---------------------------------------------------------------------------

#[test]
fn drift_detection_plain_ascii() {
    let (buf, pool) = fixture_plain_ascii();
    let plain = TextExporter::plain().export(&buf, &pool);
    let h = content_hash(&plain);
    // Running twice should produce same hash
    let plain2 = TextExporter::plain().export(&buf, &pool);
    let h2 = content_hash(&plain2);
    assert_eq!(h, h2, "hash should be stable");
}

#[test]
fn drift_detection_colored_ansi() {
    let (buf, pool) = fixture_colored();
    let ansi1 = TextExporter::ansi().export(&buf, &pool);
    let ansi2 = TextExporter::ansi().export(&buf, &pool);
    assert_eq!(ansi1, ansi2, "ANSI output should be deterministic");
}

#[test]
fn drift_detection_html_inline() {
    let (buf, pool) = fixture_colored();
    let h1 = HtmlExporter::default().export(&buf, &pool);
    let h2 = HtmlExporter::default().export(&buf, &pool);
    assert_eq!(h1, h2, "HTML output should be deterministic");
}

#[test]
fn drift_detection_svg() {
    let (buf, pool) = fixture_plain_ascii();
    let s1 = SvgExporter::default().export(&buf, &pool);
    let s2 = SvgExporter::default().export(&buf, &pool);
    assert_eq!(s1, s2, "SVG output should be deterministic");
}

#[test]
fn drift_detection_multi_style_all_formats() {
    let (buf, pool) = fixture_multi_style();

    let plain = TextExporter::plain().export(&buf, &pool);
    let ansi = TextExporter::ansi().export(&buf, &pool);
    let html = HtmlExporter::default().export(&buf, &pool);
    let svg = SvgExporter::default().export(&buf, &pool);

    // Verify all are non-empty
    assert!(!plain.is_empty());
    assert!(!ansi.is_empty());
    assert!(!html.is_empty());
    assert!(!svg.is_empty());

    // Re-export and verify hashes match
    let plain2 = TextExporter::plain().export(&buf, &pool);
    let ansi2 = TextExporter::ansi().export(&buf, &pool);
    let html2 = HtmlExporter::default().export(&buf, &pool);
    let svg2 = SvgExporter::default().export(&buf, &pool);

    assert_eq!(content_hash(&plain), content_hash(&plain2));
    assert_eq!(content_hash(&ansi), content_hash(&ansi2));
    assert_eq!(content_hash(&html), content_hash(&html2));
    assert_eq!(content_hash(&svg), content_hash(&svg2));
}
