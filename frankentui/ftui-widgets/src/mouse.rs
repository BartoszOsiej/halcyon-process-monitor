//! Shared mouse event result type for widget mouse handling.

/// Result of processing a mouse event on a widget.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MouseResult {
    /// Event not relevant to this widget.
    Ignored,
    /// Selection changed to the given index.
    Selected(usize),
    /// Item activated (double-click, expand/collapse).
    Activated(usize),
    /// Scroll position changed.
    Scrolled,
    /// Hover state changed.
    HoverChanged,
}
