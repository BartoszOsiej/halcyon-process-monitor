//! Input forwarding for PTY processes.
//!
//! Converts keyboard input events into ANSI escape sequences for terminal input.
//!
//! # Invariants
//!
//! 1. **UTF-8 validity**: All output sequences are valid UTF-8 or raw bytes.
//! 2. **Modifier precedence**: Ctrl > Alt > Shift in key transformation.
//! 3. **Bracketed paste**: Paste content is wrapped when mode is enabled.
//!
//! # Failure Modes
//!
//! | Failure | Cause | Behavior |
//! |---------|-------|----------|
//! | Invalid key | Unsupported key code | Returns empty sequence |
//! | Encoding error | Non-UTF8 char | Silently dropped |

use std::io::{self, Write};

/// Keyboard modifiers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Modifiers {
    /// Control key is pressed.
    pub ctrl: bool,
    /// Alt/Meta key is pressed.
    pub alt: bool,
    /// Shift key is pressed.
    pub shift: bool,
}

impl Modifiers {
    /// No modifiers.
    pub const NONE: Self = Self {
        ctrl: false,
        alt: false,
        shift: false,
    };

    /// Ctrl modifier only.
    pub const CTRL: Self = Self {
        ctrl: true,
        alt: false,
        shift: false,
    };

    /// Alt modifier only.
    pub const ALT: Self = Self {
        ctrl: false,
        alt: true,
        shift: false,
    };

    /// Shift modifier only.
    pub const SHIFT: Self = Self {
        ctrl: false,
        alt: false,
        shift: true,
    };

    /// Check if any modifier is active.
    #[must_use]
    pub const fn any(self) -> bool {
        self.ctrl || self.alt || self.shift
    }

    /// Get the CSI modifier parameter (1 + sum of modifier bits).
    /// Used for extended key sequences like `CSI 1;{mod} A`.
    #[must_use]
    pub fn csi_param(self) -> u8 {
        let mut param = 1u8;
        if self.shift {
            param += 1;
        }
        if self.alt {
            param += 2;
        }
        if self.ctrl {
            param += 4;
        }
        param
    }
}

/// Key codes for keyboard input.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Key {
    /// A printable character.
    Char(char),
    /// Function key (F1-F12).
    F(u8),
    /// Backspace key.
    Backspace,
    /// Enter/Return key.
    Enter,
    /// Tab key.
    Tab,
    /// Escape key.
    Escape,
    /// Up arrow.
    Up,
    /// Down arrow.
    Down,
    /// Left arrow.
    Left,
    /// Right arrow.
    Right,
    /// Home key.
    Home,
    /// End key.
    End,
    /// Page Up.
    PageUp,
    /// Page Down.
    PageDown,
    /// Insert key.
    Insert,
    /// Delete key.
    Delete,
}

/// A keyboard event with key and modifiers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KeyEvent {
    /// The key that was pressed.
    pub key: Key,
    /// Active modifiers.
    pub modifiers: Modifiers,
}

impl KeyEvent {
    /// Create a new key event.
    #[must_use]
    pub const fn new(key: Key, modifiers: Modifiers) -> Self {
        Self { key, modifiers }
    }

    /// Create a key event with no modifiers.
    #[must_use]
    pub const fn plain(key: Key) -> Self {
        Self {
            key,
            modifiers: Modifiers::NONE,
        }
    }
}

impl From<char> for KeyEvent {
    fn from(c: char) -> Self {
        Self::plain(Key::Char(c))
    }
}

/// Converts key events to ANSI escape sequences.
///
/// # Example
///
/// ```
/// use ftui_pty::input_forwarding::{KeyEvent, Key, Modifiers, key_to_sequence};
///
/// // Simple character
/// assert_eq!(key_to_sequence(KeyEvent::plain(Key::Char('a'))), b"a".to_vec());
///
/// // Ctrl+C
/// let event = KeyEvent::new(Key::Char('c'), Modifiers::CTRL);
/// assert_eq!(key_to_sequence(event), vec![0x03]); // ETX
///
/// // Up arrow
/// assert_eq!(key_to_sequence(KeyEvent::plain(Key::Up)), b"\x1b[A".to_vec());
/// ```
#[must_use]
pub fn key_to_sequence(event: KeyEvent) -> Vec<u8> {
    let KeyEvent { key, modifiers } = event;

    match key {
        Key::Char(c) => char_sequence(c, modifiers),
        Key::F(n) => function_key_sequence(n, modifiers),
        Key::Backspace => backspace_sequence(modifiers),
        Key::Enter => enter_sequence(modifiers),
        Key::Tab => tab_sequence(modifiers),
        Key::Escape => escape_sequence(modifiers),
        Key::Up => cursor_key_sequence(b'A', modifiers),
        Key::Down => cursor_key_sequence(b'B', modifiers),
        Key::Right => cursor_key_sequence(b'C', modifiers),
        Key::Left => cursor_key_sequence(b'D', modifiers),
        Key::Home => home_end_sequence(b'H', modifiers),
        Key::End => home_end_sequence(b'F', modifiers),
        Key::PageUp => page_key_sequence(5, modifiers),
        Key::PageDown => page_key_sequence(6, modifiers),
        Key::Insert => insert_delete_sequence(2, modifiers),
        Key::Delete => insert_delete_sequence(3, modifiers),
    }
}

/// Convert a character with modifiers to an escape sequence.
fn char_sequence(c: char, modifiers: Modifiers) -> Vec<u8> {
    // Handle Ctrl+<key> combinations
    if modifiers.ctrl
        && !modifiers.alt
        && let Some(ctrl_byte) = ctrl_char(c)
    {
        return vec![ctrl_byte];
    }

    // Handle Alt+<key> - prefix with ESC
    if modifiers.alt && !modifiers.ctrl {
        let mut seq = vec![0x1b]; // ESC
        let ch = if modifiers.shift {
            c.to_ascii_uppercase()
        } else {
            c
        };
        let mut buf = [0u8; 4];
        seq.extend_from_slice(ch.encode_utf8(&mut buf).as_bytes());
        return seq;
    }

    // Handle Ctrl+Alt combinations
    if modifiers.ctrl
        && modifiers.alt
        && let Some(ctrl_byte) = ctrl_char(c)
    {
        return vec![0x1b, ctrl_byte]; // ESC + Ctrl code
    }

    // Plain character (with optional shift)
    let ch = if modifiers.shift {
        c.to_ascii_uppercase()
    } else {
        c
    };
    let mut buf = [0u8; 4];
    ch.encode_utf8(&mut buf).as_bytes().to_vec()
}

/// Get the control character code for a letter (a-z, A-Z, some punctuation).
fn ctrl_char(c: char) -> Option<u8> {
    match c.to_ascii_lowercase() {
        'a'..='z' => Some(c.to_ascii_lowercase() as u8 - b'a' + 1),
        '[' | '3' => Some(0x1b),       // ESC
        '\\' | '4' => Some(0x1c),      // FS
        ']' | '5' => Some(0x1d),       // GS
        '^' | '6' => Some(0x1e),       // RS
        '_' | '7' => Some(0x1f),       // US
        '?' | '8' => Some(0x7f),       // DEL
        '@' | '2' | ' ' => Some(0x00), // NUL
        _ => None,
    }
}

/// Generate function key sequence (F1-F12).
fn function_key_sequence(n: u8, modifiers: Modifiers) -> Vec<u8> {
    // F1-F4 use ESC O P/Q/R/S (or ESC [ with modifiers)
    // F5-F12 use ESC [ <code> ~
    let (code, use_tilde) = match n {
        1 => (b'P', false),
        2 => (b'Q', false),
        3 => (b'R', false),
        4 => (b'S', false),
        5 => (15, true),
        6 => (17, true),
        7 => (18, true),
        8 => (19, true),
        9 => (20, true),
        10 => (21, true),
        11 => (23, true),
        12 => (24, true),
        _ => return Vec::new(),
    };

    if use_tilde {
        if modifiers.any() {
            // ESC [ <code> ; <mod> ~
            format!("\x1b[{};{}~", code, modifiers.csi_param()).into_bytes()
        } else {
            // ESC [ <code> ~
            format!("\x1b[{}~", code).into_bytes()
        }
    } else if modifiers.any() {
        // ESC [ 1 ; <mod> <code>
        format!("\x1b[1;{}{}", modifiers.csi_param(), code as char).into_bytes()
    } else {
        // ESC O <code>
        vec![0x1b, b'O', code]
    }
}

/// Generate backspace sequence.
fn backspace_sequence(modifiers: Modifiers) -> Vec<u8> {
    if modifiers.ctrl {
        vec![0x08] // BS (Ctrl+H behavior)
    } else if modifiers.alt {
        vec![0x1b, 0x7f] // ESC DEL
    } else {
        vec![0x7f] // DEL (standard backspace)
    }
}

/// Generate enter sequence.
fn enter_sequence(modifiers: Modifiers) -> Vec<u8> {
    if modifiers.alt {
        vec![0x1b, 0x0d] // ESC CR
    } else {
        vec![0x0d] // CR
    }
}

/// Generate tab sequence.
fn tab_sequence(modifiers: Modifiers) -> Vec<u8> {
    if modifiers.shift {
        b"\x1b[Z".to_vec() // CSI Z (backtab)
    } else if modifiers.alt {
        vec![0x1b, 0x09] // ESC TAB
    } else {
        vec![0x09] // TAB
    }
}

/// Generate escape sequence.
fn escape_sequence(modifiers: Modifiers) -> Vec<u8> {
    if modifiers.alt {
        vec![0x1b, 0x1b] // ESC ESC
    } else {
        vec![0x1b] // ESC
    }
}

/// Generate cursor key sequence (arrows).
fn cursor_key_sequence(code: u8, modifiers: Modifiers) -> Vec<u8> {
    if modifiers.any() {
        // ESC [ 1 ; <mod> <code>
        format!("\x1b[1;{}{}", modifiers.csi_param(), code as char).into_bytes()
    } else {
        // ESC [ <code>
        vec![0x1b, b'[', code]
    }
}

/// Generate Home/End sequence.
fn home_end_sequence(code: u8, modifiers: Modifiers) -> Vec<u8> {
    if modifiers.any() {
        // ESC [ 1 ; <mod> <code>
        format!("\x1b[1;{}{}", modifiers.csi_param(), code as char).into_bytes()
    } else {
        // ESC [ <code>
        vec![0x1b, b'[', code]
    }
}

/// Generate Page Up/Down sequence.
fn page_key_sequence(code: u8, modifiers: Modifiers) -> Vec<u8> {
    if modifiers.any() {
        // ESC [ <code> ; <mod> ~
        format!("\x1b[{};{}~", code, modifiers.csi_param()).into_bytes()
    } else {
        // ESC [ <code> ~
        format!("\x1b[{}~", code).into_bytes()
    }
}

/// Generate Insert/Delete sequence.
fn insert_delete_sequence(code: u8, modifiers: Modifiers) -> Vec<u8> {
    if modifiers.any() {
        // ESC [ <code> ; <mod> ~
        format!("\x1b[{};{}~", code, modifiers.csi_param()).into_bytes()
    } else {
        // ESC [ <code> ~
        format!("\x1b[{}~", code).into_bytes()
    }
}

/// Wrapper for bracketed paste.
///
/// In bracketed paste mode, pasted text is wrapped with start/end markers
/// so the terminal can distinguish it from typed input.
#[derive(Debug, Clone)]
pub struct BracketedPaste {
    enabled: bool,
}

impl Default for BracketedPaste {
    fn default() -> Self {
        Self::new()
    }
}

impl BracketedPaste {
    /// Start marker for bracketed paste.
    pub const START: &'static [u8] = b"\x1b[200~";
    /// End marker for bracketed paste.
    pub const END: &'static [u8] = b"\x1b[201~";

    /// Create a new bracketed paste handler (disabled by default).
    #[must_use]
    pub const fn new() -> Self {
        Self { enabled: false }
    }

    /// Check if bracketed paste mode is enabled.
    #[must_use]
    pub const fn is_enabled(&self) -> bool {
        self.enabled
    }

    /// Enable bracketed paste mode.
    pub fn enable(&mut self) {
        self.enabled = true;
    }

    /// Disable bracketed paste mode.
    pub fn disable(&mut self) {
        self.enabled = false;
    }

    /// Set bracketed paste mode.
    pub fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
    }

    /// Wrap text for paste, adding markers if enabled.
    ///
    /// Safety fallback:
    /// If the payload already contains bracketed-paste delimiters, we
    /// degrade to raw passthrough for this paste operation. This avoids
    /// delimiter-injection ambiguity (`ESC [ 200~` / `ESC [ 201~`) while
    /// preserving payload bytes exactly.
    #[must_use]
    pub fn wrap(&self, text: &[u8]) -> Vec<u8> {
        if self.enabled {
            if Self::contains_delimiter(text) {
                return text.to_vec();
            }
            let mut result = Vec::with_capacity(Self::START.len() + text.len() + Self::END.len());
            result.extend_from_slice(Self::START);
            result.extend_from_slice(text);
            result.extend_from_slice(Self::END);
            result
        } else {
            text.to_vec()
        }
    }

    #[must_use]
    fn contains_delimiter(text: &[u8]) -> bool {
        text.windows(Self::START.len())
            .any(|window| window == Self::START)
            || text
                .windows(Self::END.len())
                .any(|window| window == Self::END)
    }
}

/// Input forwarder that manages state and writes to a PTY.
pub struct InputForwarder<W: Write> {
    writer: W,
    bracketed_paste: BracketedPaste,
}

impl<W: Write> InputForwarder<W> {
    /// Create a new input forwarder.
    pub fn new(writer: W) -> Self {
        Self {
            writer,
            bracketed_paste: BracketedPaste::new(),
        }
    }

    /// Get a reference to the bracketed paste handler.
    #[must_use]
    pub const fn bracketed_paste(&self) -> &BracketedPaste {
        &self.bracketed_paste
    }

    /// Get a mutable reference to the bracketed paste handler.
    pub fn bracketed_paste_mut(&mut self) -> &mut BracketedPaste {
        &mut self.bracketed_paste
    }

    /// Set bracketed paste mode.
    pub fn set_bracketed_paste(&mut self, enabled: bool) {
        self.bracketed_paste.set_enabled(enabled);
    }

    /// Forward a key event to the PTY.
    pub fn forward_key(&mut self, event: KeyEvent) -> io::Result<()> {
        let seq = key_to_sequence(event);
        if !seq.is_empty() {
            self.writer.write_all(&seq)?;
            self.writer.flush()?;
        }
        Ok(())
    }

    /// Forward multiple key events.
    pub fn forward_keys(&mut self, events: &[KeyEvent]) -> io::Result<()> {
        for event in events {
            let seq = key_to_sequence(*event);
            if !seq.is_empty() {
                self.writer.write_all(&seq)?;
            }
        }
        self.writer.flush()
    }

    /// Forward raw bytes to the PTY.
    pub fn forward_raw(&mut self, data: &[u8]) -> io::Result<()> {
        self.writer.write_all(data)?;
        self.writer.flush()
    }

    /// Forward text as a paste (with bracketing if enabled).
    pub fn forward_paste(&mut self, text: &str) -> io::Result<()> {
        let data = self.bracketed_paste.wrap(text.as_bytes());
        self.writer.write_all(&data)?;
        self.writer.flush()
    }

    /// Get mutable access to the underlying writer.
    pub fn writer_mut(&mut self) -> &mut W {
        &mut self.writer
    }

    /// Consume the forwarder and return the underlying writer.
    pub fn into_writer(self) -> W {
        self.writer
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_plain_char() {
        let event = KeyEvent::plain(Key::Char('a'));
        assert_eq!(key_to_sequence(event), b"a");
    }

    #[test]
    fn test_shift_char() {
        let event = KeyEvent::new(Key::Char('a'), Modifiers::SHIFT);
        assert_eq!(key_to_sequence(event), b"A");
    }

    #[test]
    fn test_ctrl_char() {
        let event = KeyEvent::new(Key::Char('c'), Modifiers::CTRL);
        assert_eq!(key_to_sequence(event), vec![0x03]); // ETX
    }

    #[test]
    fn test_ctrl_a() {
        let event = KeyEvent::new(Key::Char('a'), Modifiers::CTRL);
        assert_eq!(key_to_sequence(event), vec![0x01]); // SOH
    }

    #[test]
    fn test_ctrl_z() {
        let event = KeyEvent::new(Key::Char('z'), Modifiers::CTRL);
        assert_eq!(key_to_sequence(event), vec![0x1a]); // SUB
    }

    #[test]
    fn test_alt_char() {
        let event = KeyEvent::new(Key::Char('x'), Modifiers::ALT);
        assert_eq!(key_to_sequence(event), vec![0x1b, b'x']);
    }

    #[test]
    fn test_ctrl_alt_char() {
        let event = KeyEvent::new(
            Key::Char('c'),
            Modifiers {
                ctrl: true,
                alt: true,
                shift: false,
            },
        );
        assert_eq!(key_to_sequence(event), vec![0x1b, 0x03]);
    }

    #[test]
    fn test_arrow_keys() {
        assert_eq!(key_to_sequence(KeyEvent::plain(Key::Up)), b"\x1b[A");
        assert_eq!(key_to_sequence(KeyEvent::plain(Key::Down)), b"\x1b[B");
        assert_eq!(key_to_sequence(KeyEvent::plain(Key::Right)), b"\x1b[C");
        assert_eq!(key_to_sequence(KeyEvent::plain(Key::Left)), b"\x1b[D");
    }

    #[test]
    fn test_arrow_with_modifiers() {
        let event = KeyEvent::new(Key::Up, Modifiers::CTRL);
        assert_eq!(key_to_sequence(event), b"\x1b[1;5A");

        let event = KeyEvent::new(Key::Down, Modifiers::SHIFT);
        assert_eq!(key_to_sequence(event), b"\x1b[1;2B");

        let event = KeyEvent::new(Key::Left, Modifiers::ALT);
        assert_eq!(key_to_sequence(event), b"\x1b[1;3D");
    }

    #[test]
    fn test_function_keys_f1_f4() {
        assert_eq!(key_to_sequence(KeyEvent::plain(Key::F(1))), b"\x1bOP");
        assert_eq!(key_to_sequence(KeyEvent::plain(Key::F(2))), b"\x1bOQ");
        assert_eq!(key_to_sequence(KeyEvent::plain(Key::F(3))), b"\x1bOR");
        assert_eq!(key_to_sequence(KeyEvent::plain(Key::F(4))), b"\x1bOS");
    }

    #[test]
    fn test_function_keys_f5_f12() {
        assert_eq!(key_to_sequence(KeyEvent::plain(Key::F(5))), b"\x1b[15~");
        assert_eq!(key_to_sequence(KeyEvent::plain(Key::F(6))), b"\x1b[17~");
        assert_eq!(key_to_sequence(KeyEvent::plain(Key::F(10))), b"\x1b[21~");
        assert_eq!(key_to_sequence(KeyEvent::plain(Key::F(12))), b"\x1b[24~");
    }

    #[test]
    fn test_function_keys_with_modifiers() {
        let event = KeyEvent::new(Key::F(1), Modifiers::SHIFT);
        assert_eq!(key_to_sequence(event), b"\x1b[1;2P");

        let event = KeyEvent::new(Key::F(5), Modifiers::CTRL);
        assert_eq!(key_to_sequence(event), b"\x1b[15;5~");
    }

    #[test]
    fn test_backspace() {
        assert_eq!(key_to_sequence(KeyEvent::plain(Key::Backspace)), vec![0x7f]);
    }

    #[test]
    fn test_enter() {
        assert_eq!(key_to_sequence(KeyEvent::plain(Key::Enter)), vec![0x0d]);
    }

    #[test]
    fn test_tab() {
        assert_eq!(key_to_sequence(KeyEvent::plain(Key::Tab)), vec![0x09]);
    }

    #[test]
    fn test_shift_tab() {
        let event = KeyEvent::new(Key::Tab, Modifiers::SHIFT);
        assert_eq!(key_to_sequence(event), b"\x1b[Z");
    }

    #[test]
    fn test_escape() {
        assert_eq!(key_to_sequence(KeyEvent::plain(Key::Escape)), vec![0x1b]);
    }

    #[test]
    fn test_home_end() {
        assert_eq!(key_to_sequence(KeyEvent::plain(Key::Home)), b"\x1b[H");
        assert_eq!(key_to_sequence(KeyEvent::plain(Key::End)), b"\x1b[F");
    }

    #[test]
    fn test_page_keys() {
        assert_eq!(key_to_sequence(KeyEvent::plain(Key::PageUp)), b"\x1b[5~");
        assert_eq!(key_to_sequence(KeyEvent::plain(Key::PageDown)), b"\x1b[6~");
    }

    #[test]
    fn test_insert_delete() {
        assert_eq!(key_to_sequence(KeyEvent::plain(Key::Insert)), b"\x1b[2~");
        assert_eq!(key_to_sequence(KeyEvent::plain(Key::Delete)), b"\x1b[3~");
    }

    #[test]
    fn test_modifiers_csi_param() {
        assert_eq!(Modifiers::NONE.csi_param(), 1);
        assert_eq!(Modifiers::SHIFT.csi_param(), 2);
        assert_eq!(Modifiers::ALT.csi_param(), 3);
        assert_eq!(Modifiers::CTRL.csi_param(), 5);

        let ctrl_shift = Modifiers {
            ctrl: true,
            alt: false,
            shift: true,
        };
        assert_eq!(ctrl_shift.csi_param(), 6);

        let all = Modifiers {
            ctrl: true,
            alt: true,
            shift: true,
        };
        assert_eq!(all.csi_param(), 8);
    }

    #[test]
    fn test_bracketed_paste_disabled() {
        let bp = BracketedPaste::new();
        assert!(!bp.is_enabled());
        assert_eq!(bp.wrap(b"hello"), b"hello");
    }

    #[test]
    fn test_bracketed_paste_enabled() {
        let mut bp = BracketedPaste::new();
        bp.enable();
        assert!(bp.is_enabled());

        let wrapped = bp.wrap(b"hello");
        assert!(wrapped.starts_with(BracketedPaste::START));
        assert!(wrapped.ends_with(BracketedPaste::END));
        assert!(wrapped[BracketedPaste::START.len()..].starts_with(b"hello"));
    }

    #[test]
    fn test_bracketed_paste_falls_back_to_raw_when_payload_contains_end_delimiter() {
        let mut bp = BracketedPaste::new();
        bp.enable();

        let payload = b"prefix\x1b[201~suffix";
        let wrapped = bp.wrap(payload);
        assert_eq!(wrapped, payload);
    }

    #[test]
    fn test_bracketed_paste_falls_back_to_raw_when_payload_contains_start_delimiter() {
        let mut bp = BracketedPaste::new();
        bp.enable();

        let payload = b"prefix\x1b[200~suffix";
        let wrapped = bp.wrap(payload);
        assert_eq!(wrapped, payload);
    }

    #[test]
    fn test_input_forwarder() {
        let mut buffer = Vec::new();

        {
            let mut forwarder = InputForwarder::new(&mut buffer);
            forwarder
                .forward_key(KeyEvent::plain(Key::Char('a')))
                .unwrap();
            forwarder.forward_key(KeyEvent::plain(Key::Enter)).unwrap();
        }

        assert_eq!(buffer, vec![b'a', 0x0d]);
    }

    #[test]
    fn test_input_forwarder_paste() {
        let mut buffer = Vec::new();

        {
            let mut forwarder = InputForwarder::new(&mut buffer);
            forwarder.forward_paste("text").unwrap();
        }

        assert_eq!(buffer, b"text");
    }

    #[test]
    fn test_input_forwarder_bracketed_paste() {
        let mut buffer = Vec::new();

        {
            let mut forwarder = InputForwarder::new(&mut buffer);
            forwarder.set_bracketed_paste(true);
            forwarder.forward_paste("text").unwrap();
        }

        let expected = [BracketedPaste::START, b"text", BracketedPaste::END].concat();
        assert_eq!(buffer, expected);
    }

    #[test]
    fn test_input_forwarder_bracketed_paste_uses_raw_fallback_for_delimiter_payload() {
        let mut buffer = Vec::new();
        let payload = "alpha\u{1b}[201~omega";

        {
            let mut forwarder = InputForwarder::new(&mut buffer);
            forwarder.set_bracketed_paste(true);
            forwarder.forward_paste(payload).unwrap();
        }

        assert_eq!(buffer, payload.as_bytes());
    }

    #[test]
    fn test_utf8_char() {
        let event = KeyEvent::plain(Key::Char('日'));
        let seq = key_to_sequence(event);
        assert_eq!(std::str::from_utf8(&seq).unwrap(), "日");
    }

    #[test]
    fn test_emoji_char() {
        let event = KeyEvent::plain(Key::Char('🎉'));
        let seq = key_to_sequence(event);
        assert_eq!(std::str::from_utf8(&seq).unwrap(), "🎉");
    }

    #[test]
    fn test_ctrl_special_chars() {
        // Ctrl+[ = ESC
        let event = KeyEvent::new(Key::Char('['), Modifiers::CTRL);
        assert_eq!(key_to_sequence(event), vec![0x1b]);

        // Ctrl+@ = NUL
        let event = KeyEvent::new(Key::Char('@'), Modifiers::CTRL);
        assert_eq!(key_to_sequence(event), vec![0x00]);

        // Ctrl+? = DEL
        let event = KeyEvent::new(Key::Char('?'), Modifiers::CTRL);
        assert_eq!(key_to_sequence(event), vec![0x7f]);
    }
}
