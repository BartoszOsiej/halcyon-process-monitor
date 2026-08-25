//! Precomputed trigonometric tables for the Doom renderer.
//!
//! Doom uses 8192 fine angles for a full circle. We precompute sin/cos/tan
//! at startup using standard f32 math.

use std::sync::OnceLock;

use super::constants::FINEANGLES;

/// Precomputed sine table (8192 entries for a full circle).
static FINE_SINE: OnceLock<Box<[f32; FINEANGLES]>> = OnceLock::new();
/// Precomputed cosine table.
static FINE_COSINE: OnceLock<Box<[f32; FINEANGLES]>> = OnceLock::new();
/// Precomputed tangent table.
static FINE_TANGENT: OnceLock<Box<[f32; FINEANGLES]>> = OnceLock::new();

fn init_sine() -> Box<[f32; FINEANGLES]> {
    let mut table = Box::new([0.0f32; FINEANGLES]);
    for i in 0..FINEANGLES {
        let angle = (i as f32) * std::f32::consts::TAU / (FINEANGLES as f32);
        table[i] = angle.sin();
    }
    table
}

fn init_cosine() -> Box<[f32; FINEANGLES]> {
    let mut table = Box::new([0.0f32; FINEANGLES]);
    for i in 0..FINEANGLES {
        let angle = (i as f32) * std::f32::consts::TAU / (FINEANGLES as f32);
        table[i] = angle.cos();
    }
    table
}

fn init_tangent() -> Box<[f32; FINEANGLES]> {
    let mut table = Box::new([0.0f32; FINEANGLES]);
    for i in 0..FINEANGLES {
        let angle = (i as f32) * std::f32::consts::TAU / (FINEANGLES as f32);
        let c = angle.cos();
        table[i] = if c.abs() < 1e-10 {
            if angle.sin() >= 0.0 {
                f32::MAX
            } else {
                f32::MIN
            }
        } else {
            angle.sin() / c
        };
    }
    table
}

/// Get the sine of a fine angle index (0..8191).
#[inline]
pub fn fine_sine(angle: usize) -> f32 {
    let table = FINE_SINE.get_or_init(init_sine);
    table[angle & (FINEANGLES - 1)]
}

/// Get the cosine of a fine angle index.
#[inline]
pub fn fine_cosine(angle: usize) -> f32 {
    let table = FINE_COSINE.get_or_init(init_cosine);
    table[angle & (FINEANGLES - 1)]
}

/// Get the tangent of a fine angle index.
#[inline]
pub fn fine_tangent(angle: usize) -> f32 {
    let table = FINE_TANGENT.get_or_init(init_tangent);
    table[angle & (FINEANGLES - 1)]
}

/// Convert a radians angle to a fine angle index.
#[inline]
pub fn radians_to_fine(rad: f32) -> usize {
    let normalized = rad.rem_euclid(std::f32::consts::TAU);
    ((normalized / std::f32::consts::TAU) * FINEANGLES as f32) as usize & (FINEANGLES - 1)
}

/// Convert a fine angle index to radians.
#[inline]
pub fn fine_to_radians(fine: usize) -> f32 {
    (fine as f32) * std::f32::consts::TAU / (FINEANGLES as f32)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sine_at_zero_is_zero() {
        assert!(fine_sine(0).abs() < 1e-5);
    }

    #[test]
    fn cosine_at_zero_is_one() {
        assert!((fine_cosine(0) - 1.0).abs() < 1e-5);
    }

    #[test]
    fn sine_at_quarter_is_one() {
        assert!((fine_sine(FINEANGLES / 4) - 1.0).abs() < 1e-4);
    }

    #[test]
    fn radians_roundtrip() {
        let angle = 1.234f32;
        let fine = radians_to_fine(angle);
        let back = fine_to_radians(fine);
        assert!((angle - back).abs() < 0.001);
    }

    #[test]
    fn tangent_at_zero_is_zero() {
        assert!(fine_tangent(0).abs() < 1e-5);
    }

    #[test]
    fn sine_at_half_is_zero() {
        // sine at PI (half circle = FINEANGLES/2) should be ~0
        assert!(fine_sine(FINEANGLES / 2).abs() < 1e-4);
    }

    #[test]
    fn cosine_at_quarter_is_zero() {
        // cosine at PI/2 (quarter circle) should be ~0
        assert!(fine_cosine(FINEANGLES / 4).abs() < 1e-4);
    }

    #[test]
    fn cosine_at_half_is_negative_one() {
        // cosine at PI should be ~-1
        assert!((fine_cosine(FINEANGLES / 2) + 1.0).abs() < 1e-4);
    }

    #[test]
    fn sine_at_three_quarters_is_negative_one() {
        // sine at 3PI/2 should be ~-1
        assert!((fine_sine(3 * FINEANGLES / 4) + 1.0).abs() < 1e-4);
    }

    #[test]
    fn angle_wraps_around() {
        // Accessing beyond FINEANGLES should wrap via bitmask
        assert!((fine_sine(FINEANGLES) - fine_sine(0)).abs() < 1e-10);
        assert!((fine_cosine(FINEANGLES + 100) - fine_cosine(100)).abs() < 1e-10);
        assert!((fine_tangent(FINEANGLES + 200) - fine_tangent(200)).abs() < 1e-10);
    }

    #[test]
    fn radians_to_fine_zero() {
        assert_eq!(radians_to_fine(0.0), 0);
    }

    #[test]
    fn radians_to_fine_negative_wraps() {
        // Negative radians should produce valid index
        let idx = radians_to_fine(-1.0);
        assert!(idx < FINEANGLES);
    }

    #[test]
    fn fine_to_radians_zero_is_zero() {
        assert!((fine_to_radians(0) - 0.0).abs() < 1e-10);
    }

    #[test]
    fn pythagorean_identity() {
        // sin^2 + cos^2 = 1 for several angles
        for angle in [0, 100, FINEANGLES / 8, FINEANGLES / 4, FINEANGLES / 3, 5000] {
            let s = fine_sine(angle);
            let c = fine_cosine(angle);
            let sum = s * s + c * c;
            assert!(
                (sum - 1.0).abs() < 1e-5,
                "sin^2+cos^2 != 1 at angle {angle}: {sum}"
            );
        }
    }

    #[test]
    fn tangent_equals_sine_over_cosine() {
        // For angles where cosine is not near zero
        for angle in [100, 500, 1000, 2000, 3000, 6000, 7000] {
            let s = fine_sine(angle);
            let c = fine_cosine(angle);
            if c.abs() > 0.01 {
                let expected = s / c;
                let actual = fine_tangent(angle);
                assert!(
                    (actual - expected).abs() < 1e-4,
                    "tan != sin/cos at angle {angle}: {actual} vs {expected}"
                );
            }
        }
    }

    #[test]
    fn radians_to_fine_tau_wraps_to_zero() {
        assert_eq!(radians_to_fine(std::f32::consts::TAU), 0);
        assert_eq!(radians_to_fine(2.0 * std::f32::consts::TAU), 0);
    }

    #[test]
    fn fine_to_radians_last_entry_is_below_tau() {
        let last = fine_to_radians(FINEANGLES - 1);
        assert!(last >= 0.0);
        assert!(last < std::f32::consts::TAU);
    }

    #[test]
    fn tangent_near_quarter_turn_has_large_magnitude() {
        let tan_q1 = fine_tangent(FINEANGLES / 4);
        let tan_q3 = fine_tangent(3 * FINEANGLES / 4);
        assert!(
            tan_q1.abs() > 1_000_000.0,
            "expected very large tangent near PI/2, got {tan_q1}"
        );
        assert!(
            tan_q3.abs() > 1_000_000.0,
            "expected very large tangent near 3PI/2, got {tan_q3}"
        );
    }

    #[test]
    fn all_sine_values_in_unit_range() {
        for i in 0..FINEANGLES {
            let s = fine_sine(i);
            assert!((-1.0..=1.0).contains(&s), "sine[{i}]={s} outside [-1, 1]");
        }
    }

    #[test]
    fn all_cosine_values_in_unit_range() {
        for i in 0..FINEANGLES {
            let c = fine_cosine(i);
            assert!((-1.0..=1.0).contains(&c), "cosine[{i}]={c} outside [-1, 1]");
        }
    }

    #[test]
    fn sine_odd_symmetry() {
        // sin(-x) = -sin(x), i.e. sin(N - i) = -sin(i)
        for i in 1..FINEANGLES {
            let a = fine_sine(i);
            let b = fine_sine(FINEANGLES - i);
            assert!(
                (a + b).abs() < 1e-4,
                "sine odd symmetry failed at {i}: sin({i})={a}, sin({})={b}",
                FINEANGLES - i
            );
        }
    }

    #[test]
    fn cosine_even_symmetry() {
        // cos(-x) = cos(x), i.e. cos(N - i) = cos(i)
        for i in 1..FINEANGLES {
            let a = fine_cosine(i);
            let b = fine_cosine(FINEANGLES - i);
            assert!(
                (a - b).abs() < 1e-4,
                "cosine even symmetry failed at {i}: cos({i})={a}, cos({})={b}",
                FINEANGLES - i
            );
        }
    }

    #[test]
    fn sine_cosine_phase_shift() {
        // sin(x + PI/2) = cos(x)
        let quarter = FINEANGLES / 4;
        for i in [0, 100, 500, 1000, 2000, 4000, 6000, 7500] {
            let sin_shifted = fine_sine((i + quarter) & (FINEANGLES - 1));
            let cos_val = fine_cosine(i);
            assert!(
                (sin_shifted - cos_val).abs() < 1e-4,
                "sin({i}+N/4)={sin_shifted} != cos({i})={cos_val}"
            );
        }
    }

    #[test]
    fn fine_to_radians_quarter_is_half_pi() {
        let rad = fine_to_radians(FINEANGLES / 4);
        assert!(
            (rad - std::f32::consts::FRAC_PI_2).abs() < 1e-4,
            "expected PI/2, got {rad}"
        );
    }

    #[test]
    fn radians_to_fine_pi_is_half_fineangles() {
        let idx = radians_to_fine(std::f32::consts::PI);
        assert_eq!(idx, FINEANGLES / 2);
    }

    #[test]
    fn large_index_wrap_around() {
        for offset in [0, 1, 100, FINEANGLES / 4, FINEANGLES - 1] {
            let direct = fine_sine(offset);
            let wrapped = fine_sine(10 * FINEANGLES + offset);
            assert!(
                (direct - wrapped).abs() < 1e-10,
                "wrap failed at offset {offset}"
            );
        }
    }

    #[test]
    fn repeated_access_returns_same_value() {
        let a = fine_sine(1234);
        let b = fine_sine(1234);
        assert_eq!(a.to_bits(), b.to_bits());

        let c = fine_cosine(5678);
        let d = fine_cosine(5678);
        assert_eq!(c.to_bits(), d.to_bits());

        let e = fine_tangent(2000);
        let f = fine_tangent(2000);
        assert_eq!(e.to_bits(), f.to_bits());
    }

    #[test]
    fn tangent_sign_per_quadrant() {
        // Q1 (0..N/4): sin>0, cos>0 → tan>0
        let t1 = fine_tangent(FINEANGLES / 8);
        assert!(t1 > 0.0, "Q1 tangent should be positive, got {t1}");

        // Q2 (N/4..N/2): sin>0, cos<0 → tan<0
        let t2 = fine_tangent(3 * FINEANGLES / 8);
        assert!(t2 < 0.0, "Q2 tangent should be negative, got {t2}");

        // Q3 (N/2..3N/4): sin<0, cos<0 → tan>0
        let t3 = fine_tangent(5 * FINEANGLES / 8);
        assert!(t3 > 0.0, "Q3 tangent should be positive, got {t3}");

        // Q4 (3N/4..N): sin<0, cos>0 → tan<0
        let t4 = fine_tangent(7 * FINEANGLES / 8);
        assert!(t4 < 0.0, "Q4 tangent should be negative, got {t4}");
    }
}
