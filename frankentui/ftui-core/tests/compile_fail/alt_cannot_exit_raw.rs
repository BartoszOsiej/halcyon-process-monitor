// Safety: Cannot exit raw directly from alt screen — must exit alt screen first.
use ftui_core::mode_typestate::*;

fn main() {
    let alt = TerminalMode::<COOKED>::new().enter_raw().enter_alt_screen();
    alt.exit_raw();
}
