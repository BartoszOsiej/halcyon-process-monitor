//! Terminal emulation components for embedded terminal widgets.
//!
//! This module provides ANSI escape sequence parsing and terminal state management
//! for building terminal emulator widgets.
//!
//! # Modules
//!
//! - [`parser`] - ANSI escape sequence parser using the `vte` crate.
//! - [`state`] - Terminal state machine (grid, cursor, scrollback).
//! - [`widget`] - Terminal emulator widget (requires `terminal-widget` feature).

pub mod parser;
pub mod state;

#[cfg(feature = "terminal-widget")]
pub mod widget;

pub use parser::{AnsiHandler, AnsiParser};
pub use state::{
    Cell, CellAttrs, ClearRegion, Cursor, CursorShape, DirtyRegion, Grid, LineFlag, Pen,
    Scrollback, TerminalModes, TerminalState, WIDE_CONTINUATION,
};

#[cfg(feature = "terminal-widget")]
pub use widget::{TerminalEmulator, TerminalEmulatorState};
