//! Structural diff and patch via datatype-generic representation.
//!
//! Works with any type implementing [`GenericRepr`] by computing diffs on
//! the Sum/Product/Unit encoding and applying patches to transform values.
//!
//! # Example
//!
//! ```
//! use ftui_core::generic_repr::*;
//! use ftui_core::generic_diff::*;
//!
//! #[derive(Clone, Debug, PartialEq)]
//! struct Point { x: f64, y: f64 }
//!
//! impl GenericRepr for Point {
//!     type Repr = Product<f64, Product<f64, Unit>>;
//!     fn into_repr(self) -> Self::Repr {
//!         Product(self.x, Product(self.y, Unit))
//!     }
//!     fn from_repr(repr: Self::Repr) -> Self {
//!         Point { x: repr.0, y: repr.1.0 }
//!     }
//! }
//!
//! let old = Point { x: 1.0, y: 2.0 };
//! let new = Point { x: 1.0, y: 3.0 };
//! let diff = generic_diff(&old, &new);
//! let patched = generic_patch(&old, &diff);
//! assert_eq!(patched, new);
//! assert!(!diff.is_empty());
//! ```

use crate::generic_repr::*;
use std::fmt;

// ── Delta type ──────────────────────────────────────────────────────

/// A change delta: either unchanged or replaced with a new value.
#[derive(Clone, PartialEq, Eq)]
pub enum Delta<T> {
    /// Value is unchanged.
    Same,
    /// Value was replaced.
    Changed(T),
}

impl<T: fmt::Debug> fmt::Debug for Delta<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Delta::Same => write!(f, "Same"),
            Delta::Changed(v) => write!(f, "Changed({v:?})"),
        }
    }
}

impl<T> Delta<T> {
    /// Whether this delta represents no change.
    pub fn is_same(&self) -> bool {
        matches!(self, Delta::Same)
    }

    /// Whether this delta represents a change.
    pub fn is_changed(&self) -> bool {
        matches!(self, Delta::Changed(_))
    }
}

// ── Diff trait ──────────────────────────────────────────────────────

/// Compute a structural diff between two values of the same type.
///
/// The diff type `Self::Diff` captures what changed between `old` and `new`.
pub trait Diff: Sized {
    /// The diff representation for this type.
    type Diff: DiffInfo;

    /// Compute the diff from `old` to `new`.
    fn diff(old: &Self, new: &Self) -> Self::Diff;
}

/// Metadata about a diff.
pub trait DiffInfo {
    /// Whether the diff represents no changes (identity patch).
    fn is_empty(&self) -> bool;
    /// Number of changed fields/variants.
    fn change_count(&self) -> usize;
}

// ── Patch trait ─────────────────────────────────────────────────────

/// Apply a diff to transform a value.
pub trait Patch: Diff {
    /// Apply the diff to `old` to produce `new`.
    fn patch(old: &Self, diff: &Self::Diff) -> Self;
}

// ── Leaf impls ──────────────────────────────────────────────────────

/// Blanket leaf-level diff: compare via PartialEq, delta is the new value.
impl<T: Clone + PartialEq> Diff for T
where
    T: LeafDiff,
{
    type Diff = Delta<T>;

    fn diff(old: &Self, new: &Self) -> Delta<T> {
        if old == new {
            Delta::Same
        } else {
            Delta::Changed(new.clone())
        }
    }
}

impl<T: Clone + PartialEq> Patch for T
where
    T: LeafDiff,
{
    fn patch(old: &Self, diff: &Delta<T>) -> Self {
        match diff {
            Delta::Same => old.clone(),
            Delta::Changed(v) => v.clone(),
        }
    }
}

/// Marker trait for types that use leaf-level (PartialEq) diffing.
///
/// Implement this for primitive types and small value types that should
/// be compared atomically rather than structurally.
pub trait LeafDiff {}

// Implement LeafDiff for common primitives
impl LeafDiff for bool {}
impl LeafDiff for u8 {}
impl LeafDiff for u16 {}
impl LeafDiff for u32 {}
impl LeafDiff for u64 {}
impl LeafDiff for u128 {}
impl LeafDiff for usize {}
impl LeafDiff for i8 {}
impl LeafDiff for i16 {}
impl LeafDiff for i32 {}
impl LeafDiff for i64 {}
impl LeafDiff for i128 {}
impl LeafDiff for isize {}
impl LeafDiff for f32 {}
impl LeafDiff for f64 {}
impl LeafDiff for char {}
impl LeafDiff for String {}
impl LeafDiff for &str {}

impl<T> DiffInfo for Delta<T> {
    fn is_empty(&self) -> bool {
        self.is_same()
    }
    fn change_count(&self) -> usize {
        if self.is_changed() { 1 } else { 0 }
    }
}

// ── Product diff ────────────────────────────────────────────────────

/// Diff of a product: diff each component independently.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProductDiff<HD, TD> {
    pub head: HD,
    pub tail: TD,
}

impl<HD: DiffInfo, TD: DiffInfo> DiffInfo for ProductDiff<HD, TD> {
    fn is_empty(&self) -> bool {
        self.head.is_empty() && self.tail.is_empty()
    }
    fn change_count(&self) -> usize {
        self.head.change_count() + self.tail.change_count()
    }
}

impl<H: Diff, T: Diff> Diff for Product<H, T> {
    type Diff = ProductDiff<H::Diff, T::Diff>;

    fn diff(old: &Self, new: &Self) -> Self::Diff {
        ProductDiff {
            head: H::diff(&old.0, &new.0),
            tail: T::diff(&old.1, &new.1),
        }
    }
}

impl<H: Patch, T: Patch> Patch for Product<H, T> {
    fn patch(old: &Self, diff: &Self::Diff) -> Self {
        Product(H::patch(&old.0, &diff.head), T::patch(&old.1, &diff.tail))
    }
}

// ── Unit diff ───────────────────────────────────────────────────────

/// Diff of a Unit: always empty (nothing to compare).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UnitDiff;

impl DiffInfo for UnitDiff {
    fn is_empty(&self) -> bool {
        true
    }
    fn change_count(&self) -> usize {
        0
    }
}

impl Diff for Unit {
    type Diff = UnitDiff;
    fn diff(_old: &Self, _new: &Self) -> UnitDiff {
        UnitDiff
    }
}

impl Patch for Unit {
    fn patch(_old: &Self, _diff: &UnitDiff) -> Self {
        Unit
    }
}

// ── Sum diff ────────────────────────────────────────────────────────

/// Diff of a sum type: either both are the same variant (structural diff)
/// or the variant changed (full replacement).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SumDiff<LD, RD, L, R> {
    /// Both values are Left; contains diff of inner values.
    BothLeft(LD),
    /// Both values are Right; contains diff of inner values.
    BothRight(RD),
    /// Variant changed from Left to Right.
    LeftToRight(R),
    /// Variant changed from Right to Left.
    RightToLeft(L),
}

impl<LD: DiffInfo, RD: DiffInfo, L, R> DiffInfo for SumDiff<LD, RD, L, R> {
    fn is_empty(&self) -> bool {
        match self {
            SumDiff::BothLeft(d) => d.is_empty(),
            SumDiff::BothRight(d) => d.is_empty(),
            SumDiff::LeftToRight(_) | SumDiff::RightToLeft(_) => false,
        }
    }
    fn change_count(&self) -> usize {
        match self {
            SumDiff::BothLeft(d) => d.change_count(),
            SumDiff::BothRight(d) => d.change_count(),
            SumDiff::LeftToRight(_) | SumDiff::RightToLeft(_) => 1,
        }
    }
}

impl<L: Diff + Clone, R: Diff + Clone> Diff for Sum<L, R> {
    type Diff = SumDiff<L::Diff, R::Diff, L, R>;

    fn diff(old: &Self, new: &Self) -> Self::Diff {
        match (old, new) {
            (Sum::Left(o), Sum::Left(n)) => SumDiff::BothLeft(L::diff(o, n)),
            (Sum::Right(o), Sum::Right(n)) => SumDiff::BothRight(R::diff(o, n)),
            (Sum::Left(_), Sum::Right(n)) => SumDiff::LeftToRight(n.clone()),
            (Sum::Right(_), Sum::Left(n)) => SumDiff::RightToLeft(n.clone()),
        }
    }
}

impl<L: Patch + Clone, R: Patch + Clone> Patch for Sum<L, R> {
    fn patch(old: &Self, diff: &Self::Diff) -> Self {
        match diff {
            SumDiff::BothLeft(d) => match old {
                Sum::Left(o) => Sum::Left(L::patch(o, d)),
                Sum::Right(_) => unreachable!("BothLeft diff applied to Right"),
            },
            SumDiff::BothRight(d) => match old {
                Sum::Right(o) => Sum::Right(R::patch(o, d)),
                Sum::Left(_) => unreachable!("BothRight diff applied to Left"),
            },
            SumDiff::LeftToRight(v) => Sum::Right(v.clone()),
            SumDiff::RightToLeft(v) => Sum::Left(v.clone()),
        }
    }
}

// ── Field diff ──────────────────────────────────────────────────────

impl<T: Diff> Diff for Field<T> {
    type Diff = T::Diff;

    fn diff(old: &Self, new: &Self) -> T::Diff {
        T::diff(&old.value, &new.value)
    }
}

impl<T: Patch> Patch for Field<T> {
    fn patch(old: &Self, diff: &T::Diff) -> Self {
        Field {
            name: old.name,
            value: T::patch(&old.value, diff),
        }
    }
}

// ── Variant diff ────────────────────────────────────────────────────

impl<T: Diff> Diff for Variant<T> {
    type Diff = T::Diff;

    fn diff(old: &Self, new: &Self) -> T::Diff {
        T::diff(&old.value, &new.value)
    }
}

impl<T: Patch> Patch for Variant<T> {
    fn patch(old: &Self, diff: &T::Diff) -> Self {
        Variant {
            name: old.name,
            value: T::patch(&old.value, diff),
        }
    }
}

// ── Void diff ───────────────────────────────────────────────────────

/// Void has no values, so Diff is trivially never called.
/// We need the impl for sum chains to terminate.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum VoidDiff {}

impl DiffInfo for VoidDiff {
    fn is_empty(&self) -> bool {
        match *self {}
    }
    fn change_count(&self) -> usize {
        match *self {}
    }
}

impl Diff for Void {
    type Diff = VoidDiff;
    fn diff(old: &Self, _new: &Self) -> VoidDiff {
        match *old {}
    }
}

impl Patch for Void {
    fn patch(old: &Self, _diff: &VoidDiff) -> Self {
        match *old {}
    }
}

// ── Generic diff/patch convenience functions ────────────────────────

/// Compute a structural diff between two values via their generic representation.
///
/// Returns the diff of the representation type, which can be used with
/// [`generic_patch`] to transform `old` into `new`.
pub fn generic_diff<T>(old: &T, new: &T) -> <T::Repr as Diff>::Diff
where
    T: GenericRepr + Clone,
    T::Repr: Diff,
{
    let old_repr = old.clone().into_repr();
    let new_repr = new.clone().into_repr();
    T::Repr::diff(&old_repr, &new_repr)
}

/// Apply a structural diff to transform a value via its generic representation.
pub fn generic_patch<T>(old: &T, diff: &<T::Repr as Diff>::Diff) -> T
where
    T: GenericRepr + Clone,
    T::Repr: Patch,
{
    let old_repr = old.clone().into_repr();
    let patched_repr = T::Repr::patch(&old_repr, diff);
    T::from_repr(patched_repr)
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── Test types ────────────────────────────────────────────────

    #[derive(Clone, Debug, PartialEq)]
    struct Point {
        x: f64,
        y: f64,
    }

    impl GenericRepr for Point {
        type Repr = Product<Field<f64>, Product<Field<f64>, Unit>>;
        fn into_repr(self) -> Self::Repr {
            Product(
                Field::new("x", self.x),
                Product(Field::new("y", self.y), Unit),
            )
        }
        fn from_repr(repr: Self::Repr) -> Self {
            Point {
                x: repr.0.value,
                y: repr.1.0.value,
            }
        }
    }

    #[derive(Clone, Debug, PartialEq)]
    enum Color {
        Red,
        Green,
        Custom(u8, u8, u8),
    }

    impl GenericRepr for Color {
        type Repr = Sum<
            Variant<Unit>,
            Sum<Variant<Unit>, Sum<Variant<Product<u8, Product<u8, Product<u8, Unit>>>>, Void>>,
        >;

        fn into_repr(self) -> Self::Repr {
            match self {
                Color::Red => Sum::Left(Variant::new("Red", Unit)),
                Color::Green => Sum::Right(Sum::Left(Variant::new("Green", Unit))),
                Color::Custom(r, g, b) => Sum::Right(Sum::Right(Sum::Left(Variant::new(
                    "Custom",
                    Product(r, Product(g, Product(b, Unit))),
                )))),
            }
        }

        fn from_repr(repr: Self::Repr) -> Self {
            match repr {
                Sum::Left(_) => Color::Red,
                Sum::Right(Sum::Left(_)) => Color::Green,
                Sum::Right(Sum::Right(Sum::Left(v))) => {
                    Color::Custom(v.value.0, v.value.1.0, v.value.1.1.0)
                }
                Sum::Right(Sum::Right(Sum::Right(v))) => match v {},
            }
        }
    }

    // ── Leaf diff ─────────────────────────────────────────────────

    #[test]
    fn leaf_diff_same() {
        let d = i32::diff(&42, &42);
        assert!(d.is_same());
        assert!(d.is_empty());
        assert_eq!(d.change_count(), 0);
    }

    #[test]
    fn leaf_diff_changed() {
        let d = i32::diff(&1, &2);
        assert!(d.is_changed());
        assert!(!d.is_empty());
        assert_eq!(d.change_count(), 1);
    }

    #[test]
    fn leaf_patch_same() {
        let d = Delta::Same;
        assert_eq!(i32::patch(&42, &d), 42);
    }

    #[test]
    fn leaf_patch_changed() {
        let d = Delta::Changed(99);
        assert_eq!(i32::patch(&42, &d), 99);
    }

    // ── Product diff ──────────────────────────────────────────────

    #[test]
    fn product_diff_same() {
        let a = Product(1u32, Product(2u32, Unit));
        let d = <Product<u32, Product<u32, Unit>>>::diff(&a, &a);
        assert!(d.is_empty());
        assert_eq!(d.change_count(), 0);
    }

    #[test]
    fn product_diff_one_field() {
        let a = Product(1u32, Product(2u32, Unit));
        let b = Product(1u32, Product(3u32, Unit));
        let d = <Product<u32, Product<u32, Unit>>>::diff(&a, &b);
        assert!(!d.is_empty());
        assert_eq!(d.change_count(), 1);
        assert!(d.head.is_same());
        assert!(d.tail.head.is_changed());
    }

    #[test]
    fn product_patch_roundtrip() {
        let a = Product(1u32, Product(2u32, Unit));
        let b = Product(3u32, Product(2u32, Unit));
        let d = <Product<u32, Product<u32, Unit>>>::diff(&a, &b);
        let patched = <Product<u32, Product<u32, Unit>>>::patch(&a, &d);
        assert_eq!(patched, b);
    }

    // ── Sum diff ──────────────────────────────────────────────────

    #[test]
    fn sum_diff_same_variant() {
        let a: Sum<u32, u32> = Sum::Left(42);
        let b: Sum<u32, u32> = Sum::Left(42);
        let d = <Sum<u32, u32>>::diff(&a, &b);
        assert!(d.is_empty());
    }

    #[test]
    fn sum_diff_same_variant_different_value() {
        let a: Sum<u32, u32> = Sum::Left(1);
        let b: Sum<u32, u32> = Sum::Left(2);
        let d = <Sum<u32, u32>>::diff(&a, &b);
        assert!(!d.is_empty());
        assert_eq!(d.change_count(), 1);
    }

    #[test]
    fn sum_diff_variant_changed() {
        let a: Sum<u32, u32> = Sum::Left(1);
        let b: Sum<u32, u32> = Sum::Right(2);
        let d = <Sum<u32, u32>>::diff(&a, &b);
        assert!(!d.is_empty());
        assert_eq!(d.change_count(), 1);
    }

    #[test]
    fn sum_patch_same_variant() {
        let a: Sum<u32, u32> = Sum::Left(1);
        let b: Sum<u32, u32> = Sum::Left(2);
        let d = <Sum<u32, u32>>::diff(&a, &b);
        let patched = <Sum<u32, u32>>::patch(&a, &d);
        assert_eq!(patched, b);
    }

    #[test]
    fn sum_patch_variant_change() {
        let a: Sum<u32, u32> = Sum::Left(1);
        let b: Sum<u32, u32> = Sum::Right(99);
        let d = <Sum<u32, u32>>::diff(&a, &b);
        let patched = <Sum<u32, u32>>::patch(&a, &d);
        assert_eq!(patched, b);
    }

    // ── Generic diff/patch on Point ───────────────────────────────

    #[test]
    fn point_diff_same() {
        let p = Point { x: 1.0, y: 2.0 };
        let d = generic_diff(&p, &p);
        assert!(d.is_empty());
    }

    #[test]
    fn point_diff_one_field_changed() {
        let a = Point { x: 1.0, y: 2.0 };
        let b = Point { x: 1.0, y: 3.0 };
        let d = generic_diff(&a, &b);
        assert!(!d.is_empty());
        assert_eq!(d.change_count(), 1);
    }

    #[test]
    fn point_diff_both_fields_changed() {
        let a = Point { x: 1.0, y: 2.0 };
        let b = Point { x: 3.0, y: 4.0 };
        let d = generic_diff(&a, &b);
        assert_eq!(d.change_count(), 2);
    }

    #[test]
    fn point_patch_roundtrip() {
        let a = Point { x: 1.0, y: 2.0 };
        let b = Point { x: 3.0, y: 4.0 };
        let d = generic_diff(&a, &b);
        let patched = generic_patch(&a, &d);
        assert_eq!(patched, b);
    }

    #[test]
    fn point_patch_identity() {
        let p = Point { x: 1.0, y: 2.0 };
        let d = generic_diff(&p, &p);
        let patched = generic_patch(&p, &d);
        assert_eq!(patched, p);
    }

    // ── Generic diff/patch on Color enum ──────────────────────────

    #[test]
    fn color_diff_same_unit_variant() {
        let d = generic_diff(&Color::Red, &Color::Red);
        assert!(d.is_empty());
    }

    #[test]
    fn color_diff_different_unit_variants() {
        let d = generic_diff(&Color::Red, &Color::Green);
        assert!(!d.is_empty());
    }

    #[test]
    fn color_diff_same_data_variant() {
        let a = Color::Custom(255, 128, 0);
        let d = generic_diff(&a, &a);
        assert!(d.is_empty());
    }

    #[test]
    fn color_diff_data_variant_changed() {
        let a = Color::Custom(255, 128, 0);
        let b = Color::Custom(255, 0, 0);
        let d = generic_diff(&a, &b);
        assert!(!d.is_empty());
    }

    #[test]
    fn color_patch_variant_change() {
        let a = Color::Red;
        let b = Color::Custom(1, 2, 3);
        let d = generic_diff(&a, &b);
        let patched = generic_patch(&a, &d);
        assert_eq!(patched, b);
    }

    #[test]
    fn color_patch_roundtrip_all() {
        let variants = [Color::Red, Color::Green, Color::Custom(10, 20, 30)];
        for old in &variants {
            for new in &variants {
                let d = generic_diff(old, new);
                let patched = generic_patch(old, &d);
                assert_eq!(&patched, new, "failed: {old:?} -> {new:?}");
            }
        }
    }

    // ── Delta debug ───────────────────────────────────────────────

    #[test]
    fn delta_debug_same() {
        let d: Delta<i32> = Delta::Same;
        assert_eq!(format!("{d:?}"), "Same");
    }

    #[test]
    fn delta_debug_changed() {
        let d = Delta::Changed(42);
        assert_eq!(format!("{d:?}"), "Changed(42)");
    }

    // ── DiffInfo on product ───────────────────────────────────────

    #[test]
    fn product_diff_change_count() {
        let a = Product(1u32, Product(2u32, Product(3u32, Unit)));
        let b = Product(1u32, Product(9u32, Product(9u32, Unit)));
        let d = <Product<u32, Product<u32, Product<u32, Unit>>>>::diff(&a, &b);
        assert_eq!(d.change_count(), 2);
    }

    // ── Proptest property tests ──────────────────────────────────

    mod proptests {
        use super::*;
        use proptest::prelude::*;

        fn arb_point() -> impl Strategy<Value = Point> {
            (any::<f64>(), any::<f64>()).prop_map(|(x, y)| Point { x, y })
        }

        fn arb_color() -> impl Strategy<Value = Color> {
            prop_oneof![
                Just(Color::Red),
                Just(Color::Green),
                (any::<u8>(), any::<u8>(), any::<u8>())
                    .prop_map(|(r, g, b)| Color::Custom(r, g, b)),
            ]
        }

        proptest! {
            #[test]
            fn point_roundtrip(old in arb_point(), new in arb_point()) {
                let d = generic_diff(&old, &new);
                let patched = generic_patch(&old, &d);
                prop_assert_eq!(patched, new);
            }

            #[test]
            fn point_self_diff_is_empty(p in arb_point()) {
                let d = generic_diff(&p, &p);
                prop_assert!(d.is_empty());
            }

            #[test]
            fn point_self_patch_is_identity(p in arb_point()) {
                let d = generic_diff(&p, &p);
                let patched = generic_patch(&p, &d);
                prop_assert_eq!(patched, p);
            }

            #[test]
            fn color_roundtrip(old in arb_color(), new in arb_color()) {
                let d = generic_diff(&old, &new);
                let patched = generic_patch(&old, &d);
                prop_assert_eq!(patched, new);
            }

            #[test]
            fn color_self_diff_is_empty(c in arb_color()) {
                let d = generic_diff(&c, &c);
                prop_assert!(d.is_empty());
            }

            #[test]
            fn color_self_patch_is_identity(c in arb_color()) {
                let d = generic_diff(&c, &c);
                let patched = generic_patch(&c, &d);
                prop_assert_eq!(patched, c);
            }

            #[test]
            fn leaf_i32_roundtrip(old in any::<i32>(), new in any::<i32>()) {
                let d = i32::diff(&old, &new);
                let patched = i32::patch(&old, &d);
                prop_assert_eq!(patched, new);
            }

            #[test]
            fn leaf_i32_self_diff_empty(v in any::<i32>()) {
                let d = i32::diff(&v, &v);
                prop_assert!(d.is_empty());
                prop_assert_eq!(d.change_count(), 0);
            }

            #[test]
            fn product_u32_triple_roundtrip(
                a0 in any::<u32>(), a1 in any::<u32>(), a2 in any::<u32>(),
                b0 in any::<u32>(), b1 in any::<u32>(), b2 in any::<u32>(),
            ) {
                let old = Product(a0, Product(a1, Product(a2, Unit)));
                let new = Product(b0, Product(b1, Product(b2, Unit)));
                let d = <Product<u32, Product<u32, Product<u32, Unit>>>>::diff(&old, &new);
                let patched = <Product<u32, Product<u32, Product<u32, Unit>>>>::patch(&old, &d);
                prop_assert_eq!(patched, new);
            }

            #[test]
            fn sum_roundtrip(
                old_left in any::<bool>(), old_val in any::<u32>(),
                new_left in any::<bool>(), new_val in any::<u32>(),
            ) {
                let old: Sum<u32, u32> = if old_left { Sum::Left(old_val) } else { Sum::Right(old_val) };
                let new: Sum<u32, u32> = if new_left { Sum::Left(new_val) } else { Sum::Right(new_val) };
                let d = <Sum<u32, u32>>::diff(&old, &new);
                let patched = <Sum<u32, u32>>::patch(&old, &d);
                prop_assert_eq!(patched, new);
            }

            #[test]
            fn change_count_bounded_by_fields(
                old in arb_point(), new in arb_point()
            ) {
                let d = generic_diff(&old, &new);
                prop_assert!(d.change_count() <= 2);
            }

            #[test]
            fn diff_same_implies_zero_changes(p in arb_point()) {
                let d = generic_diff(&p, &p);
                prop_assert_eq!(d.change_count(), 0);
            }
        }
    }
}
