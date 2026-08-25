//! Typestate encoding for terminal mode transitions.
//!
//! Maps terminal mode states and transitions to Rust's type system so that
//! invalid transitions become compile-time errors. Uses const-generic bit
//! flags for composite states (multiple modes active simultaneously).
//!
//! # State Machine
//!
//! ```text
//!          enter_raw()
//! Cooked ─────────────► Raw
//!   ▲                     │
//!   │ exit_raw()          │ enter_alt_screen()
//!   │                     ▼
//!   │                  AltScreen
//!   │                  ┌──────┐
//!   │                  │ Optional: mouse_capture()
//!   │                  │ Optional: bracketed_paste()
//!   │                  │ Optional: focus_events()
//!   │                  └──────┘
//!   │                     │
//!   └─────────────────────┘
//!         teardown()
//! ```
//!
//! # Design
//!
//! Uses a sealed `ModeFlags` const generic to encode which modes are active:
//!
//! ```
//! use ftui_core::mode_typestate::*;
//!
//! // Type-safe mode transitions:
//! let cooked = TerminalMode::<COOKED>::new();
//! let raw = cooked.enter_raw();
//! let alt = raw.enter_alt_screen();
//! let with_mouse = alt.enable_mouse();
//!
//! // This would NOT compile:
//! // let invalid = cooked.enter_alt_screen(); // error: no method on TerminalMode<COOKED>
//!
//! // Build composite state:
//! assert!(with_mouse.has_raw());
//! assert!(with_mouse.has_alt_screen());
//! assert!(with_mouse.has_mouse());
//! ```

use std::fmt;
use std::marker::PhantomData;

// ── Flag constants ──────────────────────────────────────────────────

/// No modes active (cooked terminal).
pub const COOKED: u8 = 0;

/// Raw mode enabled (no line buffering, no echo).
pub const RAW: u8 = 1 << 0;

/// Alternate screen buffer active.
pub const ALT_SCREEN: u8 = 1 << 1;

/// Mouse capture enabled (SGR mode).
pub const MOUSE: u8 = 1 << 2;

/// Bracketed paste mode enabled.
pub const BRACKETED_PASTE: u8 = 1 << 3;

/// Focus event reporting enabled.
pub const FOCUS_EVENTS: u8 = 1 << 4;

/// Full TUI setup: raw + alt screen.
pub const TUI_BASE: u8 = RAW | ALT_SCREEN;

/// Full TUI with all features.
pub const TUI_FULL: u8 = RAW | ALT_SCREEN | MOUSE | BRACKETED_PASTE | FOCUS_EVENTS;

// ── Core type ───────────────────────────────────────────────────────

/// A terminal mode state encoded as a const-generic bit flag.
///
/// The type parameter `FLAGS` encodes which modes are currently active.
/// Methods are only available when the prerequisite modes are set,
/// making invalid transitions a compile-time error.
pub struct TerminalMode<const FLAGS: u8> {
    _phantom: PhantomData<()>,
}

impl<const FLAGS: u8> TerminalMode<FLAGS> {
    /// Query whether raw mode is active.
    pub const fn has_raw(&self) -> bool {
        FLAGS & RAW != 0
    }

    /// Query whether alternate screen is active.
    pub const fn has_alt_screen(&self) -> bool {
        FLAGS & ALT_SCREEN != 0
    }

    /// Query whether mouse capture is active.
    pub const fn has_mouse(&self) -> bool {
        FLAGS & MOUSE != 0
    }

    /// Query whether bracketed paste is active.
    pub const fn has_bracketed_paste(&self) -> bool {
        FLAGS & BRACKETED_PASTE != 0
    }

    /// Query whether focus event reporting is active.
    pub const fn has_focus_events(&self) -> bool {
        FLAGS & FOCUS_EVENTS != 0
    }

    /// Get the raw flags value.
    pub const fn flags(&self) -> u8 {
        FLAGS
    }
}

impl<const FLAGS: u8> fmt::Debug for TerminalMode<FLAGS> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut modes = Vec::new();
        if FLAGS & RAW != 0 {
            modes.push("Raw");
        }
        if FLAGS & ALT_SCREEN != 0 {
            modes.push("AltScreen");
        }
        if FLAGS & MOUSE != 0 {
            modes.push("Mouse");
        }
        if FLAGS & BRACKETED_PASTE != 0 {
            modes.push("BracketedPaste");
        }
        if FLAGS & FOCUS_EVENTS != 0 {
            modes.push("FocusEvents");
        }
        if modes.is_empty() {
            write!(f, "TerminalMode<Cooked>")
        } else {
            write!(f, "TerminalMode<{}>", modes.join("+"))
        }
    }
}

impl<const FLAGS: u8> Clone for TerminalMode<FLAGS> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<const FLAGS: u8> Copy for TerminalMode<FLAGS> {}

// ── Cooked → Raw ────────────────────────────────────────────────────

impl TerminalMode<COOKED> {
    /// Create a new terminal in cooked mode.
    pub fn new() -> Self {
        Self {
            _phantom: PhantomData,
        }
    }

    /// Enter raw mode (no line buffering, no echo).
    pub fn enter_raw(self) -> TerminalMode<RAW> {
        TerminalMode {
            _phantom: PhantomData,
        }
    }
}

impl Default for TerminalMode<COOKED> {
    fn default() -> Self {
        Self::new()
    }
}

// ── Raw mode transitions ────────────────────────────────────────────

impl TerminalMode<RAW> {
    /// Exit raw mode back to cooked.
    pub fn exit_raw(self) -> TerminalMode<COOKED> {
        TerminalMode {
            _phantom: PhantomData,
        }
    }

    /// Enter alternate screen (requires raw mode).
    pub fn enter_alt_screen(self) -> TerminalMode<{ RAW | ALT_SCREEN }> {
        TerminalMode {
            _phantom: PhantomData,
        }
    }
}

// ── Alt screen transitions ──────────────────────────────────────────
// These are available when both RAW and ALT_SCREEN are set.

impl TerminalMode<{ RAW | ALT_SCREEN }> {
    /// Enable mouse capture.
    pub fn enable_mouse(self) -> TerminalMode<{ RAW | ALT_SCREEN | MOUSE }> {
        TerminalMode {
            _phantom: PhantomData,
        }
    }

    /// Enable bracketed paste.
    pub fn enable_bracketed_paste(self) -> TerminalMode<{ RAW | ALT_SCREEN | BRACKETED_PASTE }> {
        TerminalMode {
            _phantom: PhantomData,
        }
    }

    /// Enable focus event reporting.
    pub fn enable_focus_events(self) -> TerminalMode<{ RAW | ALT_SCREEN | FOCUS_EVENTS }> {
        TerminalMode {
            _phantom: PhantomData,
        }
    }

    /// Exit alternate screen back to raw mode.
    pub fn exit_alt_screen(self) -> TerminalMode<RAW> {
        TerminalMode {
            _phantom: PhantomData,
        }
    }

    /// Full teardown back to cooked mode.
    pub fn teardown(self) -> TerminalMode<COOKED> {
        TerminalMode {
            _phantom: PhantomData,
        }
    }
}

// ── With mouse ──────────────────────────────────────────────────────

impl TerminalMode<{ RAW | ALT_SCREEN | MOUSE }> {
    /// Disable mouse capture.
    pub fn disable_mouse(self) -> TerminalMode<{ RAW | ALT_SCREEN }> {
        TerminalMode {
            _phantom: PhantomData,
        }
    }

    /// Enable bracketed paste (adding to existing modes).
    pub fn enable_bracketed_paste(
        self,
    ) -> TerminalMode<{ RAW | ALT_SCREEN | MOUSE | BRACKETED_PASTE }> {
        TerminalMode {
            _phantom: PhantomData,
        }
    }

    /// Enable focus events (adding to existing modes).
    pub fn enable_focus_events(self) -> TerminalMode<{ RAW | ALT_SCREEN | MOUSE | FOCUS_EVENTS }> {
        TerminalMode {
            _phantom: PhantomData,
        }
    }

    /// Full teardown back to cooked mode.
    pub fn teardown(self) -> TerminalMode<COOKED> {
        TerminalMode {
            _phantom: PhantomData,
        }
    }
}

// ── With bracketed paste ────────────────────────────────────────────

impl TerminalMode<{ RAW | ALT_SCREEN | BRACKETED_PASTE }> {
    /// Disable bracketed paste.
    pub fn disable_bracketed_paste(self) -> TerminalMode<{ RAW | ALT_SCREEN }> {
        TerminalMode {
            _phantom: PhantomData,
        }
    }

    /// Enable mouse capture.
    pub fn enable_mouse(self) -> TerminalMode<{ RAW | ALT_SCREEN | MOUSE | BRACKETED_PASTE }> {
        TerminalMode {
            _phantom: PhantomData,
        }
    }

    /// Full teardown back to cooked mode.
    pub fn teardown(self) -> TerminalMode<COOKED> {
        TerminalMode {
            _phantom: PhantomData,
        }
    }
}

// ── With mouse + bracketed paste ────────────────────────────────────

impl TerminalMode<{ RAW | ALT_SCREEN | MOUSE | BRACKETED_PASTE }> {
    /// Disable mouse.
    pub fn disable_mouse(self) -> TerminalMode<{ RAW | ALT_SCREEN | BRACKETED_PASTE }> {
        TerminalMode {
            _phantom: PhantomData,
        }
    }

    /// Disable bracketed paste.
    pub fn disable_bracketed_paste(self) -> TerminalMode<{ RAW | ALT_SCREEN | MOUSE }> {
        TerminalMode {
            _phantom: PhantomData,
        }
    }

    /// Enable focus events.
    pub fn enable_focus_events(
        self,
    ) -> TerminalMode<{ RAW | ALT_SCREEN | MOUSE | BRACKETED_PASTE | FOCUS_EVENTS }> {
        TerminalMode {
            _phantom: PhantomData,
        }
    }

    /// Full teardown back to cooked mode.
    pub fn teardown(self) -> TerminalMode<COOKED> {
        TerminalMode {
            _phantom: PhantomData,
        }
    }
}

// ── Full TUI mode ───────────────────────────────────────────────────

impl TerminalMode<TUI_FULL> {
    /// Full teardown back to cooked mode.
    pub fn teardown(self) -> TerminalMode<COOKED> {
        TerminalMode {
            _phantom: PhantomData,
        }
    }
}

// ── Builder for ergonomic setup ─────────────────────────────────────

/// Builder for constructing a terminal mode configuration.
///
/// Provides a runtime-checked alternative to the typestate pattern
/// for cases where the mode set isn't known at compile time.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ModeBuilder {
    flags: u8,
}

impl ModeBuilder {
    /// Start building from cooked mode.
    pub fn new() -> Self {
        Self { flags: COOKED }
    }

    /// Add raw mode.
    pub fn raw(mut self) -> Self {
        self.flags |= RAW;
        self
    }

    /// Add alternate screen (requires raw).
    pub fn alt_screen(mut self) -> Self {
        assert!(self.flags & RAW != 0, "alternate screen requires raw mode");
        self.flags |= ALT_SCREEN;
        self
    }

    /// Add mouse capture (requires alt screen).
    pub fn mouse(mut self) -> Self {
        assert!(
            self.flags & ALT_SCREEN != 0,
            "mouse capture requires alternate screen"
        );
        self.flags |= MOUSE;
        self
    }

    /// Add bracketed paste (requires alt screen).
    pub fn bracketed_paste(mut self) -> Self {
        assert!(
            self.flags & ALT_SCREEN != 0,
            "bracketed paste requires alternate screen"
        );
        self.flags |= BRACKETED_PASTE;
        self
    }

    /// Add focus events (requires alt screen).
    pub fn focus_events(mut self) -> Self {
        assert!(
            self.flags & ALT_SCREEN != 0,
            "focus events requires alternate screen"
        );
        self.flags |= FOCUS_EVENTS;
        self
    }

    /// Get the resulting flags.
    pub fn flags(self) -> u8 {
        self.flags
    }

    /// Check if a specific flag is set.
    pub fn has(self, flag: u8) -> bool {
        self.flags & flag != 0
    }
}

impl Default for ModeBuilder {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── Basic transitions ───────────────────────────────────────────

    #[test]
    fn cooked_to_raw() {
        let cooked = TerminalMode::<COOKED>::new();
        assert!(!cooked.has_raw());
        let raw = cooked.enter_raw();
        assert!(raw.has_raw());
    }

    #[test]
    fn raw_to_cooked() {
        let raw = TerminalMode::<COOKED>::new().enter_raw();
        let cooked = raw.exit_raw();
        assert!(!cooked.has_raw());
    }

    #[test]
    fn raw_to_alt_screen() {
        let raw = TerminalMode::<COOKED>::new().enter_raw();
        let alt = raw.enter_alt_screen();
        assert!(alt.has_raw());
        assert!(alt.has_alt_screen());
    }

    #[test]
    fn alt_screen_to_raw() {
        let alt = TerminalMode::<COOKED>::new().enter_raw().enter_alt_screen();
        let raw = alt.exit_alt_screen();
        assert!(raw.has_raw());
        assert!(!raw.has_alt_screen());
    }

    // ── Optional modes ──────────────────────────────────────────────

    #[test]
    fn enable_mouse() {
        let mode = TerminalMode::<COOKED>::new()
            .enter_raw()
            .enter_alt_screen()
            .enable_mouse();
        assert!(mode.has_mouse());
        assert!(mode.has_raw());
        assert!(mode.has_alt_screen());
    }

    #[test]
    fn disable_mouse() {
        let with_mouse = TerminalMode::<COOKED>::new()
            .enter_raw()
            .enter_alt_screen()
            .enable_mouse();
        let without = with_mouse.disable_mouse();
        assert!(!without.has_mouse());
        assert!(without.has_alt_screen());
    }

    #[test]
    fn enable_bracketed_paste() {
        let mode = TerminalMode::<COOKED>::new()
            .enter_raw()
            .enter_alt_screen()
            .enable_bracketed_paste();
        assert!(mode.has_bracketed_paste());
    }

    #[test]
    fn composite_mouse_and_paste() {
        let mode = TerminalMode::<COOKED>::new()
            .enter_raw()
            .enter_alt_screen()
            .enable_mouse()
            .enable_bracketed_paste();
        assert!(mode.has_mouse());
        assert!(mode.has_bracketed_paste());
        assert!(mode.has_raw());
        assert!(mode.has_alt_screen());
    }

    // ── Teardown ────────────────────────────────────────────────────

    #[test]
    fn teardown_from_alt_screen() {
        let mode = TerminalMode::<COOKED>::new().enter_raw().enter_alt_screen();
        let cooked = mode.teardown();
        assert!(!cooked.has_raw());
        assert!(!cooked.has_alt_screen());
    }

    #[test]
    fn teardown_from_full_mode() {
        let mode = TerminalMode::<COOKED>::new()
            .enter_raw()
            .enter_alt_screen()
            .enable_mouse()
            .enable_bracketed_paste();
        let cooked = mode.teardown();
        assert!(!cooked.has_raw());
    }

    // ── Flag queries ────────────────────────────────────────────────

    #[test]
    fn flags_value() {
        let mode = TerminalMode::<COOKED>::new();
        assert_eq!(mode.flags(), COOKED);

        let raw = mode.enter_raw();
        assert_eq!(raw.flags(), RAW);

        let alt = raw.enter_alt_screen();
        assert_eq!(alt.flags(), RAW | ALT_SCREEN);
    }

    #[test]
    fn tui_base_constant() {
        assert_eq!(TUI_BASE, RAW | ALT_SCREEN);
    }

    #[test]
    fn tui_full_constant() {
        assert_eq!(
            TUI_FULL,
            RAW | ALT_SCREEN | MOUSE | BRACKETED_PASTE | FOCUS_EVENTS
        );
    }

    // ── Debug formatting ────────────────────────────────────────────

    #[test]
    fn debug_cooked() {
        let mode = TerminalMode::<COOKED>::new();
        assert_eq!(format!("{mode:?}"), "TerminalMode<Cooked>");
    }

    #[test]
    fn debug_raw() {
        let mode = TerminalMode::<COOKED>::new().enter_raw();
        assert_eq!(format!("{mode:?}"), "TerminalMode<Raw>");
    }

    #[test]
    fn debug_composite() {
        let mode = TerminalMode::<COOKED>::new()
            .enter_raw()
            .enter_alt_screen()
            .enable_mouse();
        let debug = format!("{mode:?}");
        assert!(debug.contains("Raw"));
        assert!(debug.contains("AltScreen"));
        assert!(debug.contains("Mouse"));
    }

    // ── Copy/Clone ──────────────────────────────────────────────────

    #[test]
    fn mode_is_copy() {
        let mode = TerminalMode::<COOKED>::new().enter_raw();
        let copy = mode;
        assert_eq!(copy.flags(), mode.flags());
    }

    // ── Builder ─────────────────────────────────────────────────────

    #[test]
    fn builder_basic() {
        let flags = ModeBuilder::new().raw().alt_screen().mouse().flags();
        assert_eq!(flags, RAW | ALT_SCREEN | MOUSE);
    }

    #[test]
    fn builder_has() {
        let b = ModeBuilder::new().raw().alt_screen();
        assert!(b.has(RAW));
        assert!(b.has(ALT_SCREEN));
        assert!(!b.has(MOUSE));
    }

    #[test]
    #[should_panic(expected = "alternate screen requires raw mode")]
    fn builder_enforces_raw_before_alt() {
        ModeBuilder::new().alt_screen();
    }

    #[test]
    #[should_panic(expected = "mouse capture requires alternate screen")]
    fn builder_enforces_alt_before_mouse() {
        ModeBuilder::new().raw().mouse();
    }

    // ── Compile-time safety verification ────────────────────────────

    // These tests verify that invalid transitions are caught at compile time.
    // Uncomment any line below to verify it produces a compile error:

    // fn compile_error_cooked_to_alt() {
    //     TerminalMode::<COOKED>::new().enter_alt_screen(); // ERROR: no method
    // }

    // fn compile_error_cooked_to_mouse() {
    //     TerminalMode::<COOKED>::new().enable_mouse(); // ERROR: no method
    // }

    // fn compile_error_raw_to_mouse() {
    //     TerminalMode::<COOKED>::new().enter_raw().enable_mouse(); // ERROR: no method
    // }
}
