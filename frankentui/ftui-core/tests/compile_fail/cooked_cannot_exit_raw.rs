// Safety: Cooked mode has no exit_raw() — you must enter raw before exiting it.
use ftui_core::mode_typestate::*;

fn main() {
    let cooked = TerminalMode::<COOKED>::new();
    cooked.exit_raw();
}
