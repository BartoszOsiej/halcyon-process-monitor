#![cfg(feature = "markdown")]

use ftui_extras::markdown::{MarkdownRenderer, MarkdownTheme};

#[test]
fn test_image_alt_text_parsing_variations() {
    let renderer = MarkdownRenderer::new(MarkdownTheme::default());

    let cases = [
        (r#"<img src='foo' alt="standard">"#, "[standard]"),
        ("<img src='foo' alt='single quotes'>", "[single quotes]"),
        (
            r#"<img src='foo' ALT="upper case attr">"#,
            "[upper case attr]",
        ),
        (
            r#"<img src='foo' alt = "spaced equals">"#,
            "[spaced equals]",
        ),
        ("<img src='foo' alt=unquoted>", "[image]"), // Unquoted not supported by current logic, fallback
    ];

    for (html, expected) in cases {
        let text = renderer.render(html);
        // The renderer produces styled text. We just want to check if the content contains the expected string.
        // The text structure is complex, but we can iterate spans.
        let mut found = false;
        for line in text.lines() {
            for span in line.spans() {
                if span.content.contains(expected) {
                    found = true;
                    break;
                }
            }
        }
        assert!(
            found,
            "Failed to parse alt text from '{}'. Expected '{}'",
            html, expected
        );
    }
}
