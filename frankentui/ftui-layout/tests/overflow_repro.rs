use ftui_core::geometry::Rect;
use ftui_layout::{Alignment, Constraint, Flex};

#[test]
fn alignment_space_between_overflow() {
    // 65537 constraints -> (len - 1) as u16 wraps to 0
    let constraints = vec![Constraint::Fixed(1); 65537];
    let flex = Flex::horizontal()
        .alignment(Alignment::SpaceBetween)
        .constraints(constraints);

    // This should panic due to division by zero if not fixed
    let _ = flex.split(Rect::new(0, 0, u16::MAX, 10));
}

#[test]
fn alignment_space_around_overflow() {
    // 32768 * 2 = 65536 wraps to 0
    let constraints = vec![Constraint::Fixed(1); 32768];
    let flex = Flex::horizontal()
        .alignment(Alignment::SpaceAround)
        .constraints(constraints);

    // This should panic due to division by zero if not fixed
    let _ = flex.split(Rect::new(0, 0, u16::MAX, 10));
}
