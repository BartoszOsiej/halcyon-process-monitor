//! [`ActivePlusAdjacent`] strategy: full-rate for neighbors, divisor for the rest.

use std::collections::HashMap;

use super::{TickDecision, TickStrategy};

/// Tick the active screen's declared neighbors at full rate while all other
/// inactive screens use a reduced divisor.
///
/// Use when navigation between adjacent screens is common (e.g. tab bars)
/// and you want instant-feeling switches without a learned model.
#[derive(Debug, Clone)]
pub struct ActivePlusAdjacent {
    adjacency: HashMap<String, Vec<String>>,
    background_divisor: u64,
}

impl ActivePlusAdjacent {
    /// Create an empty adjacency strategy with the given background divisor.
    ///
    /// A divisor of 0 is clamped to 1.
    #[must_use]
    pub fn new(background_divisor: u64) -> Self {
        Self {
            adjacency: HashMap::new(),
            background_divisor: background_divisor.max(1),
        }
    }

    /// Declare that `screen` is adjacent to the given `neighbors`.
    pub fn add_adjacency(&mut self, screen: &str, neighbors: &[&str]) {
        self.adjacency
            .entry(screen.to_owned())
            .or_default()
            .extend(neighbors.iter().map(|s| (*s).to_owned()));
    }

    /// Build adjacency from a linear tab order.
    ///
    /// Each screen becomes adjacent to its immediate left and right neighbors
    /// (edges only have one neighbor).
    ///
    /// ```text
    /// from_tab_order(["A", "B", "C"], 10)
    /// // A <-> B, B <-> C
    /// ```
    #[must_use]
    pub fn from_tab_order(screens: &[&str], background_divisor: u64) -> Self {
        let mut s = Self::new(background_divisor);
        for window in screens.windows(2) {
            let (a, b) = (window[0], window[1]);
            s.adjacency
                .entry(a.to_owned())
                .or_default()
                .push(b.to_owned());
            s.adjacency
                .entry(b.to_owned())
                .or_default()
                .push(a.to_owned());
        }
        s
    }

    /// Return the background divisor.
    #[must_use]
    pub const fn background_divisor(&self) -> u64 {
        self.background_divisor
    }

    /// Return a reference to the adjacency map.
    #[must_use]
    pub fn adjacency(&self) -> &HashMap<String, Vec<String>> {
        &self.adjacency
    }
}

impl TickStrategy for ActivePlusAdjacent {
    fn should_tick(
        &mut self,
        screen_id: &str,
        tick_count: u64,
        active_screen: &str,
    ) -> TickDecision {
        // Check adjacency from the active screen
        if self
            .adjacency
            .get(active_screen)
            .is_some_and(|adj| adj.iter().any(|a| a == screen_id))
        {
            return TickDecision::Tick;
        }

        // Fallback: background divisor
        if tick_count.is_multiple_of(self.background_divisor) {
            TickDecision::Tick
        } else {
            TickDecision::Skip
        }
    }

    fn name(&self) -> &str {
        "ActivePlusAdjacent"
    }

    fn debug_stats(&self) -> Vec<(String, String)> {
        vec![
            ("strategy".into(), "ActivePlusAdjacent".into()),
            (
                "background_divisor".into(),
                self.background_divisor.to_string(),
            ),
            ("adjacency_entries".into(), self.adjacency.len().to_string()),
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn adjacent_screens_always_tick() {
        let mut s = ActivePlusAdjacent::from_tab_order(&["A", "B", "C"], 100);
        // B is adjacent to A → ticks when A is active
        assert_eq!(s.should_tick("B", 1, "A"), TickDecision::Tick);
        assert_eq!(s.should_tick("B", 7, "A"), TickDecision::Tick);
    }

    #[test]
    fn non_adjacent_respects_divisor() {
        let mut s = ActivePlusAdjacent::from_tab_order(&["A", "B", "C"], 5);
        // C is not adjacent to A
        assert_eq!(s.should_tick("C", 1, "A"), TickDecision::Skip);
        assert_eq!(s.should_tick("C", 5, "A"), TickDecision::Tick);
    }

    #[test]
    fn from_tab_order_builds_bidirectional() {
        let s = ActivePlusAdjacent::from_tab_order(&["X", "Y", "Z"], 10);
        let adj = s.adjacency();
        assert!(adj["X"].contains(&"Y".to_owned()));
        assert!(adj["Y"].contains(&"X".to_owned()));
        assert!(adj["Y"].contains(&"Z".to_owned()));
        assert!(adj["Z"].contains(&"Y".to_owned()));
        // X and Z are not adjacent
        assert!(!adj["X"].contains(&"Z".to_owned()));
        assert!(!adj["Z"].contains(&"X".to_owned()));
    }

    #[test]
    fn edge_screens_have_one_neighbor() {
        let s = ActivePlusAdjacent::from_tab_order(&["A", "B", "C", "D"], 10);
        assert_eq!(s.adjacency()["A"].len(), 1); // only B
        assert_eq!(s.adjacency()["D"].len(), 1); // only C
        assert_eq!(s.adjacency()["B"].len(), 2); // A and C
    }

    #[test]
    fn unknown_screen_uses_divisor() {
        let mut s = ActivePlusAdjacent::from_tab_order(&["A", "B"], 4);
        // "unknown" is not in any adjacency list
        assert_eq!(s.should_tick("unknown", 3, "A"), TickDecision::Skip);
        assert_eq!(s.should_tick("unknown", 4, "A"), TickDecision::Tick);
    }

    #[test]
    fn add_adjacency_appends() {
        let mut s = ActivePlusAdjacent::new(10);
        s.add_adjacency("home", &["settings", "profile"]);
        assert_eq!(s.adjacency()["home"].len(), 2);
        s.add_adjacency("home", &["help"]);
        assert_eq!(s.adjacency()["home"].len(), 3);
    }

    #[test]
    fn name_is_stable() {
        let s = ActivePlusAdjacent::new(5);
        assert_eq!(s.name(), "ActivePlusAdjacent");
    }

    #[test]
    fn debug_stats_reports_fields() {
        let s = ActivePlusAdjacent::from_tab_order(&["A", "B", "C"], 8);
        let stats = s.debug_stats();
        assert!(
            stats
                .iter()
                .any(|(k, v)| k == "background_divisor" && v == "8")
        );
        assert!(
            stats
                .iter()
                .any(|(k, v)| k == "adjacency_entries" && v == "3")
        );
    }

    #[test]
    fn divisor_0_clamped() {
        let s = ActivePlusAdjacent::new(0);
        assert_eq!(s.background_divisor(), 1);
    }

    #[test]
    fn empty_tab_order() {
        let s = ActivePlusAdjacent::from_tab_order(&[], 5);
        assert!(s.adjacency().is_empty());
    }

    #[test]
    fn single_screen_tab_order() {
        let s = ActivePlusAdjacent::from_tab_order(&["only"], 5);
        assert!(s.adjacency().is_empty());
    }
}
