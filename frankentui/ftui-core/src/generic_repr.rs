//! Datatype-generic programming via Sum/Product/Unit encoding.
//!
//! Maps Rust structs and enums to a universal representation:
//! - Structs → [`Product`] chains (e.g., `Product<A, Product<B, Unit>>`)
//! - Enums → [`Sum`] chains (e.g., `Sum<A, Sum<B, Void>>`)
//! - Empty → [`Unit`] or [`Void`]
//!
//! This enables generic algorithms like Diff, Patch, and StableHash
//! that operate on the representation type and work for any type that
//! implements [`GenericRepr`].
//!
//! # Example
//!
//! ```
//! use ftui_core::generic_repr::*;
//!
//! #[derive(Clone, Debug, PartialEq)]
//! struct Point { x: f64, y: f64 }
//!
//! impl GenericRepr for Point {
//!     type Repr = Product<f64, Product<f64, Unit>>;
//!
//!     fn into_repr(self) -> Self::Repr {
//!         Product(self.x, Product(self.y, Unit))
//!     }
//!
//!     fn from_repr(repr: Self::Repr) -> Self {
//!         Point { x: repr.0, y: repr.1.0 }
//!     }
//! }
//!
//! let p = Point { x: 1.0, y: 2.0 };
//! let repr = p.clone().into_repr();
//! let back = Point::from_repr(repr);
//! assert_eq!(p, back);
//! ```

use std::fmt;

/// Convert between a concrete type and its generic representation.
///
/// # Laws
///
/// 1. **Round-trip**: `T::from_repr(t.into_repr()) == t`
/// 2. **Inverse**: `T::into_repr(T::from_repr(r)) == r`
pub trait GenericRepr: Sized {
    /// The generic representation type (built from Sum/Product/Unit/Void).
    type Repr;

    /// Convert this value to its generic representation.
    fn into_repr(self) -> Self::Repr;

    /// Reconstruct a value from its generic representation.
    fn from_repr(repr: Self::Repr) -> Self;
}

// ── Product (struct fields) ─────────────────────────────────────────

/// A product of two types (struct field pair).
///
/// Products chain to represent multiple fields:
/// `Product<A, Product<B, Product<C, Unit>>>` represents a struct with fields A, B, C.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct Product<H, T>(pub H, pub T);

impl<H: fmt::Debug, T: fmt::Debug> fmt::Debug for Product<H, T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "({:?} × {:?})", self.0, self.1)
    }
}

/// The empty product (unit type for terminating product chains).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub struct Unit;

// ── Sum (enum variants) ─────────────────────────────────────────────

/// A sum of two types (enum with two variants).
///
/// Sums chain to represent multiple enum variants:
/// `Sum<A, Sum<B, Sum<C, Void>>>` represents an enum with variants A, B, C.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub enum Sum<L, R> {
    /// The left (earlier) variant.
    Left(L),
    /// The right (later) variant or remaining variants.
    Right(R),
}

impl<L: fmt::Debug, R: fmt::Debug> fmt::Debug for Sum<L, R> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Sum::Left(l) => write!(f, "Left({l:?})"),
            Sum::Right(r) => write!(f, "Right({r:?})"),
        }
    }
}

/// The empty sum (uninhabited type for terminating sum chains).
///
/// Analogous to `!` (never type). No values of this type can exist.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Void {}

// ── Field metadata ──────────────────────────────────────────────────

/// A named field wrapping a value.
///
/// Carries the field name as a const string for introspection by
/// generic algorithms (e.g., Diff output can include field names).
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct Field<T> {
    /// The field name.
    pub name: &'static str,
    /// The field value.
    pub value: T,
}

impl<T> Field<T> {
    /// Create a named field.
    pub fn new(name: &'static str, value: T) -> Self {
        Self { name, value }
    }

    /// Map the value.
    pub fn map<U>(self, f: impl FnOnce(T) -> U) -> Field<U> {
        Field {
            name: self.name,
            value: f(self.value),
        }
    }
}

impl<T: fmt::Debug> fmt::Debug for Field<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}={:?}", self.name, self.value)
    }
}

/// A named variant wrapping a value.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct Variant<T> {
    /// The variant name.
    pub name: &'static str,
    /// The variant payload.
    pub value: T,
}

impl<T> Variant<T> {
    pub fn new(name: &'static str, value: T) -> Self {
        Self { name, value }
    }
}

impl<T: fmt::Debug> fmt::Debug for Variant<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}({:?})", self.name, self.value)
    }
}

// ── GenericRepr impls for standard types ─────────────────────────────

impl GenericRepr for () {
    type Repr = Unit;
    fn into_repr(self) -> Unit {
        Unit
    }
    fn from_repr(_: Unit) -> Self {}
}

impl GenericRepr for bool {
    type Repr = Sum<Unit, Unit>;
    fn into_repr(self) -> Self::Repr {
        if self {
            Sum::Left(Unit)
        } else {
            Sum::Right(Unit)
        }
    }
    fn from_repr(repr: Self::Repr) -> Self {
        matches!(repr, Sum::Left(_))
    }
}

impl<T> GenericRepr for Option<T> {
    type Repr = Sum<T, Unit>;
    fn into_repr(self) -> Self::Repr {
        match self {
            Some(v) => Sum::Left(v),
            None => Sum::Right(Unit),
        }
    }
    fn from_repr(repr: Self::Repr) -> Self {
        match repr {
            Sum::Left(v) => Some(v),
            Sum::Right(_) => None,
        }
    }
}

impl<T, E> GenericRepr for Result<T, E> {
    type Repr = Sum<T, E>;
    fn into_repr(self) -> Self::Repr {
        match self {
            Ok(v) => Sum::Left(v),
            Err(e) => Sum::Right(e),
        }
    }
    fn from_repr(repr: Self::Repr) -> Self {
        match repr {
            Sum::Left(v) => Ok(v),
            Sum::Right(e) => Err(e),
        }
    }
}

impl<A, B> GenericRepr for (A, B) {
    type Repr = Product<A, Product<B, Unit>>;
    fn into_repr(self) -> Self::Repr {
        Product(self.0, Product(self.1, Unit))
    }
    fn from_repr(repr: Self::Repr) -> Self {
        (repr.0, repr.1.0)
    }
}

impl<A, B, C> GenericRepr for (A, B, C) {
    type Repr = Product<A, Product<B, Product<C, Unit>>>;
    fn into_repr(self) -> Self::Repr {
        Product(self.0, Product(self.1, Product(self.2, Unit)))
    }
    fn from_repr(repr: Self::Repr) -> Self {
        (repr.0, repr.1.0, repr.1.1.0)
    }
}

// ── Utility traits for generic algorithms ───────────────────────────

/// Count the number of fields in a product chain.
pub trait ProductLen {
    fn product_len() -> usize;
}

impl ProductLen for Unit {
    fn product_len() -> usize {
        0
    }
}

impl<H, T: ProductLen> ProductLen for Product<H, T> {
    fn product_len() -> usize {
        1 + T::product_len()
    }
}

/// Count the number of variants in a sum chain.
pub trait SumLen {
    fn sum_len() -> usize;
}

impl SumLen for Void {
    fn sum_len() -> usize {
        0
    }
}

impl<L, R: SumLen> SumLen for Sum<L, R> {
    fn sum_len() -> usize {
        1 + R::sum_len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
        Blue,
        Custom(u8, u8, u8),
    }

    impl GenericRepr for Color {
        type Repr = Sum<
            Variant<Unit>,
            Sum<
                Variant<Unit>,
                Sum<Variant<Unit>, Sum<Variant<Product<u8, Product<u8, Product<u8, Unit>>>>, Void>>,
            >,
        >;

        fn into_repr(self) -> Self::Repr {
            match self {
                Color::Red => Sum::Left(Variant::new("Red", Unit)),
                Color::Green => Sum::Right(Sum::Left(Variant::new("Green", Unit))),
                Color::Blue => Sum::Right(Sum::Right(Sum::Left(Variant::new("Blue", Unit)))),
                Color::Custom(r, g, b) => Sum::Right(Sum::Right(Sum::Right(Sum::Left(
                    Variant::new("Custom", Product(r, Product(g, Product(b, Unit)))),
                )))),
            }
        }

        fn from_repr(repr: Self::Repr) -> Self {
            match repr {
                Sum::Left(_) => Color::Red,
                Sum::Right(Sum::Left(_)) => Color::Green,
                Sum::Right(Sum::Right(Sum::Left(_))) => Color::Blue,
                Sum::Right(Sum::Right(Sum::Right(Sum::Left(v)))) => {
                    Color::Custom(v.value.0, v.value.1.0, v.value.1.1.0)
                }
                Sum::Right(Sum::Right(Sum::Right(Sum::Right(v)))) => match v {},
            }
        }
    }

    // ── Round-trip law ──────────────────────────────────────────────

    #[test]
    fn point_roundtrip() {
        let p = Point { x: 1.0, y: 2.0 };
        let repr = p.clone().into_repr();
        let back = Point::from_repr(repr);
        assert_eq!(p, back);
    }

    #[test]
    fn color_roundtrip_unit_variants() {
        for c in [Color::Red, Color::Green, Color::Blue] {
            let back = Color::from_repr(c.clone().into_repr());
            assert_eq!(c, back);
        }
    }

    #[test]
    fn color_roundtrip_data_variant() {
        let c = Color::Custom(255, 128, 0);
        let back = Color::from_repr(c.clone().into_repr());
        assert_eq!(c, back);
    }

    // ── Standard type impls ─────────────────────────────────────────

    #[test]
    fn unit_roundtrip() {
        let repr = ().into_repr();
        assert_eq!(repr, Unit);
        assert_eq!(<()>::from_repr(repr), ());
    }

    #[test]
    fn bool_roundtrip() {
        assert!(bool::from_repr(true.into_repr()));
        assert!(!bool::from_repr(false.into_repr()));
    }

    #[test]
    fn option_roundtrip() {
        let some = Some(42);
        assert_eq!(Option::from_repr(some.into_repr()), Some(42));

        let none: Option<i32> = None;
        assert_eq!(Option::from_repr(none.into_repr()), None);
    }

    #[test]
    fn result_roundtrip() {
        let ok: Result<i32, String> = Ok(42);
        assert_eq!(Result::from_repr(ok.into_repr()), Ok(42));

        let err: Result<i32, String> = Err("fail".into());
        assert_eq!(Result::from_repr(err.into_repr()), Err("fail".into()));
    }

    #[test]
    fn tuple2_roundtrip() {
        let t = (1u32, "hello");
        let back = <(u32, &str)>::from_repr(t.into_repr());
        assert_eq!(back, (1, "hello"));
    }

    #[test]
    fn tuple3_roundtrip() {
        let t = (1u32, 2.0f64, true);
        let back = <(u32, f64, bool)>::from_repr(t.into_repr());
        assert_eq!(back, (1, 2.0, true));
    }

    // ── Product/Sum length ──────────────────────────────────────────

    #[test]
    fn product_len_unit() {
        assert_eq!(Unit::product_len(), 0);
    }

    #[test]
    fn product_len_two() {
        assert_eq!(<Product<i32, Product<i32, Unit>>>::product_len(), 2);
    }

    #[test]
    fn product_len_three() {
        assert_eq!(
            <Product<i32, Product<i32, Product<i32, Unit>>>>::product_len(),
            3
        );
    }

    #[test]
    fn sum_len_void() {
        assert_eq!(Void::sum_len(), 0);
    }

    #[test]
    fn sum_len_two() {
        assert_eq!(<Sum<i32, Sum<i32, Void>>>::sum_len(), 2);
    }

    // ── Debug formatting ────────────────────────────────────────────

    #[test]
    fn product_debug() {
        let p = Product(1, Product(2, Unit));
        let debug = format!("{p:?}");
        assert!(debug.contains("1"));
        assert!(debug.contains("2"));
    }

    #[test]
    fn sum_debug() {
        let s: Sum<i32, i32> = Sum::Left(42);
        assert_eq!(format!("{s:?}"), "Left(42)");
    }

    #[test]
    fn field_debug() {
        let f = Field::new("x", 1.0);
        assert_eq!(format!("{f:?}"), "x=1.0");
    }

    #[test]
    fn variant_debug() {
        let v = Variant::new("Red", Unit);
        let debug = format!("{v:?}");
        assert!(debug.contains("Red"));
    }

    #[test]
    fn field_map() {
        let f = Field::new("count", 5);
        let doubled = f.map(|v| v * 2);
        assert_eq!(doubled.name, "count");
        assert_eq!(doubled.value, 10);
    }

    // ── Named fields in repr ────────────────────────────────────────

    #[test]
    fn point_repr_has_field_names() {
        let p = Point { x: 3.0, y: 4.0 };
        let repr = p.into_repr();
        assert_eq!(repr.0.name, "x");
        assert_eq!(repr.0.value, 3.0);
        assert_eq!(repr.1.0.name, "y");
        assert_eq!(repr.1.0.value, 4.0);
    }

    #[test]
    fn color_repr_has_variant_names() {
        let c = Color::Red;
        let repr = c.into_repr();
        match repr {
            Sum::Left(v) => assert_eq!(v.name, "Red"),
            _ => panic!("expected Left"),
        }
    }
}
