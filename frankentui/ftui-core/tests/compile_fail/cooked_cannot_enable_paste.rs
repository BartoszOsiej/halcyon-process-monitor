// Safety: Cannot enable bracketed paste from cooked — requires alt screen.
use ftui_core::mode_typestate::*;

fn main() {
    let cooked = TerminalMode::<COOKED>::new();
    cooked.enable_bracketed_paste();
}
