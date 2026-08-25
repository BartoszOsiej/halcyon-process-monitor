//! Bidirectional lenses for state-widget binding.
//!
//! A lens focuses on a part of a larger structure, providing both read
//! (view) and write (set) access with algebraic guarantees:
//!
//! - **GetPut**: Setting the value you just read is a no-op.
//! - **PutGet**: Reading after a set returns the value you set.
//!
//! # Usage
//!
//! ```
//! use ftui_runtime::lens::{Lens, field_lens, compose};
//!
//! #[derive(Clone, Debug, PartialEq)]
//! struct Config { volume: u8, brightness: u8 }
//!
//! let volume = field_lens(
//!     |c: &Config| c.volume,
//!     |c: &mut Config, v| c.volume = v,
//! );
//!
//! let mut config = Config { volume: 50, brightness: 100 };
//! assert_eq!(volume.view(&config), 50);
//!
//! volume.set(&mut config, 75);
//! assert_eq!(config.volume, 75);
//! ```
//!
//! # Composition
//!
//! Lenses compose for nested state access:
//!
//! ```
//! use ftui_runtime::lens::{Lens, field_lens, compose};
//!
//! #[derive(Clone, Debug, PartialEq)]
//! struct App { settings: Settings }
//!
//! #[derive(Clone, Debug, PartialEq)]
//! struct Settings { font_size: u16 }
//!
//! let settings = field_lens(
//!     |a: &App| a.settings.clone(),
//!     |a: &mut App, s| a.settings = s,
//! );
//! let font_size = field_lens(
//!     |s: &Settings| s.font_size,
//!     |s: &mut Settings, v| s.font_size = v,
//! );
//!
//! let app_font_size = compose(settings, font_size);
//!
//! let mut app = App { settings: Settings { font_size: 14 } };
//! assert_eq!(app_font_size.view(&app), 14);
//!
//! app_font_size.set(&mut app, 18);
//! assert_eq!(app.settings.font_size, 18);
//! ```

/// A bidirectional lens focusing on part `A` of a whole `S`.
///
/// # Laws
///
/// A well-behaved lens satisfies:
/// 1. **GetPut**: `lens.set(s, lens.view(s))` leaves `s` unchanged.
/// 2. **PutGet**: After `lens.set(s, a)`, `lens.view(s)` returns `a`.
pub trait Lens<S, A> {
    /// View the focused part.
    fn view(&self, whole: &S) -> A;

    /// Set the focused part, mutating the whole in place.
    fn set(&self, whole: &mut S, part: A);

    /// Modify the focused part with a function.
    fn over(&self, whole: &mut S, f: impl FnOnce(A) -> A)
    where
        A: Clone,
    {
        let current = self.view(whole);
        self.set(whole, f(current));
    }

    /// Compose this lens with an inner lens to focus deeper.
    fn then<B, L2: Lens<A, B>>(self, inner: L2) -> Composed<Self, L2, A>
    where
        Self: Sized,
        A: Clone,
    {
        Composed {
            outer: self,
            inner,
            _mid: std::marker::PhantomData,
        }
    }
}

/// A lens built from getter and setter closures.
pub struct FieldLens<G, P> {
    getter: G,
    putter: P,
}

impl<S, A, G, P> Lens<S, A> for FieldLens<G, P>
where
    G: Fn(&S) -> A,
    P: Fn(&mut S, A),
{
    fn view(&self, whole: &S) -> A {
        (self.getter)(whole)
    }

    fn set(&self, whole: &mut S, part: A) {
        (self.putter)(whole, part);
    }
}

/// Create a lens from getter and setter functions.
///
/// # Example
///
/// ```
/// use ftui_runtime::lens::{Lens, field_lens};
///
/// struct Point { x: f64, y: f64 }
///
/// let x_lens = field_lens(|p: &Point| p.x, |p: &mut Point, v| p.x = v);
///
/// let mut p = Point { x: 1.0, y: 2.0 };
/// assert_eq!(x_lens.view(&p), 1.0);
/// x_lens.set(&mut p, 3.0);
/// assert_eq!(p.x, 3.0);
/// ```
pub fn field_lens<S, A>(
    getter: impl Fn(&S) -> A + 'static,
    setter: impl Fn(&mut S, A) + 'static,
) -> FieldLens<impl Fn(&S) -> A, impl Fn(&mut S, A)> {
    FieldLens {
        getter,
        putter: setter,
    }
}

/// Composed lens: outer ∘ inner.
///
/// First uses `outer` to access an intermediate value, then `inner` to
/// access the final target within it.
pub struct Composed<L1, L2, B> {
    outer: L1,
    inner: L2,
    _mid: std::marker::PhantomData<B>,
}

impl<S, B, A, L1, L2> Lens<S, A> for Composed<L1, L2, B>
where
    L1: Lens<S, B>,
    L2: Lens<B, A>,
    B: Clone,
{
    fn view(&self, whole: &S) -> A {
        let mid = self.outer.view(whole);
        self.inner.view(&mid)
    }

    fn set(&self, whole: &mut S, part: A) {
        let mut mid = self.outer.view(whole);
        self.inner.set(&mut mid, part);
        self.outer.set(whole, mid);
    }
}

/// Compose two lenses: `outer` then `inner`.
///
/// The resulting lens focuses on `inner`'s target through `outer`'s target.
pub fn compose<S, B, A, L1, L2>(outer: L1, inner: L2) -> Composed<L1, L2, B>
where
    L1: Lens<S, B>,
    L2: Lens<B, A>,
    B: Clone,
{
    Composed {
        outer,
        inner,
        _mid: std::marker::PhantomData,
    }
}

/// Identity lens that focuses on the whole value.
pub struct Identity;

impl<S: Clone> Lens<S, S> for Identity {
    fn view(&self, whole: &S) -> S {
        whole.clone()
    }

    fn set(&self, whole: &mut S, part: S) {
        *whole = part;
    }
}

/// Lens into the first element of a tuple.
pub struct Fst;

impl<A: Clone, B: Clone> Lens<(A, B), A> for Fst {
    fn view(&self, whole: &(A, B)) -> A {
        whole.0.clone()
    }

    fn set(&self, whole: &mut (A, B), part: A) {
        whole.0 = part;
    }
}

/// Lens into the second element of a tuple.
pub struct Snd;

impl<A: Clone, B: Clone> Lens<(A, B), B> for Snd {
    fn view(&self, whole: &(A, B)) -> B {
        whole.1.clone()
    }

    fn set(&self, whole: &mut (A, B), part: B) {
        whole.1 = part;
    }
}

/// Lens into a Vec element by index.
///
/// # Panics
///
/// Panics if the index is out of bounds.
pub struct AtIndex {
    index: usize,
}

impl AtIndex {
    pub fn new(index: usize) -> Self {
        Self { index }
    }
}

impl<T: Clone> Lens<Vec<T>, T> for AtIndex {
    fn view(&self, whole: &Vec<T>) -> T {
        whole[self.index].clone()
    }

    fn set(&self, whole: &mut Vec<T>, part: T) {
        whole[self.index] = part;
    }
}

/// Create a lens that focuses on a Vec element by index.
pub fn at_index(index: usize) -> AtIndex {
    AtIndex::new(index)
}

/// A prism for optional focusing (lens that may fail to view).
///
/// Unlike a lens, a prism's `preview` returns `Option<A>` — the target
/// may not exist. Useful for enum variants and optional fields.
pub trait Prism<S, A> {
    /// Try to view the focused part. Returns `None` if the prism doesn't match.
    fn preview(&self, whole: &S) -> Option<A>;

    /// Set the focused part if the prism matches. Returns true if applied.
    fn set_if(&self, whole: &mut S, part: A) -> bool;
}

/// Prism into an `Option<T>` value.
pub struct SomePrism;

impl<T: Clone> Prism<Option<T>, T> for SomePrism {
    fn preview(&self, whole: &Option<T>) -> Option<T> {
        whole.clone()
    }

    fn set_if(&self, whole: &mut Option<T>, part: T) -> bool {
        if whole.is_some() {
            *whole = Some(part);
            true
        } else {
            false
        }
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

    #[derive(Clone, Debug, PartialEq)]
    struct Line {
        start: Point,
        end: Point,
    }

    fn x_lens() -> FieldLens<impl Fn(&Point) -> f64, impl Fn(&mut Point, f64)> {
        field_lens(|p: &Point| p.x, |p: &mut Point, v| p.x = v)
    }

    fn y_lens() -> FieldLens<impl Fn(&Point) -> f64, impl Fn(&mut Point, f64)> {
        field_lens(|p: &Point| p.y, |p: &mut Point, v| p.y = v)
    }

    fn start_lens() -> FieldLens<impl Fn(&Line) -> Point, impl Fn(&mut Line, Point)> {
        field_lens(|l: &Line| l.start.clone(), |l: &mut Line, p| l.start = p)
    }

    fn end_lens() -> FieldLens<impl Fn(&Line) -> Point, impl Fn(&mut Line, Point)> {
        field_lens(|l: &Line| l.end.clone(), |l: &mut Line, p| l.end = p)
    }

    // ── Basic lens operations ───────────────────────────────────────

    #[test]
    fn view_reads_field() {
        let lens = x_lens();
        let p = Point { x: 3.0, y: 4.0 };
        assert_eq!(lens.view(&p), 3.0);
    }

    #[test]
    fn set_writes_field() {
        let lens = x_lens();
        let mut p = Point { x: 3.0, y: 4.0 };
        lens.set(&mut p, 5.0);
        assert_eq!(p.x, 5.0);
        assert_eq!(p.y, 4.0); // y unchanged
    }

    #[test]
    fn over_modifies_with_function() {
        let lens = x_lens();
        let mut p = Point { x: 3.0, y: 4.0 };
        lens.over(&mut p, |x| x * 2.0);
        assert_eq!(p.x, 6.0);
    }

    // ── Lens laws ───────────────────────────────────────────────────

    #[test]
    fn law_get_put() {
        // Setting what you got is a no-op
        let lens = x_lens();
        let mut p = Point { x: 3.0, y: 4.0 };
        let original = p.clone();
        let viewed = lens.view(&p);
        lens.set(&mut p, viewed);
        assert_eq!(p, original);
    }

    #[test]
    fn law_put_get() {
        // Getting after a set returns the set value
        let lens = x_lens();
        let mut p = Point { x: 3.0, y: 4.0 };
        lens.set(&mut p, 99.0);
        assert_eq!(lens.view(&p), 99.0);
    }

    #[test]
    fn law_put_put() {
        // Setting twice is the same as setting once with the second value
        let lens = x_lens();
        let mut p1 = Point { x: 3.0, y: 4.0 };
        let mut p2 = p1.clone();

        lens.set(&mut p1, 10.0);
        lens.set(&mut p1, 20.0);

        lens.set(&mut p2, 20.0);

        assert_eq!(p1, p2);
    }

    // ── Composition ─────────────────────────────────────────────────

    #[test]
    fn compose_view() {
        let start_x = compose(start_lens(), x_lens());
        let line = Line {
            start: Point { x: 1.0, y: 2.0 },
            end: Point { x: 3.0, y: 4.0 },
        };
        assert_eq!(start_x.view(&line), 1.0);
    }

    #[test]
    fn compose_set() {
        let start_x = compose(start_lens(), x_lens());
        let mut line = Line {
            start: Point { x: 1.0, y: 2.0 },
            end: Point { x: 3.0, y: 4.0 },
        };
        start_x.set(&mut line, 99.0);
        assert_eq!(line.start.x, 99.0);
        assert_eq!(line.start.y, 2.0); // y unchanged
        assert_eq!(line.end.x, 3.0); // end unchanged
    }

    #[test]
    fn compose_laws_hold() {
        let start_x = compose(start_lens(), x_lens());
        let mut line = Line {
            start: Point { x: 1.0, y: 2.0 },
            end: Point { x: 3.0, y: 4.0 },
        };

        // GetPut
        let original = line.clone();
        let v = start_x.view(&line);
        start_x.set(&mut line, v);
        assert_eq!(line, original);

        // PutGet
        start_x.set(&mut line, 42.0);
        assert_eq!(start_x.view(&line), 42.0);
    }

    #[test]
    fn compose_end_y() {
        let end_y = compose(end_lens(), y_lens());
        let mut line = Line {
            start: Point { x: 1.0, y: 2.0 },
            end: Point { x: 3.0, y: 4.0 },
        };
        assert_eq!(end_y.view(&line), 4.0);
        end_y.set(&mut line, 100.0);
        assert_eq!(line.end.y, 100.0);
    }

    // ── Identity lens ───────────────────────────────────────────────

    #[test]
    fn identity_view() {
        let p = Point { x: 1.0, y: 2.0 };
        assert_eq!(Identity.view(&p), p);
    }

    #[test]
    fn identity_set() {
        let mut p = Point { x: 1.0, y: 2.0 };
        let new = Point { x: 5.0, y: 6.0 };
        Identity.set(&mut p, new.clone());
        assert_eq!(p, new);
    }

    // ── Tuple lenses ────────────────────────────────────────────────

    #[test]
    fn fst_lens() {
        let mut pair = (10u32, "hello");
        assert_eq!(Fst.view(&pair), 10);
        Fst.set(&mut pair, 20);
        assert_eq!(pair, (20, "hello"));
    }

    #[test]
    fn snd_lens() {
        let mut pair = (10u32, 20u32);
        assert_eq!(Snd.view(&pair), 20);
        Snd.set(&mut pair, 30);
        assert_eq!(pair, (10, 30));
    }

    // ── AtIndex lens ────────────────────────────────────────────────

    #[test]
    fn at_index_view() {
        let v = vec![10, 20, 30];
        assert_eq!(at_index(1).view(&v), 20);
    }

    #[test]
    fn at_index_set() {
        let mut v = vec![10, 20, 30];
        at_index(1).set(&mut v, 99);
        assert_eq!(v, vec![10, 99, 30]);
    }

    #[test]
    #[should_panic]
    fn at_index_out_of_bounds() {
        let v = vec![1, 2, 3];
        at_index(5).view(&v);
    }

    // ── Prism ───────────────────────────────────────────────────────

    #[test]
    fn some_prism_preview() {
        let opt = Some(42);
        assert_eq!(SomePrism.preview(&opt), Some(42));

        let none: Option<i32> = None;
        assert_eq!(SomePrism.preview(&none), None);
    }

    #[test]
    fn some_prism_set_if_some() {
        let mut opt = Some(42);
        assert!(SomePrism.set_if(&mut opt, 99));
        assert_eq!(opt, Some(99));
    }

    #[test]
    fn some_prism_set_if_none() {
        let mut opt: Option<i32> = None;
        assert!(!SomePrism.set_if(&mut opt, 99));
        assert_eq!(opt, None);
    }

    // ── Composed lens with over ─────────────────────────────────────

    #[test]
    fn composed_over() {
        let start_x = compose(start_lens(), x_lens());
        let mut line = Line {
            start: Point { x: 10.0, y: 20.0 },
            end: Point { x: 30.0, y: 40.0 },
        };
        start_x.over(&mut line, |x| x + 5.0);
        assert_eq!(line.start.x, 15.0);
    }

    // ── Method chaining with then() ─────────────────────────────────

    #[test]
    fn then_composition() {
        let start_x = start_lens().then(x_lens());
        let line = Line {
            start: Point { x: 7.0, y: 8.0 },
            end: Point { x: 9.0, y: 10.0 },
        };
        assert_eq!(start_x.view(&line), 7.0);
    }
}
