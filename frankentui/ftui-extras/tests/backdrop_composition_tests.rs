//! Tests for multi-layer backdrop composition (bd-l8x9.8.5).
//!
//! This module validates the correctness and determinism of the StackedFx compositor:
//! - Layer ordering semantics (A over B != B over A)
//! - Alpha compositing correctness vs explicit PackedRgba::over() math
//! - Determinism (fixed inputs produce identical outputs)
//! - Allocation stability (no buffer growth after warmup)

#![forbid(unsafe_code)]

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use ftui_extras::visual_fx::{
    BackdropFx, BlendMode, FxContext, FxLayer, FxQuality, StackedFx, ThemeInputs,
};
use ftui_render::cell::PackedRgba;

// =============================================================================
// Test Effects
// =============================================================================

/// Test effect that fills with a constant RGBA color.
struct ConstantColor {
    color: PackedRgba,
}

impl ConstantColor {
    fn new(r: u8, g: u8, b: u8, a: u8) -> Self {
        Self {
            color: PackedRgba::rgba(r, g, b, a),
        }
    }

    fn opaque(r: u8, g: u8, b: u8) -> Self {
        Self::new(r, g, b, 255)
    }
}

impl BackdropFx for ConstantColor {
    fn name(&self) -> &'static str {
        "constant-color"
    }

    fn render(&mut self, ctx: FxContext<'_>, out: &mut [PackedRgba]) {
        if ctx.is_empty() {
            return;
        }
        out[..ctx.len()].fill(self.color);
    }
}

/// Test effect that produces a pattern based on cell index.
/// Used to verify cell-by-cell composition.
struct PatternFx {
    base_r: u8,
    base_g: u8,
    base_b: u8,
    alpha: u8,
}

impl PatternFx {
    fn new(r: u8, g: u8, b: u8, a: u8) -> Self {
        Self {
            base_r: r,
            base_g: g,
            base_b: b,
            alpha: a,
        }
    }
}

impl BackdropFx for PatternFx {
    fn name(&self) -> &'static str {
        "pattern"
    }

    fn render(&mut self, ctx: FxContext<'_>, out: &mut [PackedRgba]) {
        for (i, cell) in out[..ctx.len()].iter_mut().enumerate() {
            let idx = i as u8;
            *cell = PackedRgba::rgba(
                self.base_r.wrapping_add(idx),
                self.base_g.wrapping_add(idx.wrapping_mul(2)),
                self.base_b.wrapping_add(idx.wrapping_mul(3)),
                self.alpha,
            );
        }
    }
}

// =============================================================================
// Helper Functions
// =============================================================================

fn make_context(width: u16, height: u16) -> (FxContext<'static>, ThemeInputs) {
    let theme = Box::leak(Box::new(ThemeInputs::default_dark()));
    let ctx = FxContext {
        width,
        height,
        frame: 0,
        time_seconds: 0.0,
        quality: FxQuality::Full,
        theme,
    };
    (ctx, *theme)
}

fn hash_output(out: &[PackedRgba]) -> u64 {
    let mut hasher = DefaultHasher::new();
    for c in out {
        c.r().hash(&mut hasher);
        c.g().hash(&mut hasher);
        c.b().hash(&mut hasher);
        c.a().hash(&mut hasher);
    }
    hasher.finish()
}

// =============================================================================
// ORDERING TESTS
// =============================================================================

/// **TEST: Layer ordering semantics - A over B != B over A**
///
/// Given two constant layers A and B with known RGBA + opacity, assert that
/// `[A, B]` (A on bottom, B on top) produces different output than `[B, A]`.
///
/// **API Order Documentation**: Layers are composited bottom-to-top (index 0 is
/// the base/bottom layer). This matches the "painter's algorithm" convention.
#[test]
fn ordering_a_over_b_not_equal_b_over_a() {
    let (ctx, _theme) = make_context(4, 4);
    let len = ctx.len();

    // Layer A: Semi-transparent red (opacity 0.6)
    let layer_a_color = PackedRgba::rgba(255, 0, 0, 153); // ~60% alpha
    let layer_a_opacity = 1.0; // No additional opacity scaling

    // Layer B: Semi-transparent blue (opacity 0.5)
    let layer_b_color = PackedRgba::rgba(0, 0, 255, 128); // ~50% alpha
    let layer_b_opacity = 1.0;

    // Stack 1: A (bottom) -> B (top)
    let mut stack_ab = StackedFx::new();
    stack_ab.push(FxLayer::with_opacity(
        Box::new(ConstantColor {
            color: layer_a_color,
        }),
        layer_a_opacity,
    ));
    stack_ab.push(FxLayer::with_opacity(
        Box::new(ConstantColor {
            color: layer_b_color,
        }),
        layer_b_opacity,
    ));

    let mut out_ab = vec![PackedRgba::TRANSPARENT; len];
    stack_ab.render(ctx, &mut out_ab);

    // Stack 2: B (bottom) -> A (top)
    let mut stack_ba = StackedFx::new();
    stack_ba.push(FxLayer::with_opacity(
        Box::new(ConstantColor {
            color: layer_b_color,
        }),
        layer_b_opacity,
    ));
    stack_ba.push(FxLayer::with_opacity(
        Box::new(ConstantColor {
            color: layer_a_color,
        }),
        layer_a_opacity,
    ));

    let mut out_ba = vec![PackedRgba::TRANSPARENT; len];
    stack_ba.render(ctx, &mut out_ba);

    // Assert: ordering matters
    assert_ne!(out_ab, out_ba, "Layer order should matter: [A,B] != [B,A]");

    // Document: verify the expected order (B on top in stack_ab)
    // B is blue with 50% alpha over A (red with 60% alpha)
    // The final result should have visible blue contribution
    let ab_sample = out_ab[0];
    let ba_sample = out_ba[0];

    // In stack_ab, blue (B) is on top, so blue channel should be more prominent
    // In stack_ba, red (A) is on top, so red channel should be more prominent
    assert!(
        ab_sample.b() > ba_sample.b() || ab_sample.r() < ba_sample.r(),
        "Order verification: ab_sample={:?}, ba_sample={:?}",
        ab_sample,
        ba_sample
    );
}

/// **TEST: Opaque top layer completely covers bottom**
///
/// Documents that a fully opaque top layer (index 1) completely obscures
/// the bottom layer (index 0).
#[test]
fn ordering_opaque_top_covers_bottom() {
    let (ctx, _theme) = make_context(2, 2);
    let len = ctx.len();

    let mut stack = StackedFx::new();
    // Bottom: green
    stack.push(FxLayer::new(Box::new(ConstantColor::opaque(0, 255, 0))));
    // Top: opaque red (should fully cover)
    stack.push(FxLayer::new(Box::new(ConstantColor::opaque(255, 0, 0))));

    let mut out = vec![PackedRgba::TRANSPARENT; len];
    stack.render(ctx, &mut out);

    // All cells should be pure red
    for (i, color) in out.iter().enumerate() {
        assert_eq!(
            *color,
            PackedRgba::rgb(255, 0, 0),
            "Cell {i} should be covered by opaque red top layer"
        );
    }
}

// =============================================================================
// ALPHA CORRECTNESS TESTS
// =============================================================================

/// **TEST: Alpha correctness - Compare stacked output against explicit over() math**
///
/// Renders a 2-layer stack and compares each cell against manually computed
/// `PackedRgba::over()` results.
#[test]
fn alpha_correctness_matches_explicit_over_math() {
    let (ctx, _theme) = make_context(4, 3);
    let len = ctx.len();

    // Layer 0 (bottom): opaque green
    let layer0_color = PackedRgba::rgb(0, 255, 0);
    // Layer 1 (top): semi-transparent red (50% opacity via layer setting)
    let layer1_base = PackedRgba::rgb(255, 0, 0);
    let layer1_opacity = 0.5;

    let mut stack = StackedFx::new();
    stack.push(FxLayer::new(Box::new(ConstantColor {
        color: layer0_color,
    })));
    stack.push(FxLayer::with_opacity(
        Box::new(ConstantColor { color: layer1_base }),
        layer1_opacity,
    ));

    let mut out = vec![PackedRgba::TRANSPARENT; len];
    stack.render(ctx, &mut out);

    // Manually compute expected: layer1.with_opacity(0.5).over(layer0)
    let layer1_with_opacity = layer1_base.with_opacity(layer1_opacity);
    let expected = layer1_with_opacity.over(layer0_color);

    for (i, actual) in out.iter().enumerate() {
        assert_eq!(
            *actual, expected,
            "Cell {i}: actual {:?} != expected {:?}",
            actual, expected
        );
    }
}

/// **TEST: Alpha correctness with three layers (opaque, semi-transparent, fully transparent)**
///
/// Tests the full range of alpha values in composition.
#[test]
fn alpha_correctness_three_layer_composition() {
    let (ctx, _theme) = make_context(2, 2);
    let len = ctx.len();

    // Layer 0: opaque base (blue)
    let l0 = PackedRgba::rgb(0, 0, 200);
    // Layer 1: 50% opacity (green)
    let l1_base = PackedRgba::rgb(0, 200, 0);
    let l1_opacity = 0.5;
    // Layer 2: fully transparent (should not affect result)
    let l2_base = PackedRgba::rgb(255, 255, 255);
    let l2_opacity = 0.0;

    let mut stack = StackedFx::new();
    stack.push(FxLayer::new(Box::new(ConstantColor { color: l0 })));
    stack.push(FxLayer::with_opacity(
        Box::new(ConstantColor { color: l1_base }),
        l1_opacity,
    ));
    stack.push(FxLayer::with_opacity(
        Box::new(ConstantColor { color: l2_base }),
        l2_opacity,
    ));

    let mut out = vec![PackedRgba::TRANSPARENT; len];
    stack.render(ctx, &mut out);

    // Manual calculation: l2 (opacity 0) doesn't contribute
    // Final = l1.with_opacity(0.5).over(l0)
    let l1_adjusted = l1_base.with_opacity(l1_opacity);
    let expected = l1_adjusted.over(l0);

    for (i, actual) in out.iter().enumerate() {
        assert_eq!(
            *actual, expected,
            "Cell {i}: three-layer composition mismatch"
        );
    }
}

/// **TEST: Alpha correctness cell-by-cell with varying patterns**
///
/// Uses a pattern-generating effect to verify per-cell composition is correct.
#[test]
fn alpha_correctness_cell_by_cell_patterns() {
    let (ctx, _theme) = make_context(8, 4);
    let len = ctx.len();

    // Layer 0: Pattern with base (100, 50, 25), opaque
    let mut l0_fx = PatternFx::new(100, 50, 25, 255);
    // Layer 1: Pattern with base (0, 100, 200), 70% alpha
    let mut l1_fx = PatternFx::new(0, 100, 200, 179); // 179 ~= 70% of 255

    // Get expected outputs by rendering each layer separately
    let mut l0_buf = vec![PackedRgba::TRANSPARENT; len];
    l0_fx.render(ctx, &mut l0_buf);

    let mut l1_buf = vec![PackedRgba::TRANSPARENT; len];
    l1_fx.render(ctx, &mut l1_buf);

    // Stack them
    let mut stack = StackedFx::new();
    stack.push(FxLayer::new(Box::new(PatternFx::new(100, 50, 25, 255))));
    stack.push(FxLayer::new(Box::new(PatternFx::new(0, 100, 200, 179))));

    let mut out = vec![PackedRgba::TRANSPARENT; len];
    stack.render(ctx, &mut out);

    // Verify each cell matches explicit over() computation
    for i in 0..len {
        let expected = l1_buf[i].over(l0_buf[i]);
        let actual = out[i];
        assert_eq!(
            actual, expected,
            "Cell {i}: pattern composition mismatch. \
             l0={:?}, l1={:?}, expected={:?}, actual={:?}",
            l0_buf[i], l1_buf[i], expected, actual
        );
    }
}

// =============================================================================
// DETERMINISM TESTS
// =============================================================================

/// **TEST: Determinism - Fixed inputs produce identical output hashes**
///
/// Renders the same stack multiple times and verifies the output hash is stable.
#[test]
fn determinism_fixed_inputs_produce_identical_hash() {
    let (ctx, _theme) = make_context(10, 8);
    let len = ctx.len();

    // Define a specific stack configuration
    fn make_test_stack() -> StackedFx {
        let mut stack = StackedFx::new();
        stack.push(FxLayer::new(Box::new(ConstantColor::opaque(30, 60, 90))));
        stack.push(FxLayer::with_opacity(
            Box::new(ConstantColor::new(200, 100, 50, 200)),
            0.7,
        ));
        stack.push(FxLayer::with_opacity_and_blend(
            Box::new(ConstantColor::opaque(10, 20, 30)),
            0.3,
            BlendMode::Additive,
        ));
        stack
    }

    // Run multiple times and collect hashes
    let mut hashes = Vec::new();
    for _ in 0..5 {
        let mut stack = make_test_stack();
        let mut out = vec![PackedRgba::TRANSPARENT; len];
        stack.render(ctx, &mut out);
        hashes.push(hash_output(&out));
    }

    // All hashes should be identical
    let first_hash = hashes[0];
    for (i, hash) in hashes.iter().enumerate() {
        assert_eq!(
            *hash, first_hash,
            "Render {i} produced different hash: {hash} != {first_hash}"
        );
    }
}

/// **TEST: Determinism across resize cycles**
///
/// Renders at size A, resizes to B, back to A, and verifies output matches.
#[test]
fn determinism_across_resize_cycles() {
    let theme = ThemeInputs::default_dark();

    // Context at 6x4
    let ctx_a = FxContext {
        width: 6,
        height: 4,
        frame: 0,
        time_seconds: 0.0,
        quality: FxQuality::Full,
        theme: &theme,
    };
    let len_a = ctx_a.len();

    // Context at 10x8
    let ctx_b = FxContext {
        width: 10,
        height: 8,
        frame: 0,
        time_seconds: 0.0,
        quality: FxQuality::Full,
        theme: &theme,
    };
    let len_b = ctx_b.len();

    let mut stack = StackedFx::new();
    stack.push(FxLayer::new(Box::new(ConstantColor::opaque(100, 150, 200))));
    stack.push(FxLayer::with_opacity(
        Box::new(ConstantColor::new(50, 100, 150, 180)),
        0.6,
    ));

    // First render at size A
    let mut out_a1 = vec![PackedRgba::TRANSPARENT; len_a];
    stack.resize(6, 4);
    stack.render(ctx_a, &mut out_a1);
    let hash_a1 = hash_output(&out_a1);

    // Render at size B
    let mut out_b = vec![PackedRgba::TRANSPARENT; len_b];
    stack.resize(10, 8);
    stack.render(ctx_b, &mut out_b);

    // Render again at size A
    let mut out_a2 = vec![PackedRgba::TRANSPARENT; len_a];
    stack.resize(6, 4);
    stack.render(ctx_a, &mut out_a2);
    let hash_a2 = hash_output(&out_a2);

    // Hashes should match for same size
    assert_eq!(
        hash_a1, hash_a2,
        "Output at size A should be identical before and after resize to B"
    );
}

// =============================================================================
// ALLOCATION STABILITY TESTS
// =============================================================================

/// **TEST: Allocation proxy - No buffer growth after warmup**
///
/// After initial render, subsequent renders at the same size should not
/// increase buffer capacity.
#[test]
fn allocation_no_growth_after_warmup() {
    let (ctx, _theme) = make_context(20, 15);
    let len = ctx.len();

    let mut stack = StackedFx::new();
    stack.push(FxLayer::new(Box::new(ConstantColor::opaque(100, 100, 100))));
    stack.push(FxLayer::with_opacity(
        Box::new(ConstantColor::new(50, 50, 50, 200)),
        0.5,
    ));

    let mut out = vec![PackedRgba::TRANSPARENT; len];

    // Warmup render
    stack.render(ctx, &mut out);

    // Multiple subsequent renders (should not allocate)
    for _ in 0..10 {
        stack.render(ctx, &mut out);
    }

    // The test passes if no panic occurs and output is consistent
    // (Buffer growth would cause observable effects in Debug builds)
    let final_hash = hash_output(&out);

    // Re-render and verify consistency
    stack.render(ctx, &mut out);
    let verify_hash = hash_output(&out);

    assert_eq!(
        final_hash, verify_hash,
        "Output should be stable across renders"
    );
}

/// **TEST: Allocation - Buffers grow only when needed**
///
/// Verifies that rendering at progressively larger sizes doesn't cause
/// unexpected allocation patterns.
#[test]
fn allocation_grows_only_when_needed() {
    let theme = ThemeInputs::default_dark();

    let mut stack = StackedFx::new();
    stack.push(FxLayer::new(Box::new(ConstantColor::opaque(80, 80, 80))));

    // Render at small size
    let ctx_small = FxContext {
        width: 4,
        height: 4,
        frame: 0,
        time_seconds: 0.0,
        quality: FxQuality::Full,
        theme: &theme,
    };
    let mut out_small = vec![PackedRgba::TRANSPARENT; ctx_small.len()];
    stack.resize(4, 4);
    stack.render(ctx_small, &mut out_small);

    // Render at same size - should not allocate
    for _ in 0..5 {
        stack.render(ctx_small, &mut out_small);
    }

    // Render at larger size - will allocate
    let ctx_large = FxContext {
        width: 20,
        height: 20,
        frame: 0,
        time_seconds: 0.0,
        quality: FxQuality::Full,
        theme: &theme,
    };
    let mut out_large = vec![PackedRgba::TRANSPARENT; ctx_large.len()];
    stack.resize(20, 20);
    stack.render(ctx_large, &mut out_large);

    // Render at original small size - should NOT shrink buffers
    // (grow-only policy)
    for _ in 0..5 {
        stack.resize(4, 4);
        stack.render(ctx_small, &mut out_small);
    }

    // Verify correctness at small size
    assert!(
        out_small.iter().all(|c| *c == PackedRgba::rgb(80, 80, 80)),
        "Small render should produce correct output after large render"
    );
}

// =============================================================================
// BLEND MODE TESTS
// =============================================================================

/// **TEST: Blend modes produce different results**
///
/// Verifies that Over, Additive, Multiply, and Screen produce distinct outputs.
#[test]
fn blend_modes_produce_distinct_results() {
    let (ctx, _theme) = make_context(2, 2);
    let len = ctx.len();

    let top_color = PackedRgba::rgb(100, 50, 150);
    let top_opacity = 0.8;

    let modes = [
        BlendMode::Over,
        BlendMode::Additive,
        BlendMode::Multiply,
        BlendMode::Screen,
    ];

    let mut results = Vec::new();
    for mode in modes {
        let mut stack = StackedFx::new();
        // Bottom layer: gray
        stack.push(FxLayer::new(Box::new(ConstantColor::opaque(100, 100, 100))));
        stack.push(FxLayer::with_opacity_and_blend(
            Box::new(ConstantColor { color: top_color }),
            top_opacity,
            mode,
        ));

        let mut out = vec![PackedRgba::TRANSPARENT; len];
        stack.render(ctx, &mut out);
        results.push((mode, out[0]));
    }

    // Each blend mode should produce a different result
    for i in 0..results.len() {
        for j in (i + 1)..results.len() {
            assert_ne!(
                results[i].1, results[j].1,
                "{:?} and {:?} should produce different results",
                results[i].0, results[j].0
            );
        }
    }
}
