#![forbid(unsafe_code)]

//! Breakpoint-based visibility helpers.
//!
//! [`Visibility`] determines whether a widget should be rendered at a given
//! breakpoint. Unlike CSS `display: none` vs `visibility: hidden`, these
//! helpers always reclaim space — a hidden widget produces zero layout area.
//!
//! # Usage
//!
//! ```ignore
//! use ftui_layout::{Breakpoint, Visibility};
//!
//! // Only visible at Md and above.
//! let vis = Visibility::visible_above(Breakpoint::Md);
//! assert!(!vis.is_visible(Breakpoint::Sm));
//! assert!(vis.is_visible(Breakpoint::Md));
//! assert!(vis.is_visible(Breakpoint::Lg));
//!
//! // Hidden at Xs and Sm only.
//! let vis = Visibility::hidden_below(Breakpoint::Md);
//! assert!(!vis.is_visible(Breakpoint::Xs));
//! assert!(vis.is_visible(Breakpoint::Md));
//! ```
//!
//! # Invariants
//!
//! 1. `Always` is visible at every breakpoint.
//! 2. `Never` is hidden at every breakpoint.
//! 3. `visible_above(bp)` shows at `bp` and all larger breakpoints.
//! 4. `visible_below(bp)` shows at `bp` and all smaller breakpoints.
//! 5. `only(bp)` shows at exactly one breakpoint.
//! 6. `custom()` allows arbitrary per-breakpoint bitmask.
//! 7. `filter_rects()` removes rects for hidden widgets (space reclamation).
//!
//! # Failure Modes
//!
//! None — all operations are infallible.

use super::Breakpoint;

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// Breakpoint-aware visibility rule.
///
/// Encodes which breakpoints a widget should be visible at.
/// Use with [`filter_rects`](Self::filter_rects) to reclaim space from
/// hidden widgets during layout.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Visibility {
    /// Bitmask: bit i set = visible at Breakpoint with ordinal i.
    mask: u8,
}

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

impl Visibility {
    /// Visible at all breakpoints.
    pub const ALWAYS: Self = Self { mask: 0b11111 };

    /// Hidden at all breakpoints.
    pub const NEVER: Self = Self { mask: 0 };
}

// ---------------------------------------------------------------------------
// Construction
// ---------------------------------------------------------------------------

impl Visibility {
    /// Visible at the given breakpoint and all larger ones.
    ///
    /// Example: `visible_above(Md)` → visible at Md, Lg, Xl.
    #[must_use]
    pub const fn visible_above(bp: Breakpoint) -> Self {
        let idx = bp as u8;
        // Set bits from idx..=4
        let mask = 0b11111u8 << idx;
        Self {
            mask: mask & 0b11111,
        }
    }

    /// Visible at the given breakpoint and all smaller ones.
    ///
    /// Example: `visible_below(Md)` → visible at Xs, Sm, Md.
    #[must_use]
    pub const fn visible_below(bp: Breakpoint) -> Self {
        let idx = bp as u8;
        // Set bits from 0..=idx
        let mask = (1u8 << (idx + 1)) - 1;
        Self { mask }
    }

    /// Visible at exactly one breakpoint.
    #[must_use]
    pub const fn only(bp: Breakpoint) -> Self {
        Self {
            mask: 1u8 << (bp as u8),
        }
    }

    /// Visible at the specified breakpoints.
    #[must_use]
    pub fn at(breakpoints: &[Breakpoint]) -> Self {
        let mut mask = 0u8;
        for &bp in breakpoints {
            mask |= 1u8 << (bp as u8);
        }
        Self { mask }
    }

    /// Hidden at the given breakpoint and all smaller ones (visible above).
    ///
    /// Example: `hidden_below(Md)` → hidden at Xs, Sm; visible at Md, Lg, Xl.
    #[must_use]
    pub const fn hidden_below(bp: Breakpoint) -> Self {
        Self::visible_above(bp)
    }

    /// Hidden at the given breakpoint and all larger ones (visible below).
    ///
    /// Example: `hidden_above(Md)` → visible at Xs, Sm; hidden at Md, Lg, Xl.
    #[must_use]
    pub const fn hidden_above(bp: Breakpoint) -> Self {
        let idx = bp as u8;
        // Visible below bp (not including bp)
        if idx == 0 {
            return Self::NEVER;
        }
        let mask = (1u8 << idx) - 1;
        Self { mask }
    }

    /// Create from a raw bitmask (bits 0–4 correspond to Xs–Xl).
    #[must_use]
    pub const fn from_mask(mask: u8) -> Self {
        Self {
            mask: mask & 0b11111,
        }
    }
}

// ---------------------------------------------------------------------------
// Queries
// ---------------------------------------------------------------------------

impl Visibility {
    /// Whether the widget is visible at the given breakpoint.
    #[must_use]
    pub const fn is_visible(self, bp: Breakpoint) -> bool {
        self.mask & (1u8 << (bp as u8)) != 0
    }

    /// Whether the widget is hidden at the given breakpoint.
    #[must_use]
    pub const fn is_hidden(self, bp: Breakpoint) -> bool {
        !self.is_visible(bp)
    }

    /// Whether the widget is always visible (at every breakpoint).
    #[must_use]
    pub const fn is_always(self) -> bool {
        self.mask == 0b11111
    }

    /// Whether the widget is never visible (hidden at every breakpoint).
    #[must_use]
    pub const fn is_never(self) -> bool {
        self.mask == 0
    }

    /// The raw bitmask.
    #[must_use]
    pub const fn mask(self) -> u8 {
        self.mask
    }

    /// Count of breakpoints where this is visible.
    #[must_use]
    pub const fn visible_count(self) -> u32 {
        self.mask.count_ones()
    }

    /// Iterator over breakpoints where visible.
    pub fn visible_breakpoints(self) -> impl Iterator<Item = Breakpoint> {
        Breakpoint::ALL
            .into_iter()
            .filter(move |&bp| self.is_visible(bp))
    }
}

// ---------------------------------------------------------------------------
// Layout integration
// ---------------------------------------------------------------------------

impl Visibility {
    /// Filter a list of rects, keeping only those whose visibility allows
    /// the given breakpoint.
    ///
    /// Returns `(index, rect)` pairs for visible items. The index is the
    /// original position in the input, useful for mapping back to widget state.
    ///
    /// This achieves space reclamation: hidden widgets don't get any layout area.
    pub fn filter_rects<'a>(
        visibilities: &'a [Visibility],
        rects: &'a [super::Rect],
        bp: Breakpoint,
    ) -> Vec<(usize, super::Rect)> {
        visibilities
            .iter()
            .zip(rects.iter())
            .enumerate()
            .filter(|(_, (vis, _))| vis.is_visible(bp))
            .map(|(i, (_, rect))| (i, *rect))
            .collect()
    }

    /// Count how many items are visible at a given breakpoint.
    pub fn count_visible(visibilities: &[Visibility], bp: Breakpoint) -> usize {
        visibilities.iter().filter(|v| v.is_visible(bp)).count()
    }
}

// ---------------------------------------------------------------------------
// Trait impls
// ---------------------------------------------------------------------------

impl Default for Visibility {
    fn default() -> Self {
        Self::ALWAYS
    }
}

impl std::fmt::Display for Visibility {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.is_always() {
            return f.write_str("always");
        }
        if self.is_never() {
            return f.write_str("never");
        }
        let mut first = true;
        for bp in Breakpoint::ALL {
            if self.is_visible(bp) {
                if !first {
                    f.write_str("+")?;
                }
                f.write_str(bp.label())?;
                first = false;
            }
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Rect;

    #[test]
    fn always_visible_at_all() {
        for bp in Breakpoint::ALL {
            assert!(Visibility::ALWAYS.is_visible(bp));
        }
        assert!(Visibility::ALWAYS.is_always());
        assert!(!Visibility::ALWAYS.is_never());
    }

    #[test]
    fn never_visible_at_none() {
        for bp in Breakpoint::ALL {
            assert!(Visibility::NEVER.is_hidden(bp));
        }
        assert!(Visibility::NEVER.is_never());
        assert!(!Visibility::NEVER.is_always());
    }

    #[test]
    fn visible_above() {
        let vis = Visibility::visible_above(Breakpoint::Md);
        assert!(!vis.is_visible(Breakpoint::Xs));
        assert!(!vis.is_visible(Breakpoint::Sm));
        assert!(vis.is_visible(Breakpoint::Md));
        assert!(vis.is_visible(Breakpoint::Lg));
        assert!(vis.is_visible(Breakpoint::Xl));
    }

    #[test]
    fn visible_above_xs() {
        let vis = Visibility::visible_above(Breakpoint::Xs);
        assert!(vis.is_always());
    }

    #[test]
    fn visible_above_xl() {
        let vis = Visibility::visible_above(Breakpoint::Xl);
        assert!(vis.is_visible(Breakpoint::Xl));
        assert!(!vis.is_visible(Breakpoint::Lg));
        assert_eq!(vis.visible_count(), 1);
    }

    #[test]
    fn visible_below() {
        let vis = Visibility::visible_below(Breakpoint::Md);
        assert!(vis.is_visible(Breakpoint::Xs));
        assert!(vis.is_visible(Breakpoint::Sm));
        assert!(vis.is_visible(Breakpoint::Md));
        assert!(!vis.is_visible(Breakpoint::Lg));
        assert!(!vis.is_visible(Breakpoint::Xl));
    }

    #[test]
    fn visible_below_xl() {
        let vis = Visibility::visible_below(Breakpoint::Xl);
        assert!(vis.is_always());
    }

    #[test]
    fn visible_below_xs() {
        let vis = Visibility::visible_below(Breakpoint::Xs);
        assert!(vis.is_visible(Breakpoint::Xs));
        assert!(!vis.is_visible(Breakpoint::Sm));
        assert_eq!(vis.visible_count(), 1);
    }

    #[test]
    fn only_single_breakpoint() {
        let vis = Visibility::only(Breakpoint::Lg);
        assert!(!vis.is_visible(Breakpoint::Xs));
        assert!(!vis.is_visible(Breakpoint::Sm));
        assert!(!vis.is_visible(Breakpoint::Md));
        assert!(vis.is_visible(Breakpoint::Lg));
        assert!(!vis.is_visible(Breakpoint::Xl));
        assert_eq!(vis.visible_count(), 1);
    }

    #[test]
    fn at_multiple() {
        let vis = Visibility::at(&[Breakpoint::Xs, Breakpoint::Lg, Breakpoint::Xl]);
        assert!(vis.is_visible(Breakpoint::Xs));
        assert!(!vis.is_visible(Breakpoint::Sm));
        assert!(!vis.is_visible(Breakpoint::Md));
        assert!(vis.is_visible(Breakpoint::Lg));
        assert!(vis.is_visible(Breakpoint::Xl));
        assert_eq!(vis.visible_count(), 3);
    }

    #[test]
    fn hidden_below() {
        let vis = Visibility::hidden_below(Breakpoint::Md);
        // Same as visible_above(Md)
        assert!(!vis.is_visible(Breakpoint::Xs));
        assert!(!vis.is_visible(Breakpoint::Sm));
        assert!(vis.is_visible(Breakpoint::Md));
    }

    #[test]
    fn hidden_above() {
        let vis = Visibility::hidden_above(Breakpoint::Md);
        assert!(vis.is_visible(Breakpoint::Xs));
        assert!(vis.is_visible(Breakpoint::Sm));
        assert!(!vis.is_visible(Breakpoint::Md));
        assert!(!vis.is_visible(Breakpoint::Lg));
    }

    #[test]
    fn hidden_above_xs() {
        let vis = Visibility::hidden_above(Breakpoint::Xs);
        assert!(vis.is_never());
    }

    #[test]
    fn from_mask() {
        let vis = Visibility::from_mask(0b10101); // Xs, Md, Xl
        assert!(vis.is_visible(Breakpoint::Xs));
        assert!(!vis.is_visible(Breakpoint::Sm));
        assert!(vis.is_visible(Breakpoint::Md));
        assert!(!vis.is_visible(Breakpoint::Lg));
        assert!(vis.is_visible(Breakpoint::Xl));
    }

    #[test]
    fn from_mask_truncates() {
        let vis = Visibility::from_mask(0xFF);
        assert_eq!(vis.mask(), 0b11111);
    }

    #[test]
    fn visible_breakpoints_iterator() {
        let vis = Visibility::at(&[Breakpoint::Sm, Breakpoint::Lg]);
        let bps: Vec<_> = vis.visible_breakpoints().collect();
        assert_eq!(bps, vec![Breakpoint::Sm, Breakpoint::Lg]);
    }

    #[test]
    fn filter_rects_basic() {
        let rects = vec![
            Rect::new(0, 0, 20, 10),
            Rect::new(20, 0, 30, 10),
            Rect::new(50, 0, 40, 10),
        ];
        let visibilities = vec![
            Visibility::ALWAYS,
            Visibility::hidden_below(Breakpoint::Md), // Hidden at Xs, Sm
            Visibility::ALWAYS,
        ];

        // At Sm: middle rect hidden
        let visible = Visibility::filter_rects(&visibilities, &rects, Breakpoint::Sm);
        assert_eq!(visible.len(), 2);
        assert_eq!(visible[0].0, 0); // index 0
        assert_eq!(visible[1].0, 2); // index 2

        // At Md: all visible
        let visible = Visibility::filter_rects(&visibilities, &rects, Breakpoint::Md);
        assert_eq!(visible.len(), 3);
    }

    #[test]
    fn count_visible_helper() {
        let visibilities = vec![
            Visibility::ALWAYS,
            Visibility::only(Breakpoint::Xl),
            Visibility::visible_above(Breakpoint::Lg),
        ];

        assert_eq!(Visibility::count_visible(&visibilities, Breakpoint::Xs), 1);
        assert_eq!(Visibility::count_visible(&visibilities, Breakpoint::Lg), 2);
        assert_eq!(Visibility::count_visible(&visibilities, Breakpoint::Xl), 3);
    }

    #[test]
    fn default_is_always() {
        assert_eq!(Visibility::default(), Visibility::ALWAYS);
    }

    #[test]
    fn display_always() {
        assert_eq!(format!("{}", Visibility::ALWAYS), "always");
    }

    #[test]
    fn display_never() {
        assert_eq!(format!("{}", Visibility::NEVER), "never");
    }

    #[test]
    fn display_partial() {
        let vis = Visibility::at(&[Breakpoint::Sm, Breakpoint::Lg]);
        assert_eq!(format!("{}", vis), "sm+lg");
    }

    #[test]
    fn equality() {
        assert_eq!(
            Visibility::visible_above(Breakpoint::Md),
            Visibility::hidden_below(Breakpoint::Md)
        );
    }

    #[test]
    fn clone_independence() {
        let a = Visibility::only(Breakpoint::Md);
        let b = a;
        assert_eq!(a, b);
    }

    #[test]
    fn debug_format() {
        let dbg = format!("{:?}", Visibility::ALWAYS);
        assert!(dbg.contains("Visibility"));
    }
}
