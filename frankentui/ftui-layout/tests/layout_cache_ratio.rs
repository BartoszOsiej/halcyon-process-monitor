use ftui_core::geometry::Rect;
use ftui_layout::{Constraint, Direction, LayoutCacheKey};

#[test]
fn ratio_canonicalization() {
    let area = Rect::new(0, 0, 100, 100);
    let c1 = [Constraint::Ratio(1, 2)];
    let c2 = [Constraint::Ratio(2, 4)];

    let k1 = LayoutCacheKey::new(area, &c1, Direction::Horizontal, None);
    let k2 = LayoutCacheKey::new(area, &c2, Direction::Horizontal, None);

    assert_eq!(
        k1, k2,
        "Ratio(1,2) and Ratio(2,4) should be equivalent for caching"
    );
}
