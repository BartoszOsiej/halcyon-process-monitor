// Safety: Cannot enter raw mode when already in raw mode — no double entry.
use ftui_core::mode_typestate::*;

fn main() {
    let raw = TerminalMode::<COOKED>::new().enter_raw();
    raw.enter_raw();
}
