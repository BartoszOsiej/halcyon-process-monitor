use ftui_text::wrap::{WrapMode, wrap_text};
fn main() {
    let text = "123 \u{00A0}\u{00A0}\u{00A0}foo"; // note: \u{00A0} and foo are grouped
    let lines = wrap_text(text, 6, WrapMode::Word);
    println!("wrapped lines 1: {:?}", lines);

    let text2 = "123 \u{00A0}\u{00A0}\u{00A0} foo"; // space separates them
    let lines2 = wrap_text(text2, 6, WrapMode::Word);
    println!("wrapped lines 2: {:?}", lines2);
}
