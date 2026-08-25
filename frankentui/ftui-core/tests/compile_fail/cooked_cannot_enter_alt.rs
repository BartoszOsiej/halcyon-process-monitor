// Safety: Cannot enter alternate screen from cooked — must be in raw mode first.
use ftui_core::mode_typestate::*;

fn main() {
    let cooked = TerminalMode::<COOKED>::new();
    cooked.enter_alt_screen();
}
