// Safety: Cannot enable mouse from cooked — must be in raw + alt screen first.
use ftui_core::mode_typestate::*;

fn main() {
    let cooked = TerminalMode::<COOKED>::new();
    cooked.enable_mouse();
}
