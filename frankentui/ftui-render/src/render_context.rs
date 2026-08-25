//! Continuation marks for layout/style debug tracing.
//!
//! During widget rendering, each level in the call tree can push a
//! [`RenderMark`] onto a thread-local stack. Debug overlays and tracing
//! subscribers can then inspect the current mark stack to answer
//! "why is this widget rendering this way?" without requiring every
//! intermediate function to thread extra parameters.
//!
//! # Usage
//!
//! ```rust,no_run
//! use ftui_render::render_context::{push_mark, pop_mark, current_marks, MarkGuard};
//!
//! // RAII guard (preferred) — mark is popped when guard drops:
//! {
//!     let _g = MarkGuard::constraint("FlexRow", "min_width=20, weight=1.0");
//!     // ... render child widgets ...
//! } // mark automatically popped here
//!
//! // Manual push/pop:
//! push_mark(ftui_render::render_context::RenderMark::style("Button", "theme.primary"));
//! // ... render ...
//! pop_mark();
//!
//! // Inspect during render:
//! let marks = current_marks();
//! ```
//!
//! # Lifecycle
//!
//! Call [`clear_marks`] at the start of each frame to prevent stale marks
//! from leaking across frames. The runtime does this automatically in the
//! render loop.

use std::cell::RefCell;
use std::fmt;

/// A single continuation mark recording why a widget renders a certain way.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenderMark {
    /// The kind of provenance this mark records.
    pub kind: MarkKind,
    /// Human-readable label (widget name, component, etc.).
    pub label: &'static str,
    /// Detail string (constraint expression, style token, etc.).
    pub detail: String,
}

/// What aspect of rendering this mark describes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MarkKind {
    /// A layout constraint (min/max size, flex weight, alignment).
    Constraint,
    /// A style origin (theme token, inline override, cascade step).
    Style,
    /// A widget boundary (entering a named widget's render scope).
    Widget,
    /// Custom application-defined provenance.
    Custom,
}

impl fmt::Display for MarkKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Constraint => write!(f, "constraint"),
            Self::Style => write!(f, "style"),
            Self::Widget => write!(f, "widget"),
            Self::Custom => write!(f, "custom"),
        }
    }
}

impl RenderMark {
    /// Create a constraint-provenance mark.
    pub fn constraint(label: &'static str, detail: impl Into<String>) -> Self {
        Self {
            kind: MarkKind::Constraint,
            label,
            detail: detail.into(),
        }
    }

    /// Create a style-provenance mark.
    pub fn style(label: &'static str, detail: impl Into<String>) -> Self {
        Self {
            kind: MarkKind::Style,
            label,
            detail: detail.into(),
        }
    }

    /// Create a widget-boundary mark.
    pub fn widget(label: &'static str) -> Self {
        Self {
            kind: MarkKind::Widget,
            label,
            detail: String::new(),
        }
    }

    /// Create a custom mark with arbitrary kind.
    pub fn custom(label: &'static str, detail: impl Into<String>) -> Self {
        Self {
            kind: MarkKind::Custom,
            label,
            detail: detail.into(),
        }
    }
}

impl fmt::Display for RenderMark {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.detail.is_empty() {
            write!(f, "[{}] {}", self.kind, self.label)
        } else {
            write!(f, "[{}] {}: {}", self.kind, self.label, self.detail)
        }
    }
}

// ---------------------------------------------------------------------------
// Thread-local mark stack
// ---------------------------------------------------------------------------

thread_local! {
    static MARK_STACK: RefCell<Vec<RenderMark>> = const { RefCell::new(Vec::new()) };
}

/// Push a mark onto the thread-local stack.
pub fn push_mark(mark: RenderMark) {
    MARK_STACK.with(|stack| stack.borrow_mut().push(mark));
}

/// Pop the most recent mark. Returns `None` if the stack is empty.
pub fn pop_mark() -> Option<RenderMark> {
    MARK_STACK.with(|stack| stack.borrow_mut().pop())
}

/// Snapshot the current mark stack (bottom to top).
pub fn current_marks() -> Vec<RenderMark> {
    MARK_STACK.with(|stack| stack.borrow().clone())
}

/// Current depth of the mark stack.
pub fn mark_depth() -> usize {
    MARK_STACK.with(|stack| stack.borrow().len())
}

/// Clear all marks. Call at the start of each frame.
pub fn clear_marks() {
    MARK_STACK.with(|stack| stack.borrow_mut().clear());
}

// ---------------------------------------------------------------------------
// RAII guard
// ---------------------------------------------------------------------------

/// RAII guard that pops a [`RenderMark`] when dropped.
///
/// This is the preferred way to use continuation marks — the mark is
/// automatically removed when the guard goes out of scope, preventing
/// stack leaks even in the presence of early returns.
pub struct MarkGuard {
    _private: (),
}

impl MarkGuard {
    /// Push a constraint mark and return a guard that pops it on drop.
    pub fn constraint(label: &'static str, detail: impl Into<String>) -> Self {
        push_mark(RenderMark::constraint(label, detail));
        Self { _private: () }
    }

    /// Push a style mark and return a guard that pops it on drop.
    pub fn style(label: &'static str, detail: impl Into<String>) -> Self {
        push_mark(RenderMark::style(label, detail));
        Self { _private: () }
    }

    /// Push a widget mark and return a guard that pops it on drop.
    pub fn widget(label: &'static str) -> Self {
        push_mark(RenderMark::widget(label));
        Self { _private: () }
    }

    /// Push a custom mark and return a guard that pops it on drop.
    pub fn custom(label: &'static str, detail: impl Into<String>) -> Self {
        push_mark(RenderMark::custom(label, detail));
        Self { _private: () }
    }

    /// Push an arbitrary mark and return a guard.
    pub fn new(mark: RenderMark) -> Self {
        push_mark(mark);
        Self { _private: () }
    }
}

impl Drop for MarkGuard {
    fn drop(&mut self) {
        pop_mark();
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn with_clean_stack(f: impl FnOnce()) {
        clear_marks();
        f();
        clear_marks();
    }

    #[test]
    fn push_pop_basic() {
        with_clean_stack(|| {
            push_mark(RenderMark::widget("Button"));
            assert_eq!(mark_depth(), 1);
            let m = pop_mark().unwrap();
            assert_eq!(m.kind, MarkKind::Widget);
            assert_eq!(m.label, "Button");
            assert_eq!(mark_depth(), 0);
        });
    }

    #[test]
    fn pop_empty_returns_none() {
        with_clean_stack(|| {
            assert!(pop_mark().is_none());
        });
    }

    #[test]
    fn guard_auto_pops() {
        with_clean_stack(|| {
            {
                let _g = MarkGuard::widget("Outer");
                assert_eq!(mark_depth(), 1);
                {
                    let _g2 = MarkGuard::constraint("Inner", "flex=1");
                    assert_eq!(mark_depth(), 2);
                }
                assert_eq!(mark_depth(), 1);
            }
            assert_eq!(mark_depth(), 0);
        });
    }

    #[test]
    fn current_marks_snapshot() {
        with_clean_stack(|| {
            push_mark(RenderMark::widget("Root"));
            push_mark(RenderMark::style("Text", "theme.fg"));
            let marks = current_marks();
            assert_eq!(marks.len(), 2);
            assert_eq!(marks[0].label, "Root");
            assert_eq!(marks[1].label, "Text");
        });
    }

    #[test]
    fn clear_removes_all() {
        with_clean_stack(|| {
            push_mark(RenderMark::widget("A"));
            push_mark(RenderMark::widget("B"));
            push_mark(RenderMark::widget("C"));
            assert_eq!(mark_depth(), 3);
            clear_marks();
            assert_eq!(mark_depth(), 0);
        });
    }

    #[test]
    fn display_formatting() {
        let m = RenderMark::constraint("FlexRow", "min=20, weight=1.0");
        assert_eq!(m.to_string(), "[constraint] FlexRow: min=20, weight=1.0");

        let m2 = RenderMark::widget("Button");
        assert_eq!(m2.to_string(), "[widget] Button");
    }

    #[test]
    fn nested_guard_early_return() {
        with_clean_stack(|| {
            let f = || -> Option<()> {
                let _g = MarkGuard::widget("Container");
                assert_eq!(mark_depth(), 1);
                if true {
                    return None; // early return — guard still drops
                }
                #[allow(unreachable_code)]
                Some(())
            };
            f();
            assert_eq!(mark_depth(), 0);
        });
    }

    #[test]
    fn mark_kind_display() {
        assert_eq!(format!("{}", MarkKind::Constraint), "constraint");
        assert_eq!(format!("{}", MarkKind::Style), "style");
        assert_eq!(format!("{}", MarkKind::Widget), "widget");
        assert_eq!(format!("{}", MarkKind::Custom), "custom");
    }

    #[test]
    fn mark_equality() {
        let a = RenderMark::widget("X");
        let b = RenderMark::widget("X");
        let c = RenderMark::widget("Y");
        assert_eq!(a, b);
        assert_ne!(a, c);
    }

    #[test]
    fn custom_mark() {
        with_clean_stack(|| {
            let _g = MarkGuard::custom("perf", "budget_exceeded=true");
            let marks = current_marks();
            assert_eq!(marks[0].kind, MarkKind::Custom);
            assert_eq!(marks[0].detail, "budget_exceeded=true");
        });
    }
}
