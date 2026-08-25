use ftui_core::geometry::Rect;
use ftui_layout::Constraint;
use ftui_layout::grid::Grid;

#[test]
fn test_grid_gap_overflow() {
    let grid = Grid::new()
        .rows([Constraint::Fixed(1), Constraint::Fixed(1)])
        .columns([Constraint::Fixed(1), Constraint::Fixed(1)])
        .row_gap(20)
        .col_gap(20);

    let area = Rect::new(0, 0, 1, 1);
    let layout = grid.split(area);
    let cell = layout.cell(1, 1);

    assert!(
        cell.x <= area.right() && cell.y <= area.bottom(),
        "Cell {cell:?} is outside area {area:?}"
    );
}
