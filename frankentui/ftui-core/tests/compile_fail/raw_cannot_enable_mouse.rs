// Safety: Cannot enable mouse from raw mode — must enter alt screen first.
use ftui_core::mode_typestate::*;

fn main() {
    let raw = TerminalMode::<COOKED>::new().enter_raw();
    raw.enable_mouse();
}
