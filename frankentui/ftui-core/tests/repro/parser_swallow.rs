use ftui_core::event::{Event, KeyCode};
use ftui_core::input_parser::InputParser;

#[test]
fn test_parser_swallows_newline_in_csi_param() {
    let mut parser = InputParser::new();
    
    // ESC [ 1 ; 

    // '1' and ';' are valid params. '
' is invalid in CSI.
    // It should abort the CSI sequence and emit the newline.
    let input = b"\x1b[1;
";
    let events = parser.parse(input);
    
    assert_eq!(events.len(), 1, "Should emit one event (Enter)");
    if let Some(Event::Key(k)) = events.first() {
        assert_eq!(k.code, KeyCode::Enter, "Expected Enter key");
    } else {
        panic!("Expected Key event, got {:?}", events.first());
    }
}

#[test]
fn test_parser_swallows_newline_in_osc() {
    let mut parser = InputParser::new();
    
    // ESC ] 0 ; title 

    // 
 is a control char < 0x20. Should abort OSC and emit newline.
    let input = b"\x1b]0;title
";
    let events = parser.parse(input);
    
    assert_eq!(events.len(), 1, "Should emit one event (Enter)");
    if let Some(Event::Key(k)) = events.first() {
        assert_eq!(k.code, KeyCode::Enter, "Expected Enter key");
    } else {
        panic!("Expected Key event, got {:?}", events.first());
    }
}
