//! Reversible computing primitives for undo.
//!
//! Each mutation knows its own inverse. This complements the command-pattern
//! undo system in [`crate::undo`] with operation-level reversibility that
//! composes algebraically.
//!
//! # Core Idea
//!
//! A [`Reversible`] operation can be run forward or backward:
//! ```
//! use ftui_runtime::reversible::{Reversible, AddOp, Sequence};
//!
//! let mut x = 10u64;
//! let op = AddOp::new(3);
//!
//! op.forward(&mut x);
//! assert_eq!(x, 13);
//!
//! op.backward(&mut x);
//! assert_eq!(x, 10);
//! ```
//!
//! Operations compose into sequences that undo in reverse order:
//! ```
//! use ftui_runtime::reversible::{Reversible, AddOp, Sequence};
//!
//! let mut x = 0u64;
//! let ops = Sequence::new(vec![
//!     Box::new(AddOp::new(10)),
//!     Box::new(AddOp::new(20)),
//! ]);
//!
//! ops.forward(&mut x);
//! assert_eq!(x, 30);
//!
//! ops.backward(&mut x);
//! assert_eq!(x, 0);
//! ```

use std::fmt;

/// A reversible operation on state `S`.
///
/// # Laws
///
/// 1. **Round-trip**: `forward(s); backward(s)` restores `s` to its original value.
/// 2. **Round-trip (reverse)**: `backward(s); forward(s)` also restores `s`.
pub trait Reversible<S>: fmt::Debug {
    /// Apply the operation.
    fn forward(&self, state: &mut S);

    /// Undo the operation (apply the inverse).
    fn backward(&self, state: &mut S);

    /// Human-readable description of this operation.
    fn description(&self) -> &str {
        "reversible op"
    }
}

// ── Arithmetic ops ──────────────────────────────────────────────────

/// Reversible addition: `state += delta` / `state -= delta`.
#[derive(Debug, Clone, Copy)]
pub struct AddOp<T> {
    delta: T,
}

impl<T> AddOp<T> {
    pub fn new(delta: T) -> Self {
        Self { delta }
    }
}

macro_rules! impl_add_op {
    ($($ty:ty),*) => {
        $(
            impl Reversible<$ty> for AddOp<$ty> {
                fn forward(&self, state: &mut $ty) {
                    *state = state.wrapping_add(self.delta);
                }

                fn backward(&self, state: &mut $ty) {
                    *state = state.wrapping_sub(self.delta);
                }

                fn description(&self) -> &str {
                    "add"
                }
            }
        )*
    };
}

impl_add_op!(u8, u16, u32, u64, usize, i8, i16, i32, i64, isize);

impl Reversible<f64> for AddOp<f64> {
    fn forward(&self, state: &mut f64) {
        *state += self.delta;
    }

    fn backward(&self, state: &mut f64) {
        *state -= self.delta;
    }

    fn description(&self) -> &str {
        "add_f64"
    }
}

impl Reversible<f32> for AddOp<f32> {
    fn forward(&self, state: &mut f32) {
        *state += self.delta;
    }

    fn backward(&self, state: &mut f32) {
        *state -= self.delta;
    }

    fn description(&self) -> &str {
        "add_f32"
    }
}

/// Reversible XOR: `state ^= mask` (self-inverse).
#[derive(Debug, Clone, Copy)]
pub struct XorOp<T> {
    mask: T,
}

impl<T> XorOp<T> {
    pub fn new(mask: T) -> Self {
        Self { mask }
    }
}

macro_rules! impl_xor_op {
    ($($ty:ty),*) => {
        $(
            impl Reversible<$ty> for XorOp<$ty> {
                fn forward(&self, state: &mut $ty) {
                    *state ^= self.mask;
                }

                fn backward(&self, state: &mut $ty) {
                    // XOR is its own inverse
                    *state ^= self.mask;
                }

                fn description(&self) -> &str {
                    "xor"
                }
            }
        )*
    };
}

impl_xor_op!(u8, u16, u32, u64, usize);

/// Reversible multiplication: `state *= factor` / `state /= factor`.
///
/// Only valid for non-zero factors. For integer types, uses wrapping arithmetic.
#[derive(Debug, Clone, Copy)]
pub struct MulOp<T> {
    factor: T,
}

impl<T> MulOp<T> {
    pub fn new(factor: T) -> Self {
        Self { factor }
    }
}

impl Reversible<f64> for MulOp<f64> {
    fn forward(&self, state: &mut f64) {
        *state *= self.factor;
    }

    fn backward(&self, state: &mut f64) {
        *state /= self.factor;
    }

    fn description(&self) -> &str {
        "mul_f64"
    }
}

// ── Swap ops ────────────────────────────────────────────────────────

/// Reversible swap of two elements in a slice.
#[derive(Debug, Clone, Copy)]
pub struct SwapOp {
    i: usize,
    j: usize,
}

impl SwapOp {
    pub fn new(i: usize, j: usize) -> Self {
        Self { i, j }
    }
}

impl<T> Reversible<Vec<T>> for SwapOp {
    fn forward(&self, state: &mut Vec<T>) {
        state.swap(self.i, self.j);
    }

    fn backward(&self, state: &mut Vec<T>) {
        // Swap is its own inverse
        state.swap(self.i, self.j);
    }

    fn description(&self) -> &str {
        "swap"
    }
}

// ── Set/Replace ops ─────────────────────────────────────────────────

/// Reversible field set: captures old value for undo.
#[derive(Debug, Clone)]
pub struct SetOp<T> {
    new_value: T,
    old_value: T,
}

impl<T: Clone> SetOp<T> {
    /// Create a set operation. `old_value` is the current value before the set.
    pub fn new(old_value: T, new_value: T) -> Self {
        Self {
            new_value,
            old_value,
        }
    }
}

impl<T: Clone + fmt::Debug> Reversible<T> for SetOp<T> {
    fn forward(&self, state: &mut T) {
        *state = self.new_value.clone();
    }

    fn backward(&self, state: &mut T) {
        *state = self.old_value.clone();
    }

    fn description(&self) -> &str {
        "set"
    }
}

// ── Collection ops ──────────────────────────────────────────────────

/// Reversible push to a Vec.
#[derive(Debug, Clone)]
pub struct PushOp<T> {
    value: T,
}

impl<T: Clone> PushOp<T> {
    pub fn new(value: T) -> Self {
        Self { value }
    }
}

impl<T: Clone + fmt::Debug> Reversible<Vec<T>> for PushOp<T> {
    fn forward(&self, state: &mut Vec<T>) {
        state.push(self.value.clone());
    }

    fn backward(&self, state: &mut Vec<T>) {
        state.pop();
    }

    fn description(&self) -> &str {
        "push"
    }
}

/// Reversible insert at index.
#[derive(Debug, Clone)]
pub struct InsertOp<T> {
    index: usize,
    value: T,
}

impl<T: Clone> InsertOp<T> {
    pub fn new(index: usize, value: T) -> Self {
        Self { index, value }
    }
}

impl<T: Clone + fmt::Debug> Reversible<Vec<T>> for InsertOp<T> {
    fn forward(&self, state: &mut Vec<T>) {
        state.insert(self.index, self.value.clone());
    }

    fn backward(&self, state: &mut Vec<T>) {
        state.remove(self.index);
    }

    fn description(&self) -> &str {
        "insert"
    }
}

/// Reversible remove at index (captures removed value for undo).
#[derive(Debug, Clone)]
pub struct RemoveOp<T> {
    index: usize,
    removed: Option<T>,
}

impl<T: Clone> RemoveOp<T> {
    pub fn new(index: usize) -> Self {
        Self {
            index,
            removed: None,
        }
    }
}

impl<T: Clone + fmt::Debug> Reversible<Vec<T>> for RemoveOp<T> {
    fn forward(&self, state: &mut Vec<T>) {
        state.remove(self.index);
    }

    fn backward(&self, state: &mut Vec<T>) {
        if let Some(val) = &self.removed {
            state.insert(self.index, val.clone());
        }
    }

    fn description(&self) -> &str {
        "remove"
    }
}

/// Create a `RemoveOp` that captures the value at `index` for undo.
pub fn remove_capturing<T: Clone>(state: &[T], index: usize) -> RemoveOp<T> {
    RemoveOp {
        index,
        removed: state.get(index).cloned(),
    }
}

// ── Sequence (composition) ──────────────────────────────────────────

/// A sequence of reversible operations applied in order.
///
/// `forward` applies all operations left-to-right.
/// `backward` undoes them right-to-left (stack discipline).
pub struct Sequence<S> {
    ops: Vec<Box<dyn Reversible<S>>>,
    label: &'static str,
}

impl<S> fmt::Debug for Sequence<S> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Sequence")
            .field("len", &self.ops.len())
            .field("label", &self.label)
            .finish()
    }
}

impl<S> Sequence<S> {
    /// Create a sequence from a list of operations.
    pub fn new(ops: Vec<Box<dyn Reversible<S>>>) -> Self {
        Self {
            ops,
            label: "sequence",
        }
    }

    /// Create an empty sequence.
    pub fn empty() -> Self {
        Self {
            ops: Vec::new(),
            label: "sequence",
        }
    }

    /// Set a label for this sequence.
    pub fn with_label(mut self, label: &'static str) -> Self {
        self.label = label;
        self
    }

    /// Add an operation to the sequence.
    pub fn push(&mut self, op: Box<dyn Reversible<S>>) {
        self.ops.push(op);
    }

    /// Number of operations in the sequence.
    pub fn len(&self) -> usize {
        self.ops.len()
    }

    /// Whether the sequence is empty.
    pub fn is_empty(&self) -> bool {
        self.ops.is_empty()
    }
}

impl<S> Reversible<S> for Sequence<S> {
    fn forward(&self, state: &mut S) {
        for op in &self.ops {
            op.forward(state);
        }
    }

    fn backward(&self, state: &mut S) {
        for op in self.ops.iter().rev() {
            op.backward(state);
        }
    }

    fn description(&self) -> &str {
        self.label
    }
}

// ── Recording journal ───────────────────────────────────────────────

/// A journal that records operations as they are applied, enabling undo.
pub struct Journal<S> {
    applied: Vec<Box<dyn Reversible<S>>>,
    undone: Vec<Box<dyn Reversible<S>>>,
}

impl<S> fmt::Debug for Journal<S> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Journal")
            .field("applied", &self.applied.len())
            .field("undone", &self.undone.len())
            .finish()
    }
}

impl<S> Journal<S> {
    /// Create an empty journal.
    pub fn new() -> Self {
        Self {
            applied: Vec::new(),
            undone: Vec::new(),
        }
    }

    /// Apply an operation and record it.
    pub fn apply(&mut self, op: Box<dyn Reversible<S>>, state: &mut S) {
        op.forward(state);
        self.applied.push(op);
        self.undone.clear(); // new operations invalidate redo stack
    }

    /// Undo the last operation.
    pub fn undo(&mut self, state: &mut S) -> bool {
        if let Some(op) = self.applied.pop() {
            op.backward(state);
            self.undone.push(op);
            true
        } else {
            false
        }
    }

    /// Redo the last undone operation.
    pub fn redo(&mut self, state: &mut S) -> bool {
        if let Some(op) = self.undone.pop() {
            op.forward(state);
            self.applied.push(op);
            true
        } else {
            false
        }
    }

    /// Number of operations that can be undone.
    pub fn undo_count(&self) -> usize {
        self.applied.len()
    }

    /// Number of operations that can be redone.
    pub fn redo_count(&self) -> usize {
        self.undone.len()
    }

    /// Clear all history.
    pub fn clear(&mut self) {
        self.applied.clear();
        self.undone.clear();
    }
}

impl<S> Default for Journal<S> {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── AddOp ───────────────────────────────────────────────────────

    #[test]
    fn add_forward_backward() {
        let mut x = 10u64;
        let op = AddOp::new(5u64);
        op.forward(&mut x);
        assert_eq!(x, 15);
        op.backward(&mut x);
        assert_eq!(x, 10);
    }

    #[test]
    fn add_signed() {
        let mut x = 0i32;
        let op = AddOp::new(-5i32);
        op.forward(&mut x);
        assert_eq!(x, -5);
        op.backward(&mut x);
        assert_eq!(x, 0);
    }

    #[test]
    fn add_f64_roundtrip() {
        let mut x = 1.0f64;
        let op = AddOp::new(0.5);
        op.forward(&mut x);
        assert!((x - 1.5).abs() < f64::EPSILON);
        op.backward(&mut x);
        assert!((x - 1.0).abs() < f64::EPSILON);
    }

    // ── XorOp ───────────────────────────────────────────────────────

    #[test]
    fn xor_is_self_inverse() {
        let mut x = 0xFFu8;
        let op = XorOp::new(0x0Fu8);
        op.forward(&mut x);
        assert_eq!(x, 0xF0);
        op.backward(&mut x);
        assert_eq!(x, 0xFF);
    }

    #[test]
    fn xor_double_apply_is_identity() {
        let mut x = 42u64;
        let op = XorOp::new(123u64);
        op.forward(&mut x);
        op.forward(&mut x); // XOR twice = identity
        assert_eq!(x, 42);
    }

    // ── MulOp ───────────────────────────────────────────────────────

    #[test]
    fn mul_f64_roundtrip() {
        let mut x = 4.0f64;
        let op = MulOp::new(2.5);
        op.forward(&mut x);
        assert!((x - 10.0).abs() < f64::EPSILON);
        op.backward(&mut x);
        assert!((x - 4.0).abs() < f64::EPSILON);
    }

    // ── SwapOp ──────────────────────────────────────────────────────

    #[test]
    fn swap_roundtrip() {
        let mut v = vec![1, 2, 3, 4, 5];
        let op = SwapOp::new(0, 4);
        op.forward(&mut v);
        assert_eq!(v, vec![5, 2, 3, 4, 1]);
        op.backward(&mut v);
        assert_eq!(v, vec![1, 2, 3, 4, 5]);
    }

    #[test]
    fn swap_same_index_is_noop() {
        let mut v = vec![1, 2, 3];
        let op = SwapOp::new(1, 1);
        op.forward(&mut v);
        assert_eq!(v, vec![1, 2, 3]);
    }

    // ── SetOp ───────────────────────────────────────────────────────

    #[test]
    fn set_roundtrip() {
        let mut x = "hello".to_string();
        let op = SetOp::new("hello".to_string(), "world".to_string());
        op.forward(&mut x);
        assert_eq!(x, "world");
        op.backward(&mut x);
        assert_eq!(x, "hello");
    }

    // ── PushOp ──────────────────────────────────────────────────────

    #[test]
    fn push_roundtrip() {
        let mut v: Vec<u32> = vec![1, 2];
        let op = PushOp::new(3u32);
        op.forward(&mut v);
        assert_eq!(v, vec![1, 2, 3]);
        op.backward(&mut v);
        assert_eq!(v, vec![1, 2]);
    }

    // ── InsertOp ────────────────────────────────────────────────────

    #[test]
    fn insert_roundtrip() {
        let mut v = vec![1, 3, 4];
        let op = InsertOp::new(1, 2);
        op.forward(&mut v);
        assert_eq!(v, vec![1, 2, 3, 4]);
        op.backward(&mut v);
        assert_eq!(v, vec![1, 3, 4]);
    }

    // ── RemoveOp ────────────────────────────────────────────────────

    #[test]
    fn remove_with_capture_roundtrip() {
        let mut v = vec![10, 20, 30];
        let op = remove_capturing(&v, 1);
        op.forward(&mut v);
        assert_eq!(v, vec![10, 30]);
        op.backward(&mut v);
        assert_eq!(v, vec![10, 20, 30]);
    }

    // ── Sequence ────────────────────────────────────────────────────

    #[test]
    fn sequence_forward_backward() {
        let mut x = 0u64;
        let seq = Sequence::new(vec![
            Box::new(AddOp::new(10u64)),
            Box::new(AddOp::new(20u64)),
            Box::new(AddOp::new(30u64)),
        ]);

        seq.forward(&mut x);
        assert_eq!(x, 60);

        seq.backward(&mut x);
        assert_eq!(x, 0);
    }

    #[test]
    fn sequence_backward_reverses_order() {
        let mut v = Vec::<u32>::new();
        let seq = Sequence::new(vec![
            Box::new(PushOp::new(1u32)),
            Box::new(PushOp::new(2u32)),
            Box::new(PushOp::new(3u32)),
        ]);

        seq.forward(&mut v);
        assert_eq!(v, vec![1, 2, 3]);

        seq.backward(&mut v);
        assert!(v.is_empty());
    }

    #[test]
    fn sequence_empty() {
        let mut x = 42u64;
        let seq = Sequence::<u64>::empty();
        seq.forward(&mut x);
        assert_eq!(x, 42);
        seq.backward(&mut x);
        assert_eq!(x, 42);
    }

    #[test]
    fn sequence_push() {
        let mut seq = Sequence::<u64>::empty();
        seq.push(Box::new(AddOp::new(5u64)));
        assert_eq!(seq.len(), 1);

        let mut x = 0u64;
        seq.forward(&mut x);
        assert_eq!(x, 5);
    }

    #[test]
    fn sequence_with_label() {
        let seq = Sequence::<u64>::empty().with_label("batch insert");
        assert_eq!(seq.description(), "batch insert");
    }

    // ── Journal ─────────────────────────────────────────────────────

    #[test]
    fn journal_apply_and_undo() {
        let mut state = 0u64;
        let mut journal = Journal::new();

        journal.apply(Box::new(AddOp::new(10u64)), &mut state);
        assert_eq!(state, 10);
        assert_eq!(journal.undo_count(), 1);

        journal.apply(Box::new(AddOp::new(20u64)), &mut state);
        assert_eq!(state, 30);
        assert_eq!(journal.undo_count(), 2);

        assert!(journal.undo(&mut state));
        assert_eq!(state, 10);
        assert_eq!(journal.undo_count(), 1);
        assert_eq!(journal.redo_count(), 1);

        assert!(journal.undo(&mut state));
        assert_eq!(state, 0);

        assert!(!journal.undo(&mut state)); // nothing to undo
    }

    #[test]
    fn journal_redo() {
        let mut state = 0u64;
        let mut journal = Journal::new();

        journal.apply(Box::new(AddOp::new(10u64)), &mut state);
        journal.apply(Box::new(AddOp::new(20u64)), &mut state);
        assert_eq!(state, 30);

        journal.undo(&mut state);
        assert_eq!(state, 10);

        assert!(journal.redo(&mut state));
        assert_eq!(state, 30);

        assert!(!journal.redo(&mut state)); // nothing to redo
    }

    #[test]
    fn journal_new_op_clears_redo() {
        let mut state = 0u64;
        let mut journal = Journal::new();

        journal.apply(Box::new(AddOp::new(10u64)), &mut state);
        journal.undo(&mut state);
        assert_eq!(journal.redo_count(), 1);

        journal.apply(Box::new(AddOp::new(5u64)), &mut state);
        assert_eq!(journal.redo_count(), 0); // redo stack cleared
        assert_eq!(state, 5);
    }

    #[test]
    fn journal_clear() {
        let mut state = 0u64;
        let mut journal = Journal::new();

        journal.apply(Box::new(AddOp::new(10u64)), &mut state);
        journal.undo(&mut state);
        journal.clear();

        assert_eq!(journal.undo_count(), 0);
        assert_eq!(journal.redo_count(), 0);
    }

    #[test]
    fn journal_complex_sequence() {
        let mut state = vec![1u32, 2, 3];
        let mut journal = Journal::new();

        // Push 4
        journal.apply(Box::new(PushOp::new(4u32)), &mut state);
        assert_eq!(state, vec![1, 2, 3, 4]);

        // Swap first and last
        journal.apply(Box::new(SwapOp::new(0, 3)), &mut state);
        assert_eq!(state, vec![4, 2, 3, 1]);

        // Insert at position 2
        journal.apply(Box::new(InsertOp::new(2, 99u32)), &mut state);
        assert_eq!(state, vec![4, 2, 99, 3, 1]);

        // Undo all
        journal.undo(&mut state);
        assert_eq!(state, vec![4, 2, 3, 1]);
        journal.undo(&mut state);
        assert_eq!(state, vec![1, 2, 3, 4]);
        journal.undo(&mut state);
        assert_eq!(state, vec![1, 2, 3]);

        // Redo all
        journal.redo(&mut state);
        journal.redo(&mut state);
        journal.redo(&mut state);
        assert_eq!(state, vec![4, 2, 99, 3, 1]);
    }

    #[test]
    fn description_returns_op_name() {
        assert_eq!(AddOp::new(1u64).description(), "add");
        assert_eq!(XorOp::new(1u64).description(), "xor");
        assert_eq!(
            Reversible::<Vec<u32>>::description(&SwapOp::new(0, 1)),
            "swap"
        );
        assert_eq!(SetOp::new(0u32, 1u32).description(), "set");
        assert_eq!(PushOp::new(1u32).description(), "push");
        assert_eq!(InsertOp::new(0, 1u32).description(), "insert");
    }

    #[test]
    fn debug_impls() {
        let op = AddOp::new(5u64);
        assert!(format!("{op:?}").contains("AddOp"));

        let seq = Sequence::<u64>::empty();
        assert!(format!("{seq:?}").contains("Sequence"));

        let journal = Journal::<u64>::new();
        assert!(format!("{journal:?}").contains("Journal"));
    }
}
