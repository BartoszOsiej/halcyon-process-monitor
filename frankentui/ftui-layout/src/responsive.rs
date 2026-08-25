#![forbid(unsafe_code)]

//! Responsive value mapping: apply different values based on breakpoint.
//!
//! [`Responsive<T>`] maps [`Breakpoint`] tiers to values of any type,
//! with inheritance from smaller breakpoints. If no value is set for a
//! given breakpoint, the value from the next smaller breakpoint is used.
//!
//! # Usage
//!
//! ```ignore
//! use ftui_layout::{Responsive, Breakpoint};
//!
//! let padding = Responsive::new(1)     // xs: 1
//!     .at(Breakpoint::Md, 2)           // md: 2
//!     .at(Breakpoint::Xl, 4);          // xl: 4
//!
//! // sm inherits from xs → 1
//! // lg inherits from md → 2
//! assert_eq!(padding.resolve(Breakpoint::Sm), &1);
//! assert_eq!(padding.resolve(Breakpoint::Lg), &2);
//! ```
//!
//! # Invariants
//!
//! 1. `Xs` always has a value (set via `new()`).
//! 2. Inheritance follows breakpoint order: a missing tier inherits from
//!    the nearest smaller tier that has a value.
//! 3. `resolve()` never fails — it always returns a reference.
//! 4. Setting a value at a tier only affects that tier and tiers that
//!    inherit from it (does not affect tiers with explicit values).
//!
//! # Failure Modes
//!
//! None — the type system guarantees a base value at `Xs`.

use super::Breakpoint;

/// A breakpoint-aware value with inheritance from smaller tiers.
///
/// Each slot is `Option<T>`. Slot 0 (Xs) is always `Some` (enforced
/// by the constructor). Resolution walks downward from the requested
/// breakpoint until a `Some` slot is found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Responsive<T> {
    /// Values indexed by `Breakpoint` ordinal (0=Xs .. 4=Xl).
    /// Slot 0 is always `Some`.
    values: [Option<T>; 5],
}

impl<T: Clone> Responsive<T> {
    /// Create a responsive value with a base value for `Xs`.
    ///
    /// All larger breakpoints inherit this value until explicitly overridden.
    #[must_use]
    pub fn new(base: T) -> Self {
        Self {
            values: [Some(base), None, None, None, None],
        }
    }

    /// Set the value for a specific breakpoint (builder pattern).
    #[must_use]
    pub fn at(mut self, bp: Breakpoint, value: T) -> Self {
        self.values[bp as usize] = Some(value);
        self
    }

    /// Set the value for a specific breakpoint (mutating).
    pub fn set(&mut self, bp: Breakpoint, value: T) {
        self.values[bp as usize] = Some(value);
    }

    /// Clear the override for a specific breakpoint, reverting to inheritance.
    ///
    /// Clearing `Xs` is a no-op (it always has a value).
    pub fn clear(&mut self, bp: Breakpoint) {
        if bp != Breakpoint::Xs {
            self.values[bp as usize] = None;
        }
    }

    /// Resolve the value for a given breakpoint.
    ///
    /// Walks downward from `bp` to `Xs` until an explicit value is found.
    /// Always succeeds because `Xs` is always set.
    #[must_use]
    pub fn resolve(&self, bp: Breakpoint) -> &T {
        let idx = bp as usize;
        for i in (0..=idx).rev() {
            if let Some(ref v) = self.values[i] {
                return v;
            }
        }
        // SAFETY: values[0] (Xs) is always Some.
        self.values[0].as_ref().expect("Xs always has a value")
    }

    /// Resolve and clone the value for a given breakpoint.
    #[must_use]
    pub fn resolve_cloned(&self, bp: Breakpoint) -> T {
        self.resolve(bp).clone()
    }

    /// Whether a specific breakpoint has an explicit (non-inherited) value.
    #[must_use]
    pub fn has_explicit(&self, bp: Breakpoint) -> bool {
        self.values[bp as usize].is_some()
    }

    /// Get all explicitly set breakpoints and their values.
    pub fn explicit_values(&self) -> impl Iterator<Item = (Breakpoint, &T)> {
        Breakpoint::ALL
            .iter()
            .zip(self.values.iter())
            .filter_map(|(&bp, v)| v.as_ref().map(|val| (bp, val)))
    }

    /// Map the values to a new type.
    #[must_use]
    pub fn map<U: Clone>(&self, f: impl Fn(&T) -> U) -> Responsive<U> {
        Responsive {
            values: [
                self.values[0].as_ref().map(&f),
                self.values[1].as_ref().map(&f),
                self.values[2].as_ref().map(&f),
                self.values[3].as_ref().map(&f),
                self.values[4].as_ref().map(&f),
            ],
        }
    }
}

impl<T: Clone + Default> Default for Responsive<T> {
    fn default() -> Self {
        Self::new(T::default())
    }
}

impl<T: Clone + std::fmt::Display> std::fmt::Display for Responsive<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Responsive(")?;
        let mut first = true;
        for (bp, val) in self.explicit_values() {
            if !first {
                write!(f, ", ")?;
            }
            write!(f, "{}={}", bp, val)?;
            first = false;
        }
        write!(f, ")")
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base_value_at_all_breakpoints() {
        let r = Responsive::new(42);
        for bp in Breakpoint::ALL {
            assert_eq!(r.resolve(bp), &42);
        }
    }

    #[test]
    fn override_single_breakpoint() {
        let r = Responsive::new(1).at(Breakpoint::Md, 2);

        assert_eq!(r.resolve(Breakpoint::Xs), &1);
        assert_eq!(r.resolve(Breakpoint::Sm), &1); // Inherits from Xs.
        assert_eq!(r.resolve(Breakpoint::Md), &2); // Explicit.
        assert_eq!(r.resolve(Breakpoint::Lg), &2); // Inherits from Md.
        assert_eq!(r.resolve(Breakpoint::Xl), &2); // Inherits from Md.
    }

    #[test]
    fn override_multiple_breakpoints() {
        let r = Responsive::new(0)
            .at(Breakpoint::Sm, 1)
            .at(Breakpoint::Lg, 3);

        assert_eq!(r.resolve(Breakpoint::Xs), &0);
        assert_eq!(r.resolve(Breakpoint::Sm), &1);
        assert_eq!(r.resolve(Breakpoint::Md), &1); // Inherits from Sm.
        assert_eq!(r.resolve(Breakpoint::Lg), &3);
        assert_eq!(r.resolve(Breakpoint::Xl), &3); // Inherits from Lg.
    }

    #[test]
    fn set_mutating() {
        let mut r = Responsive::new(0);
        r.set(Breakpoint::Xl, 5);
        assert_eq!(r.resolve(Breakpoint::Xl), &5);
    }

    #[test]
    fn clear_reverts_to_inheritance() {
        let mut r = Responsive::new(1).at(Breakpoint::Md, 2);
        assert_eq!(r.resolve(Breakpoint::Md), &2);

        r.clear(Breakpoint::Md);
        assert_eq!(r.resolve(Breakpoint::Md), &1); // Back to inheriting from Xs.
    }

    #[test]
    fn clear_xs_is_noop() {
        let mut r = Responsive::new(42);
        r.clear(Breakpoint::Xs);
        assert_eq!(r.resolve(Breakpoint::Xs), &42);
    }

    #[test]
    fn has_explicit() {
        let r = Responsive::new(0).at(Breakpoint::Lg, 3);

        assert!(r.has_explicit(Breakpoint::Xs));
        assert!(!r.has_explicit(Breakpoint::Sm));
        assert!(!r.has_explicit(Breakpoint::Md));
        assert!(r.has_explicit(Breakpoint::Lg));
        assert!(!r.has_explicit(Breakpoint::Xl));
    }

    #[test]
    fn explicit_values_iterator() {
        let r = Responsive::new(0)
            .at(Breakpoint::Md, 2)
            .at(Breakpoint::Xl, 4);

        let explicit: Vec<_> = r.explicit_values().collect();
        assert_eq!(explicit.len(), 3);
        assert_eq!(explicit[0], (Breakpoint::Xs, &0));
        assert_eq!(explicit[1], (Breakpoint::Md, &2));
        assert_eq!(explicit[2], (Breakpoint::Xl, &4));
    }

    #[test]
    fn map_values() {
        let r = Responsive::new(10).at(Breakpoint::Lg, 20);
        let doubled = r.map(|v| v * 2);

        assert_eq!(doubled.resolve(Breakpoint::Xs), &20);
        assert_eq!(doubled.resolve(Breakpoint::Lg), &40);
    }

    #[test]
    fn resolve_cloned() {
        let r = Responsive::new("hello".to_string());
        let val: String = r.resolve_cloned(Breakpoint::Md);
        assert_eq!(val, "hello");
    }

    #[test]
    fn default() {
        let r: Responsive<i32> = Responsive::default();
        assert_eq!(r.resolve(Breakpoint::Xs), &0);
    }

    #[test]
    fn clone_independence() {
        let r1 = Responsive::new(1);
        let mut r2 = r1.clone();
        r2.set(Breakpoint::Md, 99);

        assert_eq!(r1.resolve(Breakpoint::Md), &1);
        assert_eq!(r2.resolve(Breakpoint::Md), &99);
    }

    #[test]
    fn display_format() {
        let r = Responsive::new(0).at(Breakpoint::Md, 2);
        let s = format!("{}", r);
        assert!(s.contains("xs=0"));
        assert!(s.contains("md=2"));
    }

    #[test]
    fn string_responsive() {
        let r = Responsive::new("compact".to_string())
            .at(Breakpoint::Md, "standard".to_string())
            .at(Breakpoint::Xl, "expanded".to_string());

        assert_eq!(r.resolve(Breakpoint::Xs), "compact");
        assert_eq!(r.resolve(Breakpoint::Sm), "compact");
        assert_eq!(r.resolve(Breakpoint::Md), "standard");
        assert_eq!(r.resolve(Breakpoint::Lg), "standard");
        assert_eq!(r.resolve(Breakpoint::Xl), "expanded");
    }

    #[test]
    fn all_breakpoints_overridden() {
        let r = Responsive::new(0)
            .at(Breakpoint::Sm, 1)
            .at(Breakpoint::Md, 2)
            .at(Breakpoint::Lg, 3)
            .at(Breakpoint::Xl, 4);

        assert_eq!(r.resolve(Breakpoint::Xs), &0);
        assert_eq!(r.resolve(Breakpoint::Sm), &1);
        assert_eq!(r.resolve(Breakpoint::Md), &2);
        assert_eq!(r.resolve(Breakpoint::Lg), &3);
        assert_eq!(r.resolve(Breakpoint::Xl), &4);
    }

    #[test]
    fn equality() {
        let r1 = Responsive::new(1).at(Breakpoint::Md, 2);
        let r2 = Responsive::new(1).at(Breakpoint::Md, 2);
        assert_eq!(r1, r2);
    }
}
