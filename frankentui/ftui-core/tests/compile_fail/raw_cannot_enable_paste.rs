// Safety: Cannot enable bracketed paste from raw mode — requires alt screen.
use ftui_core::mode_typestate::*;

fn main() {
    let raw = TerminalMode::<COOKED>::new().enter_raw();
    raw.enable_bracketed_paste();
}
