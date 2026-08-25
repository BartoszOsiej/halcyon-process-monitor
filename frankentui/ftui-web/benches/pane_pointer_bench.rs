#![forbid(unsafe_code)]

use criterion::{BatchSize, Criterion, criterion_group, criterion_main};
use ftui_layout::{
    PaneId, PaneModifierSnapshot, PanePointerButton, PanePointerPosition, PaneResizeTarget,
    SplitAxis,
};
use ftui_web::pane_pointer_capture::{PanePointerCaptureAdapter, PanePointerCaptureConfig};
use std::hint::black_box;

fn target() -> PaneResizeTarget {
    PaneResizeTarget {
        split_id: PaneId::MIN,
        axis: SplitAxis::Horizontal,
    }
}

fn pos(x: i32, y: i32) -> PanePointerPosition {
    PanePointerPosition::new(x, y)
}

fn bench_pane_pointer_lifecycle(c: &mut Criterion) {
    let mut group = c.benchmark_group("pane/web_pointer/lifecycle");
    let modifiers = PaneModifierSnapshot::default();

    group.bench_function("down_ack_move_32_up", |b| {
        b.iter(|| {
            let mut adapter = PanePointerCaptureAdapter::new(PanePointerCaptureConfig::default())
                .expect("default adapter config should be valid");

            let down = adapter.pointer_down(
                target(),
                11,
                PanePointerButton::Primary,
                pos(4, 4),
                modifiers,
            );
            black_box(down.log.sequence);
            let ack = adapter.capture_acquired(11);
            black_box(ack.log.phase);

            for step in 0..32 {
                let dispatch = adapter.pointer_move(11, pos(5 + step, 4), modifiers);
                black_box(dispatch.motion.map(|motion| motion.speed));
            }

            let up = adapter.pointer_up(11, PanePointerButton::Primary, pos(40, 4), modifiers);
            black_box(up.inertial_throw);
        });
    });

    group.bench_function("down_ack_move_120_up", |b| {
        b.iter(|| {
            let mut adapter = PanePointerCaptureAdapter::new(PanePointerCaptureConfig::default())
                .expect("default adapter config should be valid");

            let down = adapter.pointer_down(
                target(),
                23,
                PanePointerButton::Primary,
                pos(3, 6),
                modifiers,
            );
            black_box(down.log.sequence);
            let ack = adapter.capture_acquired(23);
            black_box(ack.log.phase);

            for step in 0..120 {
                let x = 4 + ((step * 3) / 2);
                let y = 6 + (step % 3);
                let dispatch = adapter.pointer_move(23, pos(x, y), modifiers);
                black_box(
                    dispatch
                        .transition
                        .as_ref()
                        .map(|transition| transition.sequence),
                );
            }

            let up = adapter.pointer_up(23, PanePointerButton::Primary, pos(186, 8), modifiers);
            black_box(up.projected_position);
        });
    });

    group.bench_function("pointer_up_after_ack_move_120", |b| {
        b.iter_batched(
            || {
                let mut adapter =
                    PanePointerCaptureAdapter::new(PanePointerCaptureConfig::default())
                        .expect("default adapter config should be valid");

                let down = adapter.pointer_down(
                    target(),
                    37,
                    PanePointerButton::Primary,
                    pos(3, 6),
                    modifiers,
                );
                black_box(down.log.sequence);
                let ack = adapter.capture_acquired(37);
                black_box(ack.log.phase);

                for step in 0..120 {
                    let x = 4 + ((step * 3) / 2);
                    let y = 6 + (step % 3);
                    let dispatch = adapter.pointer_move(37, pos(x, y), modifiers);
                    black_box(dispatch.motion.map(|motion| motion.direction_changes));
                }

                adapter
            },
            |mut adapter| {
                let up = adapter.pointer_up(37, PanePointerButton::Primary, pos(186, 8), modifiers);
                black_box(up.projected_position);
                black_box(up.inertial_throw);
            },
            BatchSize::SmallInput,
        );
    });

    group.bench_function("down_ack_move_pause_reverse_120_up", |b| {
        b.iter(|| {
            let mut adapter = PanePointerCaptureAdapter::new(PanePointerCaptureConfig::default())
                .expect("default adapter config should be valid");

            let down = adapter.pointer_down(
                target(),
                29,
                PanePointerButton::Primary,
                pos(8, 8),
                modifiers,
            );
            black_box(down.log.sequence);
            let ack = adapter.capture_acquired(29);
            black_box(ack.log.phase);

            for step in 0..40 {
                let dispatch = adapter.pointer_move(29, pos(9 + step, 8), modifiers);
                black_box(dispatch.motion.map(|motion| motion.direction_changes));
            }

            for _ in 0..40 {
                let dispatch = adapter.pointer_move(29, pos(48, 8), modifiers);
                black_box(dispatch.motion.map(|motion| motion.direction_changes));
            }

            for step in 0..40 {
                let dispatch = adapter.pointer_move(29, pos(47 - step, 8), modifiers);
                black_box(dispatch.motion.map(|motion| motion.direction_changes));
            }

            let up = adapter.pointer_up(29, PanePointerButton::Primary, pos(8, 8), modifiers);
            black_box(up.inertial_throw);
        });
    });

    group.bench_function("blur_after_ack", |b| {
        b.iter(|| {
            let mut adapter = PanePointerCaptureAdapter::new(PanePointerCaptureConfig::default())
                .expect("default adapter config should be valid");

            let down = adapter.pointer_down(
                target(),
                31,
                PanePointerButton::Primary,
                pos(9, 9),
                modifiers,
            );
            black_box(down.capture_command);
            let ack = adapter.capture_acquired(31);
            black_box(ack.log.outcome);
            let blur = adapter.blur();
            black_box(blur.capture_command);
        });
    });

    group.finish();
}

criterion_group!(benches, bench_pane_pointer_lifecycle);
criterion_main!(benches);
