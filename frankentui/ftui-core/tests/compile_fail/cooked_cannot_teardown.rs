// Safety: No teardown from cooked — already in base state.
use ftui_core::mode_typestate::*;

fn main() {
    let cooked = TerminalMode::<COOKED>::new();
    cooked.teardown();
}
