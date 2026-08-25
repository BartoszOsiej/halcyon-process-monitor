//! Benchmarks for terminal pane drag/resize adapter paths (bd-1y0ph).
//!
//! Run with:
//!   cargo bench -p ftui-runtime --bench pane_terminal_bench

use criterion::{BatchSize, Criterion, criterion_group, criterion_main};
use ftui_core::event::{Event, MouseButton, MouseEvent, MouseEventKind};
use ftui_core::geometry::Rect;
use ftui_layout::{
    PaneLeaf, PaneNodeKind, PaneOperation, PanePlacement, PaneSplitRatio, PaneTree, SplitAxis,
};
use ftui_runtime::{
    PaneTerminalAdapter, PaneTerminalAdapterConfig, pane_terminal_resolve_splitter_target,
    pane_terminal_splitter_handles,
};
use std::collections::VecDeque;
use std::hint::black_box;

fn build_pane_tree(leaf_count: usize) -> PaneTree {
    assert!(
        leaf_count >= 1,
        "pane benchmark tree requires at least one leaf"
    );
    let mut tree = PaneTree::singleton("leaf-0");
    if leaf_count == 1 {
        return tree;
    }

    let ratio = PaneSplitRatio::new(1, 1).expect("ratio 1:1 should be valid");
    let mut split_queue = VecDeque::from([tree.root()]);
    for idx in 1..leaf_count {
        let target = split_queue
            .pop_front()
            .expect("split queue should always provide a leaf target");
        let axis = if idx % 2 == 0 {
            SplitAxis::Horizontal
        } else {
            SplitAxis::Vertical
        };
        let outcome = tree
            .apply_operation(
                idx as u64,
                PaneOperation::SplitLeaf {
                    target,
                    axis,
                    ratio,
                    placement: PanePlacement::ExistingFirst,
                    new_leaf: PaneLeaf::new(format!("leaf-{idx}")),
                },
            )
            .expect("deterministic bench split should succeed");
        let new_leaf_id = outcome
            .touched_nodes
            .into_iter()
            .find(|node_id| {
                *node_id != target
                    && matches!(tree.node(*node_id), Some(node) if matches!(node.kind, PaneNodeKind::Leaf(_)))
            })
            .expect("split operation should create a new leaf id");
        split_queue.push_back(target);
        split_queue.push_back(new_leaf_id);
    }
    tree
}

fn drag_event(x: u16, y: u16) -> Event {
    Event::Mouse(MouseEvent::new(
        MouseEventKind::Drag(MouseButton::Left),
        x,
        y,
    ))
}

fn down_event(x: u16, y: u16) -> Event {
    Event::Mouse(MouseEvent::new(
        MouseEventKind::Down(MouseButton::Left),
        x,
        y,
    ))
}

fn up_event(x: u16, y: u16) -> Event {
    Event::Mouse(MouseEvent::new(MouseEventKind::Up(MouseButton::Left), x, y))
}

fn bench_pane_terminal_lifecycle(c: &mut Criterion) {
    let mut group = c.benchmark_group("pane/terminal/lifecycle");

    let tree = build_pane_tree(32);
    let layout = tree
        .solve_layout(Rect::new(0, 0, 240, 80))
        .expect("pane layout should solve");
    let handles = pane_terminal_splitter_handles(&tree, &layout, 3);
    let down = down_event(120, 18);
    let up = up_event(152, 18);
    let target = pane_terminal_resolve_splitter_target(&handles, 120, 18)
        .expect("bench pointer-down should resolve to a splitter target");

    group.bench_function("down_drag_32_up", |b| {
        let drag_positions: Vec<(u16, u16)> = (0..32u16).map(|step| (121 + step, 18)).collect();
        b.iter_batched(
            || {
                PaneTerminalAdapter::new(PaneTerminalAdapterConfig::default())
                    .expect("default pane terminal adapter should be valid")
            },
            |mut adapter| {
                let down_dispatch = adapter.translate(black_box(&down), Some(target));
                black_box(down_dispatch.log.sequence);

                for &(x, y) in &drag_positions {
                    let dispatch = adapter.translate(black_box(&drag_event(x, y)), None);
                    black_box(dispatch.motion.map(|motion| motion.speed));
                }

                let up_dispatch = adapter.translate(black_box(&up), None);
                black_box(up_dispatch.projected_position);
            },
            BatchSize::SmallInput,
        );
    });

    group.bench_function("down_drag_120_up", |b| {
        let drag_positions: Vec<(u16, u16)> = (0..120u16)
            .map(|step| (121 + step, 18 + (step % 3)))
            .collect();
        b.iter_batched(
            || {
                PaneTerminalAdapter::new(PaneTerminalAdapterConfig::default())
                    .expect("default pane terminal adapter should be valid")
            },
            |mut adapter| {
                let down_dispatch = adapter.translate(black_box(&down), Some(target));
                black_box(down_dispatch.primary_transition.as_ref().map(|t| t.to));

                for &(x, y) in &drag_positions {
                    let dispatch = adapter.translate(black_box(&drag_event(x, y)), None);
                    black_box(
                        dispatch
                            .primary_transition
                            .as_ref()
                            .map(|transition| transition.sequence),
                    );
                }

                let up_dispatch = adapter.translate(black_box(&up_event(216, 20)), None);
                black_box(up_dispatch.inertial_throw);
            },
            BatchSize::SmallInput,
        );
    });

    group.finish();
}

fn bench_pane_terminal_with_handles(c: &mut Criterion) {
    let mut group = c.benchmark_group("pane/terminal/with_handles");

    let tree = build_pane_tree(32);
    let layout = tree
        .solve_layout(Rect::new(0, 0, 240, 80))
        .expect("pane layout should solve");
    let handles = pane_terminal_splitter_handles(&tree, &layout, 3);
    let down = down_event(120, 18);
    let up = up_event(152, 18);

    group.bench_function("translate_with_handles_drag_32_up", |b| {
        let drag_positions: Vec<(u16, u16)> = (0..32u16).map(|step| (121 + step, 18)).collect();
        b.iter_batched(
            || {
                PaneTerminalAdapter::new(PaneTerminalAdapterConfig::default())
                    .expect("default pane terminal adapter should be valid")
            },
            |mut adapter| {
                let down_dispatch = adapter.translate_with_handles(black_box(&down), &handles);
                black_box(down_dispatch.log.sequence);

                for &(x, y) in &drag_positions {
                    let event = drag_event(x, y);
                    let dispatch = adapter.translate_with_handles(black_box(&event), &handles);
                    black_box(dispatch.motion.map(|motion| motion.speed));
                }

                let up_dispatch = adapter.translate_with_handles(black_box(&up), &handles);
                black_box(up_dispatch.projected_position);
            },
            BatchSize::SmallInput,
        );
    });

    group.finish();
}

criterion_group!(
    benches,
    bench_pane_terminal_lifecycle,
    bench_pane_terminal_with_handles
);
criterion_main!(benches);
