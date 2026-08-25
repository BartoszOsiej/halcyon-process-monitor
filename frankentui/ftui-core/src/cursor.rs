#![forbid(unsafe_code)]

//! Cursor save/restore strategy for inline mode robustness.
//!
//! This module implements a layered cursor save/restore strategy to handle
//! the variety of terminal behaviors. Inline mode requires saving cursor
//! position before drawing UI and restoring after.
//!
//! # Strategy Layers
//!
//! 1. **DEC (preferred)**: `ESC 7` / `ESC 8` (DECSC/DECRC)
//!    - Most widely supported on modern terminals
//!    - Saves cursor position, attributes, and charset
//!    - Works in tmux/screen with passthrough
//!
//! 2. **ANSI (fallback)**: `CSI s` / `CSI u`
//!    - Alternative when DEC has issues
//!    - Only saves cursor position (not attributes)
//!    - May conflict with some terminal modes
//!
//! 3. **Emulated (last resort)**: Track position and use `CSI row;col H`
//!    - Works everywhere that supports CUP
//!    - Requires tracking cursor position throughout
//!    - More overhead but guaranteed to work
//!
//! # Example
//!
//! ```
//! use ftui_core::cursor::{CursorManager, CursorSaveStrategy};
//! use ftui_core::terminal_capabilities::TerminalCapabilities;
//!
//! let caps = TerminalCapabilities::detect();
//! let mut cursor = CursorManager::new(CursorSaveStrategy::detect(&caps));
//!
//! // In your render loop:
//! let mut output = Vec::new();
//! cursor.save(&mut output, (10, 5))?;  // Save at column 10, row 5
//! // ... draw UI ...
//! cursor.restore(&mut output)?;
//! # Ok::<(), std::io::Error>(())
//! ```

use std::io::{self, Write};

use crate::terminal_capabilities::TerminalCapabilities;

/// DEC cursor save (DECSC): `ESC 7`
///
/// Saves cursor position, character attributes, character set, and origin mode.
const DEC_SAVE: &[u8] = b"\x1b7";

/// DEC cursor restore (DECRC): `ESC 8`
///
/// Restores cursor position and attributes saved by DECSC.
const DEC_RESTORE: &[u8] = b"\x1b8";

/// ANSI cursor save: `CSI s`
///
/// Saves cursor position only (not attributes).
const ANSI_SAVE: &[u8] = b"\x1b[s";

/// ANSI cursor restore: `CSI u`
///
/// Restores cursor position saved by `CSI s`.
const ANSI_RESTORE: &[u8] = b"\x1b[u";

/// Strategy for cursor save/restore operations.
///
/// Different terminals support different cursor save/restore mechanisms.
/// This enum allows selecting the appropriate strategy based on terminal
/// capabilities.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CursorSaveStrategy {
    /// DEC save/restore (`ESC 7` / `ESC 8`).
    ///
    /// The preferred strategy for most terminals. Saves cursor position,
    /// attributes, and character set.
    #[default]
    Dec,

    /// ANSI save/restore (`CSI s` / `CSI u`).
    ///
    /// Fallback for terminals where DEC sequences have issues.
    /// Only saves cursor position, not attributes.
    Ansi,

    /// Emulated save/restore using position tracking and CUP.
    ///
    /// Last resort that works on any terminal supporting cursor positioning.
    /// Requires the caller to provide current position when saving.
    Emulated,
}

impl CursorSaveStrategy {
    /// Detect the best strategy for the current environment.
    ///
    /// Uses terminal capabilities to choose the most reliable strategy.
    #[must_use]
    pub fn detect(caps: &TerminalCapabilities) -> Self {
        // GNU screen has quirks with DEC save/restore in some configurations
        if caps.in_screen {
            return Self::Ansi;
        }

        // Most modern terminals support DEC sequences well
        // tmux, zellij, and direct terminal all work with DEC
        Self::Dec
    }

    /// Get the save escape sequence for this strategy.
    ///
    /// Returns `None` for `Emulated` strategy (no escape sequence needed).
    #[must_use]
    pub const fn save_sequence(&self) -> Option<&'static [u8]> {
        match self {
            Self::Dec => Some(DEC_SAVE),
            Self::Ansi => Some(ANSI_SAVE),
            Self::Emulated => None,
        }
    }

    /// Get the restore escape sequence for this strategy.
    ///
    /// Returns `None` for `Emulated` strategy (uses CUP instead).
    #[must_use]
    pub const fn restore_sequence(&self) -> Option<&'static [u8]> {
        match self {
            Self::Dec => Some(DEC_RESTORE),
            Self::Ansi => Some(ANSI_RESTORE),
            Self::Emulated => None,
        }
    }
}

/// Manages cursor save/restore operations.
///
/// This struct handles the complexity of cursor save/restore across different
/// strategies. It tracks the saved position for emulated mode and provides
/// a unified interface regardless of the underlying mechanism.
///
/// # Contract
///
/// - `save()` must be called before `restore()`
/// - Calling `restore()` without a prior `save()` is safe but may have no effect
/// - Multiple `save()` calls overwrite the previous save (no nesting)
#[derive(Debug, Clone)]
pub struct CursorManager {
    strategy: CursorSaveStrategy,
    /// Saved cursor position for emulated mode: (column, row), 0-indexed.
    saved_position: Option<(u16, u16)>,
}

impl CursorManager {
    /// Create a new cursor manager with the specified strategy.
    #[must_use]
    pub const fn new(strategy: CursorSaveStrategy) -> Self {
        Self {
            strategy,
            saved_position: None,
        }
    }

    /// Create a cursor manager with auto-detected strategy.
    #[must_use]
    pub fn detect(caps: &TerminalCapabilities) -> Self {
        Self::new(CursorSaveStrategy::detect(caps))
    }

    /// Get the current strategy.
    #[must_use]
    pub const fn strategy(&self) -> CursorSaveStrategy {
        self.strategy
    }

    /// Save the cursor position.
    ///
    /// # Arguments
    ///
    /// * `writer` - The output writer (typically stdout)
    /// * `current_pos` - Current cursor position (column, row), 0-indexed.
    ///   Required for emulated mode, ignored for DEC/ANSI modes.
    ///
    /// # Errors
    ///
    /// Returns an error if writing to the output fails.
    pub fn save<W: Write>(&mut self, writer: &mut W, current_pos: (u16, u16)) -> io::Result<()> {
        match self.strategy {
            CursorSaveStrategy::Dec => writer.write_all(DEC_SAVE),
            CursorSaveStrategy::Ansi => writer.write_all(ANSI_SAVE),
            CursorSaveStrategy::Emulated => {
                self.saved_position = Some(current_pos);
                Ok(())
            }
        }
    }

    /// Restore the cursor position.
    ///
    /// # Errors
    ///
    /// Returns an error if writing to the output fails.
    /// For emulated mode, does nothing if no position was saved.
    pub fn restore<W: Write>(&self, writer: &mut W) -> io::Result<()> {
        match self.strategy {
            CursorSaveStrategy::Dec => writer.write_all(DEC_RESTORE),
            CursorSaveStrategy::Ansi => writer.write_all(ANSI_RESTORE),
            CursorSaveStrategy::Emulated => {
                if let Some((col, row)) = self.saved_position {
                    // CUP uses 1-indexed coordinates
                    write!(writer, "\x1b[{};{}H", (row as u32) + 1, (col as u32) + 1)
                } else {
                    Ok(())
                }
            }
        }
    }

    /// Clear the saved position (for emulated mode).
    ///
    /// This has no effect on DEC/ANSI modes.
    pub fn clear(&mut self) {
        self.saved_position = None;
    }

    /// Get the saved position (for emulated mode).
    ///
    /// Returns `None` for DEC/ANSI modes or if no position was saved.
    #[must_use]
    pub const fn saved_position(&self) -> Option<(u16, u16)> {
        self.saved_position
    }
}

impl Default for CursorManager {
    fn default() -> Self {
        Self::new(CursorSaveStrategy::default())
    }
}

/// Move cursor to a specific position.
///
/// Writes a CUP (Cursor Position) sequence to move the cursor.
///
/// # Arguments
///
/// * `writer` - The output writer
/// * `col` - Column (0-indexed)
/// * `row` - Row (0-indexed)
///
/// # Errors
///
/// Returns an error if writing to the output fails.
pub fn move_to<W: Write>(writer: &mut W, col: u16, row: u16) -> io::Result<()> {
    // CUP uses 1-indexed coordinates
    write!(writer, "\x1b[{};{}H", (row as u32) + 1, (col as u32) + 1)
}

/// Hide the cursor.
///
/// Writes `CSI ? 25 l` to hide the cursor.
pub fn hide<W: Write>(writer: &mut W) -> io::Result<()> {
    writer.write_all(b"\x1b[?25l")
}

/// Show the cursor.
///
/// Writes `CSI ? 25 h` to show the cursor.
pub fn show<W: Write>(writer: &mut W) -> io::Result<()> {
    writer.write_all(b"\x1b[?25h")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dec_save_restore_sequences() {
        let strategy = CursorSaveStrategy::Dec;
        assert_eq!(strategy.save_sequence(), Some(b"\x1b7".as_slice()));
        assert_eq!(strategy.restore_sequence(), Some(b"\x1b8".as_slice()));
    }

    #[test]
    fn ansi_save_restore_sequences() {
        let strategy = CursorSaveStrategy::Ansi;
        assert_eq!(strategy.save_sequence(), Some(b"\x1b[s".as_slice()));
        assert_eq!(strategy.restore_sequence(), Some(b"\x1b[u".as_slice()));
    }

    #[test]
    fn emulated_has_no_sequences() {
        let strategy = CursorSaveStrategy::Emulated;
        assert_eq!(strategy.save_sequence(), None);
        assert_eq!(strategy.restore_sequence(), None);
    }

    #[test]
    fn detect_uses_dec_for_normal_terminal() {
        let caps = TerminalCapabilities::basic();
        let strategy = CursorSaveStrategy::detect(&caps);
        assert_eq!(strategy, CursorSaveStrategy::Dec);
    }

    #[test]
    fn detect_uses_ansi_for_screen() {
        let mut caps = TerminalCapabilities::basic();
        caps.in_screen = true;
        let strategy = CursorSaveStrategy::detect(&caps);
        assert_eq!(strategy, CursorSaveStrategy::Ansi);
    }

    #[test]
    fn detect_uses_dec_for_tmux() {
        let mut caps = TerminalCapabilities::basic();
        caps.in_tmux = true;
        let strategy = CursorSaveStrategy::detect(&caps);
        assert_eq!(strategy, CursorSaveStrategy::Dec);
    }

    #[test]
    fn cursor_manager_dec_save() {
        let mut manager = CursorManager::new(CursorSaveStrategy::Dec);
        let mut output = Vec::new();

        manager.save(&mut output, (10, 5)).unwrap();
        assert_eq!(output, b"\x1b7");
    }

    #[test]
    fn cursor_manager_dec_restore() {
        let manager = CursorManager::new(CursorSaveStrategy::Dec);
        let mut output = Vec::new();

        manager.restore(&mut output).unwrap();
        assert_eq!(output, b"\x1b8");
    }

    #[test]
    fn cursor_manager_ansi_save_restore() {
        let mut manager = CursorManager::new(CursorSaveStrategy::Ansi);
        let mut output = Vec::new();

        manager.save(&mut output, (0, 0)).unwrap();
        assert_eq!(output, b"\x1b[s");

        output.clear();
        manager.restore(&mut output).unwrap();
        assert_eq!(output, b"\x1b[u");
    }

    #[test]
    fn cursor_manager_emulated_save_restore() {
        let mut manager = CursorManager::new(CursorSaveStrategy::Emulated);
        let mut output = Vec::new();

        // Save at column 10, row 5 (0-indexed)
        manager.save(&mut output, (10, 5)).unwrap();
        assert!(output.is_empty()); // No output for save
        assert_eq!(manager.saved_position(), Some((10, 5)));

        // Restore outputs CUP with 1-indexed coordinates
        manager.restore(&mut output).unwrap();
        assert_eq!(output, b"\x1b[6;11H"); // row=6, col=11 (1-indexed)
    }

    #[test]
    fn cursor_manager_emulated_restore_without_save() {
        let manager = CursorManager::new(CursorSaveStrategy::Emulated);
        let mut output = Vec::new();

        // Restore without save does nothing
        manager.restore(&mut output).unwrap();
        assert!(output.is_empty());
    }

    #[test]
    fn cursor_manager_clear() {
        let mut manager = CursorManager::new(CursorSaveStrategy::Emulated);
        let mut output = Vec::new();

        manager.save(&mut output, (5, 10)).unwrap();
        assert_eq!(manager.saved_position(), Some((5, 10)));

        manager.clear();
        assert_eq!(manager.saved_position(), None);
    }

    #[test]
    fn cursor_manager_default_uses_dec() {
        let manager = CursorManager::default();
        assert_eq!(manager.strategy(), CursorSaveStrategy::Dec);
    }

    #[test]
    fn move_to_outputs_cup() {
        let mut output = Vec::new();
        move_to(&mut output, 0, 0).unwrap();
        assert_eq!(output, b"\x1b[1;1H");

        output.clear();
        move_to(&mut output, 79, 23).unwrap();
        assert_eq!(output, b"\x1b[24;80H");
    }

    #[test]
    fn hide_and_show_cursor() {
        let mut output = Vec::new();

        hide(&mut output).unwrap();
        assert_eq!(output, b"\x1b[?25l");

        output.clear();
        show(&mut output).unwrap();
        assert_eq!(output, b"\x1b[?25h");
    }

    #[test]
    fn emulated_save_overwrites_previous_position() {
        let mut manager = CursorManager::new(CursorSaveStrategy::Emulated);
        let mut output = Vec::new();

        manager.save(&mut output, (1, 2)).unwrap();
        assert_eq!(manager.saved_position(), Some((1, 2)));

        manager.save(&mut output, (30, 40)).unwrap();
        assert_eq!(manager.saved_position(), Some((30, 40)));

        manager.restore(&mut output).unwrap();
        assert_eq!(output, b"\x1b[41;31H");
    }

    #[test]
    fn cursor_save_strategy_default_is_dec() {
        let strategy = CursorSaveStrategy::default();
        assert_eq!(strategy, CursorSaveStrategy::Dec);
    }

    #[test]
    fn cursor_manager_clone_preserves_saved_position() {
        let mut manager = CursorManager::new(CursorSaveStrategy::Emulated);
        let mut output = Vec::new();
        manager.save(&mut output, (7, 13)).unwrap();

        let cloned = manager.clone();
        assert_eq!(cloned.saved_position(), Some((7, 13)));
        assert_eq!(cloned.strategy(), CursorSaveStrategy::Emulated);
    }
}
