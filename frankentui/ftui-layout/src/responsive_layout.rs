#![forbid(unsafe_code)]

//! Responsive layout switching: different [`Flex`] configurations per breakpoint.
//!
//! [`ResponsiveLayout`] maps [`Breakpoint`] tiers to [`Flex`] layouts. When
//! splitting an area, it auto-detects the current breakpoint from the area
//! width and resolves the appropriate layout using [`Responsive`] inheritance.
//!
//! # Usage
//!
//! ```ignore
//! use ftui_layout::{Flex, Constraint, Breakpoint, Breakpoints, ResponsiveLayout};
//!
//! // Mobile: single column. Desktop: sidebar + content.
//! let layout = ResponsiveLayout::new(
//!         Flex::vertical()
//!             .constraints([Constraint::Fill, Constraint::Fill]),
//!     )
//!     .at(Breakpoint::Md,
//!         Flex::horizontal()
//!             .constraints([Constraint::Fixed(30), Constraint::Fill]),
//!     );
//!
//! let area = ftui_core::geometry::Rect::new(0, 0, 120, 40);
//! let result = layout.split(area);
//! assert_eq!(result.breakpoint, Breakpoint::Lg);
//! assert_eq!(result.rects.len(), 2);
//! ```
//!
//! # Invariants
//!
//! 1. The base layout (`Xs`) always has a value (enforced by constructor).
//! 2. Breakpoint resolution inherits from smaller tiers (via [`Responsive`]).
//! 3. `split()` auto-detects breakpoint from area width.
//! 4. `split_for()` uses an explicit breakpoint (no auto-detection).
//! 5. Result count may differ between breakpoints (caller must handle this).
//!
//! # Failure Modes
//!
//! - Empty area: delegates to [`Flex::split`] (returns zero-sized rects).
//! - Breakpoint changes mid-session: caller must handle state transitions
//!   (e.g., re-mapping children). Use [`ResponsiveSplit::breakpoint`] to
//!   detect changes.

use super::{Breakpoint, Breakpoints, Flex, Rect, Responsive};

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// Result of a responsive layout split.
#[derive(Debug, Clone, PartialEq)]
pub struct ResponsiveSplit {
    /// The breakpoint that was active for this split.
    pub breakpoint: Breakpoint,
    /// The resulting layout rectangles.
    pub rects: crate::Rects,
}

/// A breakpoint-aware layout that switches [`Flex`] configuration at different
/// terminal widths.
///
/// Wraps [`Responsive<Flex>`] with auto-detection of breakpoints from area
/// width. Each breakpoint tier can define a completely different layout
/// (direction, constraints, gaps, margins).
#[derive(Debug, Clone)]
pub struct ResponsiveLayout {
    /// Per-breakpoint Flex configurations.
    layouts: Responsive<Flex>,
    /// Breakpoint thresholds for width classification.
    breakpoints: Breakpoints,
}

// ---------------------------------------------------------------------------
// Construction
// ---------------------------------------------------------------------------

impl ResponsiveLayout {
    /// Create a responsive layout with a base layout for `Xs`.
    ///
    /// All larger breakpoints inherit this layout until explicitly overridden.
    #[must_use]
    pub fn new(base: Flex) -> Self {
        Self {
            layouts: Responsive::new(base),
            breakpoints: Breakpoints::DEFAULT,
        }
    }

    /// Set the layout for a specific breakpoint (builder pattern).
    #[must_use]
    pub fn at(mut self, bp: Breakpoint, layout: Flex) -> Self {
        self.layouts.set(bp, layout);
        self
    }

    /// Override the breakpoint thresholds (builder pattern).
    ///
    /// Defaults to [`Breakpoints::DEFAULT`] (60/90/120/160).
    #[must_use]
    pub fn with_breakpoints(mut self, breakpoints: Breakpoints) -> Self {
        self.breakpoints = breakpoints;
        self
    }

    /// Set the layout for a specific breakpoint (mutating).
    pub fn set(&mut self, bp: Breakpoint, layout: Flex) {
        self.layouts.set(bp, layout);
    }

    /// Clear the override for a specific breakpoint, reverting to inheritance.
    ///
    /// Clearing `Xs` is a no-op.
    pub fn clear(&mut self, bp: Breakpoint) {
        self.layouts.clear(bp);
    }
}

// ---------------------------------------------------------------------------
// Splitting
// ---------------------------------------------------------------------------

impl ResponsiveLayout {
    /// Split the area using auto-detected breakpoint from width.
    ///
    /// Classifies `area.width` into a [`Breakpoint`], resolves the
    /// corresponding [`Flex`], and splits the area.
    #[must_use]
    pub fn split(&self, area: Rect) -> ResponsiveSplit {
        let bp = self.breakpoints.classify_width(area.width);
        self.split_for(bp, area)
    }

    /// Split the area using an explicit breakpoint.
    ///
    /// Use this when you already know the active breakpoint (e.g., from
    /// a shared app-level breakpoint state).
    #[must_use]
    pub fn split_for(&self, bp: Breakpoint, area: Rect) -> ResponsiveSplit {
        let flex = self.layouts.resolve(bp);
        ResponsiveSplit {
            breakpoint: bp,
            rects: flex.split(area),
        }
    }

    /// Get the active breakpoint for a given width.
    #[must_use]
    pub fn classify(&self, width: u16) -> Breakpoint {
        self.breakpoints.classify_width(width)
    }

    /// Get the Flex configuration for a given breakpoint.
    #[must_use]
    pub fn layout_for(&self, bp: Breakpoint) -> &Flex {
        self.layouts.resolve(bp)
    }

    /// Whether a specific breakpoint has an explicit (non-inherited) layout.
    #[must_use]
    pub fn has_explicit(&self, bp: Breakpoint) -> bool {
        self.layouts.has_explicit(bp)
    }

    /// Get the breakpoint thresholds.
    #[must_use]
    pub fn breakpoints(&self) -> Breakpoints {
        self.breakpoints
    }

    /// Number of rects that would be produced for a given breakpoint.
    ///
    /// Useful for pre-allocating or checking layout changes without
    /// performing the full split.
    #[must_use]
    pub fn constraint_count(&self, bp: Breakpoint) -> usize {
        self.layouts.resolve(bp).constraint_count()
    }

    /// Check if a width change would cause a breakpoint transition.
    ///
    /// Returns `Some((old, new))` if the breakpoint changed, `None` otherwise.
    #[must_use]
    pub fn detect_transition(
        &self,
        old_width: u16,
        new_width: u16,
    ) -> Option<(Breakpoint, Breakpoint)> {
        let old_bp = self.breakpoints.classify_width(old_width);
        let new_bp = self.breakpoints.classify_width(new_width);
        if old_bp != new_bp {
            Some((old_bp, new_bp))
        } else {
            None
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Constraint;

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

    fn area(w: u16, h: u16) -> Rect {
        Rect::new(0, 0, w, h)
    }

    #[test]
    fn base_layout_at_all_breakpoints() {
        let layout = ResponsiveLayout::new(single_column());
        for bp in Breakpoint::ALL {
            let result = layout.split_for(bp, area(80, 24));
            assert_eq!(result.rects.len(), 1);
        }
    }

    #[test]
    fn switches_at_breakpoint() {
        let layout = ResponsiveLayout::new(single_column()).at(Breakpoint::Md, two_column());

        // Xs (width < 60): single column
        let result = layout.split(area(50, 24));
        assert_eq!(result.breakpoint, Breakpoint::Xs);
        assert_eq!(result.rects.len(), 1);

        // Md (width 90-119): two columns
        let result = layout.split(area(100, 24));
        assert_eq!(result.breakpoint, Breakpoint::Md);
        assert_eq!(result.rects.len(), 2);
    }

    #[test]
    fn inherits_from_smaller() {
        let layout = ResponsiveLayout::new(single_column()).at(Breakpoint::Md, two_column());

        // Lg inherits from Md
        let result = layout.split(area(130, 24));
        assert_eq!(result.breakpoint, Breakpoint::Lg);
        assert_eq!(result.rects.len(), 2);
    }

    #[test]
    fn three_tier_layout() {
        let layout = ResponsiveLayout::new(single_column())
            .at(Breakpoint::Sm, two_column())
            .at(Breakpoint::Lg, three_column());

        assert_eq!(layout.split(area(40, 24)).rects.len(), 1); // Xs
        assert_eq!(layout.split(area(70, 24)).rects.len(), 2); // Sm
        assert_eq!(layout.split(area(100, 24)).rects.len(), 2); // Md inherits Sm
        assert_eq!(layout.split(area(130, 24)).rects.len(), 3); // Lg
        assert_eq!(layout.split(area(170, 24)).rects.len(), 3); // Xl inherits Lg
    }

    #[test]
    fn split_for_ignores_width() {
        let layout = ResponsiveLayout::new(single_column()).at(Breakpoint::Lg, two_column());

        // Even though area is narrow, split_for uses the explicit breakpoint.
        let result = layout.split_for(Breakpoint::Lg, area(40, 24));
        assert_eq!(result.breakpoint, Breakpoint::Lg);
        assert_eq!(result.rects.len(), 2);
    }

    #[test]
    fn custom_breakpoints() {
        let layout = ResponsiveLayout::new(single_column())
            .at(Breakpoint::Sm, two_column())
            .with_breakpoints(Breakpoints::new(40, 80, 120));

        // Width 50 ≥ 40 → Sm (custom threshold)
        let result = layout.split(area(50, 24));
        assert_eq!(result.breakpoint, Breakpoint::Sm);
        assert_eq!(result.rects.len(), 2);
    }

    #[test]
    fn detect_transition_some() {
        let layout = ResponsiveLayout::new(single_column());

        // 50→100 crosses from Xs to Md (default breakpoints: sm=60, md=90)
        let transition = layout.detect_transition(50, 100);
        assert!(transition.is_some());
        let (old, new) = transition.unwrap();
        assert_eq!(old, Breakpoint::Xs);
        assert_eq!(new, Breakpoint::Md);
    }

    #[test]
    fn detect_transition_none() {
        let layout = ResponsiveLayout::new(single_column());

        // 70→80 stays within Sm
        assert!(layout.detect_transition(70, 80).is_none());
    }

    #[test]
    fn classify_width() {
        let layout = ResponsiveLayout::new(single_column());
        assert_eq!(layout.classify(40), Breakpoint::Xs);
        assert_eq!(layout.classify(60), Breakpoint::Sm);
        assert_eq!(layout.classify(90), Breakpoint::Md);
        assert_eq!(layout.classify(120), Breakpoint::Lg);
        assert_eq!(layout.classify(160), Breakpoint::Xl);
    }

    #[test]
    fn constraint_count() {
        let layout = ResponsiveLayout::new(single_column())
            .at(Breakpoint::Md, two_column())
            .at(Breakpoint::Lg, three_column());

        assert_eq!(layout.constraint_count(Breakpoint::Xs), 1);
        assert_eq!(layout.constraint_count(Breakpoint::Sm), 1); // Inherits Xs
        assert_eq!(layout.constraint_count(Breakpoint::Md), 2);
        assert_eq!(layout.constraint_count(Breakpoint::Lg), 3);
    }

    #[test]
    fn layout_for_access() {
        let layout = ResponsiveLayout::new(single_column()).at(Breakpoint::Md, two_column());

        let flex = layout.layout_for(Breakpoint::Md);
        assert_eq!(flex.constraint_count(), 2);
    }

    #[test]
    fn has_explicit_check() {
        let layout = ResponsiveLayout::new(single_column()).at(Breakpoint::Lg, two_column());

        assert!(layout.has_explicit(Breakpoint::Xs));
        assert!(!layout.has_explicit(Breakpoint::Sm));
        assert!(!layout.has_explicit(Breakpoint::Md));
        assert!(layout.has_explicit(Breakpoint::Lg));
    }

    #[test]
    fn set_mutating() {
        let mut layout = ResponsiveLayout::new(single_column());
        layout.set(Breakpoint::Xl, three_column());
        assert_eq!(layout.constraint_count(Breakpoint::Xl), 3);
    }

    #[test]
    fn clear_reverts_to_inheritance() {
        let mut layout = ResponsiveLayout::new(single_column()).at(Breakpoint::Md, two_column());

        assert_eq!(layout.constraint_count(Breakpoint::Md), 2);
        layout.clear(Breakpoint::Md);
        assert_eq!(layout.constraint_count(Breakpoint::Md), 1); // Inherits Xs
    }

    #[test]
    fn empty_area_returns_zero_rects() {
        let layout = ResponsiveLayout::new(two_column());
        let result = layout.split(area(0, 0));
        assert_eq!(result.breakpoint, Breakpoint::Xs);
        // Flex::split returns default rects for empty area
        assert_eq!(result.rects.len(), 2);
        assert!(result.rects.iter().all(|r| r.width == 0 && r.height == 0));
    }

    #[test]
    fn rect_dimensions_correct() {
        let layout = ResponsiveLayout::new(
            Flex::horizontal().constraints([Constraint::Fixed(20), Constraint::Fill]),
        );

        let result = layout.split(area(100, 30));
        assert_eq!(result.rects[0].width, 20);
        assert_eq!(result.rects[0].height, 30);
        assert_eq!(result.rects[1].width, 80);
        assert_eq!(result.rects[1].height, 30);
    }

    #[test]
    fn breakpoints_accessor() {
        let bps = Breakpoints::new(50, 80, 110);
        let layout = ResponsiveLayout::new(single_column()).with_breakpoints(bps);
        assert_eq!(layout.breakpoints(), bps);
    }

    #[test]
    fn responsive_split_debug() {
        let split = ResponsiveSplit {
            breakpoint: Breakpoint::Md,
            rects: smallvec::smallvec![Rect::new(0, 0, 50, 24)],
        };
        let dbg = format!("{:?}", split);
        assert!(dbg.contains("Md"));
    }
}
