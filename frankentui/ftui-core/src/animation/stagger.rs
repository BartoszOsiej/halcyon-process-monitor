#![forbid(unsafe_code)]

//! Stagger utilities: coordinated delay offsets for animation lists.
//!
//! `stagger` produces delay offsets for a sequence of items, useful for
//! cascading entrance/exit animations. Combined with `Delayed` and
//! `AnimationGroup`, this enables staggered list-item animations.
//!
//! # Usage
//!
//! ```ignore
//! use std::time::Duration;
//! use ftui_core::animation::{Fade, AnimationGroup, stagger};
//! use ftui_core::animation::stagger::{StaggerMode, stagger_offsets};
//!
//! let offsets = stagger_offsets(5, Duration::from_millis(50), StaggerMode::Linear);
//! let mut group = AnimationGroup::new();
//! for (i, offset) in offsets.into_iter().enumerate() {
//!     let anim = ftui_core::animation::delay(offset, Fade::new(Duration::from_millis(200)));
//!     group.insert(&format!("item_{i}"), Box::new(anim));
//! }
//! ```
//!
//! # Invariants
//!
//! 1. `stagger_offsets(0, ..)` returns an empty vec.
//! 2. First offset is always `Duration::ZERO`.
//! 3. Offsets are monotonically non-decreasing for all modes except `Random`.
//! 4. For `Linear`, `offset[i] = i * delay`.
//! 5. For `EaseIn`/`EaseOut`/`EaseInOut`, offsets follow the corresponding
//!    easing curve scaled to `(count - 1) * delay`.
//! 6. For `Random`, offsets are within `[0, (count - 1) * delay]` with
//!    deterministic seeding when a seed is provided.
//!
//! # Failure Modes
//!
//! - Zero count: returns empty vec.
//! - Count of 1: returns `[Duration::ZERO]`.
//! - Zero delay: all offsets are `Duration::ZERO`.

use std::time::Duration;

use super::{EasingFn, ease_in, ease_in_out, ease_out};

const NANOS_PER_SEC: u128 = 1_000_000_000;

#[must_use]
fn duration_from_nanos_u128(nanos: u128) -> Duration {
    let secs_u128 = nanos / NANOS_PER_SEC;
    let sub_nanos = (nanos % NANOS_PER_SEC) as u32;

    match u64::try_from(secs_u128) {
        Ok(secs) => Duration::new(secs, sub_nanos),
        Err(_) => {
            // Saturate to Duration::MAX.
            Duration::new(u64::MAX, (NANOS_PER_SEC - 1) as u32)
        }
    }
}

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// How to distribute delay offsets across items.
#[derive(Debug, Clone, Copy)]
pub enum StaggerMode {
    /// Equal spacing: `offset[i] = i * delay`.
    Linear,
    /// Slow start, accelerating gaps (quadratic ease-in).
    EaseIn,
    /// Fast start, decelerating gaps (quadratic ease-out).
    EaseOut,
    /// Slow start and end, faster middle (quadratic ease-in-out).
    EaseInOut,
    /// Custom easing function applied to normalized position.
    Custom(EasingFn),
}

// ---------------------------------------------------------------------------
// Core function
// ---------------------------------------------------------------------------

/// Compute stagger delay offsets for `count` items.
///
/// Each offset represents when that item's animation should begin.
/// The first item always starts at `Duration::ZERO`, and the last item
/// starts at `(count - 1) * delay` (for `Linear` mode).
///
/// For eased modes, the total span is `(count - 1) * delay` but the
/// distribution follows the easing curve.
#[must_use]
pub fn stagger_offsets(count: usize, delay: Duration, mode: StaggerMode) -> Vec<Duration> {
    if count == 0 {
        return Vec::new();
    }
    if count == 1 {
        return vec![Duration::ZERO];
    }

    match mode {
        // For Linear mode, use exact integer arithmetic to avoid float drift.
        StaggerMode::Linear => {
            let mut offsets = Vec::with_capacity(count);
            let mut current = Duration::ZERO;
            for _ in 0..count {
                offsets.push(current);
                current = current.saturating_add(delay);
            }
            offsets
        }
        StaggerMode::EaseIn => stagger_offsets(count, delay, StaggerMode::Custom(ease_in)),
        StaggerMode::EaseOut => stagger_offsets(count, delay, StaggerMode::Custom(ease_out)),
        StaggerMode::EaseInOut => stagger_offsets(count, delay, StaggerMode::Custom(ease_in_out)),
        StaggerMode::Custom(easing) => {
            let total_nanos = delay.as_nanos() as f64 * (count - 1) as f64;
            (0..count)
                .map(|i| {
                    let t = i as f32 / (count - 1) as f32;
                    let eased = easing(t);
                    let nanos = (total_nanos * eased as f64) as u128;
                    duration_from_nanos_u128(nanos)
                })
                .collect()
        }
    }
}

/// Compute stagger offsets with random jitter.
///
/// Each offset gets a random perturbation in `[-jitter, +jitter]`,
/// clamped to `Duration::ZERO` at the lower bound. Uses a simple
/// deterministic PRNG seeded from `seed` for reproducibility.
///
/// The base offsets follow `mode` before jitter is applied.
#[must_use]
pub fn stagger_offsets_with_jitter(
    count: usize,
    delay: Duration,
    mode: StaggerMode,
    jitter: Duration,
    seed: u64,
) -> Vec<Duration> {
    let mut offsets = stagger_offsets(count, delay, mode);
    if jitter.is_zero() || offsets.is_empty() {
        return offsets;
    }

    // Simple xorshift64 PRNG for deterministic jitter.
    let mut state = seed.wrapping_add(1); // Avoid 0 state.
    // Clamp to avoid overflow in span computation and signed arithmetic.
    let max_jitter = ((i128::MAX as u128) - 1) / 2;
    let jitter_nanos = jitter.as_nanos().min(max_jitter) as i128;
    let span = (jitter_nanos as u128).saturating_mul(2).saturating_add(1);

    for offset in &mut offsets {
        // xorshift64
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;

        // Map to [-jitter_nanos, +jitter_nanos]
        let r = (state as u128) % span; // [0, span)
        let raw = r as i128 - jitter_nanos; // [-jitter_nanos, +jitter_nanos]
        let base_nanos = offset.as_nanos().min(i128::MAX as u128) as i128;
        let jittered_nanos = base_nanos.saturating_add(raw).max(0) as u128;
        *offset = duration_from_nanos_u128(jittered_nanos);
    }

    offsets
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    const MS_50: Duration = Duration::from_millis(50);
    const MS_100: Duration = Duration::from_millis(100);

    #[test]
    fn zero_count_returns_empty() {
        let offsets = stagger_offsets(0, MS_100, StaggerMode::Linear);
        assert!(offsets.is_empty());
    }

    #[test]
    fn single_item_returns_zero() {
        let offsets = stagger_offsets(1, MS_100, StaggerMode::Linear);
        assert_eq!(offsets, vec![Duration::ZERO]);
    }

    #[test]
    fn linear_equal_spacing() {
        let offsets = stagger_offsets(4, MS_50, StaggerMode::Linear);
        assert_eq!(offsets.len(), 4);
        assert_eq!(offsets[0], Duration::ZERO);
        assert_eq!(offsets[1], MS_50);
        assert_eq!(offsets[2], Duration::from_millis(100));
        assert_eq!(offsets[3], Duration::from_millis(150));
    }

    #[test]
    fn linear_first_is_zero_last_is_total() {
        let offsets = stagger_offsets(5, MS_100, StaggerMode::Linear);
        assert_eq!(offsets[0], Duration::ZERO);
        assert_eq!(offsets[4], Duration::from_millis(400));
    }

    #[test]
    fn ease_in_first_is_zero() {
        let offsets = stagger_offsets(5, MS_100, StaggerMode::EaseIn);
        assert_eq!(offsets[0], Duration::ZERO);
    }

    #[test]
    fn ease_in_gaps_increase() {
        let offsets = stagger_offsets(5, MS_100, StaggerMode::EaseIn);
        // Gaps between consecutive items should increase
        let gaps: Vec<Duration> = offsets.windows(2).map(|w| w[1] - w[0]).collect();
        for i in 1..gaps.len() {
            assert!(
                gaps[i] >= gaps[i - 1],
                "ease_in gaps should increase: {:?}",
                gaps
            );
        }
    }

    #[test]
    fn ease_out_gaps_decrease() {
        let offsets = stagger_offsets(5, MS_100, StaggerMode::EaseOut);
        let gaps: Vec<Duration> = offsets.windows(2).map(|w| w[1] - w[0]).collect();
        for i in 1..gaps.len() {
            assert!(
                gaps[i] <= gaps[i - 1],
                "ease_out gaps should decrease: {:?}",
                gaps
            );
        }
    }

    #[test]
    fn ease_in_out_symmetric() {
        let offsets = stagger_offsets(5, MS_100, StaggerMode::EaseInOut);
        assert_eq!(offsets[0], Duration::ZERO);
        // Middle offset should be near half the total span
        let total = Duration::from_millis(400);
        let mid = offsets[2];
        let diff = mid.abs_diff(total / 2);
        assert!(
            diff < Duration::from_millis(10),
            "middle should be near half"
        );
    }

    #[test]
    fn custom_easing() {
        // Use a custom "jump to end" easing
        let offsets = stagger_offsets(
            3,
            MS_100,
            StaggerMode::Custom(|t| if t > 0.0 { 1.0 } else { 0.0 }),
        );
        assert_eq!(offsets[0], Duration::ZERO);
        assert_eq!(offsets[1], Duration::from_millis(200)); // easing(0.5) = 1.0
        assert_eq!(offsets[2], Duration::from_millis(200)); // easing(1.0) = 1.0
    }

    #[test]
    fn zero_delay_all_zero() {
        let offsets = stagger_offsets(5, Duration::ZERO, StaggerMode::Linear);
        assert!(offsets.iter().all(|d| *d == Duration::ZERO));
    }

    #[test]
    fn jitter_deterministic_with_same_seed() {
        let a = stagger_offsets_with_jitter(5, MS_100, StaggerMode::Linear, MS_50, 42);
        let b = stagger_offsets_with_jitter(5, MS_100, StaggerMode::Linear, MS_50, 42);
        assert_eq!(a, b);
    }

    #[test]
    fn jitter_different_with_different_seed() {
        let a = stagger_offsets_with_jitter(5, MS_100, StaggerMode::Linear, MS_50, 42);
        let b = stagger_offsets_with_jitter(5, MS_100, StaggerMode::Linear, MS_50, 99);
        // Very unlikely to be identical
        assert_ne!(a, b);
    }

    #[test]
    fn jitter_offsets_non_negative() {
        let offsets = stagger_offsets_with_jitter(
            10,
            MS_50,
            StaggerMode::Linear,
            Duration::from_millis(200),
            12345,
        );
        for offset in &offsets {
            assert!(*offset >= Duration::ZERO);
        }
    }

    #[test]
    fn jitter_offsets_within_bounds() {
        let count = 50;
        let delay = MS_100;
        let jitter = MS_50;

        let base = stagger_offsets(count, delay, StaggerMode::Linear);
        let jittered = stagger_offsets_with_jitter(count, delay, StaggerMode::Linear, jitter, 123);

        for (b, j) in base.into_iter().zip(jittered) {
            let lower = b.saturating_sub(jitter);
            let upper = b.saturating_add(jitter);
            assert!(
                j >= lower && j <= upper,
                "jittered offset out of bounds: base={b:?} jittered={j:?} bounds=[{lower:?}, {upper:?}]"
            );
        }
    }

    #[test]
    fn jitter_zero_is_noop() {
        let base = stagger_offsets(5, MS_100, StaggerMode::Linear);
        let jittered =
            stagger_offsets_with_jitter(5, MS_100, StaggerMode::Linear, Duration::ZERO, 42);
        assert_eq!(base, jittered);
    }

    #[test]
    fn monotonic_linear() {
        let offsets = stagger_offsets(10, MS_50, StaggerMode::Linear);
        for w in offsets.windows(2) {
            assert!(w[1] >= w[0]);
        }
    }

    #[test]
    fn monotonic_ease_in() {
        let offsets = stagger_offsets(10, MS_50, StaggerMode::EaseIn);
        for w in offsets.windows(2) {
            assert!(w[1] >= w[0]);
        }
    }

    #[test]
    fn monotonic_ease_out() {
        let offsets = stagger_offsets(10, MS_50, StaggerMode::EaseOut);
        for w in offsets.windows(2) {
            assert!(w[1] >= w[0]);
        }
    }
}
