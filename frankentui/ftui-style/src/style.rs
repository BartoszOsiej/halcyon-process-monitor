#![forbid(unsafe_code)]

//! Style types for terminal UI styling with CSS-like cascading semantics.

use ftui_render::cell::PackedRgba;
use tracing::{instrument, trace};

/// CSS-like text-transform property.
///
/// Terminal backends apply this at render time by transforming the
/// text content before measuring cell widths. Transformations are
/// locale-independent (ASCII-only) for deterministic output.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "kebab-case"))]
pub enum TextTransform {
    /// No transformation (identity).
    #[default]
    None,
    /// Convert all characters to uppercase.
    Uppercase,
    /// Convert all characters to lowercase.
    Lowercase,
    /// Capitalize the first character of each word.
    Capitalize,
}

impl TextTransform {
    /// Apply the text transform to a string.
    ///
    /// Uses ASCII-only transforms for determinism across locales.
    #[must_use]
    pub fn apply(self, text: &str) -> String {
        match self {
            Self::None => text.to_string(),
            Self::Uppercase => text.to_ascii_uppercase(),
            Self::Lowercase => text.to_ascii_lowercase(),
            Self::Capitalize => capitalize_words(text),
        }
    }
}

/// Capitalize the first ASCII letter of each whitespace-delimited word.
fn capitalize_words(text: &str) -> String {
    let mut result = String::with_capacity(text.len());
    let mut at_word_start = true;
    for ch in text.chars() {
        if ch.is_ascii_whitespace() {
            result.push(ch);
            at_word_start = true;
        } else if at_word_start {
            result.push(ch.to_ascii_uppercase());
            at_word_start = false;
        } else {
            result.push(ch);
        }
    }
    result
}

/// CSS-like text-overflow property for truncation behavior.
///
/// Controls what happens when text content overflows its container.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "kebab-case"))]
pub enum TextOverflow {
    /// Clip overflowing text at the container edge.
    #[default]
    Clip,
    /// Replace overflowing text with an ellipsis ("…").
    Ellipsis,
    /// Replace overflowing text with a custom single-char indicator.
    Indicator(char),
}

/// CSS-like overflow property for layout containers.
///
/// Controls the behavior when content exceeds the allocated area.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "kebab-case"))]
pub enum Overflow {
    /// Content is not clipped; may render outside the container.
    Visible,
    /// Content is clipped at the container boundary.
    #[default]
    Hidden,
    /// Content is scrollable (requires scroll state).
    Scroll,
    /// Browser-like auto: scroll only when needed.
    Auto,
}

/// CSS-like white-space property controlling wrapping and whitespace handling.
///
/// Maps CSS white-space modes to terminal text behavior.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "kebab-case"))]
pub enum WhiteSpaceMode {
    /// Collapse whitespace runs, wrap at container width (CSS `normal`).
    #[default]
    Normal,
    /// Preserve all whitespace and newlines, no wrapping (CSS `pre`).
    Pre,
    /// Preserve whitespace and newlines, wrap at container width (CSS `pre-wrap`).
    PreWrap,
    /// Collapse whitespace runs, preserve newlines, wrap (CSS `pre-line`).
    PreLine,
    /// Collapse whitespace, suppress wrapping (CSS `nowrap`).
    NoWrap,
}

impl WhiteSpaceMode {
    /// Whether this mode collapses consecutive whitespace to a single space.
    #[inline]
    #[must_use]
    pub const fn collapses_whitespace(self) -> bool {
        matches!(self, Self::Normal | Self::PreLine | Self::NoWrap)
    }

    /// Whether this mode allows line wrapping at container width.
    #[inline]
    #[must_use]
    pub const fn allows_wrap(self) -> bool {
        matches!(self, Self::Normal | Self::PreWrap | Self::PreLine)
    }

    /// Whether this mode preserves explicit newlines in the source.
    #[inline]
    #[must_use]
    pub const fn preserves_newlines(self) -> bool {
        matches!(self, Self::Pre | Self::PreWrap | Self::PreLine)
    }
}

/// CSS-like text-align property.
///
/// Controls horizontal alignment of text within its container.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "kebab-case"))]
pub enum TextAlign {
    /// Left-aligned (default for LTR text).
    #[default]
    Left,
    /// Right-aligned.
    Right,
    /// Centered.
    Center,
    /// Justified (stretch to fill width).
    Justify,
}

/// Line clamp configuration.
///
/// Limits the number of visible lines, truncating with an ellipsis
/// on the last visible line when content exceeds the limit.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct LineClamp {
    /// Maximum number of visible lines. `0` means unlimited.
    pub max_lines: u16,
}

impl LineClamp {
    /// Create an unlimited (no clamping) configuration.
    pub const UNLIMITED: Self = Self { max_lines: 0 };

    /// Create a line clamp with a specific maximum.
    #[must_use]
    pub const fn new(max_lines: u16) -> Self {
        Self { max_lines }
    }

    /// Whether clamping is active.
    #[inline]
    #[must_use]
    pub const fn is_active(self) -> bool {
        self.max_lines > 0
    }

    /// Apply clamping to a line count: returns the number of lines to render
    /// and whether truncation occurred.
    #[must_use]
    pub const fn clamp(self, line_count: usize) -> (usize, bool) {
        if self.max_lines == 0 || line_count <= self.max_lines as usize {
            (line_count, false)
        } else {
            (self.max_lines as usize, true)
        }
    }
}

/// Text attribute flags (16 bits for extended attribute support).
///
/// These flags represent visual attributes that can be applied to text.
/// Using u16 allows for additional underline variants beyond basic SGR.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[repr(transparent)]
pub struct StyleFlags(pub u16);

impl StyleFlags {
    /// No attributes set.
    pub const NONE: Self = Self(0);
    /// Bold / increased intensity.
    pub const BOLD: Self = Self(1 << 0);
    /// Dim / decreased intensity.
    pub const DIM: Self = Self(1 << 1);
    /// Italic text.
    pub const ITALIC: Self = Self(1 << 2);
    /// Single underline.
    pub const UNDERLINE: Self = Self(1 << 3);
    /// Blinking text.
    pub const BLINK: Self = Self(1 << 4);
    /// Reverse video (swap fg/bg).
    pub const REVERSE: Self = Self(1 << 5);
    /// Hidden / invisible text.
    pub const HIDDEN: Self = Self(1 << 6);
    /// Strikethrough text.
    pub const STRIKETHROUGH: Self = Self(1 << 7);
    /// Double underline (extended attribute).
    pub const DOUBLE_UNDERLINE: Self = Self(1 << 8);
    /// Curly / wavy underline (extended attribute).
    pub const CURLY_UNDERLINE: Self = Self(1 << 9);

    /// Check if this flags set contains another flags set.
    #[inline]
    pub const fn contains(self, other: Self) -> bool {
        (self.0 & other.0) == other.0
    }

    /// Insert flags into this set.
    #[inline]
    pub fn insert(&mut self, other: Self) {
        self.0 |= other.0;
    }

    /// Remove flags from this set.
    #[inline]
    pub fn remove(&mut self, other: Self) {
        self.0 &= !other.0;
    }

    /// Check if the flags set is empty.
    #[inline]
    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }

    /// Combine two flag sets (OR operation).
    #[inline]
    #[must_use]
    pub const fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }
}

impl core::ops::BitOr for StyleFlags {
    type Output = Self;

    #[inline]
    fn bitor(self, rhs: Self) -> Self::Output {
        Self(self.0 | rhs.0)
    }
}

impl core::ops::BitOrAssign for StyleFlags {
    #[inline]
    fn bitor_assign(&mut self, rhs: Self) {
        self.0 |= rhs.0;
    }
}

/// Unified styling type with CSS-like cascading semantics.
///
/// # Design Rationale
/// - Option fields allow inheritance (None = inherit from parent)
/// - Explicit masks track which properties are intentionally set
/// - Copy + small size for cheap passing
/// - Builder pattern for ergonomic construction
///
/// # Example
/// ```
/// use ftui_style::{Style, StyleFlags};
/// use ftui_render::cell::PackedRgba;
///
/// let style = Style::new()
///     .fg(PackedRgba::rgb(255, 0, 0))
///     .bg(PackedRgba::rgb(0, 0, 0))
///     .bold()
///     .underline();
/// ```
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Style {
    /// Foreground color (text color).
    pub fg: Option<PackedRgba>,
    /// Background color.
    pub bg: Option<PackedRgba>,
    /// Text attributes (bold, italic, etc.).
    pub attrs: Option<StyleFlags>,
    /// Underline color (separate from fg for flexibility).
    pub underline_color: Option<PackedRgba>,
}

impl Style {
    /// Create an empty style (all properties inherit).
    #[inline]
    pub const fn new() -> Self {
        Self {
            fg: None,
            bg: None,
            attrs: None,
            underline_color: None,
        }
    }

    /// Set foreground color.
    #[inline]
    #[must_use]
    pub fn fg<C: Into<PackedRgba>>(mut self, color: C) -> Self {
        self.fg = Some(color.into());
        self
    }

    /// Set background color.
    #[inline]
    #[must_use]
    pub fn bg<C: Into<PackedRgba>>(mut self, color: C) -> Self {
        self.bg = Some(color.into());
        self
    }

    /// Add bold attribute.
    #[inline]
    #[must_use]
    pub fn bold(self) -> Self {
        self.add_attr(StyleFlags::BOLD)
    }

    /// Add italic attribute.
    #[inline]
    #[must_use]
    pub fn italic(self) -> Self {
        self.add_attr(StyleFlags::ITALIC)
    }

    /// Add underline attribute.
    #[inline]
    #[must_use]
    pub fn underline(self) -> Self {
        self.add_attr(StyleFlags::UNDERLINE)
    }

    /// Add dim attribute.
    #[inline]
    #[must_use]
    pub fn dim(self) -> Self {
        self.add_attr(StyleFlags::DIM)
    }

    /// Add reverse video attribute.
    #[inline]
    #[must_use]
    pub fn reverse(self) -> Self {
        self.add_attr(StyleFlags::REVERSE)
    }

    /// Add strikethrough attribute.
    #[inline]
    #[must_use]
    pub fn strikethrough(self) -> Self {
        self.add_attr(StyleFlags::STRIKETHROUGH)
    }

    /// Add blink attribute.
    #[inline]
    #[must_use]
    pub fn blink(self) -> Self {
        self.add_attr(StyleFlags::BLINK)
    }

    /// Add hidden attribute.
    #[inline]
    #[must_use]
    pub fn hidden(self) -> Self {
        self.add_attr(StyleFlags::HIDDEN)
    }

    /// Add double underline attribute.
    #[inline]
    #[must_use]
    pub fn double_underline(self) -> Self {
        self.add_attr(StyleFlags::DOUBLE_UNDERLINE)
    }

    /// Add curly underline attribute.
    #[inline]
    #[must_use]
    pub fn curly_underline(self) -> Self {
        self.add_attr(StyleFlags::CURLY_UNDERLINE)
    }

    /// Add an attribute flag.
    #[inline]
    fn add_attr(mut self, flag: StyleFlags) -> Self {
        match &mut self.attrs {
            Some(attrs) => attrs.insert(flag),
            None => self.attrs = Some(flag),
        }
        self
    }

    /// Set underline color.
    #[inline]
    #[must_use]
    pub const fn underline_color(mut self, color: PackedRgba) -> Self {
        self.underline_color = Some(color);
        self
    }

    /// Set attributes directly.
    #[inline]
    #[must_use]
    pub const fn attrs(mut self, attrs: StyleFlags) -> Self {
        self.attrs = Some(attrs);
        self
    }

    /// Cascade merge: Fill in None fields from parent.
    ///
    /// `child.merge(parent)` returns a style where child's Some values
    /// take precedence, and parent fills in any None values.
    ///
    /// For attributes, the flags are combined (OR operation) so both
    /// parent and child attributes apply.
    ///
    /// # Example
    /// ```
    /// use ftui_style::Style;
    /// use ftui_render::cell::PackedRgba;
    ///
    /// let parent = Style::new().fg(PackedRgba::rgb(255, 0, 0)).bold();
    /// let child = Style::new().bg(PackedRgba::rgb(0, 0, 255));
    /// let merged = child.merge(&parent);
    /// // merged has: fg=RED (from parent), bg=BLUE (from child), bold (from parent)
    /// ```
    #[instrument(skip(self, parent), level = "trace")]
    pub fn merge(&self, parent: &Style) -> Style {
        trace!("Merging child style into parent");
        Style {
            fg: self.fg.or(parent.fg),
            bg: self.bg.or(parent.bg),
            attrs: match (self.attrs, parent.attrs) {
                (Some(c), Some(p)) => Some(c.union(p)),
                (Some(c), None) => Some(c),
                (None, Some(p)) => Some(p),
                (None, None) => None,
            },
            underline_color: self.underline_color.or(parent.underline_color),
        }
    }

    /// Patch merge: Override parent with child's Some values.
    ///
    /// `parent.patch(&child)` returns a style where child's Some values
    /// replace parent's values.
    ///
    /// This is the inverse perspective of merge().
    #[inline]
    pub fn patch(&self, child: &Style) -> Style {
        child.merge(self)
    }

    /// Check if this style has any properties set.
    #[inline]
    pub const fn is_empty(&self) -> bool {
        self.fg.is_none()
            && self.bg.is_none()
            && self.attrs.is_none()
            && self.underline_color.is_none()
    }

    /// Check if a specific attribute is set.
    #[inline]
    pub fn has_attr(&self, flag: StyleFlags) -> bool {
        self.attrs.is_some_and(|a| a.contains(flag))
    }
}

/// Convert from cell-level StyleFlags (8-bit) to style-level StyleFlags (16-bit).
impl From<ftui_render::cell::StyleFlags> for StyleFlags {
    fn from(flags: ftui_render::cell::StyleFlags) -> Self {
        let mut result = StyleFlags::NONE;
        if flags.contains(ftui_render::cell::StyleFlags::BOLD) {
            result.insert(StyleFlags::BOLD);
        }
        if flags.contains(ftui_render::cell::StyleFlags::DIM) {
            result.insert(StyleFlags::DIM);
        }
        if flags.contains(ftui_render::cell::StyleFlags::ITALIC) {
            result.insert(StyleFlags::ITALIC);
        }
        if flags.contains(ftui_render::cell::StyleFlags::UNDERLINE) {
            result.insert(StyleFlags::UNDERLINE);
        }
        if flags.contains(ftui_render::cell::StyleFlags::BLINK) {
            result.insert(StyleFlags::BLINK);
        }
        if flags.contains(ftui_render::cell::StyleFlags::REVERSE) {
            result.insert(StyleFlags::REVERSE);
        }
        if flags.contains(ftui_render::cell::StyleFlags::STRIKETHROUGH) {
            result.insert(StyleFlags::STRIKETHROUGH);
        }
        if flags.contains(ftui_render::cell::StyleFlags::HIDDEN) {
            result.insert(StyleFlags::HIDDEN);
        }
        result
    }
}

/// Convert from style-level StyleFlags (16-bit) to cell-level StyleFlags (8-bit).
///
/// Note: Extended flags (DOUBLE_UNDERLINE, CURLY_UNDERLINE) are mapped to
/// basic UNDERLINE since the cell-level representation doesn't support them.
impl From<StyleFlags> for ftui_render::cell::StyleFlags {
    fn from(flags: StyleFlags) -> Self {
        use ftui_render::cell::StyleFlags as CellFlags;
        let mut result = CellFlags::empty();
        if flags.contains(StyleFlags::BOLD) {
            result |= CellFlags::BOLD;
        }
        if flags.contains(StyleFlags::DIM) {
            result |= CellFlags::DIM;
        }
        if flags.contains(StyleFlags::ITALIC) {
            result |= CellFlags::ITALIC;
        }
        // Map all underline variants to basic underline
        if flags.contains(StyleFlags::UNDERLINE)
            || flags.contains(StyleFlags::DOUBLE_UNDERLINE)
            || flags.contains(StyleFlags::CURLY_UNDERLINE)
        {
            result |= CellFlags::UNDERLINE;
        }
        if flags.contains(StyleFlags::BLINK) {
            result |= CellFlags::BLINK;
        }
        if flags.contains(StyleFlags::REVERSE) {
            result |= CellFlags::REVERSE;
        }
        if flags.contains(StyleFlags::STRIKETHROUGH) {
            result |= CellFlags::STRIKETHROUGH;
        }
        if flags.contains(StyleFlags::HIDDEN) {
            result |= CellFlags::HIDDEN;
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_is_empty() {
        let s = Style::default();
        assert!(s.is_empty());
        assert_eq!(s.fg, None);
        assert_eq!(s.bg, None);
        assert_eq!(s.attrs, None);
        assert_eq!(s.underline_color, None);
    }

    #[test]
    fn test_new_is_empty() {
        let s = Style::new();
        assert!(s.is_empty());
    }

    #[test]
    fn test_builder_pattern_colors() {
        let red = PackedRgba::rgb(255, 0, 0);
        let black = PackedRgba::rgb(0, 0, 0);

        let s = Style::new().fg(red).bg(black);

        assert_eq!(s.fg, Some(red));
        assert_eq!(s.bg, Some(black));
        assert!(!s.is_empty());
    }

    #[test]
    fn test_builder_pattern_attrs() {
        let s = Style::new().bold().underline().italic();

        assert!(s.has_attr(StyleFlags::BOLD));
        assert!(s.has_attr(StyleFlags::UNDERLINE));
        assert!(s.has_attr(StyleFlags::ITALIC));
        assert!(!s.has_attr(StyleFlags::DIM));
    }

    #[test]
    fn test_all_attribute_builders() {
        let s = Style::new()
            .bold()
            .dim()
            .italic()
            .underline()
            .blink()
            .reverse()
            .hidden()
            .strikethrough()
            .double_underline()
            .curly_underline();

        assert!(s.has_attr(StyleFlags::BOLD));
        assert!(s.has_attr(StyleFlags::DIM));
        assert!(s.has_attr(StyleFlags::ITALIC));
        assert!(s.has_attr(StyleFlags::UNDERLINE));
        assert!(s.has_attr(StyleFlags::BLINK));
        assert!(s.has_attr(StyleFlags::REVERSE));
        assert!(s.has_attr(StyleFlags::HIDDEN));
        assert!(s.has_attr(StyleFlags::STRIKETHROUGH));
        assert!(s.has_attr(StyleFlags::DOUBLE_UNDERLINE));
        assert!(s.has_attr(StyleFlags::CURLY_UNDERLINE));
    }

    #[test]
    fn test_merge_child_wins_on_conflict() {
        let red = PackedRgba::rgb(255, 0, 0);
        let blue = PackedRgba::rgb(0, 0, 255);

        let parent = Style::new().fg(red);
        let child = Style::new().fg(blue);
        let merged = child.merge(&parent);

        assert_eq!(merged.fg, Some(blue)); // Child wins
    }

    #[test]
    fn test_merge_parent_fills_gaps() {
        let red = PackedRgba::rgb(255, 0, 0);
        let blue = PackedRgba::rgb(0, 0, 255);
        let white = PackedRgba::rgb(255, 255, 255);

        let parent = Style::new().fg(red).bg(white);
        let child = Style::new().fg(blue); // No bg
        let merged = child.merge(&parent);

        assert_eq!(merged.fg, Some(blue)); // Child fg
        assert_eq!(merged.bg, Some(white)); // Parent fills bg
    }

    #[test]
    fn test_merge_attrs_combine() {
        let parent = Style::new().bold();
        let child = Style::new().italic();
        let merged = child.merge(&parent);

        assert!(merged.has_attr(StyleFlags::BOLD)); // From parent
        assert!(merged.has_attr(StyleFlags::ITALIC)); // From child
    }

    #[test]
    fn test_merge_with_empty_returns_self() {
        let red = PackedRgba::rgb(255, 0, 0);
        let style = Style::new().fg(red).bold();
        let empty = Style::default();

        let merged = style.merge(&empty);
        assert_eq!(merged, style);
    }

    #[test]
    fn test_empty_merge_with_parent() {
        let red = PackedRgba::rgb(255, 0, 0);
        let parent = Style::new().fg(red).bold();
        let child = Style::default();

        let merged = child.merge(&parent);
        assert_eq!(merged, parent);
    }

    #[test]
    fn test_patch_is_symmetric_with_merge() {
        let red = PackedRgba::rgb(255, 0, 0);
        let blue = PackedRgba::rgb(0, 0, 255);

        let parent = Style::new().fg(red);
        let child = Style::new().bg(blue);

        let merged1 = child.merge(&parent);
        let merged2 = parent.patch(&child);

        assert_eq!(merged1, merged2);
    }

    #[test]
    fn test_underline_color() {
        let red = PackedRgba::rgb(255, 0, 0);
        let s = Style::new().underline().underline_color(red);

        assert!(s.has_attr(StyleFlags::UNDERLINE));
        assert_eq!(s.underline_color, Some(red));
    }

    #[test]
    fn test_style_flags_operations() {
        let mut flags = StyleFlags::NONE;
        assert!(flags.is_empty());

        flags.insert(StyleFlags::BOLD);
        flags.insert(StyleFlags::ITALIC);

        assert!(flags.contains(StyleFlags::BOLD));
        assert!(flags.contains(StyleFlags::ITALIC));
        assert!(!flags.contains(StyleFlags::UNDERLINE));
        assert!(!flags.is_empty());

        flags.remove(StyleFlags::BOLD);
        assert!(!flags.contains(StyleFlags::BOLD));
        assert!(flags.contains(StyleFlags::ITALIC));
    }

    #[test]
    fn test_style_flags_bitor() {
        let flags = StyleFlags::BOLD | StyleFlags::ITALIC;
        assert!(flags.contains(StyleFlags::BOLD));
        assert!(flags.contains(StyleFlags::ITALIC));
    }

    #[test]
    fn test_style_flags_bitor_assign() {
        let mut flags = StyleFlags::BOLD;
        flags |= StyleFlags::ITALIC;
        assert!(flags.contains(StyleFlags::BOLD));
        assert!(flags.contains(StyleFlags::ITALIC));
    }

    #[test]
    fn test_style_flags_union() {
        let a = StyleFlags::BOLD;
        let b = StyleFlags::ITALIC;
        let c = a.union(b);
        assert!(c.contains(StyleFlags::BOLD));
        assert!(c.contains(StyleFlags::ITALIC));
    }

    #[test]
    fn test_style_size() {
        // Style should fit in a reasonable size
        // 4 Option<PackedRgba> = 4 * 8 = 32 bytes (with Option overhead)
        // + 1 Option<StyleFlags> = 4 bytes
        // Total should be <= 40 bytes
        assert!(
            core::mem::size_of::<Style>() <= 40,
            "Style is {} bytes, expected <= 40",
            core::mem::size_of::<Style>()
        );
    }

    #[test]
    fn test_style_flags_size() {
        assert_eq!(core::mem::size_of::<StyleFlags>(), 2);
    }

    #[test]
    fn test_convert_to_cell_flags() {
        let flags = StyleFlags::BOLD | StyleFlags::ITALIC | StyleFlags::UNDERLINE;
        let cell_flags: ftui_render::cell::StyleFlags = flags.into();

        assert!(cell_flags.contains(ftui_render::cell::StyleFlags::BOLD));
        assert!(cell_flags.contains(ftui_render::cell::StyleFlags::ITALIC));
        assert!(cell_flags.contains(ftui_render::cell::StyleFlags::UNDERLINE));
    }

    #[test]
    fn test_convert_to_cell_flags_all_basic() {
        let flags = StyleFlags::BOLD
            | StyleFlags::DIM
            | StyleFlags::ITALIC
            | StyleFlags::UNDERLINE
            | StyleFlags::BLINK
            | StyleFlags::REVERSE
            | StyleFlags::STRIKETHROUGH
            | StyleFlags::HIDDEN;
        let cell_flags: ftui_render::cell::StyleFlags = flags.into();

        assert!(cell_flags.contains(ftui_render::cell::StyleFlags::BOLD));
        assert!(cell_flags.contains(ftui_render::cell::StyleFlags::DIM));
        assert!(cell_flags.contains(ftui_render::cell::StyleFlags::ITALIC));
        assert!(cell_flags.contains(ftui_render::cell::StyleFlags::UNDERLINE));
        assert!(cell_flags.contains(ftui_render::cell::StyleFlags::BLINK));
        assert!(cell_flags.contains(ftui_render::cell::StyleFlags::REVERSE));
        assert!(cell_flags.contains(ftui_render::cell::StyleFlags::STRIKETHROUGH));
        assert!(cell_flags.contains(ftui_render::cell::StyleFlags::HIDDEN));
    }

    #[test]
    fn test_convert_from_cell_flags() {
        use ftui_render::cell::StyleFlags as CellFlags;
        let cell_flags = CellFlags::BOLD | CellFlags::ITALIC;
        let style_flags: StyleFlags = cell_flags.into();

        assert!(style_flags.contains(StyleFlags::BOLD));
        assert!(style_flags.contains(StyleFlags::ITALIC));
    }

    #[test]
    fn test_cell_flags_round_trip_preserves_basic_flags() {
        use ftui_render::cell::StyleFlags as CellFlags;
        let original = StyleFlags::BOLD
            | StyleFlags::DIM
            | StyleFlags::ITALIC
            | StyleFlags::UNDERLINE
            | StyleFlags::BLINK
            | StyleFlags::REVERSE
            | StyleFlags::STRIKETHROUGH
            | StyleFlags::HIDDEN;
        let cell_flags: CellFlags = original.into();
        let round_trip: StyleFlags = cell_flags.into();

        assert!(round_trip.contains(StyleFlags::BOLD));
        assert!(round_trip.contains(StyleFlags::DIM));
        assert!(round_trip.contains(StyleFlags::ITALIC));
        assert!(round_trip.contains(StyleFlags::UNDERLINE));
        assert!(round_trip.contains(StyleFlags::BLINK));
        assert!(round_trip.contains(StyleFlags::REVERSE));
        assert!(round_trip.contains(StyleFlags::STRIKETHROUGH));
        assert!(round_trip.contains(StyleFlags::HIDDEN));
    }

    #[test]
    fn test_extended_underline_maps_to_basic() {
        let flags = StyleFlags::DOUBLE_UNDERLINE | StyleFlags::CURLY_UNDERLINE;
        let cell_flags: ftui_render::cell::StyleFlags = flags.into();

        // Extended underlines map to basic underline in cell representation
        assert!(cell_flags.contains(ftui_render::cell::StyleFlags::UNDERLINE));
    }
}

#[cfg(test)]
mod property_tests {
    use super::*;
    use proptest::prelude::*;

    fn arb_packed_rgba() -> impl Strategy<Value = PackedRgba> {
        any::<u32>().prop_map(PackedRgba)
    }

    fn arb_style_flags() -> impl Strategy<Value = StyleFlags> {
        any::<u16>().prop_map(StyleFlags)
    }

    fn arb_style() -> impl Strategy<Value = Style> {
        (
            proptest::option::of(arb_packed_rgba()),
            proptest::option::of(arb_packed_rgba()),
            proptest::option::of(arb_style_flags()),
            proptest::option::of(arb_packed_rgba()),
        )
            .prop_map(|(fg, bg, attrs, underline_color)| Style {
                fg,
                bg,
                attrs,
                underline_color,
            })
    }

    proptest! {
        #[test]
        fn merge_with_empty_is_identity(s in arb_style()) {
            let empty = Style::default();
            prop_assert_eq!(s.merge(&empty), s);
        }

        #[test]
        fn empty_merge_with_any_equals_any(parent in arb_style()) {
            let empty = Style::default();
            prop_assert_eq!(empty.merge(&parent), parent);
        }

        #[test]
        fn merge_is_deterministic(a in arb_style(), b in arb_style()) {
            let merged1 = a.merge(&b);
            let merged2 = a.merge(&b);
            prop_assert_eq!(merged1, merged2);
        }

        #[test]
        fn patch_equals_reverse_merge(parent in arb_style(), child in arb_style()) {
            let via_merge = child.merge(&parent);
            let via_patch = parent.patch(&child);
            prop_assert_eq!(via_merge, via_patch);
        }

        #[test]
        fn style_flags_union_is_commutative(a in arb_style_flags(), b in arb_style_flags()) {
            prop_assert_eq!(a.union(b), b.union(a));
        }

        #[test]
        fn style_flags_union_is_associative(
            a in arb_style_flags(),
            b in arb_style_flags(),
            c in arb_style_flags()
        ) {
            prop_assert_eq!(a.union(b).union(c), a.union(b.union(c)));
        }
    }
}

#[cfg(test)]
mod merge_semantic_tests {
    //! Tests for merge behavior and determinism.
    //!
    //! These tests verify the cascading semantics: child overrides parent
    //! for colors, and flags combine for attributes.

    use super::*;

    #[test]
    fn merge_chain_three_styles() {
        // Test merging a chain: grandchild -> child -> parent
        let red = PackedRgba::rgb(255, 0, 0);
        let green = PackedRgba::rgb(0, 255, 0);
        let blue = PackedRgba::rgb(0, 0, 255);
        let white = PackedRgba::rgb(255, 255, 255);

        let grandparent = Style::new().fg(red).bg(white).bold();
        let parent = Style::new().fg(green).italic();
        let child = Style::new().fg(blue);

        // First merge: parent <- grandparent
        let parent_merged = parent.merge(&grandparent);
        assert_eq!(parent_merged.fg, Some(green)); // parent wins
        assert_eq!(parent_merged.bg, Some(white)); // inherited from grandparent
        assert!(parent_merged.has_attr(StyleFlags::BOLD)); // inherited
        assert!(parent_merged.has_attr(StyleFlags::ITALIC)); // parent's

        // Second merge: child <- parent_merged
        let child_merged = child.merge(&parent_merged);
        assert_eq!(child_merged.fg, Some(blue)); // child wins
        assert_eq!(child_merged.bg, Some(white)); // inherited from grandparent
        assert!(child_merged.has_attr(StyleFlags::BOLD)); // inherited
        assert!(child_merged.has_attr(StyleFlags::ITALIC)); // inherited
    }

    #[test]
    fn merge_chain_attrs_accumulate() {
        // Attributes from all ancestors should accumulate
        let s1 = Style::new().bold();
        let s2 = Style::new().italic();
        let s3 = Style::new().underline();

        let merged = s3.merge(&s2.merge(&s1));

        assert!(merged.has_attr(StyleFlags::BOLD));
        assert!(merged.has_attr(StyleFlags::ITALIC));
        assert!(merged.has_attr(StyleFlags::UNDERLINE));
    }

    #[test]
    fn has_attr_returns_false_for_none() {
        let style = Style::new(); // attrs is None
        assert!(!style.has_attr(StyleFlags::BOLD));
        assert!(!style.has_attr(StyleFlags::ITALIC));
        assert!(!style.has_attr(StyleFlags::NONE));
    }

    #[test]
    fn has_attr_returns_true_for_set_flags() {
        let style = Style::new().bold().italic();
        assert!(style.has_attr(StyleFlags::BOLD));
        assert!(style.has_attr(StyleFlags::ITALIC));
        assert!(!style.has_attr(StyleFlags::UNDERLINE));
    }

    #[test]
    fn attrs_method_sets_directly() {
        let flags = StyleFlags::BOLD | StyleFlags::DIM | StyleFlags::ITALIC;
        let style = Style::new().attrs(flags);

        assert_eq!(style.attrs, Some(flags));
        assert!(style.has_attr(StyleFlags::BOLD));
        assert!(style.has_attr(StyleFlags::DIM));
        assert!(style.has_attr(StyleFlags::ITALIC));
    }

    #[test]
    fn attrs_method_overwrites_previous() {
        let style = Style::new().bold().italic().attrs(StyleFlags::UNDERLINE); // overwrites, doesn't combine

        assert!(style.has_attr(StyleFlags::UNDERLINE));
        // Bold and italic are NOT preserved when using attrs() directly
        assert!(!style.has_attr(StyleFlags::BOLD));
        assert!(!style.has_attr(StyleFlags::ITALIC));
    }

    #[test]
    fn merge_preserves_explicit_transparent_color() {
        // TRANSPARENT is a valid explicit color, should not be treated as "unset"
        let transparent = PackedRgba::TRANSPARENT;
        let red = PackedRgba::rgb(255, 0, 0);

        let parent = Style::new().fg(red);
        let child = Style::new().fg(transparent);

        let merged = child.merge(&parent);
        // Child explicitly sets transparent, should win over parent's red
        assert_eq!(merged.fg, Some(transparent));
    }

    #[test]
    fn merge_all_fields_independently() {
        let parent = Style::new()
            .fg(PackedRgba::rgb(1, 1, 1))
            .bg(PackedRgba::rgb(2, 2, 2))
            .underline_color(PackedRgba::rgb(3, 3, 3))
            .bold();

        let child = Style::new()
            .fg(PackedRgba::rgb(10, 10, 10))
            // no bg - should inherit
            .underline_color(PackedRgba::rgb(30, 30, 30))
            .italic();

        let merged = child.merge(&parent);

        // Child overrides fg
        assert_eq!(merged.fg, Some(PackedRgba::rgb(10, 10, 10)));
        // Parent fills bg
        assert_eq!(merged.bg, Some(PackedRgba::rgb(2, 2, 2)));
        // Child overrides underline_color
        assert_eq!(merged.underline_color, Some(PackedRgba::rgb(30, 30, 30)));
        // Both attrs combined
        assert!(merged.has_attr(StyleFlags::BOLD));
        assert!(merged.has_attr(StyleFlags::ITALIC));
    }

    #[test]
    fn style_is_copy() {
        let style = Style::new().fg(PackedRgba::rgb(255, 0, 0)).bold();
        let copy = style; // Copy, not move
        assert_eq!(style, copy);
    }

    #[test]
    fn style_is_eq() {
        let a = Style::new().fg(PackedRgba::rgb(255, 0, 0)).bold();
        let b = Style::new().fg(PackedRgba::rgb(255, 0, 0)).bold();
        let c = Style::new().fg(PackedRgba::rgb(0, 255, 0)).bold();

        assert_eq!(a, b);
        assert_ne!(a, c);
    }

    #[test]
    fn style_is_hashable() {
        use std::collections::HashSet;
        let mut set = HashSet::new();

        let a = Style::new().fg(PackedRgba::rgb(255, 0, 0)).bold();
        let b = Style::new().fg(PackedRgba::rgb(0, 255, 0)).italic();

        set.insert(a);
        set.insert(b);
        set.insert(a); // duplicate

        assert_eq!(set.len(), 2);
    }

    #[test]
    fn style_flags_contains_combined() {
        let combined = StyleFlags::BOLD | StyleFlags::ITALIC | StyleFlags::UNDERLINE;

        // contains should return true for individual flags
        assert!(combined.contains(StyleFlags::BOLD));
        assert!(combined.contains(StyleFlags::ITALIC));
        assert!(combined.contains(StyleFlags::UNDERLINE));

        // contains should return true for subsets
        assert!(combined.contains(StyleFlags::BOLD | StyleFlags::ITALIC));

        // contains should return false for non-subset
        assert!(!combined.contains(StyleFlags::DIM));
        assert!(!combined.contains(StyleFlags::BOLD | StyleFlags::DIM));
    }

    #[test]
    fn style_flags_none_is_identity_for_union() {
        let flags = StyleFlags::BOLD | StyleFlags::ITALIC;
        assert_eq!(flags.union(StyleFlags::NONE), flags);
        assert_eq!(StyleFlags::NONE.union(flags), flags);
    }

    #[test]
    fn style_flags_remove_nonexistent_is_noop() {
        let mut flags = StyleFlags::BOLD;
        flags.remove(StyleFlags::ITALIC); // Not set, should be no-op
        assert!(flags.contains(StyleFlags::BOLD));
        assert!(!flags.contains(StyleFlags::ITALIC));
    }
}

#[cfg(test)]
mod performance_tests {
    use super::*;

    #[test]
    fn test_style_merge_performance() {
        let red = PackedRgba::rgb(255, 0, 0);
        let blue = PackedRgba::rgb(0, 0, 255);

        let parent = Style::new().fg(red).bold();
        let child = Style::new().bg(blue).italic();

        let start = std::time::Instant::now();
        for _ in 0..1_000_000 {
            let _ = std::hint::black_box(child.merge(&parent));
        }
        let elapsed = start.elapsed();

        // 1M merges should be < 100ms (< 100ns each)
        // Being generous with threshold for CI variability
        assert!(
            elapsed.as_millis() < 100,
            "Merge too slow: {:?} for 1M iterations",
            elapsed
        );
    }
}

#[cfg(test)]
mod parity_tests {
    use super::*;

    // ── TextTransform ─────────────────────────────────────────────

    #[test]
    fn text_transform_none_is_identity() {
        assert_eq!(TextTransform::None.apply("Hello World"), "Hello World");
    }

    #[test]
    fn text_transform_uppercase() {
        assert_eq!(TextTransform::Uppercase.apply("hello world"), "HELLO WORLD");
    }

    #[test]
    fn text_transform_lowercase() {
        assert_eq!(TextTransform::Lowercase.apply("HELLO WORLD"), "hello world");
    }

    #[test]
    fn text_transform_capitalize() {
        assert_eq!(
            TextTransform::Capitalize.apply("hello world"),
            "Hello World"
        );
        assert_eq!(
            TextTransform::Capitalize.apply("  two  spaces"),
            "  Two  Spaces"
        );
    }

    #[test]
    fn text_transform_empty_string() {
        assert_eq!(TextTransform::Uppercase.apply(""), "");
        assert_eq!(TextTransform::Capitalize.apply(""), "");
    }

    #[test]
    fn text_transform_default_is_none() {
        assert_eq!(TextTransform::default(), TextTransform::None);
    }

    // ── TextOverflow ──────────────────────────────────────────────

    #[test]
    fn text_overflow_default_is_clip() {
        assert_eq!(TextOverflow::default(), TextOverflow::Clip);
    }

    #[test]
    fn text_overflow_indicator_stores_char() {
        let overflow = TextOverflow::Indicator('>');
        assert_eq!(overflow, TextOverflow::Indicator('>'));
    }

    // ── Overflow ──────────────────────────────────────────────────

    #[test]
    fn overflow_default_is_hidden() {
        assert_eq!(Overflow::default(), Overflow::Hidden);
    }

    // ── WhiteSpaceMode ───────────────────────────────────────────

    #[test]
    fn whitespace_normal_collapses_and_wraps() {
        let mode = WhiteSpaceMode::Normal;
        assert!(mode.collapses_whitespace());
        assert!(mode.allows_wrap());
        assert!(!mode.preserves_newlines());
    }

    #[test]
    fn whitespace_pre_preserves_all() {
        let mode = WhiteSpaceMode::Pre;
        assert!(!mode.collapses_whitespace());
        assert!(!mode.allows_wrap());
        assert!(mode.preserves_newlines());
    }

    #[test]
    fn whitespace_pre_wrap_preserves_and_wraps() {
        let mode = WhiteSpaceMode::PreWrap;
        assert!(!mode.collapses_whitespace());
        assert!(mode.allows_wrap());
        assert!(mode.preserves_newlines());
    }

    #[test]
    fn whitespace_pre_line_collapses_preserves_newlines_and_wraps() {
        let mode = WhiteSpaceMode::PreLine;
        assert!(mode.collapses_whitespace());
        assert!(mode.allows_wrap());
        assert!(mode.preserves_newlines());
    }

    #[test]
    fn whitespace_nowrap_collapses_no_wrap() {
        let mode = WhiteSpaceMode::NoWrap;
        assert!(mode.collapses_whitespace());
        assert!(!mode.allows_wrap());
        assert!(!mode.preserves_newlines());
    }

    #[test]
    fn whitespace_default_is_normal() {
        assert_eq!(WhiteSpaceMode::default(), WhiteSpaceMode::Normal);
    }

    // ── TextAlign ────────────────────────────────────────────────

    #[test]
    fn text_align_default_is_left() {
        assert_eq!(TextAlign::default(), TextAlign::Left);
    }

    // ── LineClamp ────────────────────────────────────────────────

    #[test]
    fn line_clamp_unlimited() {
        let clamp = LineClamp::UNLIMITED;
        assert!(!clamp.is_active());
        assert_eq!(clamp.clamp(100), (100, false));
    }

    #[test]
    fn line_clamp_active() {
        let clamp = LineClamp::new(3);
        assert!(clamp.is_active());
        assert_eq!(clamp.clamp(5), (3, true));
        assert_eq!(clamp.clamp(3), (3, false));
        assert_eq!(clamp.clamp(1), (1, false));
    }

    #[test]
    fn line_clamp_default_is_unlimited() {
        let clamp = LineClamp::default();
        assert!(!clamp.is_active());
        assert_eq!(clamp.max_lines, 0);
    }
}
