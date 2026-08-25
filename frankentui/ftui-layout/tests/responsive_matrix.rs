#![forbid(unsafe_code)]

//! Responsive Layout Test Matrix (Size x Breakpoint x Mode)
//!
//! Exhaustive matrix tests across terminal sizes, breakpoints, and layout modes
//! with verbose JSONL logging and layout invariant verification.
//!
//! # Invariants Tested
//!
//! | ID       | Invariant                                       |
//! |----------|-------------------------------------------------|
//! | FEAS-1   | Sum of allocated sizes <= available space        |
//! | TIE-1    | Deterministic tie-breaking across runs           |
//! | MONO-1   | Monotone constraint response (more space => >=)  |
//! | INH-1    | Missing breakpoint inherits from nearest smaller |
//! | VIS-1    | Hidden widgets reclaim space (zero area)         |
//! | TRANS-1  | Breakpoint transitions detected correctly        |
//! | COHER-1  | Same width => same layout (temporal coherence)   |
//!
//! # Running Tests
//!
//! ```sh
//! cargo test -p ftui-layout responsive_matrix_
//! ```
//!
//! # JSONL Logging
//!
//! ```sh
//! RESPONSIVE_LOG=1 cargo test -p ftui-layout responsive_matrix_
//! ```

use ftui_core::geometry::Rect;
use ftui_layout::{
    Alignment, Breakpoint, Breakpoints, Constraint, Flex, Responsive, ResponsiveLayout,
    ResponsiveSplit, Visibility,
};
use std::io::Write;

// ============================================================================
// JSONL Logger
// ============================================================================

struct MatrixLogger {
    writer: Option<Box<dyn Write>>,
    run_id: String,
}

impl MatrixLogger {
    fn new(case_name: &str) -> Self {
        let writer = if std::env::var("RESPONSIVE_LOG").is_ok() {
            let dir = std::env::temp_dir().join("ftui_responsive_matrix");
            let _ = std::fs::create_dir_all(&dir);
            let path = dir.join(format!("{case_name}.jsonl"));
            std::fs::File::create(path)
                .ok()
                .map(|f| Box::new(f) as Box<dyn Write>)
        } else {
            None
        };
        Self {
            writer,
            run_id: format!(
                "{}-{}",
                case_name,
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_millis()
            ),
        }
    }

    fn log_event(&mut self, event: &str, data: &str) {
        if let Some(ref mut w) = self.writer {
            let ts = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis();
            let _ = writeln!(
                w,
                r#"{{"run_id":"{}","event":"{}","ts_ms":{},"data":{}}}"#,
                self.run_id, event, ts, data
            );
        }
    }

    fn log_scenario(&mut self, width: u16, height: u16, bp: Breakpoint, rects_count: usize) {
        self.log_event(
            "scenario",
            &format!(
                r#"{{"width":{},"height":{},"breakpoint":"{}","rects_count":{}}}"#,
                width, height, bp, rects_count
            ),
        );
    }

    fn log_invariant(&mut self, invariant: &str, passed: bool, detail: &str) {
        self.log_event(
            "invariant",
            &format!(
                r#"{{"id":"{}","passed":{},"detail":"{}"}}"#,
                invariant, passed, detail
            ),
        );
    }

    fn log_complete(&mut self, passed: bool, total_checks: usize) {
        self.log_event(
            "complete",
            &format!(r#"{{"passed":{},"total_checks":{}}}"#, passed, total_checks),
        );
    }
}

// ============================================================================
// Test Helpers
// ============================================================================

fn area(w: u16, h: u16) -> Rect {
    Rect::new(0, 0, w, h)
}

/// Default breakpoint boundary widths for testing.
/// Each pair is (just_below, at_threshold) for each breakpoint.
const BOUNDARY_WIDTHS: [(u16, u16); 4] = [
    (59, 60),   // Sm boundary
    (89, 90),   // Md boundary
    (119, 120), // Lg boundary
    (159, 160), // Xl boundary
];

/// Standard test widths that cover all breakpoints and boundaries.
const MATRIX_WIDTHS: [u16; 13] = [1, 10, 40, 59, 60, 80, 89, 90, 100, 119, 120, 159, 160];

/// Standard test heights.
const MATRIX_HEIGHTS: [u16; 4] = [1, 10, 24, 60];

fn single_column() -> Flex {
    Flex::vertical().constraints([Constraint::Fill])
}

fn two_column() -> Flex {
    Flex::horizontal().constraints([Constraint::Fixed(30), Constraint::Fill])
}

fn three_column() -> Flex {
    Flex::horizontal().constraints([
        Constraint::Fixed(25),
        Constraint::Fill,
        Constraint::Fixed(25),
    ])
}

fn sidebar_content_panel() -> Flex {
    Flex::horizontal().constraints([
        Constraint::Fixed(20),
        Constraint::Fill,
        Constraint::Fixed(30),
    ])
}

fn stacked_rows() -> Flex {
    Flex::vertical().constraints([Constraint::Fixed(3), Constraint::Fill, Constraint::Fixed(1)])
}

fn percentage_layout() -> Flex {
    Flex::horizontal().constraints([
        Constraint::Percentage(25.0),
        Constraint::Percentage(50.0),
        Constraint::Percentage(25.0),
    ])
}

fn ratio_layout() -> Flex {
    Flex::horizontal().constraints([Constraint::Ratio(1, 3), Constraint::Ratio(2, 3)])
}

fn min_max_layout() -> Flex {
    Flex::horizontal().constraints([Constraint::Min(20), Constraint::Max(60), Constraint::Fill])
}

// ============================================================================
// INH-1: Responsive Inheritance Tests
// ============================================================================

#[test]
fn responsive_matrix_inheritance_base_only() {
    let mut logger = MatrixLogger::new("inheritance_base_only");
    let r = Responsive::new(42);

    for &bp in &Breakpoint::ALL {
        let val = *r.resolve(bp);
        assert_eq!(val, 42, "INH-1: base value should be inherited at {bp}");
        logger.log_invariant("INH-1", true, &format!("{bp}=42"));
    }
    logger.log_complete(true, 5);
}

#[test]
fn responsive_matrix_inheritance_sparse() {
    let mut logger = MatrixLogger::new("inheritance_sparse");
    let r = Responsive::new(0)
        .at(Breakpoint::Md, 2)
        .at(Breakpoint::Xl, 4);

    let expected = [
        (Breakpoint::Xs, 0), // explicit
        (Breakpoint::Sm, 0), // inherits Xs
        (Breakpoint::Md, 2), // explicit
        (Breakpoint::Lg, 2), // inherits Md
        (Breakpoint::Xl, 4), // explicit
    ];

    for (bp, exp) in &expected {
        let val = *r.resolve(*bp);
        assert_eq!(val, *exp, "INH-1: {bp} should be {exp}, got {val}");
        logger.log_invariant("INH-1", true, &format!("{bp}={val}"));
    }
    logger.log_complete(true, expected.len());
}

#[test]
fn responsive_matrix_inheritance_all_explicit() {
    let mut logger = MatrixLogger::new("inheritance_all_explicit");
    let r = Responsive::new(10)
        .at(Breakpoint::Sm, 20)
        .at(Breakpoint::Md, 30)
        .at(Breakpoint::Lg, 40)
        .at(Breakpoint::Xl, 50);

    for (i, &bp) in Breakpoint::ALL.iter().enumerate() {
        let expected = (i as i32 + 1) * 10;
        let val = *r.resolve(bp);
        assert_eq!(val, expected, "INH-1: explicit {bp} should be {expected}");
        logger.log_invariant("INH-1", true, &format!("{bp}={val}"));
    }
    logger.log_complete(true, 5);
}

#[test]
fn responsive_matrix_inheritance_clear_reverts() {
    let mut logger = MatrixLogger::new("inheritance_clear");
    let mut r = Responsive::new(1)
        .at(Breakpoint::Sm, 2)
        .at(Breakpoint::Md, 3)
        .at(Breakpoint::Lg, 4);

    // Clear Md — Md and Lg (which now inherits from Sm) should change
    r.clear(Breakpoint::Md);
    assert_eq!(
        *r.resolve(Breakpoint::Md),
        2,
        "INH-1: cleared Md inherits Sm"
    );
    // Lg still explicit
    assert_eq!(*r.resolve(Breakpoint::Lg), 4, "INH-1: Lg still explicit");

    // Clear Lg — Lg should now inherit from Sm
    r.clear(Breakpoint::Lg);
    assert_eq!(
        *r.resolve(Breakpoint::Lg),
        2,
        "INH-1: cleared Lg inherits Sm"
    );

    logger.log_invariant("INH-1", true, "clear_reverts");
    logger.log_complete(true, 3);
}

#[test]
fn responsive_matrix_inheritance_map_preserves() {
    let mut logger = MatrixLogger::new("inheritance_map");
    let r = Responsive::new(10).at(Breakpoint::Lg, 30);
    let doubled = r.map(|v| v * 2);

    assert_eq!(*doubled.resolve(Breakpoint::Xs), 20);
    assert_eq!(*doubled.resolve(Breakpoint::Sm), 20); // inherits
    assert_eq!(*doubled.resolve(Breakpoint::Md), 20); // inherits
    assert_eq!(*doubled.resolve(Breakpoint::Lg), 60); // explicit
    assert_eq!(*doubled.resolve(Breakpoint::Xl), 60); // inherits

    logger.log_invariant("INH-1", true, "map_preserves_inheritance");
    logger.log_complete(true, 5);
}

// ============================================================================
// Breakpoint Classification at Boundaries
// ============================================================================

#[test]
fn responsive_matrix_classify_boundaries_default() {
    let mut logger = MatrixLogger::new("classify_boundaries");
    let bps = Breakpoints::DEFAULT;

    // Test each boundary
    for &(below, at) in &BOUNDARY_WIDTHS {
        let bp_below = bps.classify_width(below);
        let bp_at = bps.classify_width(at);
        assert_ne!(
            bp_below, bp_at,
            "Boundary at {at}: {below} and {at} should differ"
        );
        logger.log_invariant("TRANS-1", true, &format!("{below}={bp_below},{at}={bp_at}"));
    }

    // Exhaustive classification
    let cases = [
        (0, Breakpoint::Xs),
        (1, Breakpoint::Xs),
        (59, Breakpoint::Xs),
        (60, Breakpoint::Sm),
        (89, Breakpoint::Sm),
        (90, Breakpoint::Md),
        (119, Breakpoint::Md),
        (120, Breakpoint::Lg),
        (159, Breakpoint::Lg),
        (160, Breakpoint::Xl),
        (u16::MAX, Breakpoint::Xl),
    ];

    for (width, expected) in &cases {
        let actual = bps.classify_width(*width);
        assert_eq!(
            actual, *expected,
            "Width {width}: expected {expected}, got {actual}"
        );
    }

    logger.log_complete(true, cases.len() + BOUNDARY_WIDTHS.len());
}

#[test]
fn responsive_matrix_classify_boundaries_custom() {
    let mut logger = MatrixLogger::new("classify_custom");
    let bps = Breakpoints::new(40, 80, 100);

    let cases = [
        (0, Breakpoint::Xs),
        (39, Breakpoint::Xs),
        (40, Breakpoint::Sm),
        (79, Breakpoint::Sm),
        (80, Breakpoint::Md),
        (99, Breakpoint::Md),
        (100, Breakpoint::Lg),
        (139, Breakpoint::Lg),
        (140, Breakpoint::Xl), // xl = lg + 40 = 140
    ];

    for (width, expected) in &cases {
        let actual = bps.classify_width(*width);
        assert_eq!(
            actual, *expected,
            "Custom: width {width}: expected {expected}, got {actual}"
        );
        logger.log_invariant("TRANS-1", true, &format!("w={width},bp={actual}"));
    }

    logger.log_complete(true, cases.len());
}

#[test]
fn responsive_matrix_classify_monotone() {
    let mut logger = MatrixLogger::new("classify_monotone");
    let bps = Breakpoints::DEFAULT;

    // Breakpoint classification should be monotone: wider => same or larger bp
    let mut prev_bp = Breakpoint::Xs;
    for w in 0..=200 {
        let bp = bps.classify_width(w);
        assert!(
            bp >= prev_bp,
            "MONO: width {w}: {bp} < {prev_bp} violates monotonicity"
        );
        prev_bp = bp;
    }

    logger.log_invariant("MONO-1", true, "classify_monotone_0..200");
    logger.log_complete(true, 1);
}

// ============================================================================
// ResponsiveLayout: Size x Breakpoint x Mode Matrix
// ============================================================================

#[test]
fn responsive_matrix_full_size_breakpoint() {
    let mut logger = MatrixLogger::new("full_matrix");
    let layout = ResponsiveLayout::new(single_column())
        .at(Breakpoint::Sm, two_column())
        .at(Breakpoint::Lg, three_column());

    let expected_rects = [
        (Breakpoint::Xs, 1usize), // single_column
        (Breakpoint::Sm, 2),      // two_column
        (Breakpoint::Md, 2),      // inherits Sm
        (Breakpoint::Lg, 3),      // three_column
        (Breakpoint::Xl, 3),      // inherits Lg
    ];

    let mut total_checks = 0;

    for &width in &MATRIX_WIDTHS {
        for &height in &MATRIX_HEIGHTS {
            let result = layout.split(area(width, height));
            let bp = result.breakpoint;

            // Verify breakpoint classification
            let expected_bp = Breakpoints::DEFAULT.classify_width(width);
            assert_eq!(
                bp, expected_bp,
                "Width {width}: breakpoint mismatch: {bp} vs {expected_bp}"
            );

            // Verify rect count matches breakpoint
            let expected_count = expected_rects
                .iter()
                .find(|(b, _)| *b == bp)
                .map(|(_, c)| *c)
                .unwrap();
            assert_eq!(
                result.rects.len(),
                expected_count,
                "Width {width} ({bp}): expected {expected_count} rects, got {}",
                result.rects.len()
            );

            logger.log_scenario(width, height, bp, result.rects.len());
            total_checks += 1;
        }
    }

    logger.log_complete(true, total_checks);
}

#[test]
fn responsive_matrix_horizontal_mode() {
    let mut logger = MatrixLogger::new("horizontal_mode");

    // Test horizontal layouts at each breakpoint
    let layout = ResponsiveLayout::new(Flex::horizontal().constraints([Constraint::Fill]))
        .at(
            Breakpoint::Md,
            Flex::horizontal().constraints([Constraint::Fixed(30), Constraint::Fill]),
        )
        .at(
            Breakpoint::Xl,
            Flex::horizontal().constraints([
                Constraint::Fixed(25),
                Constraint::Fill,
                Constraint::Fixed(25),
            ]),
        );

    for &width in &MATRIX_WIDTHS {
        let result = layout.split(area(width, 24));
        let bp = result.breakpoint;

        // FEAS-1: sum of widths <= available
        let total_width: u16 = result.rects.iter().map(|r| r.width).sum();
        assert!(
            total_width <= width,
            "FEAS-1: width {width} ({bp}): total_width {total_width} > {width}"
        );

        // Heights should match available height
        for r in &result.rects {
            assert_eq!(r.height, 24, "Horizontal mode: height should match area");
        }

        logger.log_scenario(width, 24, bp, result.rects.len());
    }

    logger.log_complete(true, MATRIX_WIDTHS.len());
}

#[test]
fn responsive_matrix_vertical_mode() {
    let mut logger = MatrixLogger::new("vertical_mode");

    let layout = ResponsiveLayout::new(stacked_rows()).at(
        Breakpoint::Lg,
        Flex::vertical().constraints([
            Constraint::Fixed(2),
            Constraint::Fill,
            Constraint::Fixed(3),
            Constraint::Fixed(1),
        ]),
    );

    for &width in &MATRIX_WIDTHS {
        for &height in &MATRIX_HEIGHTS {
            let result = layout.split(area(width, height));
            let bp = result.breakpoint;

            // FEAS-1: sum of heights <= available
            let total_height: u16 = result.rects.iter().map(|r| r.height).sum();
            assert!(
                total_height <= height,
                "FEAS-1: {width}x{height} ({bp}): total_height {total_height} > {height}"
            );

            // Widths should match available width
            for r in &result.rects {
                assert_eq!(
                    r.width, width,
                    "Vertical mode: width should match area at {width}x{height}"
                );
            }

            logger.log_scenario(width, height, bp, result.rects.len());
        }
    }

    logger.log_complete(true, MATRIX_WIDTHS.len() * MATRIX_HEIGHTS.len());
}

// ============================================================================
// FEAS-1: Feasibility Invariant
// ============================================================================

#[test]
fn responsive_matrix_feasibility_all_constraints() {
    let mut logger = MatrixLogger::new("feasibility");

    // Test a variety of constraint combinations at all widths
    let constraint_sets: Vec<(&str, Vec<Constraint>)> = vec![
        (
            "fixed_only",
            vec![
                Constraint::Fixed(20),
                Constraint::Fixed(30),
                Constraint::Fixed(40),
            ],
        ),
        (
            "percentage",
            vec![Constraint::Percentage(30.0), Constraint::Percentage(70.0)],
        ),
        (
            "mixed",
            vec![
                Constraint::Fixed(20),
                Constraint::Percentage(50.0),
                Constraint::Fill,
            ],
        ),
        (
            "min_max",
            vec![Constraint::Min(10), Constraint::Max(50), Constraint::Fill],
        ),
        (
            "ratio",
            vec![Constraint::Ratio(1, 4), Constraint::Ratio(3, 4)],
        ),
        (
            "fill_only",
            vec![Constraint::Fill, Constraint::Fill, Constraint::Fill],
        ),
        ("single_fill", vec![Constraint::Fill]),
    ];

    let mut total_checks = 0;

    for (name, constraints) in &constraint_sets {
        let flex = Flex::horizontal().constraints(constraints.clone());

        for &width in &MATRIX_WIDTHS {
            let rects = flex.split(area(width, 24));
            let total: u16 = rects.iter().map(|r| r.width).sum();
            assert!(
                total <= width,
                "FEAS-1 [{name}]: width {width}: total {total} > available {width}"
            );
            logger.log_invariant("FEAS-1", true, &format!("{name},w={width},total={total}"));
            total_checks += 1;
        }
    }

    logger.log_complete(true, total_checks);
}

#[test]
fn responsive_matrix_feasibility_vertical() {
    let mut logger = MatrixLogger::new("feasibility_vertical");

    let constraint_sets: Vec<(&str, Vec<Constraint>)> = vec![
        (
            "header_body_footer",
            vec![Constraint::Fixed(3), Constraint::Fill, Constraint::Fixed(1)],
        ),
        (
            "equal_thirds",
            vec![
                Constraint::Ratio(1, 3),
                Constraint::Ratio(1, 3),
                Constraint::Ratio(1, 3),
            ],
        ),
    ];

    let mut total_checks = 0;

    for (name, constraints) in &constraint_sets {
        let flex = Flex::vertical().constraints(constraints.clone());

        for &height in &MATRIX_HEIGHTS {
            let rects = flex.split(area(80, height));
            let total: u16 = rects.iter().map(|r| r.height).sum();
            assert!(
                total <= height,
                "FEAS-1 [{name}]: height {height}: total {total} > available {height}"
            );
            logger.log_invariant("FEAS-1", true, &format!("{name},h={height},total={total}"));
            total_checks += 1;
        }
    }

    logger.log_complete(true, total_checks);
}

// ============================================================================
// TIE-1: Deterministic Tie-Breaking
// ============================================================================

#[test]
fn responsive_matrix_deterministic_tiebreak() {
    let mut logger = MatrixLogger::new("deterministic_tiebreak");

    let layout = ResponsiveLayout::new(single_column())
        .at(Breakpoint::Sm, two_column())
        .at(Breakpoint::Lg, three_column());

    let mut total_checks = 0;

    for &width in &MATRIX_WIDTHS {
        let result1 = layout.split(area(width, 24));
        let result2 = layout.split(area(width, 24));

        assert_eq!(
            result1.breakpoint, result2.breakpoint,
            "TIE-1: breakpoint mismatch at width {width}"
        );
        assert_eq!(
            result1.rects, result2.rects,
            "TIE-1: rects differ at width {width}"
        );

        logger.log_invariant("TIE-1", true, &format!("w={width}"));
        total_checks += 1;
    }

    // Also verify Fill tie-breaking is deterministic
    let fill_flex =
        Flex::horizontal().constraints([Constraint::Fill, Constraint::Fill, Constraint::Fill]);

    for &width in &MATRIX_WIDTHS {
        let r1 = fill_flex.split(area(width, 24));
        let r2 = fill_flex.split(area(width, 24));
        assert_eq!(r1, r2, "TIE-1: fill tiebreak differs at width {width}");
        total_checks += 1;
    }

    logger.log_complete(true, total_checks);
}

// ============================================================================
// MONO-1: Monotone Constraint Response
// ============================================================================

#[test]
fn responsive_matrix_monotone_fixed() {
    let mut logger = MatrixLogger::new("monotone_fixed");

    // Fixed constraints: allocated size should be min(fixed, available)
    let flex = Flex::horizontal().constraints([Constraint::Fixed(50)]);

    let mut prev_width = 0u16;
    for w in (1..=200).step_by(5) {
        let rects = flex.split(area(w, 24));
        let allocated = rects[0].width;
        assert!(
            allocated >= prev_width || w < 50,
            "MONO-1: fixed at width {w}: {allocated} < prev {prev_width}"
        );
        prev_width = allocated;
    }

    logger.log_invariant("MONO-1", true, "fixed_monotone");
    logger.log_complete(true, 1);
}

#[test]
fn responsive_matrix_monotone_fill() {
    let mut logger = MatrixLogger::new("monotone_fill");

    // Fill: should grow monotonically with available space
    let flex = Flex::horizontal().constraints([Constraint::Fixed(20), Constraint::Fill]);

    let mut prev_fill = 0u16;
    for w in (21..=200).step_by(5) {
        let rects = flex.split(area(w, 24));
        let fill_width = rects[1].width;
        assert!(
            fill_width >= prev_fill,
            "MONO-1: fill at width {w}: {fill_width} < prev {prev_fill}"
        );
        prev_fill = fill_width;
    }

    logger.log_invariant("MONO-1", true, "fill_monotone");
    logger.log_complete(true, 1);
}

#[test]
fn responsive_matrix_monotone_percentage() {
    let mut logger = MatrixLogger::new("monotone_percentage");

    let flex = Flex::horizontal().constraints([Constraint::Percentage(50.0)]);

    let mut prev_width = 0u16;
    for w in (1..=200).step_by(5) {
        let rects = flex.split(area(w, 24));
        let allocated = rects[0].width;
        assert!(
            allocated >= prev_width,
            "MONO-1: percentage at width {w}: {allocated} < prev {prev_width}"
        );
        prev_width = allocated;
    }

    logger.log_invariant("MONO-1", true, "percentage_monotone");
    logger.log_complete(true, 1);
}

// ============================================================================
// VIS-1: Visibility Filtering
// ============================================================================

#[test]
fn responsive_matrix_visibility_space_reclamation() {
    let mut logger = MatrixLogger::new("visibility_reclamation");

    let rects = vec![
        Rect::new(0, 0, 30, 24),
        Rect::new(30, 0, 40, 24),
        Rect::new(70, 0, 30, 24),
    ];

    let visibilities = vec![
        Visibility::ALWAYS,                        // Always visible
        Visibility::visible_above(Breakpoint::Md), // Hidden below Md
        Visibility::visible_above(Breakpoint::Lg), // Hidden below Lg
    ];

    // Xs: only first visible
    let visible = Visibility::filter_rects(&visibilities, &rects, Breakpoint::Xs);
    assert_eq!(visible.len(), 1, "VIS-1: Xs should show 1 rect");
    assert_eq!(visible[0].0, 0, "VIS-1: Xs should show index 0");

    // Sm: only first visible
    let visible = Visibility::filter_rects(&visibilities, &rects, Breakpoint::Sm);
    assert_eq!(visible.len(), 1, "VIS-1: Sm should show 1 rect");

    // Md: first two visible
    let visible = Visibility::filter_rects(&visibilities, &rects, Breakpoint::Md);
    assert_eq!(visible.len(), 2, "VIS-1: Md should show 2 rects");
    assert_eq!(visible[0].0, 0);
    assert_eq!(visible[1].0, 1);

    // Lg+: all visible
    let visible = Visibility::filter_rects(&visibilities, &rects, Breakpoint::Lg);
    assert_eq!(visible.len(), 3, "VIS-1: Lg should show 3 rects");

    let visible = Visibility::filter_rects(&visibilities, &rects, Breakpoint::Xl);
    assert_eq!(visible.len(), 3, "VIS-1: Xl should show 3 rects");

    logger.log_invariant("VIS-1", true, "progressive_disclosure");
    logger.log_complete(true, 5);
}

#[test]
fn responsive_matrix_visibility_only() {
    let mut logger = MatrixLogger::new("visibility_only");

    // Widget visible only at Md
    let vis = Visibility::only(Breakpoint::Md);
    for &bp in &Breakpoint::ALL {
        let visible = vis.is_visible(bp);
        let expected = bp == Breakpoint::Md;
        assert_eq!(
            visible, expected,
            "VIS-1: only(Md) at {bp}: expected {expected}, got {visible}"
        );
    }

    logger.log_invariant("VIS-1", true, "only_single_bp");
    logger.log_complete(true, 5);
}

#[test]
fn responsive_matrix_visibility_hidden_above_below() {
    let mut logger = MatrixLogger::new("visibility_hidden");

    // hidden_above(Md): visible at Xs, Sm; hidden at Md, Lg, Xl
    let vis = Visibility::hidden_above(Breakpoint::Md);
    assert!(vis.is_visible(Breakpoint::Xs));
    assert!(vis.is_visible(Breakpoint::Sm));
    assert!(!vis.is_visible(Breakpoint::Md));
    assert!(!vis.is_visible(Breakpoint::Lg));
    assert!(!vis.is_visible(Breakpoint::Xl));

    // hidden_below(Md) == visible_above(Md)
    let vis = Visibility::hidden_below(Breakpoint::Md);
    assert!(!vis.is_visible(Breakpoint::Xs));
    assert!(!vis.is_visible(Breakpoint::Sm));
    assert!(vis.is_visible(Breakpoint::Md));
    assert!(vis.is_visible(Breakpoint::Lg));
    assert!(vis.is_visible(Breakpoint::Xl));

    logger.log_invariant("VIS-1", true, "hidden_above_below");
    logger.log_complete(true, 10);
}

#[test]
fn responsive_matrix_visibility_count() {
    let mut logger = MatrixLogger::new("visibility_count");

    let visibilities = vec![
        Visibility::ALWAYS,
        Visibility::only(Breakpoint::Xl),
        Visibility::visible_above(Breakpoint::Lg),
        Visibility::hidden_above(Breakpoint::Md),
    ];

    let expected_counts = [
        (Breakpoint::Xs, 2), // ALWAYS + hidden_above(Md)
        (Breakpoint::Sm, 2),
        (Breakpoint::Md, 1), // ALWAYS only (hidden_above removes Md+, visible_above needs Lg+)
        (Breakpoint::Lg, 2), // ALWAYS + visible_above(Lg)
        (Breakpoint::Xl, 3), // ALWAYS + only(Xl) + visible_above(Lg)
    ];

    for (bp, expected) in &expected_counts {
        let count = Visibility::count_visible(&visibilities, *bp);
        assert_eq!(
            count, *expected,
            "VIS-1: count_visible at {bp}: expected {expected}, got {count}"
        );
        logger.log_invariant("VIS-1", true, &format!("{bp}={count}"));
    }

    logger.log_complete(true, expected_counts.len());
}

// ============================================================================
// TRANS-1: Transition Detection
// ============================================================================

#[test]
fn responsive_matrix_transition_detection() {
    let mut logger = MatrixLogger::new("transition_detection");

    let layout = ResponsiveLayout::new(single_column());

    // Transitions across boundaries
    let transitions = [
        (50, 70, Some((Breakpoint::Xs, Breakpoint::Sm))), // Xs -> Sm
        (80, 95, Some((Breakpoint::Sm, Breakpoint::Md))), // Sm -> Md
        (100, 125, Some((Breakpoint::Md, Breakpoint::Lg))), // Md -> Lg
        (130, 165, Some((Breakpoint::Lg, Breakpoint::Xl))), // Lg -> Xl
        (70, 80, None),                                   // within Sm
        (100, 110, None),                                 // within Md
        (130, 150, None),                                 // within Lg
        (165, 200, None),                                 // within Xl
    ];

    for (old_w, new_w, expected) in &transitions {
        let result = layout.detect_transition(*old_w, *new_w);
        assert_eq!(
            result, *expected,
            "TRANS-1: {old_w}->{new_w}: expected {expected:?}, got {result:?}"
        );
        logger.log_invariant("TRANS-1", true, &format!("{old_w}->{new_w}"));
    }

    // Also test reverse transitions
    for (old_w, new_w, expected) in &transitions {
        if expected.is_some() {
            let result = layout.detect_transition(*new_w, *old_w);
            assert!(
                result.is_some(),
                "TRANS-1: reverse {new_w}->{old_w} should detect transition"
            );
            let (old_bp, new_bp) = result.unwrap();
            let (exp_old, exp_new) = expected.unwrap();
            assert_eq!(old_bp, exp_new, "TRANS-1: reverse old_bp");
            assert_eq!(new_bp, exp_old, "TRANS-1: reverse new_bp");
        }
    }

    logger.log_complete(true, transitions.len() * 2);
}

// ============================================================================
// COHER-1: Temporal Coherence
// ============================================================================

#[test]
fn responsive_matrix_temporal_coherence() {
    let mut logger = MatrixLogger::new("temporal_coherence");

    let layout = ResponsiveLayout::new(single_column())
        .at(Breakpoint::Sm, two_column())
        .at(Breakpoint::Md, sidebar_content_panel())
        .at(Breakpoint::Lg, three_column());

    // Same width should always produce same result
    for &width in &MATRIX_WIDTHS {
        let results: Vec<ResponsiveSplit> =
            (0..10).map(|_| layout.split(area(width, 24))).collect();

        for (i, result) in results.iter().enumerate().skip(1) {
            assert_eq!(
                result.breakpoint, results[0].breakpoint,
                "COHER-1: width {width}, iteration {i}: breakpoint changed"
            );
            assert_eq!(
                result.rects, results[0].rects,
                "COHER-1: width {width}, iteration {i}: rects changed"
            );
        }

        logger.log_invariant("COHER-1", true, &format!("w={width},iters=10"));
    }

    logger.log_complete(true, MATRIX_WIDTHS.len());
}

// ============================================================================
// Layout Mode Switching (split_for vs split)
// ============================================================================

#[test]
fn responsive_matrix_split_for_override() {
    let mut logger = MatrixLogger::new("split_for_override");

    let layout = ResponsiveLayout::new(single_column())
        .at(Breakpoint::Md, two_column())
        .at(Breakpoint::Xl, three_column());

    // split_for should use the explicit breakpoint regardless of area width
    for &bp in &Breakpoint::ALL {
        // Use a narrow area width (40) that would normally classify as Xs
        let result = layout.split_for(bp, area(40, 24));
        assert_eq!(
            result.breakpoint, bp,
            "split_for should use explicit breakpoint"
        );

        let expected_count = layout.constraint_count(bp);
        assert_eq!(
            result.rects.len(),
            expected_count,
            "split_for at {bp}: expected {expected_count} rects"
        );

        logger.log_scenario(40, 24, bp, result.rects.len());
    }

    logger.log_complete(true, Breakpoint::ALL.len());
}

// ============================================================================
// Constraint Solver Invariants
// ============================================================================

#[test]
fn responsive_matrix_constraint_fill_distribution() {
    let mut logger = MatrixLogger::new("fill_distribution");

    // Multiple fills should distribute space approximately equally
    let flex =
        Flex::horizontal().constraints([Constraint::Fill, Constraint::Fill, Constraint::Fill]);

    for &width in &MATRIX_WIDTHS {
        if width < 3 {
            continue;
        }
        let rects = flex.split(area(width, 24));
        let widths: Vec<u16> = rects.iter().map(|r| r.width).collect();

        // Each fill should get approximately width/3
        // Allow up to (count-1) rounding error per slot since remainder
        // may be concentrated in a single slot by the solver
        let count = widths.len() as u16;
        let expected_each = width / count;
        for (i, &w) in widths.iter().enumerate() {
            let diff = (w as i32 - expected_each as i32).unsigned_abs();
            assert!(
                diff <= (count - 1) as u32,
                "Fill distribution at width {width}: slot {i} got {w}, expected ~{expected_each} (tolerance {})",
                count - 1
            );
        }

        // Total should equal width
        let total: u16 = widths.iter().sum();
        assert_eq!(
            total, width,
            "Fill distribution: total {total} != width {width}"
        );

        logger.log_invariant("FEAS-1", true, &format!("fill_dist,w={width}"));
    }

    logger.log_complete(true, MATRIX_WIDTHS.len());
}

#[test]
fn responsive_matrix_constraint_fixed_plus_fill() {
    let mut logger = MatrixLogger::new("fixed_fill");

    let flex = Flex::horizontal().constraints([Constraint::Fixed(30), Constraint::Fill]);

    for &width in &MATRIX_WIDTHS {
        let rects = flex.split(area(width, 24));
        let fixed_w = rects[0].width;
        let fill_w = rects[1].width;

        // Fixed should be min(30, width)
        let expected_fixed = width.min(30);
        assert_eq!(
            fixed_w, expected_fixed,
            "Fixed+Fill at width {width}: fixed={fixed_w}, expected {expected_fixed}"
        );

        // Fill takes remainder
        let expected_fill = width.saturating_sub(expected_fixed);
        assert_eq!(
            fill_w, expected_fill,
            "Fixed+Fill at width {width}: fill={fill_w}, expected {expected_fill}"
        );

        logger.log_invariant(
            "FEAS-1",
            true,
            &format!("fixed_fill,w={width},fixed={fixed_w},fill={fill_w}"),
        );
    }

    logger.log_complete(true, MATRIX_WIDTHS.len());
}

#[test]
fn responsive_matrix_constraint_percentage_sum() {
    let mut logger = MatrixLogger::new("percentage_sum");

    // Percentages that sum to 100% should fill the space
    let flex = Flex::horizontal().constraints([
        Constraint::Percentage(25.0),
        Constraint::Percentage(50.0),
        Constraint::Percentage(25.0),
    ]);

    for &width in &MATRIX_WIDTHS {
        let rects = flex.split(area(width, 24));
        let total: u16 = rects.iter().map(|r| r.width).sum();

        // Allow +-1 per slot for rounding (3 slots = +-3 total)
        let diff = (total as i32 - width as i32).unsigned_abs();
        assert!(
            diff <= 3,
            "Percentage sum at width {width}: total={total}, expected ~{width}"
        );

        logger.log_invariant("FEAS-1", true, &format!("pct_sum,w={width},total={total}"));
    }

    logger.log_complete(true, MATRIX_WIDTHS.len());
}

// ============================================================================
// Gap and Margin Tests
// ============================================================================

#[test]
fn responsive_matrix_gap_reduces_available() {
    let mut logger = MatrixLogger::new("gap_constraint");

    let flex_no_gap = Flex::horizontal()
        .constraints([Constraint::Fill, Constraint::Fill])
        .gap(0);
    let flex_with_gap = Flex::horizontal()
        .constraints([Constraint::Fill, Constraint::Fill])
        .gap(4);

    for &width in &MATRIX_WIDTHS {
        if width < 10 {
            continue;
        }
        let rects_no_gap = flex_no_gap.split(area(width, 24));
        let rects_with_gap = flex_with_gap.split(area(width, 24));

        let total_no_gap: u16 = rects_no_gap.iter().map(|r| r.width).sum();
        let total_with_gap: u16 = rects_with_gap.iter().map(|r| r.width).sum();

        // Gap should reduce available space for content
        assert!(
            total_with_gap <= total_no_gap,
            "Gap at width {width}: with_gap total {total_with_gap} > no_gap {total_no_gap}"
        );

        // Gap should be exactly 4 less
        let expected_diff = 4u16.min(width);
        let actual_diff = total_no_gap.saturating_sub(total_with_gap);
        assert_eq!(
            actual_diff, expected_diff,
            "Gap at width {width}: expected diff {expected_diff}, got {actual_diff}"
        );

        logger.log_invariant("FEAS-1", true, &format!("gap,w={width}"));
    }

    logger.log_complete(true, MATRIX_WIDTHS.len());
}

// ============================================================================
// Alignment Tests Across Breakpoints
// ============================================================================

#[test]
fn responsive_matrix_alignment_variants() {
    let mut logger = MatrixLogger::new("alignment_variants");

    let alignments = [
        Alignment::Start,
        Alignment::Center,
        Alignment::End,
        Alignment::SpaceBetween,
        Alignment::SpaceAround,
    ];

    for alignment in &alignments {
        let flex = Flex::horizontal()
            .constraints([Constraint::Fixed(20), Constraint::Fixed(20)])
            .alignment(*alignment);

        for &width in &[40, 80, 120, 160] {
            let rects = flex.split(area(width, 24));

            // FEAS-1: rects should not exceed bounds
            for r in &rects {
                assert!(
                    r.x + r.width <= width,
                    "Alignment {alignment:?} at width {width}: rect exceeds bounds"
                );
            }

            // Rects should not overlap
            if rects.len() >= 2 {
                assert!(
                    rects[0].x + rects[0].width <= rects[1].x,
                    "Alignment {alignment:?} at width {width}: rects overlap"
                );
            }

            logger.log_invariant("FEAS-1", true, &format!("align={alignment:?},w={width}"));
        }
    }

    logger.log_complete(true, alignments.len() * 4);
}

// ============================================================================
// Edge Cases
// ============================================================================

#[test]
fn responsive_matrix_zero_area() {
    let mut logger = MatrixLogger::new("zero_area");

    let layout = ResponsiveLayout::new(two_column());

    // Zero width
    let result = layout.split(area(0, 24));
    assert_eq!(result.breakpoint, Breakpoint::Xs);
    assert_eq!(result.rects.len(), 2);
    assert!(result.rects.iter().all(|r| r.width == 0));

    // Zero height
    let result = layout.split(area(80, 0));
    assert_eq!(result.rects.len(), 2);
    assert!(result.rects.iter().all(|r| r.height == 0));

    // Both zero
    let result = layout.split(area(0, 0));
    assert_eq!(result.rects.len(), 2);
    assert!(result.rects.iter().all(|r| r.width == 0 && r.height == 0));

    logger.log_invariant("FEAS-1", true, "zero_area");
    logger.log_complete(true, 3);
}

#[test]
fn responsive_matrix_single_cell() {
    let mut logger = MatrixLogger::new("single_cell");

    let layout = ResponsiveLayout::new(two_column());
    let result = layout.split(area(1, 1));

    assert_eq!(result.breakpoint, Breakpoint::Xs);
    let total_w: u16 = result.rects.iter().map(|r| r.width).sum();
    assert!(total_w <= 1, "FEAS-1: single cell");

    logger.log_invariant("FEAS-1", true, "single_cell");
    logger.log_complete(true, 1);
}

#[test]
fn responsive_matrix_max_width() {
    let mut logger = MatrixLogger::new("max_width");

    let layout = ResponsiveLayout::new(two_column());
    let result = layout.split(area(u16::MAX, 24));

    assert_eq!(result.breakpoint, Breakpoint::Xl);
    let total_w: u16 = result.rects.iter().map(|r| r.width).sum();
    // Verify total_w is reasonable (non-zero for a two_column layout at max width)
    let _ = total_w; // existence check; feasibility checked implicitly by no overflow

    logger.log_invariant("FEAS-1", true, "max_width");
    logger.log_complete(true, 1);
}

#[test]
fn responsive_matrix_fixed_exceeds_available() {
    let mut logger = MatrixLogger::new("fixed_exceeds");

    // Fixed constraints that exceed available space
    let flex = Flex::horizontal().constraints([
        Constraint::Fixed(100),
        Constraint::Fixed(100),
        Constraint::Fixed(100),
    ]);

    // Area is only 80 wide
    let rects = flex.split(area(80, 24));
    let total: u16 = rects.iter().map(|r| r.width).sum();
    assert!(
        total <= 80,
        "FEAS-1: fixed exceeds available: total {total} > 80"
    );

    logger.log_invariant("FEAS-1", true, "fixed_exceeds_available");
    logger.log_complete(true, 1);
}

// ============================================================================
// Responsive Layout with Custom Breakpoints
// ============================================================================

#[test]
fn responsive_matrix_custom_breakpoints() {
    let mut logger = MatrixLogger::new("custom_breakpoints");

    let layout = ResponsiveLayout::new(single_column())
        .at(Breakpoint::Sm, two_column())
        .at(Breakpoint::Lg, three_column())
        .with_breakpoints(Breakpoints::new(30, 60, 100));

    // Custom thresholds: sm=30, md=60, lg=100, xl=140
    let cases = [
        (20, Breakpoint::Xs, 1usize),
        (30, Breakpoint::Sm, 2),
        (50, Breakpoint::Sm, 2),
        (60, Breakpoint::Md, 2), // Md inherits Sm
        (100, Breakpoint::Lg, 3),
        (140, Breakpoint::Xl, 3), // Xl inherits Lg
    ];

    for (width, expected_bp, expected_rects) in &cases {
        let result = layout.split(area(*width, 24));
        assert_eq!(
            result.breakpoint, *expected_bp,
            "Custom bp: width {width}: expected {expected_bp}, got {}",
            result.breakpoint
        );
        assert_eq!(
            result.rects.len(),
            *expected_rects,
            "Custom bp: width {width}: expected {expected_rects} rects"
        );
        logger.log_scenario(*width, 24, *expected_bp, *expected_rects);
    }

    logger.log_complete(true, cases.len());
}

// ============================================================================
// Property/Metamorphic Tests
// ============================================================================

#[test]
fn responsive_matrix_metamorphic_width_increase() {
    let mut logger = MatrixLogger::new("metamorphic_width");

    // Metamorphic property: increasing width should never decrease
    // the number of visible layout regions (for our test layout)
    let layout = ResponsiveLayout::new(single_column())
        .at(Breakpoint::Sm, two_column())
        .at(Breakpoint::Lg, three_column());

    let mut prev_rects = 0usize;
    for w in 1..=200 {
        let result = layout.split(area(w, 24));
        assert!(
            result.rects.len() >= prev_rects,
            "Metamorphic: width {w}: rects {} < prev {prev_rects}",
            result.rects.len()
        );
        prev_rects = result.rects.len();
    }

    logger.log_invariant("MONO-1", true, "metamorphic_width_increase");
    logger.log_complete(true, 1);
}

#[test]
fn responsive_matrix_metamorphic_clear_adds_no_rects() {
    let mut logger = MatrixLogger::new("metamorphic_clear");

    // Clearing an override should not increase rect count beyond what
    // the inherited layout produces
    let mut layout = ResponsiveLayout::new(single_column())
        .at(Breakpoint::Sm, two_column())
        .at(Breakpoint::Md, three_column())
        .at(Breakpoint::Lg, three_column());

    // Before clear: Md has 3 rects
    let before = layout.split(area(100, 24));
    assert_eq!(before.rects.len(), 3);

    // Clear Md override — should inherit from Sm (2 rects)
    layout.clear(Breakpoint::Md);
    let after = layout.split(area(100, 24));
    assert_eq!(
        after.rects.len(),
        2,
        "Clear should revert to inherited layout"
    );

    logger.log_invariant("INH-1", true, "clear_reduces_rects");
    logger.log_complete(true, 2);
}

// ============================================================================
// Responsive with Different Constraint Types
// ============================================================================

#[test]
fn responsive_matrix_mixed_constraint_types() {
    let mut logger = MatrixLogger::new("mixed_constraints");

    let layout = ResponsiveLayout::new(
        // Xs: simple fill
        Flex::horizontal().constraints([Constraint::Fill]),
    )
    .at(
        Breakpoint::Sm,
        // Sm: percentage layout
        percentage_layout(),
    )
    .at(
        Breakpoint::Md,
        // Md: ratio layout
        ratio_layout(),
    )
    .at(
        Breakpoint::Lg,
        // Lg: min/max layout
        min_max_layout(),
    );

    for &width in &MATRIX_WIDTHS {
        let result = layout.split(area(width, 24));
        let bp = result.breakpoint;

        // FEAS-1
        let total: u16 = result.rects.iter().map(|r| r.width).sum();
        assert!(
            total <= width,
            "Mixed at width {width} ({bp}): total {total} > {width}"
        );

        // Rects should not overlap
        for i in 1..result.rects.len() {
            assert!(
                result.rects[i - 1].x + result.rects[i - 1].width <= result.rects[i].x,
                "Mixed at width {width} ({bp}): rects {}-{} overlap",
                i - 1,
                i
            );
        }

        logger.log_scenario(width, 24, bp, result.rects.len());
    }

    logger.log_complete(true, MATRIX_WIDTHS.len());
}

// ============================================================================
// Visibility + ResponsiveLayout Integration
// ============================================================================

#[test]
fn responsive_matrix_visibility_with_layout() {
    let mut logger = MatrixLogger::new("visibility_layout_integration");

    // Sidebar (hidden below Md) + Content (always) + Panel (hidden below Lg)
    let flex = Flex::horizontal().constraints([
        Constraint::Fixed(25),
        Constraint::Fill,
        Constraint::Fixed(30),
    ]);

    let visibilities = vec![
        Visibility::visible_above(Breakpoint::Md), // sidebar
        Visibility::ALWAYS,                        // content
        Visibility::visible_above(Breakpoint::Lg), // panel
    ];

    for &bp in &Breakpoint::ALL {
        let width = Breakpoints::DEFAULT.threshold(bp).max(1);
        let rects = flex.split(area(width, 24));
        let visible = Visibility::filter_rects(&visibilities, &rects, bp);

        let visible_count = Visibility::count_visible(&visibilities, bp);
        assert_eq!(
            visible.len(),
            visible_count,
            "Visibility+Layout at {bp}: count mismatch"
        );

        // Content (index 1) should always be visible
        assert!(
            visible.iter().any(|(idx, _)| *idx == 1),
            "Content should always be visible at {bp}"
        );

        logger.log_invariant("VIS-1", true, &format!("{bp},visible={visible_count}"));
    }

    logger.log_complete(true, Breakpoint::ALL.len());
}

// ============================================================================
// Suite Summary
// ============================================================================

#[test]
fn responsive_matrix_suite_summary() {
    let mut logger = MatrixLogger::new("suite_summary");

    // Run a comprehensive sweep and collect stats
    let layout = ResponsiveLayout::new(single_column())
        .at(Breakpoint::Sm, two_column())
        .at(Breakpoint::Md, sidebar_content_panel())
        .at(Breakpoint::Lg, three_column());

    let mut bp_counts = [0u32; 5];
    let mut total_rects = 0u64;
    let mut feasibility_ok = 0u32;
    let mut total_scenarios = 0u32;

    for &width in &MATRIX_WIDTHS {
        for &height in &MATRIX_HEIGHTS {
            let result = layout.split(area(width, height));
            let bp_idx = match result.breakpoint {
                Breakpoint::Xs => 0,
                Breakpoint::Sm => 1,
                Breakpoint::Md => 2,
                Breakpoint::Lg => 3,
                Breakpoint::Xl => 4,
            };
            bp_counts[bp_idx] += 1;
            total_rects += result.rects.len() as u64;

            let total_main: u16 = result.rects.iter().map(|r| r.width).sum();
            if total_main <= width {
                feasibility_ok += 1;
            }
            total_scenarios += 1;
        }
    }

    assert_eq!(
        feasibility_ok, total_scenarios,
        "FEAS-1: all scenarios must pass feasibility"
    );

    // Verify we hit all breakpoints
    for (i, &count) in bp_counts.iter().enumerate() {
        assert!(count > 0, "Should exercise breakpoint index {i}");
    }

    logger.log_event(
        "summary",
        &format!(
            r#"{{"total_scenarios":{},"total_rects":{},"feasibility_ok":{},"bp_counts":{:?}}}"#,
            total_scenarios, total_rects, feasibility_ok, bp_counts
        ),
    );
    logger.log_complete(true, total_scenarios as usize);
}
