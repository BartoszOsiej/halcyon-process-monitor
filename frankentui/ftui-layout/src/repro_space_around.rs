#[cfg(test)]
mod tests {
    use crate::{Alignment, Constraint, Flex, Rect};

    #[test]
    fn space_around_remainder() {
        // Case: 10 pixels, 2 items of size 2.
        // Leftover = 10 - 4 = 6.
        // SpaceAround: slots = 2 * 2 = 4.
        // unit = 6 / 4 = 1.
        // rem = 6 % 4 = 2.
        //
        // Current implementation:
        // Start offset = unit = 1.
        // Item 0 at 1. Size 2. Right = 3.
        // Gap after Item 0 = 2 * unit = 2.
        // Item 1 at 3 + 2 = 5. Size 2. Right = 7.
        // Gap after Item 1 (implied) = unit = 1.
        // Total covered: 1 + 2 + 2 + 2 + 1 = 8.
        // Available: 10.
        // Unused: 2 pixels at the end.
        //
        // This means the layout is:
        // [ G(1) ] [ I0(2) ] [ G(2) ] [ I1(2) ] [ G(1) + Unused(2) ]
        // The visual center is shifted left.

        let flex = Flex::horizontal()
            .alignment(Alignment::SpaceAround)
            .constraints([Constraint::Fixed(2), Constraint::Fixed(2)]);

        let rects = flex.split(Rect::new(0, 0, 10, 10));

        println!("Rects: {:?}", rects);

        // Check if items are centered
        // Center of area is 5.
        // Item 0 center: x + w/2 = rects[0].x + 1
        // Item 1 center: x + w/2 = rects[1].x + 1
        // Midpoint of items: (center0 + center1) / 2

        let center0 = rects[0].x as f32 + 1.0;
        let center1 = rects[1].x as f32 + 1.0;
        let midpoint = (center0 + center1) / 2.0;

        println!("Midpoint: {}", midpoint);

        // Ideally midpoint should be 5.0.
        // With current impl:
        // Rect 0 at 1. Center 2.
        // Rect 1 at 5. Center 6.
        // Midpoint 4.0.
        // Shifted left by 1 pixel.

        assert!(
            (midpoint - 5.0).abs() <= 0.5,
            "Items should be roughly centered in SpaceAround (was {})",
            midpoint
        );
    }
}
