use ftui_text::wrap::{WrapOptions, wrap_with_options};

#[test]
fn test_wrap_spaces() {
    let text = "Hello  world";
    let opts = WrapOptions::new(6).preserve_indent(false);
    let wrapped = wrap_with_options(text, &opts);
    assert_eq!(wrapped, vec!["Hello", "world"]);
}
