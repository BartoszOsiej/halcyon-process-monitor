#![forbid(unsafe_code)]

//! Ready-to-use animation presets built from core primitives.
//!
//! Each preset composes [`Fade`], [`Slide`], `Delayed`, `AnimationGroup`,
//! and [`stagger_offsets`] into common UI animation patterns. All presets
//! return concrete types that implement [`Animation`].
//!
//! # Available Presets
//!
//! | Preset | Description |
//! |--------|-------------|
//! | [`cascade_in`] | Staggered top-to-bottom fade-in |
//! | [`cascade_out`] | Staggered top-to-bottom fade-out (reverse values) |
//! | [`fan_out`] | Spread from center outward |
//! | [`typewriter`] | Character-by-character reveal |
//! | [`pulse_sequence`] | Sequential attention pulses |
//! | [`slide_in_left`] | Slide element in from left edge |
//! | [`slide_in_right`] | Slide element in from right edge |
//! | [`fade_through`] | Fade out then fade in (crossfade) |
//!
//! # Invariants
//!
//! 1. All presets produce deterministic output for given parameters.
//! 2. Preset groups with `count=0` return an empty group (immediately complete).
//! 3. Timing parameters are clamped to sane minimums (1ns) to avoid division by zero.
//! 4. All presets compose only public primitives — no internal shortcuts.

use std::time::Duration;

use super::group::AnimationGroup;
use super::stagger::{StaggerMode, stagger_offsets};
use super::{Animation, EasingFn, Fade, Sequence, Slide, delay, ease_in, ease_out, sequence};

// ---------------------------------------------------------------------------
// Cascade
// ---------------------------------------------------------------------------

/// Staggered top-to-bottom fade-in for `count` items.
///
/// Each item fades in over `item_duration` with a `stagger_delay` between
/// successive starts. The stagger follows `mode` distribution.
///
/// Returns an [`AnimationGroup`] with labels `"item_0"`, `"item_1"`, etc.
#[must_use]
pub fn cascade_in(
    count: usize,
    item_duration: Duration,
    stagger_delay: Duration,
    mode: StaggerMode,
) -> AnimationGroup {
    let offsets = stagger_offsets(count, stagger_delay, mode);
    let mut group = AnimationGroup::new();
    for (i, offset) in offsets.into_iter().enumerate() {
        let anim = delay(offset, Fade::new(item_duration).easing(ease_out));
        group.insert(&format!("item_{i}"), Box::new(anim));
    }
    group
}

/// Staggered top-to-bottom fade-out for `count` items.
///
/// Like [`cascade_in`] but each item's value starts at 1.0 and decreases to 0.0.
/// Internally uses a [`Fade`] whose value is inverted: `1.0 - fade.value()`.
#[must_use]
pub fn cascade_out(
    count: usize,
    item_duration: Duration,
    stagger_delay: Duration,
    mode: StaggerMode,
) -> AnimationGroup {
    let offsets = stagger_offsets(count, stagger_delay, mode);
    let mut group = AnimationGroup::new();
    for (i, offset) in offsets.into_iter().enumerate() {
        let anim = delay(
            offset,
            InvertedFade(Fade::new(item_duration).easing(ease_in)),
        );
        group.insert(&format!("item_{i}"), Box::new(anim));
    }
    group
}

// ---------------------------------------------------------------------------
// Fan out
// ---------------------------------------------------------------------------

/// Spread animation from center outward.
///
/// Items near the center start first; items at the edges start last.
/// Uses ease-out staggering so the spread accelerates outward.
///
/// Returns an [`AnimationGroup`] with labels `"item_0"` .. `"item_{count-1}"`.
#[must_use]
pub fn fan_out(count: usize, item_duration: Duration, total_spread: Duration) -> AnimationGroup {
    if count == 0 {
        return AnimationGroup::new();
    }

    let mut group = AnimationGroup::new();

    for i in 0..count {
        // Distance from center, normalized to [0.0, 1.0].
        let center = (count as f64 - 1.0) / 2.0;
        let dist = if count <= 1 {
            0.0
        } else {
            ((i as f64 - center).abs() / center).min(1.0)
        };

        // Apply ease-out to the distance for natural spread feel.
        let eased = 1.0 - (1.0 - dist) * (1.0 - dist);
        let offset = total_spread.mul_f64(eased);

        let anim = delay(offset, Fade::new(item_duration).easing(ease_out));
        group.insert(&format!("item_{i}"), Box::new(anim));
    }

    group
}

// ---------------------------------------------------------------------------
// Typewriter
// ---------------------------------------------------------------------------

/// Character-by-character reveal over `total_duration`.
///
/// Returns a [`TypewriterAnim`] that tracks how many characters out of
/// `char_count` should be visible at the current time.
#[must_use]
pub fn typewriter(char_count: usize, total_duration: Duration) -> TypewriterAnim {
    TypewriterAnim {
        char_count,
        fade: Fade::new(total_duration),
    }
}

/// Animation that reveals characters progressively.
///
/// Use [`TypewriterAnim::visible_chars`] to get the count of characters
/// that should be displayed at the current time.
#[derive(Debug, Clone)]
pub struct TypewriterAnim {
    char_count: usize,
    fade: Fade,
}

impl TypewriterAnim {
    /// Number of characters that should be visible now.
    pub fn visible_chars(&self) -> usize {
        let t = self.fade.value();
        let count = (t * self.char_count as f32).round() as usize;
        count.min(self.char_count)
    }
}

impl Animation for TypewriterAnim {
    fn tick(&mut self, dt: Duration) {
        self.fade.tick(dt);
    }

    fn is_complete(&self) -> bool {
        self.fade.is_complete()
    }

    fn value(&self) -> f32 {
        self.fade.value()
    }

    fn reset(&mut self) {
        self.fade.reset();
    }

    fn overshoot(&self) -> Duration {
        self.fade.overshoot()
    }
}

// ---------------------------------------------------------------------------
// Pulse sequence
// ---------------------------------------------------------------------------

/// Sequential attention pulses: each item pulses once then the next starts.
///
/// Each pulse is a single sine half-cycle (0→1→0) lasting `pulse_duration`,
/// with items staggered linearly. Useful for drawing attention to a sequence
/// of UI elements.
///
/// Returns an [`AnimationGroup`] with labels `"pulse_0"` .. `"pulse_{count-1}"`.
#[must_use]
pub fn pulse_sequence(
    count: usize,
    pulse_duration: Duration,
    stagger_delay: Duration,
) -> AnimationGroup {
    let offsets = stagger_offsets(count, stagger_delay, StaggerMode::Linear);
    let mut group = AnimationGroup::new();
    for (i, offset) in offsets.into_iter().enumerate() {
        let anim = delay(offset, PulseOnce::new(pulse_duration));
        group.insert(&format!("pulse_{i}"), Box::new(anim));
    }
    group
}

/// A single pulse: ramps 0→1→0 over the duration using a sine half-cycle.
#[derive(Debug, Clone, Copy)]
struct PulseOnce {
    elapsed: Duration,
    duration: Duration,
}

impl PulseOnce {
    fn new(duration: Duration) -> Self {
        Self {
            elapsed: Duration::ZERO,
            duration: if duration.is_zero() {
                Duration::from_nanos(1)
            } else {
                duration
            },
        }
    }
}

impl Animation for PulseOnce {
    fn tick(&mut self, dt: Duration) {
        self.elapsed = self.elapsed.saturating_add(dt);
    }

    fn is_complete(&self) -> bool {
        self.elapsed >= self.duration
    }

    fn value(&self) -> f32 {
        let t = (self.elapsed.as_secs_f64() / self.duration.as_secs_f64()).min(1.0) as f32;
        (t * std::f32::consts::PI).sin()
    }

    fn reset(&mut self) {
        self.elapsed = Duration::ZERO;
    }

    fn overshoot(&self) -> Duration {
        self.elapsed.saturating_sub(self.duration)
    }
}

// ---------------------------------------------------------------------------
// Slide presets
// ---------------------------------------------------------------------------

/// Slide an element in from the left edge.
///
/// `distance` is the starting offset in cells (positive = further left).
/// Element slides from `-distance` to `0` over `duration` with ease-out.
#[must_use]
pub fn slide_in_left(distance: i16, duration: Duration) -> Slide {
    Slide::new(-distance, 0, duration).easing(ease_out)
}

/// Slide an element in from the right edge.
///
/// `distance` is the starting offset in cells (positive = further right).
/// Element slides from `+distance` to `0` over `duration` with ease-out.
#[must_use]
pub fn slide_in_right(distance: i16, duration: Duration) -> Slide {
    Slide::new(distance, 0, duration).easing(ease_out)
}

// ---------------------------------------------------------------------------
// Fade-through (crossfade)
// ---------------------------------------------------------------------------

/// Fade out then fade in, useful for content transitions.
///
/// Total animation is `2 * half_duration`. During the first half, value goes
/// from 1.0 to 0.0 (fade out). During the second half, 0.0 to 1.0 (fade in).
#[must_use]
pub fn fade_through(half_duration: Duration) -> Sequence<InvertedFade, Fade> {
    let out = InvertedFade(Fade::new(half_duration).easing(ease_in));
    let into = Fade::new(half_duration).easing(ease_out);
    sequence(out, into)
}

// ---------------------------------------------------------------------------
// InvertedFade helper
// ---------------------------------------------------------------------------

/// A fade animation whose value is inverted: starts at 1.0, ends at 0.0.
#[derive(Debug, Clone, Copy)]
pub struct InvertedFade(Fade);

impl InvertedFade {
    /// Create an inverted fade (1.0 → 0.0) with the given duration.
    pub fn new(duration: Duration) -> Self {
        Self(Fade::new(duration))
    }

    /// Set the easing function.
    pub fn easing(mut self, easing: EasingFn) -> Self {
        self.0 = self.0.easing(easing);
        self
    }
}

impl Animation for InvertedFade {
    fn tick(&mut self, dt: Duration) {
        self.0.tick(dt);
    }

    fn is_complete(&self) -> bool {
        self.0.is_complete()
    }

    fn value(&self) -> f32 {
        1.0 - self.0.value()
    }

    fn reset(&mut self) {
        self.0.reset();
    }

    fn overshoot(&self) -> Duration {
        self.0.overshoot()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    const MS50: Duration = Duration::from_millis(50);
    const MS100: Duration = Duration::from_millis(100);
    const MS200: Duration = Duration::from_millis(200);
    const MS500: Duration = Duration::from_millis(500);

    // ---- cascade_in -------------------------------------------------------

    #[test]
    fn cascade_in_empty() {
        let group = cascade_in(0, MS200, MS50, StaggerMode::Linear);
        assert!(group.is_empty());
        assert!(group.all_complete());
    }

    #[test]
    fn cascade_in_single_item() {
        let mut group = cascade_in(1, MS200, MS50, StaggerMode::Linear);
        assert_eq!(group.len(), 1);
        assert!(!group.all_complete());

        // Tick past item duration.
        group.tick(MS200);
        assert!(group.all_complete());
    }

    #[test]
    fn cascade_in_multiple_items_staggered() {
        let mut group = cascade_in(3, MS200, MS100, StaggerMode::Linear);
        assert_eq!(group.len(), 3);

        // At t=0, only item_0 has started.
        assert!(group.get("item_0").unwrap().value() == 0.0);

        // Tick 100ms: item_0 is halfway, item_1 just started, item_2 not yet.
        group.tick(MS100);
        let v0 = group.get("item_0").unwrap().value();
        let v1 = group.get("item_1").unwrap().value();
        let v2 = group.get("item_2").unwrap().value();
        assert!(v0 > 0.0, "item_0 should have progressed");
        assert!(v1 == 0.0, "item_1 just started (delay elapsed)");
        assert!(v2 == 0.0, "item_2 hasn't started yet");

        // Tick to 400ms: all should be complete.
        group.tick(Duration::from_millis(300));
        assert!(group.all_complete());
    }

    #[test]
    fn cascade_in_values_increase() {
        let mut group = cascade_in(5, MS500, MS100, StaggerMode::EaseOut);
        let mut prev = 0.0f32;
        for _ in 0..20 {
            group.tick(MS50);
            let val = group.overall_progress();
            assert!(val >= prev, "overall progress should not decrease");
            prev = val;
        }
    }

    // ---- cascade_out ------------------------------------------------------

    #[test]
    fn cascade_out_starts_near_one() {
        let mut group = cascade_out(3, MS200, MS50, StaggerMode::Linear);
        // Delayed wrapper needs a tick to start. After a tiny tick, item_0
        // (zero delay) should be near 1.0.
        group.tick(Duration::from_nanos(1));
        let v0 = group.get("item_0").unwrap().value();
        assert!(
            (v0 - 1.0).abs() < 0.01,
            "cascade_out should start near 1.0, got {v0}"
        );
    }

    #[test]
    fn cascade_out_ends_at_zero() {
        let mut group = cascade_out(3, MS200, MS50, StaggerMode::Linear);
        // Tick well past total duration.
        group.tick(Duration::from_secs(1));
        for i in 0..3 {
            let v = group.get(&format!("item_{i}")).unwrap().value();
            assert!(
                v < 0.01,
                "item_{i} should be near 0.0 after completion, got {v}"
            );
        }
    }

    // ---- fan_out ----------------------------------------------------------

    #[test]
    fn fan_out_empty() {
        let group = fan_out(0, MS200, MS200);
        assert!(group.is_empty());
    }

    #[test]
    fn fan_out_single() {
        let group = fan_out(1, MS200, MS200);
        assert_eq!(group.len(), 1);
        // Single item has zero distance from center, so no delay.
        let v = group.get("item_0").unwrap().value();
        assert!((v - 0.0).abs() < 0.01);
    }

    #[test]
    fn fan_out_center_starts_first() {
        let mut group = fan_out(5, MS200, MS200);
        // Tick just a tiny bit — center item should progress, edges should not.
        group.tick(Duration::from_millis(10));
        let center = group.get("item_2").unwrap().value();
        let edge = group.get("item_0").unwrap().value();
        assert!(
            center >= edge,
            "center ({center}) should start before edges ({edge})"
        );
    }

    #[test]
    fn fan_out_symmetric() {
        let group = fan_out(5, MS200, MS200);
        // Items equidistant from center should have the same initial value.
        let v0 = group.get("item_0").unwrap().value();
        let v4 = group.get("item_4").unwrap().value();
        assert!(
            (v0 - v4).abs() < 0.01,
            "symmetric items should match: {v0} vs {v4}"
        );

        let v1 = group.get("item_1").unwrap().value();
        let v3 = group.get("item_3").unwrap().value();
        assert!(
            (v1 - v3).abs() < 0.01,
            "symmetric items should match: {v1} vs {v3}"
        );
    }

    // ---- typewriter -------------------------------------------------------

    #[test]
    fn typewriter_starts_at_zero() {
        let tw = typewriter(100, MS500);
        assert_eq!(tw.visible_chars(), 0);
    }

    #[test]
    fn typewriter_ends_at_full() {
        let mut tw = typewriter(100, MS500);
        tw.tick(MS500);
        assert_eq!(tw.visible_chars(), 100);
        assert!(tw.is_complete());
    }

    #[test]
    fn typewriter_progresses_monotonically() {
        let mut tw = typewriter(50, MS500);
        let mut prev = 0;
        for _ in 0..20 {
            tw.tick(Duration::from_millis(25));
            let chars = tw.visible_chars();
            assert!(
                chars >= prev,
                "visible chars should not decrease: {prev} -> {chars}"
            );
            prev = chars;
        }
    }

    #[test]
    fn typewriter_zero_chars() {
        let mut tw = typewriter(0, MS200);
        assert_eq!(tw.visible_chars(), 0);
        tw.tick(MS200);
        assert_eq!(tw.visible_chars(), 0);
        assert!(tw.is_complete());
    }

    // ---- pulse_sequence ---------------------------------------------------

    #[test]
    fn pulse_sequence_empty() {
        let group = pulse_sequence(0, MS200, MS100);
        assert!(group.is_empty());
    }

    #[test]
    fn pulse_sequence_peaks_then_returns() {
        let mut group = pulse_sequence(1, MS200, MS100);
        // At start: 0.
        assert!(group.get("pulse_0").unwrap().value() < 0.01);

        // At midpoint: should be near peak.
        group.tick(MS100);
        let mid = group.get("pulse_0").unwrap().value();
        assert!(mid > 0.9, "pulse midpoint should be near 1.0, got {mid}");

        // At end: should return near 0.
        group.tick(MS100);
        let end = group.get("pulse_0").unwrap().value();
        assert!(end < 0.1, "pulse end should be near 0.0, got {end}");
    }

    #[test]
    fn pulse_sequence_items_staggered() {
        let mut group = pulse_sequence(3, MS200, MS200);
        // At t=100ms: pulse_0 is at peak, pulse_1 hasn't started.
        group.tick(MS100);
        let p0 = group.get("pulse_0").unwrap().value();
        let p1 = group.get("pulse_1").unwrap().value();
        assert!(p0 > 0.9, "pulse_0 should be at peak");
        assert!(p1 < 0.01, "pulse_1 should not have started");
    }

    // ---- slide presets ----------------------------------------------------

    #[test]
    fn slide_in_left_starts_offscreen() {
        let slide = slide_in_left(20, MS200);
        assert_eq!(slide.position(), -20);
    }

    #[test]
    fn slide_in_left_ends_at_zero() {
        let mut slide = slide_in_left(20, MS200);
        slide.tick(MS200);
        assert_eq!(slide.position(), 0);
        assert!(slide.is_complete());
    }

    #[test]
    fn slide_in_right_starts_offscreen() {
        let slide = slide_in_right(20, MS200);
        assert_eq!(slide.position(), 20);
    }

    #[test]
    fn slide_in_right_ends_at_zero() {
        let mut slide = slide_in_right(20, MS200);
        slide.tick(MS200);
        assert_eq!(slide.position(), 0);
    }

    // ---- fade_through -----------------------------------------------------

    #[test]
    fn fade_through_starts_at_one() {
        let ft = fade_through(MS200);
        assert!((ft.value() - 1.0).abs() < 0.01, "should start at 1.0");
    }

    #[test]
    fn fade_through_midpoint_near_zero() {
        let mut ft = fade_through(MS200);
        ft.tick(MS200);
        // At half_duration, the first (inverted) fade is complete → value ≈ 0.
        // The second fade just started → value ≈ 0.
        assert!(
            ft.value() < 0.1,
            "midpoint should be near 0.0, got {}",
            ft.value()
        );
    }

    #[test]
    fn fade_through_ends_at_one() {
        let mut ft = fade_through(MS200);
        ft.tick(Duration::from_millis(400));
        assert!(ft.is_complete());
        assert!(
            (ft.value() - 1.0).abs() < 0.01,
            "should end at 1.0, got {}",
            ft.value()
        );
    }

    // ---- InvertedFade -----------------------------------------------------

    #[test]
    fn inverted_fade_starts_at_one() {
        let f = InvertedFade::new(MS200);
        assert!((f.value() - 1.0).abs() < 0.001);
    }

    #[test]
    fn inverted_fade_ends_at_zero() {
        let mut f = InvertedFade::new(MS200);
        f.tick(MS200);
        assert!(f.value() < 0.001);
        assert!(f.is_complete());
    }

    #[test]
    fn inverted_fade_reset() {
        let mut f = InvertedFade::new(MS200);
        f.tick(MS200);
        assert!(f.is_complete());
        f.reset();
        assert!(!f.is_complete());
        assert!((f.value() - 1.0).abs() < 0.001);
    }

    // ---- determinism ------------------------------------------------------

    #[test]
    fn cascade_in_deterministic() {
        let run = || {
            let mut group = cascade_in(5, MS200, MS50, StaggerMode::EaseInOut);
            let mut values = Vec::new();
            for _ in 0..10 {
                group.tick(MS50);
                values.push(group.overall_progress());
            }
            values
        };
        assert_eq!(run(), run(), "cascade_in must be deterministic");
    }

    #[test]
    fn typewriter_deterministic() {
        let run = || {
            let mut tw = typewriter(100, MS500);
            let mut counts = Vec::new();
            for _ in 0..20 {
                tw.tick(Duration::from_millis(25));
                counts.push(tw.visible_chars());
            }
            counts
        };
        assert_eq!(run(), run(), "typewriter must be deterministic");
    }

    #[test]
    fn fan_out_deterministic() {
        let run = || {
            let mut group = fan_out(7, MS200, MS200);
            let mut values = Vec::new();
            for _ in 0..10 {
                group.tick(MS50);
                values.push(group.overall_progress());
            }
            values
        };
        assert_eq!(run(), run(), "fan_out must be deterministic");
    }
}
