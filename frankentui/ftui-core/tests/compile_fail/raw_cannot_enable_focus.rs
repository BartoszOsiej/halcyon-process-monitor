// Safety: Cannot enable focus events from raw mode — requires alt screen.
use ftui_core::mode_typestate::*;

fn main() {
    let raw = TerminalMode::<COOKED>::new().enter_raw();
    raw.enable_focus_events();
}
