#![forbid(unsafe_code)]

//! Layout and text reuse strategy with strict invalidation contracts (bd-dwrhv).
//!
//! Defines what layout/text computations can be cached, what cache keys are
//! required, and what events invalidate cached results. This prevents repeated
//! pure work (measurement, text wrapping, constraint solving) without risking
//! stale layout or width bugs.
//!
//! # Reuse strategy: single primary lever
//!
//! The primary reuse lever is **constraint-keyed layout memoization**: if the
//! same widget receives identical constraints (area + direction + content hash),
//! the layout result from the previous frame can be reused without recomputation.
//!
//! # What CAN be reused
//!
//! | Computation | Key | Invalidation |
//! |-------------|-----|-------------|
//! | Layout solve | area + constraints_hash + direction | Resize, constraint change, content change |
//! | Text width | grapheme + font assumptions | Never (pure function of content) |
//! | Text wrap | content_hash + max_width | Width change, content change |
//! | Style resolution | style_id + theme_epoch | Theme change |
//!
//! # What CANNOT be reused (correctness risks)
//!
//! | Computation | Why not |
//! |-------------|---------|
//! | Cursor position | Depends on focus state, changes every frame |
//! | Animation state | Time-dependent, must recompute |
//! | Scroll offset | User-driven, changes unpredictably |
//! | Random/seed-dependent | Non-deterministic across frames |

// ============================================================================
// Reusable Computation Categories
// ============================================================================

/// Categories of computation that can potentially be cached.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ReusableComputation {
    /// Flex/Grid constraint solving for a widget subtree.
    LayoutSolve,
    /// Unicode grapheme width calculation.
    TextWidth,
    /// Text wrapping / line breaking.
    TextWrap,
    /// CSS-like style cascade resolution.
    StyleResolution,
    /// Widget intrinsic size measurement.
    IntrinsicMeasure,
}

impl ReusableComputation {
    #[must_use]
    pub const fn label(&self) -> &'static str {
        match self {
            Self::LayoutSolve => "layout-solve",
            Self::TextWidth => "text-width",
            Self::TextWrap => "text-wrap",
            Self::StyleResolution => "style-resolution",
            Self::IntrinsicMeasure => "intrinsic-measure",
        }
    }

    /// Whether the result is a pure function of its inputs (no side effects).
    #[must_use]
    pub const fn is_pure(&self) -> bool {
        true // All listed computations are pure
    }

    /// Typical cost of recomputation (microseconds at standard viewport).
    #[must_use]
    pub const fn typical_cost_us(&self) -> u32 {
        match self {
            Self::LayoutSolve => 50,      // constraint solving
            Self::TextWidth => 5,         // per-grapheme lookup
            Self::TextWrap => 30,         // per-paragraph wrap
            Self::StyleResolution => 10,  // cascade resolution
            Self::IntrinsicMeasure => 20, // widget measurement
        }
    }

    pub const ALL: &'static [ReusableComputation] = &[
        Self::LayoutSolve,
        Self::TextWidth,
        Self::TextWrap,
        Self::StyleResolution,
        Self::IntrinsicMeasure,
    ];
}

// ============================================================================
// Non-Reusable Computations (Safety Boundary)
// ============================================================================

/// Computations that must NOT be cached across frames.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum NonReusableComputation {
    /// Cursor position depends on focus state.
    CursorPosition,
    /// Animation values are time-dependent.
    AnimationState,
    /// Scroll offset changes with user interaction.
    ScrollOffset,
    /// Random/seed-dependent visual effects.
    RandomEffect,
    /// Selection state (mouse drag, keyboard select).
    SelectionState,
    /// Notification toast positions (time-dependent dismiss).
    NotificationPosition,
}

impl NonReusableComputation {
    #[must_use]
    pub const fn label(&self) -> &'static str {
        match self {
            Self::CursorPosition => "cursor-position",
            Self::AnimationState => "animation-state",
            Self::ScrollOffset => "scroll-offset",
            Self::RandomEffect => "random-effect",
            Self::SelectionState => "selection-state",
            Self::NotificationPosition => "notification-position",
        }
    }

    /// Why caching this computation is unsafe.
    #[must_use]
    pub const fn reason(&self) -> &'static str {
        match self {
            Self::CursorPosition => "Focus state changes between frames",
            Self::AnimationState => "Time-dependent values are non-repeatable",
            Self::ScrollOffset => "User-driven, changes unpredictably",
            Self::RandomEffect => "Seed-dependent, intentionally varies",
            Self::SelectionState => "Mouse/keyboard selection is frame-local",
            Self::NotificationPosition => "Toast dismiss timing is time-dependent",
        }
    }

    pub const ALL: &'static [NonReusableComputation] = &[
        Self::CursorPosition,
        Self::AnimationState,
        Self::ScrollOffset,
        Self::RandomEffect,
        Self::SelectionState,
        Self::NotificationPosition,
    ];
}

// ============================================================================
// Cache Key Specification
// ============================================================================

/// Required components of a layout cache key.
#[derive(Debug, Clone)]
pub struct CacheKeySpec {
    /// Which computation this key is for.
    pub computation: ReusableComputation,
    /// Required key components.
    pub components: Vec<KeyComponent>,
    /// What invalidates this cache entry.
    pub invalidation_triggers: Vec<InvalidationTrigger>,
}

/// A component of a cache key.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum KeyComponent {
    /// Widget area (x, y, width, height).
    Area,
    /// Layout direction (horizontal/vertical).
    Direction,
    /// Hash of constraints (flex ratios, min/max, etc.).
    ConstraintsHash,
    /// Hash of text content.
    ContentHash,
    /// Maximum available width for wrapping.
    MaxWidth,
    /// Style/theme identifier.
    StyleId,
    /// Theme epoch counter.
    ThemeEpoch,
    /// Intrinsic size hash (for FitContent).
    IntrinsicHash,
}

impl KeyComponent {
    #[must_use]
    pub const fn label(&self) -> &'static str {
        match self {
            Self::Area => "area",
            Self::Direction => "direction",
            Self::ConstraintsHash => "constraints-hash",
            Self::ContentHash => "content-hash",
            Self::MaxWidth => "max-width",
            Self::StyleId => "style-id",
            Self::ThemeEpoch => "theme-epoch",
            Self::IntrinsicHash => "intrinsic-hash",
        }
    }
}

/// Events that invalidate a cached result.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum InvalidationTrigger {
    /// Viewport dimensions changed.
    Resize,
    /// Constraint parameters changed.
    ConstraintChange,
    /// Text content changed.
    ContentChange,
    /// Theme or style epoch changed.
    ThemeChange,
    /// Font metrics changed (e.g., web font loaded).
    FontChange,
    /// Layout generation bumped (explicit invalidate_all).
    GenerationBump,
}

impl InvalidationTrigger {
    #[must_use]
    pub const fn label(&self) -> &'static str {
        match self {
            Self::Resize => "resize",
            Self::ConstraintChange => "constraint-change",
            Self::ContentChange => "content-change",
            Self::ThemeChange => "theme-change",
            Self::FontChange => "font-change",
            Self::GenerationBump => "generation-bump",
        }
    }

    pub const ALL: &'static [InvalidationTrigger] = &[
        Self::Resize,
        Self::ConstraintChange,
        Self::ContentChange,
        Self::ThemeChange,
        Self::FontChange,
        Self::GenerationBump,
    ];
}

// ============================================================================
// Cache Policy
// ============================================================================

/// Policy for when to use cached results vs recompute.
#[derive(Debug, Clone)]
pub struct CachePolicy {
    /// Maximum cache entries per computation type.
    pub max_entries: u32,
    /// Whether to use generation-based invalidation (O(1) invalidate_all).
    pub generation_invalidation: bool,
    /// Minimum cost (microseconds) to justify caching overhead.
    pub min_cost_threshold_us: u32,
    /// Whether to log cache hit/miss/invalidation events.
    pub log_events: bool,
}

impl CachePolicy {
    /// Default policy: moderate cache size, generation invalidation, 10us threshold.
    #[must_use]
    pub const fn default_policy() -> Self {
        Self {
            max_entries: 256,
            generation_invalidation: true,
            min_cost_threshold_us: 10,
            log_events: false,
        }
    }

    /// Aggressive caching for large viewports.
    #[must_use]
    pub const fn aggressive() -> Self {
        Self {
            max_entries: 1024,
            generation_invalidation: true,
            min_cost_threshold_us: 5,
            log_events: false,
        }
    }

    /// Conservative: small cache, higher threshold.
    #[must_use]
    pub const fn conservative() -> Self {
        Self {
            max_entries: 64,
            generation_invalidation: true,
            min_cost_threshold_us: 20,
            log_events: true,
        }
    }
}

/// Result of a cache lookup decision.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CacheDecision {
    /// Cache hit — use cached result.
    Hit,
    /// Cache miss — compute and store.
    Miss,
    /// Invalidated — recompute due to trigger.
    Invalidated,
    /// Bypassed — cost below threshold, not worth caching.
    Bypassed,
}

impl CacheDecision {
    #[must_use]
    pub const fn label(&self) -> &'static str {
        match self {
            Self::Hit => "hit",
            Self::Miss => "miss",
            Self::Invalidated => "invalidated",
            Self::Bypassed => "bypassed",
        }
    }

    /// Whether this decision used a cached result.
    #[must_use]
    pub const fn used_cache(&self) -> bool {
        matches!(self, Self::Hit)
    }
}

// ============================================================================
// Canonical Cache Key Specifications
// ============================================================================

/// Returns the canonical cache key specifications for all reusable computations.
#[must_use]
pub fn canonical_key_specs() -> Vec<CacheKeySpec> {
    vec![
        CacheKeySpec {
            computation: ReusableComputation::LayoutSolve,
            components: vec![
                KeyComponent::Area,
                KeyComponent::Direction,
                KeyComponent::ConstraintsHash,
                KeyComponent::IntrinsicHash,
            ],
            invalidation_triggers: vec![
                InvalidationTrigger::Resize,
                InvalidationTrigger::ConstraintChange,
                InvalidationTrigger::ContentChange,
                InvalidationTrigger::GenerationBump,
            ],
        },
        CacheKeySpec {
            computation: ReusableComputation::TextWidth,
            components: vec![KeyComponent::ContentHash],
            invalidation_triggers: vec![InvalidationTrigger::FontChange],
        },
        CacheKeySpec {
            computation: ReusableComputation::TextWrap,
            components: vec![KeyComponent::ContentHash, KeyComponent::MaxWidth],
            invalidation_triggers: vec![
                InvalidationTrigger::ContentChange,
                InvalidationTrigger::Resize,
                InvalidationTrigger::FontChange,
            ],
        },
        CacheKeySpec {
            computation: ReusableComputation::StyleResolution,
            components: vec![KeyComponent::StyleId, KeyComponent::ThemeEpoch],
            invalidation_triggers: vec![InvalidationTrigger::ThemeChange],
        },
        CacheKeySpec {
            computation: ReusableComputation::IntrinsicMeasure,
            components: vec![KeyComponent::ContentHash, KeyComponent::ConstraintsHash],
            invalidation_triggers: vec![
                InvalidationTrigger::ContentChange,
                InvalidationTrigger::ConstraintChange,
                InvalidationTrigger::FontChange,
            ],
        },
    ]
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_reusable_computations_are_pure() {
        for comp in ReusableComputation::ALL {
            assert!(comp.is_pure(), "{} should be pure", comp.label());
        }
    }

    #[test]
    fn reusable_computation_labels() {
        for comp in ReusableComputation::ALL {
            assert!(!comp.label().is_empty());
            assert!(comp.typical_cost_us() > 0);
        }
        assert_eq!(ReusableComputation::ALL.len(), 5);
    }

    #[test]
    fn non_reusable_computations_have_reasons() {
        for comp in NonReusableComputation::ALL {
            assert!(!comp.label().is_empty());
            assert!(!comp.reason().is_empty());
        }
        assert_eq!(NonReusableComputation::ALL.len(), 6);
    }

    #[test]
    fn canonical_key_specs_cover_all_computations() {
        let specs = canonical_key_specs();
        let covered: Vec<ReusableComputation> = specs.iter().map(|s| s.computation).collect();
        for comp in ReusableComputation::ALL {
            assert!(
                covered.contains(comp),
                "missing key spec for {}",
                comp.label()
            );
        }
    }

    #[test]
    fn every_key_spec_has_components() {
        for spec in canonical_key_specs() {
            assert!(
                !spec.components.is_empty(),
                "{} has no key components",
                spec.computation.label()
            );
        }
    }

    #[test]
    fn every_key_spec_has_invalidation_triggers() {
        for spec in canonical_key_specs() {
            assert!(
                !spec.invalidation_triggers.is_empty(),
                "{} has no invalidation triggers",
                spec.computation.label()
            );
        }
    }

    #[test]
    fn text_width_is_cheapest() {
        let min = ReusableComputation::ALL
            .iter()
            .min_by_key(|c| c.typical_cost_us())
            .unwrap();
        assert_eq!(*min, ReusableComputation::TextWidth);
    }

    #[test]
    fn layout_solve_is_most_expensive() {
        let max = ReusableComputation::ALL
            .iter()
            .max_by_key(|c| c.typical_cost_us())
            .unwrap();
        assert_eq!(*max, ReusableComputation::LayoutSolve);
    }

    #[test]
    fn cache_policy_defaults() {
        let policy = CachePolicy::default_policy();
        assert_eq!(policy.max_entries, 256);
        assert!(policy.generation_invalidation);
        assert_eq!(policy.min_cost_threshold_us, 10);
    }

    #[test]
    fn cache_policy_aggressive_larger() {
        let agg = CachePolicy::aggressive();
        let def = CachePolicy::default_policy();
        assert!(agg.max_entries > def.max_entries);
        assert!(agg.min_cost_threshold_us < def.min_cost_threshold_us);
    }

    #[test]
    fn cache_decision_labels() {
        for decision in [
            CacheDecision::Hit,
            CacheDecision::Miss,
            CacheDecision::Invalidated,
            CacheDecision::Bypassed,
        ] {
            assert!(!decision.label().is_empty());
        }
    }

    #[test]
    fn only_hit_uses_cache() {
        assert!(CacheDecision::Hit.used_cache());
        assert!(!CacheDecision::Miss.used_cache());
        assert!(!CacheDecision::Invalidated.used_cache());
        assert!(!CacheDecision::Bypassed.used_cache());
    }

    #[test]
    fn invalidation_trigger_labels() {
        for trigger in InvalidationTrigger::ALL {
            assert!(!trigger.label().is_empty());
        }
        assert_eq!(InvalidationTrigger::ALL.len(), 6);
    }

    #[test]
    fn key_component_labels() {
        for comp in [
            KeyComponent::Area,
            KeyComponent::Direction,
            KeyComponent::ConstraintsHash,
            KeyComponent::ContentHash,
            KeyComponent::MaxWidth,
            KeyComponent::StyleId,
            KeyComponent::ThemeEpoch,
            KeyComponent::IntrinsicHash,
        ] {
            assert!(!comp.label().is_empty());
        }
    }

    #[test]
    fn layout_solve_key_requires_area_and_constraints() {
        let specs = canonical_key_specs();
        let layout_spec = specs
            .iter()
            .find(|s| s.computation == ReusableComputation::LayoutSolve)
            .unwrap();
        assert!(layout_spec.components.contains(&KeyComponent::Area));
        assert!(
            layout_spec
                .components
                .contains(&KeyComponent::ConstraintsHash)
        );
        assert!(layout_spec.components.contains(&KeyComponent::Direction));
    }

    #[test]
    fn text_width_key_minimal() {
        let specs = canonical_key_specs();
        let width_spec = specs
            .iter()
            .find(|s| s.computation == ReusableComputation::TextWidth)
            .unwrap();
        assert_eq!(width_spec.components.len(), 1);
        assert_eq!(width_spec.components[0], KeyComponent::ContentHash);
    }

    #[test]
    fn style_resolution_invalidated_by_theme() {
        let specs = canonical_key_specs();
        let style_spec = specs
            .iter()
            .find(|s| s.computation == ReusableComputation::StyleResolution)
            .unwrap();
        assert!(
            style_spec
                .invalidation_triggers
                .contains(&InvalidationTrigger::ThemeChange)
        );
    }
}
