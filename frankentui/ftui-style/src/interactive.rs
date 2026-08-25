// SPDX-License-Identifier: Apache-2.0
//! Interactive style variants for stateful widgets.
//!
//! [`InteractiveStyle`] holds style overrides for different interaction states:
//! normal, hovered, focused, active (pressed), and disabled. When resolving
//! the current style, the appropriate variant is merged on top of the base
//! style using [`Style::patch`].
//!
//! # Migration rationale
//!
//! Web CSS uses pseudo-classes (`:hover`, `:focus`, `:active`, `:disabled`)
//! to apply conditional styles. This module provides an equivalent
//! terminal-native model that the migration code emitter can target.
//!
//! # Example
//!
//! ```
//! use ftui_style::Style;
//! use ftui_style::interactive::{InteractiveStyle, InteractionState};
//! use ftui_render::cell::PackedRgba;
//!
//! let interactive = InteractiveStyle::new(
//!     Style::new().fg(PackedRgba::WHITE).bg(PackedRgba::rgb(64, 64, 64)),
//! )
//! .hover(Style::new().bg(PackedRgba::rgb(128, 128, 128)))
//! .focused(Style::new().bg(PackedRgba::BLUE))
//! .disabled(Style::new().fg(PackedRgba::rgb(64, 64, 64)));
//!
//! // Resolve for the current state
//! let current = interactive.resolve(InteractionState::Hovered);
//! ```

#![forbid(unsafe_code)]

use crate::style::Style;

/// The interaction state of a widget.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum InteractionState {
    /// Default state — no user interaction.
    Normal,
    /// Mouse cursor is over the widget.
    Hovered,
    /// Widget has keyboard focus.
    Focused,
    /// Widget is being pressed/activated.
    Active,
    /// Widget is non-interactive.
    Disabled,
    /// Widget has both focus and hover.
    FocusedHovered,
}

/// Style variants for different interaction states.
///
/// Each variant is an optional [`Style`] overlay. When resolving, the variant
/// for the current state is merged on top of the `normal` base style using
/// [`Style::patch`], preserving CSS-like specificity: the more specific state
/// wins for any property it sets.
#[derive(Debug, Clone, PartialEq)]
pub struct InteractiveStyle {
    /// Base style applied in all states.
    pub normal: Style,
    /// Override applied when hovered.
    pub hover: Option<Style>,
    /// Override applied when focused.
    pub focus: Option<Style>,
    /// Override applied when active (pressed).
    pub active: Option<Style>,
    /// Override applied when disabled.
    pub disabled: Option<Style>,
}

impl InteractiveStyle {
    /// Create an interactive style with the given base style.
    pub fn new(normal: Style) -> Self {
        Self {
            normal,
            hover: None,
            focus: None,
            active: None,
            disabled: None,
        }
    }

    /// Set the hover style override.
    #[must_use]
    pub fn hover(mut self, style: Style) -> Self {
        self.hover = Some(style);
        self
    }

    /// Set the focus style override.
    #[must_use]
    pub fn focused(mut self, style: Style) -> Self {
        self.focus = Some(style);
        self
    }

    /// Set the active (pressed) style override.
    #[must_use]
    pub fn active(mut self, style: Style) -> Self {
        self.active = Some(style);
        self
    }

    /// Set the disabled style override.
    #[must_use]
    pub fn disabled(mut self, style: Style) -> Self {
        self.disabled = Some(style);
        self
    }

    /// Resolve the style for the given interaction state.
    ///
    /// Starts with `normal` and patches the state-specific override on top.
    /// For `FocusedHovered`, both focus and hover are applied (focus first,
    /// then hover, so hover wins for conflicting properties).
    pub fn resolve(&self, state: InteractionState) -> Style {
        let base = self.normal;
        match state {
            InteractionState::Normal => base,
            InteractionState::Hovered => {
                if let Some(h) = &self.hover {
                    base.patch(h)
                } else {
                    base
                }
            }
            InteractionState::Focused => {
                if let Some(f) = &self.focus {
                    base.patch(f)
                } else {
                    base
                }
            }
            InteractionState::Active => {
                if let Some(a) = &self.active {
                    base.patch(a)
                } else {
                    base
                }
            }
            InteractionState::Disabled => {
                if let Some(d) = &self.disabled {
                    base.patch(d)
                } else {
                    base
                }
            }
            InteractionState::FocusedHovered => {
                let mut result = base;
                if let Some(f) = &self.focus {
                    result = result.patch(f);
                }
                if let Some(h) = &self.hover {
                    result = result.patch(h);
                }
                result
            }
        }
    }

    /// Check whether the given state has a specific override.
    pub fn has_override(&self, state: InteractionState) -> bool {
        match state {
            InteractionState::Normal => true,
            InteractionState::Hovered => self.hover.is_some(),
            InteractionState::Focused => self.focus.is_some(),
            InteractionState::Active => self.active.is_some(),
            InteractionState::Disabled => self.disabled.is_some(),
            InteractionState::FocusedHovered => self.focus.is_some() || self.hover.is_some(),
        }
    }
}

impl Default for InteractiveStyle {
    fn default() -> Self {
        Self::new(Style::new())
    }
}

impl From<Style> for InteractiveStyle {
    fn from(style: Style) -> Self {
        Self::new(style)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ftui_render::cell::PackedRgba;

    const WHITE: PackedRgba = PackedRgba::WHITE;
    const BLACK: PackedRgba = PackedRgba::BLACK;
    const BLUE: PackedRgba = PackedRgba::BLUE;
    const RED: PackedRgba = PackedRgba::RED;
    const YELLOW: PackedRgba = PackedRgba::rgb(255, 255, 0);
    const GRAY: PackedRgba = PackedRgba::rgb(128, 128, 128);
    const DARK_GRAY: PackedRgba = PackedRgba::rgb(64, 64, 64);

    #[test]
    fn normal_returns_base_style() {
        let style = InteractiveStyle::new(Style::new().fg(WHITE));
        let resolved = style.resolve(InteractionState::Normal);
        assert_eq!(resolved.fg, Some(WHITE));
    }

    #[test]
    fn hover_patches_over_base() {
        let style =
            InteractiveStyle::new(Style::new().fg(WHITE).bg(BLACK)).hover(Style::new().bg(GRAY));
        let resolved = style.resolve(InteractionState::Hovered);
        assert_eq!(resolved.fg, Some(WHITE)); // inherited from base
        assert_eq!(resolved.bg, Some(GRAY)); // overridden by hover
    }

    #[test]
    fn hover_without_override_returns_base() {
        let style = InteractiveStyle::new(Style::new().fg(WHITE));
        let resolved = style.resolve(InteractionState::Hovered);
        assert_eq!(resolved.fg, Some(WHITE));
    }

    #[test]
    fn focus_patches_over_base() {
        let style = InteractiveStyle::new(Style::new().fg(WHITE)).focused(Style::new().fg(BLUE));
        let resolved = style.resolve(InteractionState::Focused);
        assert_eq!(resolved.fg, Some(BLUE));
    }

    #[test]
    fn active_patches_over_base() {
        let style = InteractiveStyle::new(Style::new().fg(WHITE)).active(Style::new().fg(RED));
        let resolved = style.resolve(InteractionState::Active);
        assert_eq!(resolved.fg, Some(RED));
    }

    #[test]
    fn disabled_patches_over_base() {
        let style =
            InteractiveStyle::new(Style::new().fg(WHITE)).disabled(Style::new().fg(DARK_GRAY));
        let resolved = style.resolve(InteractionState::Disabled);
        assert_eq!(resolved.fg, Some(DARK_GRAY));
    }

    #[test]
    fn focused_hovered_applies_both() {
        let style = InteractiveStyle::new(Style::new().fg(WHITE).bg(BLACK))
            .focused(Style::new().fg(BLUE))
            .hover(Style::new().bg(GRAY));
        let resolved = style.resolve(InteractionState::FocusedHovered);
        assert_eq!(resolved.bg, Some(GRAY)); // hover patches last
        assert_eq!(resolved.fg, Some(BLUE)); // focus sets fg, hover doesn't override
    }

    #[test]
    fn focused_hovered_hover_overrides_focus_on_conflict() {
        let style = InteractiveStyle::new(Style::new())
            .focused(Style::new().fg(BLUE))
            .hover(Style::new().fg(RED));
        let resolved = style.resolve(InteractionState::FocusedHovered);
        assert_eq!(resolved.fg, Some(RED)); // hover applied last
    }

    #[test]
    fn has_override_reports_correctly() {
        let style = InteractiveStyle::new(Style::new()).hover(Style::new().fg(RED));
        assert!(style.has_override(InteractionState::Normal));
        assert!(style.has_override(InteractionState::Hovered));
        assert!(!style.has_override(InteractionState::Focused));
        assert!(!style.has_override(InteractionState::Active));
        assert!(!style.has_override(InteractionState::Disabled));
        assert!(style.has_override(InteractionState::FocusedHovered)); // hover exists
    }

    #[test]
    fn default_has_no_overrides() {
        let style = InteractiveStyle::default();
        assert!(!style.has_override(InteractionState::Hovered));
        assert!(!style.has_override(InteractionState::Focused));
        assert!(!style.has_override(InteractionState::Active));
        assert!(!style.has_override(InteractionState::Disabled));
    }

    #[test]
    fn from_style_creates_normal_only() {
        let style: InteractiveStyle = Style::new().fg(WHITE).into();
        assert_eq!(style.normal.fg, Some(WHITE));
        assert!(style.hover.is_none());
        assert!(style.focus.is_none());
    }

    #[test]
    fn all_states_set() {
        let style = InteractiveStyle::new(Style::new().fg(WHITE))
            .hover(Style::new().fg(YELLOW))
            .focused(Style::new().fg(BLUE))
            .active(Style::new().fg(RED))
            .disabled(Style::new().fg(DARK_GRAY));

        assert_eq!(style.resolve(InteractionState::Normal).fg, Some(WHITE));
        assert_eq!(style.resolve(InteractionState::Hovered).fg, Some(YELLOW));
        assert_eq!(style.resolve(InteractionState::Focused).fg, Some(BLUE));
        assert_eq!(style.resolve(InteractionState::Active).fg, Some(RED));
        assert_eq!(
            style.resolve(InteractionState::Disabled).fg,
            Some(DARK_GRAY)
        );
    }

    #[test]
    fn debug_impl_works() {
        let style = InteractiveStyle::default();
        let _ = format!("{style:?}");
    }

    #[test]
    fn interaction_state_eq_and_clone() {
        let state = InteractionState::Hovered;
        let cloned = state;
        assert_eq!(state, cloned);
    }
}
