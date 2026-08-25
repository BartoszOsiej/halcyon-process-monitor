//! Benchmarks for PlasmaFx (bd-l8x9.9.3)
//!
//! Performance budgets:
//! - plasma_wave() call: < 200ns (panic at 1μs)
//! - plasma_wave_low() call: < 100ns (panic at 500ns)
//! - PlasmaFx render 80x24 Full: < 200μs (panic at 1ms)
//! - PlasmaFx render 80x24 Minimal: < 100μs (panic at 500μs)
//! - PlasmaFx render 120x40 Full: < 500μs (panic at 2ms)
//! - PlasmaFx render 120x40 Minimal: < 250μs (panic at 1ms)
//! - PlasmaFx render 240x80 Full: < 2ms (panic at 10ms)
//! - PlasmaFx render 240x80 Minimal: < 1ms (panic at 5ms)
//!
//! Run with: cargo bench -p ftui-extras --bench plasma_bench --features visual-fx

use criterion::{Criterion, criterion_group, criterion_main};
use std::hint::black_box;

#[cfg(feature = "visual-fx")]
use ftui_extras::visual_fx::{
    BackdropFx, FxContext, FxQuality, PlasmaFx, PlasmaPalette, ThemeInputs, plasma_wave,
    plasma_wave_low,
};
#[cfg(feature = "visual-fx")]
use ftui_render::cell::PackedRgba;

// =============================================================================
// Wave Function Benchmarks
// =============================================================================

#[cfg(feature = "visual-fx")]
fn bench_wave_functions(c: &mut Criterion) {
    let mut group = c.benchmark_group("plasma_fx/wave");

    // Budget: < 200ns for full, < 100ns for low
    group.bench_function("plasma_wave_center", |b| {
        b.iter(|| black_box(plasma_wave(black_box(0.5), black_box(0.5), black_box(1.0))))
    });

    group.bench_function("plasma_wave_corner", |b| {
        b.iter(|| black_box(plasma_wave(black_box(0.0), black_box(0.0), black_box(1.0))))
    });

    group.bench_function("plasma_wave_varying_time", |b| {
        let mut t = 0.0f64;
        b.iter(|| {
            t += 0.1;
            black_box(plasma_wave(black_box(0.5), black_box(0.5), black_box(t)))
        })
    });

    group.bench_function("plasma_wave_low_center", |b| {
        b.iter(|| {
            black_box(plasma_wave_low(
                black_box(0.5),
                black_box(0.5),
                black_box(1.0),
            ))
        })
    });

    group.bench_function("plasma_wave_low_corner", |b| {
        b.iter(|| {
            black_box(plasma_wave_low(
                black_box(0.0),
                black_box(0.0),
                black_box(1.0),
            ))
        })
    });

    group.bench_function("plasma_wave_low_varying_time", |b| {
        let mut t = 0.0f64;
        b.iter(|| {
            t += 0.1;
            black_box(plasma_wave_low(
                black_box(0.5),
                black_box(0.5),
                black_box(t),
            ))
        })
    });

    group.finish();
}

// =============================================================================
// Render Benchmarks - Size: 80x24 (1,920 cells)
// =============================================================================

#[cfg(feature = "visual-fx")]
fn bench_render_80x24(c: &mut Criterion) {
    let mut group = c.benchmark_group("plasma_fx/render_80x24");
    let theme = ThemeInputs::default_dark();
    let size = 80 * 24;

    // Full quality
    group.bench_function("full", |b| {
        let mut fx = PlasmaFx::new(PlasmaPalette::Sunset);
        let mut out = vec![PackedRgba::TRANSPARENT; size];
        let ctx = FxContext {
            width: 80,
            height: 24,
            frame: 1,
            time_seconds: 1.0,
            quality: FxQuality::Full,
            theme: &theme,
        };
        b.iter(|| {
            fx.render(black_box(ctx), &mut out);
            black_box(&out);
        })
    });

    // Reduced quality
    group.bench_function("reduced", |b| {
        let mut fx = PlasmaFx::new(PlasmaPalette::Sunset);
        let mut out = vec![PackedRgba::TRANSPARENT; size];
        let ctx = FxContext {
            width: 80,
            height: 24,
            frame: 1,
            time_seconds: 1.0,
            quality: FxQuality::Reduced,
            theme: &theme,
        };
        b.iter(|| {
            fx.render(black_box(ctx), &mut out);
            black_box(&out);
        })
    });

    // Minimal quality
    group.bench_function("minimal", |b| {
        let mut fx = PlasmaFx::new(PlasmaPalette::Sunset);
        let mut out = vec![PackedRgba::TRANSPARENT; size];
        let ctx = FxContext {
            width: 80,
            height: 24,
            frame: 1,
            time_seconds: 1.0,
            quality: FxQuality::Minimal,
            theme: &theme,
        };
        b.iter(|| {
            fx.render(black_box(ctx), &mut out);
            black_box(&out);
        })
    });

    group.finish();
}

// =============================================================================
// Render Benchmarks - Size: 120x40 (4,800 cells)
// =============================================================================

#[cfg(feature = "visual-fx")]
fn bench_render_120x40(c: &mut Criterion) {
    let mut group = c.benchmark_group("plasma_fx/render_120x40");
    let theme = ThemeInputs::default_dark();
    let size = 120 * 40;

    // Full quality
    group.bench_function("full", |b| {
        let mut fx = PlasmaFx::new(PlasmaPalette::Sunset);
        let mut out = vec![PackedRgba::TRANSPARENT; size];
        let ctx = FxContext {
            width: 120,
            height: 40,
            frame: 1,
            time_seconds: 1.0,
            quality: FxQuality::Full,
            theme: &theme,
        };
        b.iter(|| {
            fx.render(black_box(ctx), &mut out);
            black_box(&out);
        })
    });

    // Reduced quality
    group.bench_function("reduced", |b| {
        let mut fx = PlasmaFx::new(PlasmaPalette::Sunset);
        let mut out = vec![PackedRgba::TRANSPARENT; size];
        let ctx = FxContext {
            width: 120,
            height: 40,
            frame: 1,
            time_seconds: 1.0,
            quality: FxQuality::Reduced,
            theme: &theme,
        };
        b.iter(|| {
            fx.render(black_box(ctx), &mut out);
            black_box(&out);
        })
    });

    // Minimal quality
    group.bench_function("minimal", |b| {
        let mut fx = PlasmaFx::new(PlasmaPalette::Sunset);
        let mut out = vec![PackedRgba::TRANSPARENT; size];
        let ctx = FxContext {
            width: 120,
            height: 40,
            frame: 1,
            time_seconds: 1.0,
            quality: FxQuality::Minimal,
            theme: &theme,
        };
        b.iter(|| {
            fx.render(black_box(ctx), &mut out);
            black_box(&out);
        })
    });

    group.finish();
}

// =============================================================================
// Render Benchmarks - Size: 240x80 (19,200 cells)
// =============================================================================

#[cfg(feature = "visual-fx")]
fn bench_render_240x80(c: &mut Criterion) {
    let mut group = c.benchmark_group("plasma_fx/render_240x80");
    let theme = ThemeInputs::default_dark();
    let size = 240 * 80;

    // Full quality
    group.bench_function("full", |b| {
        let mut fx = PlasmaFx::new(PlasmaPalette::Sunset);
        let mut out = vec![PackedRgba::TRANSPARENT; size];
        let ctx = FxContext {
            width: 240,
            height: 80,
            frame: 1,
            time_seconds: 1.0,
            quality: FxQuality::Full,
            theme: &theme,
        };
        b.iter(|| {
            fx.render(black_box(ctx), &mut out);
            black_box(&out);
        })
    });

    // Reduced quality
    group.bench_function("reduced", |b| {
        let mut fx = PlasmaFx::new(PlasmaPalette::Sunset);
        let mut out = vec![PackedRgba::TRANSPARENT; size];
        let ctx = FxContext {
            width: 240,
            height: 80,
            frame: 1,
            time_seconds: 1.0,
            quality: FxQuality::Reduced,
            theme: &theme,
        };
        b.iter(|| {
            fx.render(black_box(ctx), &mut out);
            black_box(&out);
        })
    });

    // Minimal quality
    group.bench_function("minimal", |b| {
        let mut fx = PlasmaFx::new(PlasmaPalette::Sunset);
        let mut out = vec![PackedRgba::TRANSPARENT; size];
        let ctx = FxContext {
            width: 240,
            height: 80,
            frame: 1,
            time_seconds: 1.0,
            quality: FxQuality::Minimal,
            theme: &theme,
        };
        b.iter(|| {
            fx.render(black_box(ctx), &mut out);
            black_box(&out);
        })
    });

    group.finish();
}

// =============================================================================
// Palette Comparison Benchmarks
// =============================================================================

#[cfg(feature = "visual-fx")]
fn bench_palettes(c: &mut Criterion) {
    let mut group = c.benchmark_group("plasma_fx/palettes");
    let theme = ThemeInputs::default_dark();
    let size = 80 * 24;

    // Theme palette (uses ThemeInputs colors)
    group.bench_function("theme_accents", |b| {
        let mut fx = PlasmaFx::new(PlasmaPalette::ThemeAccents);
        let mut out = vec![PackedRgba::TRANSPARENT; size];
        let ctx = FxContext {
            width: 80,
            height: 24,
            frame: 1,
            time_seconds: 1.0,
            quality: FxQuality::Full,
            theme: &theme,
        };
        b.iter(|| {
            fx.render(black_box(ctx), &mut out);
            black_box(&out);
        })
    });

    // Sunset palette
    group.bench_function("sunset", |b| {
        let mut fx = PlasmaFx::sunset();
        let mut out = vec![PackedRgba::TRANSPARENT; size];
        let ctx = FxContext {
            width: 80,
            height: 24,
            frame: 1,
            time_seconds: 1.0,
            quality: FxQuality::Full,
            theme: &theme,
        };
        b.iter(|| {
            fx.render(black_box(ctx), &mut out);
            black_box(&out);
        })
    });

    // Ocean palette
    group.bench_function("ocean", |b| {
        let mut fx = PlasmaFx::ocean();
        let mut out = vec![PackedRgba::TRANSPARENT; size];
        let ctx = FxContext {
            width: 80,
            height: 24,
            frame: 1,
            time_seconds: 1.0,
            quality: FxQuality::Full,
            theme: &theme,
        };
        b.iter(|| {
            fx.render(black_box(ctx), &mut out);
            black_box(&out);
        })
    });

    // Fire palette
    group.bench_function("fire", |b| {
        let mut fx = PlasmaFx::fire();
        let mut out = vec![PackedRgba::TRANSPARENT; size];
        let ctx = FxContext {
            width: 80,
            height: 24,
            frame: 1,
            time_seconds: 1.0,
            quality: FxQuality::Full,
            theme: &theme,
        };
        b.iter(|| {
            fx.render(black_box(ctx), &mut out);
            black_box(&out);
        })
    });

    // Neon palette (uses HSV conversion - potentially slower)
    group.bench_function("neon", |b| {
        let mut fx = PlasmaFx::neon();
        let mut out = vec![PackedRgba::TRANSPARENT; size];
        let ctx = FxContext {
            width: 80,
            height: 24,
            frame: 1,
            time_seconds: 1.0,
            quality: FxQuality::Full,
            theme: &theme,
        };
        b.iter(|| {
            fx.render(black_box(ctx), &mut out);
            black_box(&out);
        })
    });

    // Cyberpunk palette
    group.bench_function("cyberpunk", |b| {
        let mut fx = PlasmaFx::cyberpunk();
        let mut out = vec![PackedRgba::TRANSPARENT; size];
        let ctx = FxContext {
            width: 80,
            height: 24,
            frame: 1,
            time_seconds: 1.0,
            quality: FxQuality::Full,
            theme: &theme,
        };
        b.iter(|| {
            fx.render(black_box(ctx), &mut out);
            black_box(&out);
        })
    });

    // Aurora palette (theme-derived, cool tones)
    group.bench_function("aurora", |b| {
        let mut fx = PlasmaFx::aurora();
        let mut out = vec![PackedRgba::TRANSPARENT; size];
        let ctx = FxContext {
            width: 80,
            height: 24,
            frame: 1,
            time_seconds: 1.0,
            quality: FxQuality::Full,
            theme: &theme,
        };
        b.iter(|| {
            fx.render(black_box(ctx), &mut out);
            black_box(&out);
        })
    });

    // Ember palette (theme-derived, warm tones)
    group.bench_function("ember", |b| {
        let mut fx = PlasmaFx::ember();
        let mut out = vec![PackedRgba::TRANSPARENT; size];
        let ctx = FxContext {
            width: 80,
            height: 24,
            frame: 1,
            time_seconds: 1.0,
            quality: FxQuality::Full,
            theme: &theme,
        };
        b.iter(|| {
            fx.render(black_box(ctx), &mut out);
            black_box(&out);
        })
    });

    // Subtle palette (theme-derived, low saturation)
    group.bench_function("subtle", |b| {
        let mut fx = PlasmaFx::subtle();
        let mut out = vec![PackedRgba::TRANSPARENT; size];
        let ctx = FxContext {
            width: 80,
            height: 24,
            frame: 1,
            time_seconds: 1.0,
            quality: FxQuality::Full,
            theme: &theme,
        };
        b.iter(|| {
            fx.render(black_box(ctx), &mut out);
            black_box(&out);
        })
    });

    // Monochrome palette (theme-derived, grayscale)
    group.bench_function("monochrome", |b| {
        let mut fx = PlasmaFx::monochrome();
        let mut out = vec![PackedRgba::TRANSPARENT; size];
        let ctx = FxContext {
            width: 80,
            height: 24,
            frame: 1,
            time_seconds: 1.0,
            quality: FxQuality::Full,
            theme: &theme,
        };
        b.iter(|| {
            fx.render(black_box(ctx), &mut out);
            black_box(&out);
        })
    });

    group.finish();
}

// =============================================================================
// Animation Benchmarks (time progression)
// =============================================================================

#[cfg(feature = "visual-fx")]
fn bench_animation(c: &mut Criterion) {
    let mut group = c.benchmark_group("plasma_fx/animation");
    let theme = ThemeInputs::default_dark();
    let size = 80 * 24;

    // Single frame at different time values
    group.bench_function("frame_progression_80x24", |b| {
        let mut fx = PlasmaFx::sunset();
        let mut out = vec![PackedRgba::TRANSPARENT; size];
        let mut frame_num = 0u64;
        let mut time = 0.0f64;

        b.iter(|| {
            frame_num += 1;
            time += 1.0 / 60.0; // 60 FPS simulation
            let ctx = FxContext {
                width: 80,
                height: 24,
                frame: frame_num,
                time_seconds: time,
                quality: FxQuality::Full,
                theme: &theme,
            };
            fx.render(ctx, &mut out);
            black_box(&out);
        })
    });

    group.finish();
}

// =============================================================================
// Throughput/Scaling Benchmarks
// =============================================================================

#[cfg(feature = "visual-fx")]
fn bench_scaling(c: &mut Criterion) {
    let mut group = c.benchmark_group("plasma_fx/scaling");
    let theme = ThemeInputs::default_dark();

    // Measure throughput in cells/second for various sizes
    for (w, h) in [(40, 12), (80, 24), (120, 40), (160, 50), (240, 80)] {
        let size = w as usize * h as usize;
        let label = format!("{w}x{h}");

        group.throughput(criterion::Throughput::Elements(size as u64));
        group.bench_function(&label, |b| {
            let mut fx = PlasmaFx::sunset();
            let mut out = vec![PackedRgba::TRANSPARENT; size];
            let ctx = FxContext {
                width: w,
                height: h,
                frame: 1,
                time_seconds: 1.0,
                quality: FxQuality::Full,
                theme: &theme,
            };
            b.iter(|| {
                fx.render(black_box(ctx), &mut out);
                black_box(&out);
            })
        });
    }

    group.finish();
}

// =============================================================================
// Criterion Groups
// =============================================================================

#[cfg(feature = "visual-fx")]
criterion_group!(
    benches,
    bench_wave_functions,
    bench_render_80x24,
    bench_render_120x40,
    bench_render_240x80,
    bench_palettes,
    bench_animation,
    bench_scaling,
);

#[cfg(not(feature = "visual-fx"))]
fn bench_placeholder(_c: &mut Criterion) {
    // Placeholder when visual-fx feature is not enabled
}

#[cfg(not(feature = "visual-fx"))]
criterion_group!(benches, bench_placeholder);

criterion_main!(benches);
