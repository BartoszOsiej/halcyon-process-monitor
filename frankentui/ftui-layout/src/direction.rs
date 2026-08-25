#![forbid(unsafe_code)]

//! RTL layout mirroring and logical direction support (bd-ic6i.3).
//!
//! Provides types for text-direction–aware layout: [`FlowDirection`] controls
//! whether horizontal content flows left-to-right or right-to-left,
//! [`LogicalSides`] maps logical start/end to physical left/right, and
//! [`LogicalAlignment`] resolves Start/End alignment relative to flow.
//!
//! # Invariants
//!
//! 1. **Idempotent mirroring**: resolving the same logical values with the same
//!    direction always produces the same physical values.
//! 2. **RTL↔LTR symmetry**: `resolve(Rtl)` is the mirror of `resolve(Ltr)`.
//! 3. **Vertical invariance**: RTL only affects the horizontal axis.
//! 4. **Composable**: logical values can be nested; each subtree resolves
//!    independently.

use crate::{Alignment, Sides};

/// Horizontal text flow direction.
///
/// Controls whether children of a horizontal flex layout are placed
/// left-to-right or right-to-left.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum FlowDirection {
    /// Left-to-right (default for Latin, Cyrillic, etc.).
    #[default]
    Ltr,
    /// Right-to-left (Arabic, Hebrew, etc.).
    Rtl,
}

impl FlowDirection {
    /// Whether this direction is right-to-left.
    pub const fn is_rtl(self) -> bool {
        matches!(self, FlowDirection::Rtl)
    }

    /// Whether this direction is left-to-right.
    pub const fn is_ltr(self) -> bool {
        matches!(self, FlowDirection::Ltr)
    }

    /// Return `true` if a locale tag (e.g. `"ar"`, `"he"`, `"fa"`) is
    /// typically RTL. Checks the primary language subtag only.
    pub fn locale_is_rtl(locale: &str) -> bool {
        let lang = locale
            .split(['-', '_'])
            .next()
            .unwrap_or("")
            .to_ascii_lowercase();
        matches!(
            lang.as_str(),
            "ar" | "he"
                | "fa"
                | "ur"
                | "ps"
                | "sd"
                | "yi"
                | "ku"
                | "dv"
                | "ks"
                | "ckb"
                | "syr"
                | "arc"
                | "nqo"
                | "man"
                | "sam"
        )
    }

    /// Detect flow direction from a locale tag.
    pub fn from_locale(locale: &str) -> Self {
        if Self::locale_is_rtl(locale) {
            FlowDirection::Rtl
        } else {
            FlowDirection::Ltr
        }
    }
}

// ---------------------------------------------------------------------------
// LogicalAlignment
// ---------------------------------------------------------------------------

/// Alignment in logical (direction-aware) terms.
///
/// `Start` and `End` resolve to physical left/right (or top/bottom) based
/// on the active [`FlowDirection`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LogicalAlignment {
    /// Start edge: left in LTR, right in RTL.
    #[default]
    Start,
    /// End edge: right in LTR, left in RTL.
    End,
    /// Center (direction-independent).
    Center,
}

impl LogicalAlignment {
    /// Resolve to a physical [`Alignment`] given the flow direction.
    ///
    /// For horizontal layouts:
    /// - `Start` → `Alignment::Start` (LTR) or `Alignment::End` (RTL)
    /// - `End`   → `Alignment::End` (LTR) or `Alignment::Start` (RTL)
    /// - `Center` → `Alignment::Center` (always)
    pub const fn resolve(self, flow: FlowDirection) -> Alignment {
        match (self, flow) {
            (LogicalAlignment::Start, FlowDirection::Ltr) => Alignment::Start,
            (LogicalAlignment::Start, FlowDirection::Rtl) => Alignment::End,
            (LogicalAlignment::End, FlowDirection::Ltr) => Alignment::End,
            (LogicalAlignment::End, FlowDirection::Rtl) => Alignment::Start,
            (LogicalAlignment::Center, _) => Alignment::Center,
        }
    }
}

// ---------------------------------------------------------------------------
// LogicalSides
// ---------------------------------------------------------------------------

/// Padding or margin expressed in logical (direction-aware) terms.
///
/// `start` and `end` resolve to physical `left` and `right` (swapped in RTL).
/// `top` and `bottom` are direction-independent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct LogicalSides {
    pub top: u16,
    pub bottom: u16,
    /// Inline start: left in LTR, right in RTL.
    pub start: u16,
    /// Inline end: right in LTR, left in RTL.
    pub end: u16,
}

impl LogicalSides {
    /// All sides equal.
    pub const fn all(val: u16) -> Self {
        Self {
            top: val,
            bottom: val,
            start: val,
            end: val,
        }
    }

    /// Inline (start/end) sides equal, block (top/bottom) sides equal.
    pub const fn symmetric(block: u16, inline: u16) -> Self {
        Self {
            top: block,
            bottom: block,
            start: inline,
            end: inline,
        }
    }

    /// Set only inline (start/end) sides.
    pub const fn inline(start: u16, end: u16) -> Self {
        Self {
            top: 0,
            bottom: 0,
            start,
            end,
        }
    }

    /// Set only block (top/bottom) sides.
    pub const fn block(top: u16, bottom: u16) -> Self {
        Self {
            top,
            bottom,
            start: 0,
            end: 0,
        }
    }

    /// Resolve to physical [`Sides`] given the flow direction.
    ///
    /// - LTR: start → left, end → right
    /// - RTL: start → right, end → left
    pub const fn resolve(self, flow: FlowDirection) -> Sides {
        match flow {
            FlowDirection::Ltr => Sides {
                top: self.top,
                right: self.end,
                bottom: self.bottom,
                left: self.start,
            },
            FlowDirection::Rtl => Sides {
                top: self.top,
                right: self.start,
                bottom: self.bottom,
                left: self.end,
            },
        }
    }

    /// The sum of inline (start + end) sides.
    pub const fn inline_sum(self) -> u16 {
        self.start + self.end
    }

    /// The sum of block (top + bottom) sides.
    pub const fn block_sum(self) -> u16 {
        self.top + self.bottom
    }
}

/// Create [`LogicalSides`] from physical [`Sides`] under a given direction.
///
/// Inverse of [`LogicalSides::resolve`].
impl LogicalSides {
    pub const fn from_physical(sides: Sides, flow: FlowDirection) -> Self {
        match flow {
            FlowDirection::Ltr => Self {
                top: sides.top,
                bottom: sides.bottom,
                start: sides.left,
                end: sides.right,
            },
            FlowDirection::Rtl => Self {
                top: sides.top,
                bottom: sides.bottom,
                start: sides.right,
                end: sides.left,
            },
        }
    }
}

// ---------------------------------------------------------------------------
// Flex extension: mirror_horizontal
// ---------------------------------------------------------------------------

/// Mirror a sequence of [`Rect`](crate::Rect)s horizontally within a
/// containing area.
///
/// Each rect's x-position is reflected: `new_x = area.right() - (old_x - area.x) - width`.
/// This preserves the left-to-right size sequence but flips their positions.
pub fn mirror_rects_horizontal(
    rects: &mut [ftui_core::geometry::Rect],
    area: ftui_core::geometry::Rect,
) {
    for rect in rects.iter_mut() {
        // new_x = right - (rect.x - area.x) - rect.width
        //       = right - rect.x + area.x - rect.width
        //       = area.x + area.width - rect.x + area.x - rect.width
        // Simplified: new_x = right - (rect.x - area.x + rect.width) + area.x
        //           = right - rect.x - rect.width + area.x... nah, simpler:
        let offset_from_left = rect.x.saturating_sub(area.x);
        let new_offset = area
            .width
            .saturating_sub(offset_from_left)
            .saturating_sub(rect.width);
        rect.x = area.x.saturating_add(new_offset);
    }
}

// ===========================================================================
// Tests
// ===========================================================================

#[cfg(test)]
mod tests {
    use super::*;

    // --- FlowDirection ---

    #[test]
    fn flow_direction_default_is_ltr() {
        assert_eq!(FlowDirection::default(), FlowDirection::Ltr);
        assert!(FlowDirection::Ltr.is_ltr());
        assert!(!FlowDirection::Ltr.is_rtl());
        assert!(FlowDirection::Rtl.is_rtl());
        assert!(!FlowDirection::Rtl.is_ltr());
    }

    #[test]
    fn flow_direction_from_locale() {
        assert_eq!(FlowDirection::from_locale("en"), FlowDirection::Ltr);
        assert_eq!(FlowDirection::from_locale("en-US"), FlowDirection::Ltr);
        assert_eq!(FlowDirection::from_locale("fr"), FlowDirection::Ltr);
        assert_eq!(FlowDirection::from_locale("ja"), FlowDirection::Ltr);
        assert_eq!(FlowDirection::from_locale("ar"), FlowDirection::Rtl);
        assert_eq!(FlowDirection::from_locale("ar-SA"), FlowDirection::Rtl);
        assert_eq!(FlowDirection::from_locale("he"), FlowDirection::Rtl);
        assert_eq!(FlowDirection::from_locale("fa"), FlowDirection::Rtl);
        assert_eq!(FlowDirection::from_locale("ur"), FlowDirection::Rtl);
        assert_eq!(FlowDirection::from_locale("yi"), FlowDirection::Rtl);
    }

    #[test]
    fn flow_direction_locale_case_insensitive() {
        assert_eq!(FlowDirection::from_locale("AR"), FlowDirection::Rtl);
        assert_eq!(FlowDirection::from_locale("He"), FlowDirection::Rtl);
        assert_eq!(FlowDirection::from_locale("EN"), FlowDirection::Ltr);
    }

    // --- LogicalAlignment ---

    #[test]
    fn logical_alignment_ltr_resolution() {
        assert_eq!(
            LogicalAlignment::Start.resolve(FlowDirection::Ltr),
            Alignment::Start
        );
        assert_eq!(
            LogicalAlignment::End.resolve(FlowDirection::Ltr),
            Alignment::End
        );
        assert_eq!(
            LogicalAlignment::Center.resolve(FlowDirection::Ltr),
            Alignment::Center
        );
    }

    #[test]
    fn logical_alignment_rtl_resolution() {
        assert_eq!(
            LogicalAlignment::Start.resolve(FlowDirection::Rtl),
            Alignment::End
        );
        assert_eq!(
            LogicalAlignment::End.resolve(FlowDirection::Rtl),
            Alignment::Start
        );
        assert_eq!(
            LogicalAlignment::Center.resolve(FlowDirection::Rtl),
            Alignment::Center
        );
    }

    // --- LogicalSides ---

    #[test]
    fn logical_sides_ltr_resolution() {
        let logical = LogicalSides {
            top: 1,
            bottom: 2,
            start: 3,
            end: 4,
        };
        let physical = logical.resolve(FlowDirection::Ltr);
        assert_eq!(physical.top, 1);
        assert_eq!(physical.bottom, 2);
        assert_eq!(physical.left, 3); // start → left
        assert_eq!(physical.right, 4); // end → right
    }

    #[test]
    fn logical_sides_rtl_resolution() {
        let logical = LogicalSides {
            top: 1,
            bottom: 2,
            start: 3,
            end: 4,
        };
        let physical = logical.resolve(FlowDirection::Rtl);
        assert_eq!(physical.top, 1);
        assert_eq!(physical.bottom, 2);
        assert_eq!(physical.left, 4); // end → left in RTL
        assert_eq!(physical.right, 3); // start → right in RTL
    }

    #[test]
    fn logical_sides_symmetry() {
        // Symmetric sides should be identical regardless of direction.
        let logical = LogicalSides::all(5);
        let ltr = logical.resolve(FlowDirection::Ltr);
        let rtl = logical.resolve(FlowDirection::Rtl);
        assert_eq!(ltr, rtl);
    }

    #[test]
    fn logical_sides_roundtrip() {
        // from_physical(resolve(dir), dir) should return original.
        let original = LogicalSides {
            top: 1,
            bottom: 2,
            start: 3,
            end: 4,
        };

        let ltr_physical = original.resolve(FlowDirection::Ltr);
        let roundtrip = LogicalSides::from_physical(ltr_physical, FlowDirection::Ltr);
        assert_eq!(original, roundtrip);

        let rtl_physical = original.resolve(FlowDirection::Rtl);
        let roundtrip = LogicalSides::from_physical(rtl_physical, FlowDirection::Rtl);
        assert_eq!(original, roundtrip);
    }

    #[test]
    fn logical_sides_constructors() {
        let all = LogicalSides::all(5);
        assert_eq!(all.top, 5);
        assert_eq!(all.bottom, 5);
        assert_eq!(all.start, 5);
        assert_eq!(all.end, 5);

        let sym = LogicalSides::symmetric(2, 4);
        assert_eq!(sym.top, 2);
        assert_eq!(sym.bottom, 2);
        assert_eq!(sym.start, 4);
        assert_eq!(sym.end, 4);

        let inline = LogicalSides::inline(3, 7);
        assert_eq!(inline.top, 0);
        assert_eq!(inline.bottom, 0);
        assert_eq!(inline.start, 3);
        assert_eq!(inline.end, 7);

        let block = LogicalSides::block(1, 9);
        assert_eq!(block.top, 1);
        assert_eq!(block.bottom, 9);
        assert_eq!(block.start, 0);
        assert_eq!(block.end, 0);
    }

    #[test]
    fn logical_sides_sums() {
        let s = LogicalSides {
            top: 1,
            bottom: 2,
            start: 3,
            end: 4,
        };
        assert_eq!(s.inline_sum(), 7);
        assert_eq!(s.block_sum(), 3);
    }

    // --- mirror_rects_horizontal ---

    #[test]
    fn mirror_rects_simple() {
        use ftui_core::geometry::Rect;

        let area = Rect::new(0, 0, 100, 20);
        let mut rects = vec![
            Rect::new(0, 0, 30, 20),
            Rect::new(30, 0, 40, 20),
            Rect::new(70, 0, 30, 20),
        ];

        mirror_rects_horizontal(&mut rects, area);

        // [0..30], [30..70], [70..100] → [70..100], [30..70], [0..30]
        assert_eq!(rects[0].x, 70);
        assert_eq!(rects[0].width, 30);
        assert_eq!(rects[1].x, 30);
        assert_eq!(rects[1].width, 40);
        assert_eq!(rects[2].x, 0);
        assert_eq!(rects[2].width, 30);
    }

    #[test]
    fn mirror_rects_with_offset() {
        use ftui_core::geometry::Rect;

        let area = Rect::new(10, 5, 80, 20);
        let mut rects = vec![
            Rect::new(10, 5, 20, 20), // offset 0 from area start
            Rect::new(30, 5, 60, 20), // offset 20 from area start
        ];

        mirror_rects_horizontal(&mut rects, area);

        // rect[0]: offset_from_left=0, new_offset=80-0-20=60, new_x=10+60=70
        // rect[1]: offset_from_left=20, new_offset=80-20-60=0, new_x=10+0=10
        assert_eq!(rects[0].x, 70);
        assert_eq!(rects[0].width, 20);
        assert_eq!(rects[1].x, 10);
        assert_eq!(rects[1].width, 60);
    }

    #[test]
    fn mirror_rects_empty() {
        use ftui_core::geometry::Rect;

        let area = Rect::new(0, 0, 100, 20);
        let mut rects: Vec<Rect> = vec![];
        mirror_rects_horizontal(&mut rects, area); // should not panic
        assert!(rects.is_empty());
    }

    #[test]
    fn mirror_rects_idempotent_double_mirror() {
        use ftui_core::geometry::Rect;

        let area = Rect::new(5, 0, 90, 20);
        let original = vec![
            Rect::new(5, 0, 30, 20),
            Rect::new(35, 0, 25, 20),
            Rect::new(60, 0, 35, 20),
        ];

        let mut rects = original.clone();
        mirror_rects_horizontal(&mut rects, area);
        mirror_rects_horizontal(&mut rects, area);

        // Double mirror should restore original.
        assert_eq!(rects, original);
    }

    // --- Flex RTL split ---

    #[test]
    fn flex_horizontal_rtl_reverses_order() {
        use crate::{Constraint, Flex};
        use ftui_core::geometry::Rect;

        let area = Rect::new(0, 0, 100, 10);

        let ltr_rects = Flex::horizontal()
            .constraints([Constraint::Fixed(30), Constraint::Fixed(70)])
            .split(area);

        let rtl_rects = Flex::horizontal()
            .constraints([Constraint::Fixed(30), Constraint::Fixed(70)])
            .flow_direction(FlowDirection::Rtl)
            .split(area);

        // LTR: [0..30] [30..100]
        assert_eq!(ltr_rects[0].x, 0);
        assert_eq!(ltr_rects[1].x, 30);

        // RTL: [70..100] [0..70] — same sizes, mirrored positions
        assert_eq!(rtl_rects[0].x, 70);
        assert_eq!(rtl_rects[0].width, 30);
        assert_eq!(rtl_rects[1].x, 0);
        assert_eq!(rtl_rects[1].width, 70);
    }

    #[test]
    fn flex_vertical_rtl_no_change() {
        use crate::{Constraint, Flex};
        use ftui_core::geometry::Rect;

        let area = Rect::new(0, 0, 80, 40);

        let ltr_rects = Flex::vertical()
            .constraints([Constraint::Fixed(10), Constraint::Fixed(30)])
            .split(area);

        let rtl_rects = Flex::vertical()
            .constraints([Constraint::Fixed(10), Constraint::Fixed(30)])
            .flow_direction(FlowDirection::Rtl)
            .split(area);

        // Vertical layout is not affected by flow direction.
        assert_eq!(ltr_rects, rtl_rects);
    }

    #[test]
    fn flex_horizontal_rtl_with_gap() {
        use crate::{Constraint, Flex};
        use ftui_core::geometry::Rect;

        let area = Rect::new(0, 0, 100, 10);

        let rtl_rects = Flex::horizontal()
            .constraints([
                Constraint::Fixed(20),
                Constraint::Fixed(30),
                Constraint::Fixed(40),
            ])
            .gap(5)
            .flow_direction(FlowDirection::Rtl)
            .split(area);

        // Total used: 20 + 5 + 30 + 5 + 40 = 100
        // LTR would be: [0..20] [25..55] [60..100]
        // RTL mirrors: [80..100] [45..75] [0..40]
        assert_eq!(rtl_rects[0].x, 80);
        assert_eq!(rtl_rects[0].width, 20);
        assert_eq!(rtl_rects[1].x, 45);
        assert_eq!(rtl_rects[1].width, 30);
        assert_eq!(rtl_rects[2].x, 0);
        assert_eq!(rtl_rects[2].width, 40);
    }

    #[test]
    fn flex_ltr_default_unchanged() {
        use crate::{Constraint, Flex};
        use ftui_core::geometry::Rect;

        let area = Rect::new(0, 0, 100, 10);

        // Default (no flow_direction set) should behave as LTR.
        let default_rects = Flex::horizontal()
            .constraints([Constraint::Fixed(30), Constraint::Fixed(70)])
            .split(area);

        let explicit_ltr = Flex::horizontal()
            .constraints([Constraint::Fixed(30), Constraint::Fixed(70)])
            .flow_direction(FlowDirection::Ltr)
            .split(area);

        assert_eq!(default_rects, explicit_ltr);
    }

    #[test]
    fn flex_mixed_direction_nested() {
        use crate::{Constraint, Flex};
        use ftui_core::geometry::Rect;

        // Simulate nested layout: RTL outer, content inside.
        let outer = Rect::new(0, 0, 100, 20);

        let rtl_cols = Flex::horizontal()
            .constraints([Constraint::Fixed(40), Constraint::Fixed(60)])
            .flow_direction(FlowDirection::Rtl)
            .split(outer);

        // RTL: first item (40w) goes to right side.
        assert_eq!(rtl_cols[0].x, 60);
        assert_eq!(rtl_cols[0].width, 40);
        assert_eq!(rtl_cols[1].x, 0);
        assert_eq!(rtl_cols[1].width, 60);

        // Inner LTR layout within the RTL-positioned first panel.
        let inner_ltr = Flex::vertical()
            .constraints([Constraint::Fixed(10), Constraint::Fill])
            .split(rtl_cols[0]);

        // Vertical layout within is unaffected.
        assert_eq!(inner_ltr[0].x, rtl_cols[0].x);
        assert_eq!(inner_ltr[0].y, rtl_cols[0].y);
        assert_eq!(inner_ltr[0].height, 10);
    }

    #[test]
    fn logical_alignment_in_flex() {
        use crate::{Constraint, Flex};
        use ftui_core::geometry::Rect;

        let area = Rect::new(0, 0, 100, 10);

        // LogicalAlignment::Start in RTL → Alignment::End → items pushed right.
        let alignment = LogicalAlignment::Start.resolve(FlowDirection::Rtl);
        let rects = Flex::horizontal()
            .constraints([Constraint::Fixed(20)])
            .alignment(alignment)
            .split(area);

        // With End alignment, single 20-wide item goes to x=80.
        assert_eq!(rects[0].x, 80);
        assert_eq!(rects[0].width, 20);
    }
}
