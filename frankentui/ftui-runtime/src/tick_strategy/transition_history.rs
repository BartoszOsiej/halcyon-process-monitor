//! Fixed-capacity ring buffer for ordered screen transition sequences.
//!
//! Supplements [`TransitionCounter`](super::TransitionCounter) which stores
//! aggregate counts. The history buffer preserves the raw ordered sequence of
//! the last N transitions, enabling:
//!
//! - Debug introspection ("show me the last 20 screen switches")
//! - Higher-order Markov analysis (bigram/trigram patterns)
//! - Session analytics (time between transitions)

use std::collections::VecDeque;

/// Default capacity if none is specified.
const DEFAULT_CAPACITY: usize = 256;

/// A single recorded screen transition.
#[derive(Debug, Clone)]
pub struct TransitionEntry<S: Clone> {
    /// Screen the user was on before the transition.
    pub from: S,
    /// Screen the user switched to.
    pub to: S,
    /// Monotonic timestamp of the transition.
    pub timestamp: web_time::Instant,
    /// The runtime tick count at the time of the transition.
    pub tick_count: u64,
}

/// Fixed-capacity ring buffer of screen transitions.
///
/// When full, the oldest entry is evicted on each new `record()`.
///
/// # Example
///
/// ```
/// use ftui_runtime::tick_strategy::TransitionHistory;
///
/// let mut history = TransitionHistory::with_capacity(100);
/// history.record("Dashboard".to_string(), "Messages".to_string(), 42);
/// assert_eq!(history.len(), 1);
/// ```
#[derive(Debug, Clone)]
pub struct TransitionHistory<S: Clone> {
    entries: VecDeque<TransitionEntry<S>>,
    capacity: usize,
}

impl<S: Clone> TransitionHistory<S> {
    /// Create a history buffer with the given capacity.
    ///
    /// A capacity of 0 is clamped to 1.
    #[must_use]
    pub fn with_capacity(capacity: usize) -> Self {
        let capacity = capacity.max(1);
        Self {
            entries: VecDeque::with_capacity(capacity),
            capacity,
        }
    }

    /// Record a new screen transition.
    ///
    /// If the buffer is at capacity, the oldest entry is evicted.
    pub fn record(&mut self, from: S, to: S, tick_count: u64) {
        if self.entries.len() == self.capacity {
            self.entries.pop_front();
        }
        self.entries.push_back(TransitionEntry {
            from,
            to,
            timestamp: web_time::Instant::now(),
            tick_count,
        });
    }

    /// Return the most recent `n` transitions (or fewer if less are stored).
    ///
    /// Entries are ordered oldest-first.
    #[must_use]
    pub fn recent(&self, n: usize) -> Vec<&TransitionEntry<S>> {
        let start = self.entries.len().saturating_sub(n);
        self.entries.range(start..).collect()
    }

    /// Return the last `n` distinct destination screens visited, most recent last.
    #[must_use]
    pub fn last_n_screens(&self, n: usize) -> Vec<&S>
    where
        S: PartialEq,
    {
        let mut screens = Vec::new();
        for entry in self.entries.iter().rev() {
            if screens.len() >= n {
                break;
            }
            if !screens.contains(&&entry.to) {
                screens.push(&entry.to);
            }
        }
        screens.reverse();
        screens
    }

    /// Number of transitions currently stored.
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether the buffer is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// The configured capacity.
    #[must_use]
    pub fn capacity(&self) -> usize {
        self.capacity
    }

    /// Clear all stored transitions.
    pub fn clear(&mut self) {
        self.entries.clear();
    }
}

impl<S: Clone> Default for TransitionHistory<S> {
    fn default() -> Self {
        Self::with_capacity(DEFAULT_CAPACITY)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_history() {
        let h = TransitionHistory::<String>::with_capacity(10);
        assert!(h.is_empty());
        assert_eq!(h.len(), 0);
        assert_eq!(h.capacity(), 10);
        assert!(h.recent(5).is_empty());
    }

    #[test]
    fn record_and_retrieve() {
        let mut h = TransitionHistory::with_capacity(10);
        h.record("A".to_owned(), "B".to_owned(), 1);
        h.record("B".to_owned(), "C".to_owned(), 2);

        assert_eq!(h.len(), 2);
        assert!(!h.is_empty());

        let recent = h.recent(10);
        assert_eq!(recent.len(), 2);
        assert_eq!(recent[0].from, "A");
        assert_eq!(recent[0].to, "B");
        assert_eq!(recent[0].tick_count, 1);
        assert_eq!(recent[1].from, "B");
        assert_eq!(recent[1].to, "C");
    }

    #[test]
    fn eviction_at_capacity() {
        let mut h = TransitionHistory::with_capacity(3);
        h.record("A".to_owned(), "B".to_owned(), 1);
        h.record("B".to_owned(), "C".to_owned(), 2);
        h.record("C".to_owned(), "D".to_owned(), 3);
        assert_eq!(h.len(), 3);

        // This should evict the oldest (A→B).
        h.record("D".to_owned(), "E".to_owned(), 4);
        assert_eq!(h.len(), 3);

        let recent = h.recent(10);
        assert_eq!(recent[0].from, "B");
        assert_eq!(recent[0].to, "C");
        assert_eq!(recent[2].from, "D");
        assert_eq!(recent[2].to, "E");
    }

    #[test]
    fn recent_returns_tail() {
        let mut h = TransitionHistory::with_capacity(10);
        for i in 0..5 {
            h.record(format!("s{i}"), format!("s{}", i + 1), i as u64);
        }

        let last2 = h.recent(2);
        assert_eq!(last2.len(), 2);
        assert_eq!(last2[0].from, "s3");
        assert_eq!(last2[1].from, "s4");
    }

    #[test]
    fn recent_more_than_stored() {
        let mut h = TransitionHistory::with_capacity(10);
        h.record("A".to_owned(), "B".to_owned(), 0);

        let all = h.recent(100);
        assert_eq!(all.len(), 1);
    }

    #[test]
    fn last_n_screens_deduplicates() {
        let mut h = TransitionHistory::with_capacity(10);
        h.record("A".to_owned(), "B".to_owned(), 1);
        h.record("B".to_owned(), "A".to_owned(), 2);
        h.record("A".to_owned(), "B".to_owned(), 3);
        h.record("B".to_owned(), "C".to_owned(), 4);

        // Last 3 unique destinations: B, A, C → but in visit order: A, B, C
        let screens = h.last_n_screens(3);
        assert_eq!(screens.len(), 3);
        assert_eq!(*screens[0], "A");
        assert_eq!(*screens[1], "B");
        assert_eq!(*screens[2], "C");
    }

    #[test]
    fn last_n_screens_fewer_than_requested() {
        let mut h = TransitionHistory::with_capacity(10);
        h.record("A".to_owned(), "B".to_owned(), 1);

        let screens = h.last_n_screens(5);
        assert_eq!(screens.len(), 1);
        assert_eq!(*screens[0], "B");
    }

    #[test]
    fn capacity_zero_clamped_to_one() {
        let mut h = TransitionHistory::with_capacity(0);
        assert_eq!(h.capacity(), 1);
        h.record("A".to_owned(), "B".to_owned(), 0);
        assert_eq!(h.len(), 1);
        h.record("B".to_owned(), "C".to_owned(), 1);
        assert_eq!(h.len(), 1);
        assert_eq!(h.recent(1)[0].to, "C");
    }

    #[test]
    fn default_capacity() {
        let h = TransitionHistory::<String>::default();
        assert_eq!(h.capacity(), 256);
    }

    #[test]
    fn clear_empties_buffer() {
        let mut h = TransitionHistory::with_capacity(10);
        h.record("A".to_owned(), "B".to_owned(), 0);
        h.record("B".to_owned(), "C".to_owned(), 1);
        assert_eq!(h.len(), 2);

        h.clear();
        assert!(h.is_empty());
        assert_eq!(h.len(), 0);
    }

    #[test]
    fn timestamps_are_monotonic() {
        let mut h = TransitionHistory::with_capacity(10);
        h.record("A".to_owned(), "B".to_owned(), 0);
        h.record("B".to_owned(), "C".to_owned(), 1);

        let entries = h.recent(2);
        assert!(entries[1].timestamp >= entries[0].timestamp);
    }

    #[test]
    fn clone_is_independent() {
        let mut h = TransitionHistory::with_capacity(5);
        h.record("A".to_owned(), "B".to_owned(), 0);

        let mut clone = h.clone();
        clone.record("B".to_owned(), "C".to_owned(), 1);

        assert_eq!(h.len(), 1);
        assert_eq!(clone.len(), 2);
    }
}
