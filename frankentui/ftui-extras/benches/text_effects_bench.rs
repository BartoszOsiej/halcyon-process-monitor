//! Benchmarks for text effects (bd-3cuk)
//!
//! Performance budgets:
//! - ease() call: < 100ns (panic at 500ns)
//! - gradient.sample(): < 50ns (panic at 200ns)
//! - StyledText creation: < 1μs
//! - StyledText effect chain (8 effects): < 1μs
//! - StyledText render: < 10μs for 20-char text
//!
//! Run with: cargo bench -p ftui-extras --bench text_effects_bench --features text-effects

use criterion::{Criterion, criterion_group, criterion_main};
use std::hint::black_box;

#[cfg(feature = "text-effects")]
use ftui_core::geometry::Rect;
#[cfg(feature = "text-effects")]
use ftui_extras::text_effects::{
    ColorGradient, Direction, Easing, StyledText, TextEffect, apply_alpha, hsv_to_rgb, lerp_color,
};
#[cfg(feature = "text-effects")]
use ftui_render::cell::PackedRgba;
#[cfg(feature = "text-effects")]
use ftui_render::frame::Frame;
#[cfg(feature = "text-effects")]
use ftui_render::grapheme_pool::GraphemePool;
#[cfg(feature = "text-effects")]
use ftui_widgets::Widget;

// =============================================================================
// Easing Benchmarks
// =============================================================================

#[cfg(feature = "text-effects")]
fn bench_easing(c: &mut Criterion) {
    let mut group = c.benchmark_group("text_effects/easing");

    // Budget: < 100ns
    group.bench_function("linear", |b| {
        b.iter(|| black_box(Easing::Linear.apply(black_box(0.5))))
    });

    group.bench_function("ease_in", |b| {
        b.iter(|| black_box(Easing::EaseIn.apply(black_box(0.5))))
    });

    group.bench_function("ease_out", |b| {
        b.iter(|| black_box(Easing::EaseOut.apply(black_box(0.5))))
    });

    group.bench_function("ease_in_out", |b| {
        b.iter(|| black_box(Easing::EaseInOut.apply(black_box(0.5))))
    });

    group.bench_function("bounce", |b| {
        b.iter(|| black_box(Easing::Bounce.apply(black_box(0.5))))
    });

    group.bench_function("elastic", |b| {
        b.iter(|| black_box(Easing::Elastic.apply(black_box(0.5))))
    });

    group.bench_function("back", |b| {
        b.iter(|| black_box(Easing::Back.apply(black_box(0.5))))
    });

    group.bench_function("step_4", |b| {
        b.iter(|| black_box(Easing::Step(4).apply(black_box(0.5))))
    });

    group.finish();
}

// =============================================================================
// Color Utility Benchmarks
// =============================================================================

#[cfg(feature = "text-effects")]
fn bench_color_utilities(c: &mut Criterion) {
    let mut group = c.benchmark_group("text_effects/color");

    let color_a = PackedRgba::rgb(255, 0, 0);
    let color_b = PackedRgba::rgb(0, 0, 255);

    // Budget: < 50ns
    group.bench_function("lerp_color", |b| {
        b.iter(|| {
            black_box(lerp_color(
                black_box(color_a),
                black_box(color_b),
                black_box(0.5),
            ))
        })
    });

    group.bench_function("apply_alpha", |b| {
        b.iter(|| black_box(apply_alpha(black_box(color_a), black_box(0.5))))
    });

    group.bench_function("hsv_to_rgb", |b| {
        b.iter(|| black_box(hsv_to_rgb(black_box(180.0), black_box(1.0), black_box(1.0))))
    });

    group.finish();
}

// =============================================================================
// Gradient Benchmarks
// =============================================================================

#[cfg(feature = "text-effects")]
fn bench_gradient(c: &mut Criterion) {
    let mut group = c.benchmark_group("text_effects/gradient");

    let rainbow = ColorGradient::rainbow();
    let cyberpunk = ColorGradient::cyberpunk();

    // Budget: < 50ns
    group.bench_function("sample_rainbow_start", |b| {
        b.iter(|| black_box(rainbow.sample(black_box(0.0))))
    });

    group.bench_function("sample_rainbow_mid", |b| {
        b.iter(|| black_box(rainbow.sample(black_box(0.5))))
    });

    group.bench_function("sample_rainbow_end", |b| {
        b.iter(|| black_box(rainbow.sample(black_box(1.0))))
    });

    group.bench_function("sample_cyberpunk_mid", |b| {
        b.iter(|| black_box(cyberpunk.sample(black_box(0.5))))
    });

    // Gradient creation
    group.bench_function("create_rainbow", |b| {
        b.iter(|| black_box(ColorGradient::rainbow()))
    });

    group.bench_function("create_custom_3_stop", |b| {
        b.iter(|| {
            black_box(ColorGradient::new(vec![
                (0.0, PackedRgba::rgb(255, 0, 0)),
                (0.5, PackedRgba::rgb(0, 255, 0)),
                (1.0, PackedRgba::rgb(0, 0, 255)),
            ]))
        })
    });

    group.finish();
}

// =============================================================================
// StyledText Creation Benchmarks
// =============================================================================

#[cfg(feature = "text-effects")]
fn bench_styled_text_creation(c: &mut Criterion) {
    let mut group = c.benchmark_group("text_effects/styled_text_create");

    // Budget: < 1μs for creation

    group.bench_function("new_10_chars", |b| {
        b.iter(|| black_box(StyledText::new(black_box("0123456789"))))
    });

    group.bench_function("new_with_1_effect", |b| {
        b.iter(|| {
            black_box(
                StyledText::new("0123456789").effect(TextEffect::RainbowGradient { speed: 1.0 }),
            )
        })
    });

    group.bench_function("new_with_4_effects", |b| {
        b.iter(|| {
            black_box(
                StyledText::new("0123456789")
                    .effect(TextEffect::RainbowGradient { speed: 1.0 })
                    .effect(TextEffect::Pulse {
                        speed: 2.0,
                        min_alpha: 0.5,
                    })
                    .effect(TextEffect::FadeIn { progress: 0.8 })
                    .effect(TextEffect::Glow {
                        color: PackedRgba::rgb(255, 255, 255),
                        intensity: 0.5,
                    }),
            )
        })
    });

    group.bench_function("new_with_8_effects", |b| {
        b.iter(|| {
            black_box(
                StyledText::new("0123456789")
                    .effect(TextEffect::RainbowGradient { speed: 1.0 })
                    .effect(TextEffect::Pulse {
                        speed: 2.0,
                        min_alpha: 0.5,
                    })
                    .effect(TextEffect::FadeIn { progress: 0.8 })
                    .effect(TextEffect::Glow {
                        color: PackedRgba::rgb(255, 255, 255),
                        intensity: 0.5,
                    })
                    .effect(TextEffect::ColorWave {
                        color1: PackedRgba::rgb(255, 0, 0),
                        color2: PackedRgba::rgb(0, 0, 255),
                        speed: 1.0,
                        wavelength: 5.0,
                    })
                    .effect(TextEffect::PulsingGlow {
                        color: PackedRgba::rgb(0, 255, 0),
                        speed: 1.0,
                    })
                    .effect(TextEffect::HorizontalGradient {
                        gradient: ColorGradient::fire(),
                    })
                    .effect(TextEffect::AnimatedGradient {
                        gradient: ColorGradient::ocean(),
                        speed: 0.5,
                    }),
            )
        })
    });

    group.finish();
}

// =============================================================================
// StyledText Render Benchmarks
// =============================================================================

#[cfg(feature = "text-effects")]
fn bench_styled_text_render(c: &mut Criterion) {
    let mut group = c.benchmark_group("text_effects/styled_text_render");

    // Budget: < 10μs for 20-char render

    // No effects - baseline
    group.bench_function("render_no_effect_20_chars", |b| {
        let text = StyledText::new("01234567890123456789").time(0.5);
        let area = Rect::new(0, 0, 25, 1);
        let mut pool = GraphemePool::new();

        b.iter(|| {
            let mut frame = Frame::new(25, 1, &mut pool);
            text.render(black_box(area), &mut frame);
            black_box(&frame.buffer);
        })
    });

    // Rainbow gradient
    group.bench_function("render_rainbow_20_chars", |b| {
        let text = StyledText::new("01234567890123456789")
            .effect(TextEffect::RainbowGradient { speed: 1.0 })
            .time(0.5);
        let area = Rect::new(0, 0, 25, 1);
        let mut pool = GraphemePool::new();

        b.iter(|| {
            let mut frame = Frame::new(25, 1, &mut pool);
            text.render(black_box(area), &mut frame);
            black_box(&frame.buffer);
        })
    });

    // Pulse effect
    group.bench_function("render_pulse_20_chars", |b| {
        let text = StyledText::new("01234567890123456789")
            .effect(TextEffect::Pulse {
                speed: 2.0,
                min_alpha: 0.3,
            })
            .time(0.5);
        let area = Rect::new(0, 0, 25, 1);
        let mut pool = GraphemePool::new();

        b.iter(|| {
            let mut frame = Frame::new(25, 1, &mut pool);
            text.render(black_box(area), &mut frame);
            black_box(&frame.buffer);
        })
    });

    // Wave effect
    group.bench_function("render_wave_20_chars", |b| {
        let text = StyledText::new("01234567890123456789")
            .effect(TextEffect::Wave {
                amplitude: 2.0,
                wavelength: 5.0,
                speed: 1.0,
                direction: Direction::Down,
            })
            .time(0.5);
        let area = Rect::new(0, 0, 25, 3);
        let mut pool = GraphemePool::new();

        b.iter(|| {
            let mut frame = Frame::new(25, 3, &mut pool);
            text.render(black_box(area), &mut frame);
            black_box(&frame.buffer);
        })
    });

    // Scramble effect
    group.bench_function("render_scramble_20_chars", |b| {
        let text = StyledText::new("01234567890123456789")
            .effect(TextEffect::Scramble { progress: 0.5 })
            .seed(42)
            .time(0.5);
        let area = Rect::new(0, 0, 25, 1);
        let mut pool = GraphemePool::new();

        b.iter(|| {
            let mut frame = Frame::new(25, 1, &mut pool);
            text.render(black_box(area), &mut frame);
            black_box(&frame.buffer);
        })
    });

    // Chained effects (4 effects)
    group.bench_function("render_chain_4_effects_20_chars", |b| {
        let text = StyledText::new("01234567890123456789")
            .effect(TextEffect::RainbowGradient { speed: 1.0 })
            .effect(TextEffect::Pulse {
                speed: 2.0,
                min_alpha: 0.5,
            })
            .effect(TextEffect::FadeIn { progress: 0.8 })
            .effect(TextEffect::Glow {
                color: PackedRgba::rgb(255, 255, 255),
                intensity: 0.5,
            })
            .time(0.5);
        let area = Rect::new(0, 0, 25, 1);
        let mut pool = GraphemePool::new();

        b.iter(|| {
            let mut frame = Frame::new(25, 1, &mut pool);
            text.render(black_box(area), &mut frame);
            black_box(&frame.buffer);
        })
    });

    group.finish();
}

// =============================================================================
// Criterion Groups
// =============================================================================

#[cfg(feature = "text-effects")]
criterion_group!(
    benches,
    bench_easing,
    bench_color_utilities,
    bench_gradient,
    bench_styled_text_creation,
    bench_styled_text_render,
);

#[cfg(not(feature = "text-effects"))]
fn bench_placeholder(_c: &mut Criterion) {
    // Placeholder when text-effects feature is not enabled
}

#[cfg(not(feature = "text-effects"))]
criterion_group!(benches, bench_placeholder);

criterion_main!(benches);
