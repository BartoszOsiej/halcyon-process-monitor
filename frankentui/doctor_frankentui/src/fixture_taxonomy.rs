// SPDX-License-Identifier: Apache-2.0
//! Fixture taxonomy and coverage dimensions for migration corpus.
//!
//! Defines the multi-dimensional coverage taxonomy used to annotate corpus
//! fixtures and quantify migration readiness. Dimensions span UI patterns,
//! state complexity, effects, styling, accessibility, and terminal behaviors.
//!
//! Aligned with the transformation policy matrix from `semantic_contract`.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::corpus::{ComplexityTag, CorpusEntry, CorpusManifest};

// ── Taxonomy Dimensions ──────────────────────────────────────────────────

/// Complete set of coverage dimensions for a fixture.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FixtureAnnotation {
    /// The corpus entry slug this annotates.
    pub slug: String,
    /// UI pattern coverage.
    pub ui_patterns: BTreeSet<UiPattern>,
    /// State management patterns.
    pub state_patterns: BTreeSet<StatePattern>,
    /// Effect/side-effect patterns.
    pub effect_patterns: BTreeSet<EffectPattern>,
    /// Styling patterns.
    pub style_patterns: BTreeSet<StylePattern>,
    /// Accessibility patterns.
    pub accessibility_patterns: BTreeSet<AccessibilityPattern>,
    /// Terminal/runtime behavior patterns.
    pub terminal_patterns: BTreeSet<TerminalPattern>,
    /// Data flow patterns.
    pub data_patterns: BTreeSet<DataPattern>,
    /// Overall complexity assessment.
    pub complexity_score: ComplexityScore,
}

/// UI composition patterns.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum UiPattern {
    /// Simple static content rendering.
    StaticContent,
    /// Conditional rendering (if/ternary/switch).
    ConditionalRender,
    /// List/collection rendering (map/flatMap).
    ListRender,
    /// Nested component composition.
    NestedComposition,
    /// Component slots/children patterns.
    SlotPattern,
    /// Higher-order component wrapping.
    HigherOrderComponent,
    /// Render props pattern.
    RenderProps,
    /// Portal/modal rendering.
    PortalModal,
    /// Error boundary.
    ErrorBoundary,
    /// Suspense/lazy loading.
    SuspenseLazy,
    /// Fragment/multi-root render.
    FragmentMultiRoot,
    /// Recursive component tree.
    RecursiveTree,
    /// Forward ref pattern.
    ForwardRef,
    /// Context provider nesting.
    ContextProviderNesting,
}

/// State management patterns.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum StatePattern {
    /// Simple useState.
    LocalState,
    /// useReducer pattern.
    Reducer,
    /// Context-based shared state.
    ContextState,
    /// External store (Redux, Zustand, etc.).
    ExternalStore,
    /// Server state (React Query, SWR).
    ServerState,
    /// URL/router state.
    UrlState,
    /// Form state management.
    FormState,
    /// Derived/computed state (useMemo).
    DerivedState,
    /// Ref-based mutable state.
    RefState,
    /// Multiple interacting state variables.
    InteractingState,
    /// Optimistic update patterns.
    OptimisticUpdate,
    /// State machine / finite automata.
    StateMachine,
}

/// Effect and side-effect patterns.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum EffectPattern {
    /// Data fetching on mount.
    MountFetch,
    /// Data fetching on dependency change.
    DependencyFetch,
    /// DOM manipulation/measurement.
    DomManipulation,
    /// Event listener setup/teardown.
    EventListener,
    /// Timer/interval management.
    TimerInterval,
    /// Subscription management.
    Subscription,
    /// Local storage sync.
    LocalStorageSync,
    /// Cleanup functions.
    EffectCleanup,
    /// Layout effect (useLayoutEffect).
    LayoutEffect,
    /// Debounced/throttled effects.
    DebouncedEffect,
    /// WebSocket connection.
    WebSocketConnection,
    /// Browser API interaction.
    BrowserApi,
}

/// Styling patterns.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum StylePattern {
    /// Inline style objects.
    InlineStyle,
    /// CSS modules.
    CssModules,
    /// CSS-in-JS (styled-components, emotion).
    CssInJs,
    /// Utility classes (Tailwind, etc.).
    UtilityClasses,
    /// Theme provider/consumer.
    ThemeSystem,
    /// Dynamic/conditional styling.
    DynamicStyling,
    /// CSS variables / custom properties.
    CssVariables,
    /// Responsive design patterns.
    ResponsiveDesign,
    /// Animation/transition.
    Animation,
    /// Global styles.
    GlobalStyles,
}

/// Accessibility patterns.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum AccessibilityPattern {
    /// ARIA attributes.
    AriaAttributes,
    /// Keyboard navigation.
    KeyboardNavigation,
    /// Focus management.
    FocusManagement,
    /// Screen reader text.
    ScreenReaderText,
    /// Skip navigation links.
    SkipNavigation,
    /// Color contrast compliance.
    ColorContrast,
    /// Semantic HTML elements.
    SemanticHtml,
    /// Live regions/announcements.
    LiveRegions,
    /// Reduced motion support.
    ReducedMotion,
}

/// Terminal/runtime behavior patterns.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum TerminalPattern {
    /// Alternate screen mode usage.
    AlternateScreen,
    /// Mouse input handling.
    MouseInput,
    /// Keyboard input handling.
    KeyboardInput,
    /// Color output (basic/256/truecolor).
    ColorOutput,
    /// Unicode/grapheme handling.
    UnicodeGrapheme,
    /// Terminal resize handling.
    TerminalResize,
    /// Scrollback preservation.
    ScrollbackPreservation,
    /// Cursor manipulation.
    CursorManipulation,
    /// Clipboard integration.
    ClipboardIntegration,
}

/// Data flow patterns.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum DataPattern {
    /// Props drilling (multiple levels).
    PropsDrilling,
    /// Event bubbling / callback props.
    EventBubbling,
    /// Unidirectional data flow.
    UnidirectionalFlow,
    /// Bidirectional data binding.
    BidirectionalBinding,
    /// Render callback chain.
    RenderCallbackChain,
    /// Code splitting / dynamic import.
    CodeSplitting,
    /// Server-side rendering data.
    ServerSideData,
}

/// Complexity scoring for a fixture.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComplexityScore {
    /// Total dimension count across all categories.
    pub dimension_count: usize,
    /// Weighted complexity score (0.0 - 1.0).
    pub normalized_score: f64,
    /// Tier classification.
    pub tier: ComplexityTier,
}

/// Complexity tier for quick categorization.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum ComplexityTier {
    /// <5 dimensions, simple patterns only.
    Basic,
    /// 5-15 dimensions, moderate patterns.
    Intermediate,
    /// 15-30 dimensions, complex patterns.
    Advanced,
    /// 30+ dimensions, comprehensive patterns.
    Comprehensive,
}

// ── Coverage Analysis ────────────────────────────────────────────────────

/// Coverage analysis across all corpus fixtures.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CoverageReport {
    /// Per-dimension coverage counts.
    pub ui_coverage: BTreeMap<String, usize>,
    pub state_coverage: BTreeMap<String, usize>,
    pub effect_coverage: BTreeMap<String, usize>,
    pub style_coverage: BTreeMap<String, usize>,
    pub accessibility_coverage: BTreeMap<String, usize>,
    pub terminal_coverage: BTreeMap<String, usize>,
    pub data_coverage: BTreeMap<String, usize>,
    /// Dimensions with zero coverage (blind spots).
    pub blind_spots: Vec<BlindSpot>,
    /// Dimensions with disproportionately high coverage.
    pub overrepresented: Vec<OverrepresentedDimension>,
    /// Summary statistics.
    pub stats: CoverageStats,
}

/// A coverage blind spot.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BlindSpot {
    pub category: String,
    pub dimension: String,
    pub impact: BlindSpotImpact,
}

/// Impact assessment for a blind spot.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum BlindSpotImpact {
    /// High impact: this pattern is commonly used in production apps.
    High,
    /// Medium impact: used in some production apps.
    Medium,
    /// Low impact: edge case or uncommon pattern.
    Low,
}

/// An overrepresented coverage dimension.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OverrepresentedDimension {
    pub category: String,
    pub dimension: String,
    pub count: usize,
    pub percentage: f64,
}

/// Coverage statistics.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CoverageStats {
    pub total_fixtures: usize,
    pub total_dimensions_possible: usize,
    pub total_dimensions_covered: usize,
    pub coverage_percentage: f64,
    pub average_dimensions_per_fixture: f64,
    pub tier_distribution: BTreeMap<String, usize>,
}

// ── Annotation Logic ─────────────────────────────────────────────────────

/// Annotate a corpus entry with inferred coverage dimensions.
/// Uses the entry's complexity_tags and feature_tags to infer patterns.
pub fn annotate_entry(entry: &CorpusEntry) -> FixtureAnnotation {
    let mut ui_patterns = BTreeSet::new();
    let mut state_patterns = BTreeSet::new();
    let mut effect_patterns = BTreeSet::new();
    let mut style_patterns = BTreeSet::new();
    let mut accessibility_patterns = BTreeSet::new();
    let mut terminal_patterns = BTreeSet::new();
    let mut data_patterns = BTreeSet::new();

    // Infer from complexity tags.
    for tag in &entry.complexity_tags {
        match tag {
            ComplexityTag::Trivial | ComplexityTag::Small => {
                ui_patterns.insert(UiPattern::StaticContent);
            }
            ComplexityTag::Medium | ComplexityTag::Large => {
                ui_patterns.insert(UiPattern::NestedComposition);
                ui_patterns.insert(UiPattern::ConditionalRender);
                ui_patterns.insert(UiPattern::ListRender);
                data_patterns.insert(DataPattern::EventBubbling);
            }
            ComplexityTag::GlobalState => {
                state_patterns.insert(StatePattern::ExternalStore);
                state_patterns.insert(StatePattern::DerivedState);
            }
            ComplexityTag::CustomHooks => {
                state_patterns.insert(StatePattern::LocalState);
                effect_patterns.insert(EffectPattern::EffectCleanup);
            }
            ComplexityTag::ThemedStyling => {
                style_patterns.insert(StylePattern::ThemeSystem);
                style_patterns.insert(StylePattern::DynamicStyling);
            }
            ComplexityTag::Accessibility => {
                accessibility_patterns.insert(AccessibilityPattern::AriaAttributes);
                accessibility_patterns.insert(AccessibilityPattern::KeyboardNavigation);
            }
            ComplexityTag::FormValidation => {
                state_patterns.insert(StatePattern::FormState);
                ui_patterns.insert(UiPattern::ConditionalRender);
            }
            ComplexityTag::RealTime => {
                effect_patterns.insert(EffectPattern::WebSocketConnection);
                effect_patterns.insert(EffectPattern::Subscription);
            }
            ComplexityTag::CodeSplitting => {
                ui_patterns.insert(UiPattern::SuspenseLazy);
                data_patterns.insert(DataPattern::CodeSplitting);
            }
            ComplexityTag::ComplexRouting => {
                state_patterns.insert(StatePattern::UrlState);
                data_patterns.insert(DataPattern::UnidirectionalFlow);
            }
            ComplexityTag::ServerRendering => {
                data_patterns.insert(DataPattern::ServerSideData);
                state_patterns.insert(StatePattern::ServerState);
            }
            ComplexityTag::TypeScript | ComplexityTag::Monorepo => {}
        }
    }

    // Infer from feature tags.
    for tag in &entry.feature_tags {
        let tag_lower = tag.to_lowercase();
        if tag_lower.contains("hook") {
            state_patterns.insert(StatePattern::LocalState);
        }
        if tag_lower.contains("reducer") {
            state_patterns.insert(StatePattern::Reducer);
        }
        if tag_lower.contains("context") {
            state_patterns.insert(StatePattern::ContextState);
            ui_patterns.insert(UiPattern::ContextProviderNesting);
        }
        if tag_lower.contains("fetch") || tag_lower.contains("api") {
            effect_patterns.insert(EffectPattern::MountFetch);
        }
        if tag_lower.contains("form") {
            state_patterns.insert(StatePattern::FormState);
        }
        if tag_lower.contains("modal") || tag_lower.contains("dialog") {
            ui_patterns.insert(UiPattern::PortalModal);
        }
        if tag_lower.contains("animation") {
            style_patterns.insert(StylePattern::Animation);
        }
        if tag_lower.contains("keyboard") {
            terminal_patterns.insert(TerminalPattern::KeyboardInput);
        }
        if tag_lower.contains("mouse") {
            terminal_patterns.insert(TerminalPattern::MouseInput);
        }
        if tag_lower.contains("color") || tag_lower.contains("theme") {
            style_patterns.insert(StylePattern::ThemeSystem);
        }
        if tag_lower.contains("tailwind") || tag_lower.contains("utility") {
            style_patterns.insert(StylePattern::UtilityClasses);
        }
        if tag_lower.contains("css-module") || tag_lower.contains("module.css") {
            style_patterns.insert(StylePattern::CssModules);
        }
        if tag_lower.contains("styled") || tag_lower.contains("emotion") {
            style_patterns.insert(StylePattern::CssInJs);
        }
    }

    let dimension_count = ui_patterns.len()
        + state_patterns.len()
        + effect_patterns.len()
        + style_patterns.len()
        + accessibility_patterns.len()
        + terminal_patterns.len()
        + data_patterns.len();

    let total_possible = total_dimension_count();
    let normalized_score = if total_possible > 0 {
        dimension_count as f64 / total_possible as f64
    } else {
        0.0
    };

    let tier = classify_tier(dimension_count);

    FixtureAnnotation {
        slug: entry.slug.clone(),
        ui_patterns,
        state_patterns,
        effect_patterns,
        style_patterns,
        accessibility_patterns,
        terminal_patterns,
        data_patterns,
        complexity_score: ComplexityScore {
            dimension_count,
            normalized_score,
            tier,
        },
    }
}

/// Compute coverage report across all annotated fixtures.
pub fn compute_coverage(annotations: &[FixtureAnnotation]) -> CoverageReport {
    let mut ui_coverage: BTreeMap<String, usize> = BTreeMap::new();
    let mut state_coverage: BTreeMap<String, usize> = BTreeMap::new();
    let mut effect_coverage: BTreeMap<String, usize> = BTreeMap::new();
    let mut style_coverage: BTreeMap<String, usize> = BTreeMap::new();
    let mut accessibility_coverage: BTreeMap<String, usize> = BTreeMap::new();
    let mut terminal_coverage: BTreeMap<String, usize> = BTreeMap::new();
    let mut data_coverage: BTreeMap<String, usize> = BTreeMap::new();
    let mut tier_dist: BTreeMap<String, usize> = BTreeMap::new();

    for ann in annotations {
        for p in &ann.ui_patterns {
            *ui_coverage.entry(format!("{p:?}")).or_default() += 1;
        }
        for p in &ann.state_patterns {
            *state_coverage.entry(format!("{p:?}")).or_default() += 1;
        }
        for p in &ann.effect_patterns {
            *effect_coverage.entry(format!("{p:?}")).or_default() += 1;
        }
        for p in &ann.style_patterns {
            *style_coverage.entry(format!("{p:?}")).or_default() += 1;
        }
        for p in &ann.accessibility_patterns {
            *accessibility_coverage.entry(format!("{p:?}")).or_default() += 1;
        }
        for p in &ann.terminal_patterns {
            *terminal_coverage.entry(format!("{p:?}")).or_default() += 1;
        }
        for p in &ann.data_patterns {
            *data_coverage.entry(format!("{p:?}")).or_default() += 1;
        }
        *tier_dist
            .entry(format!("{:?}", ann.complexity_score.tier))
            .or_default() += 1;
    }

    // Identify blind spots.
    let blind_spots = find_blind_spots(
        &ui_coverage,
        &state_coverage,
        &effect_coverage,
        &style_coverage,
        &accessibility_coverage,
        &terminal_coverage,
        &data_coverage,
    );

    // Identify overrepresented dimensions.
    let total = annotations.len();
    let overrepresented = find_overrepresented(
        total,
        &ui_coverage,
        &state_coverage,
        &effect_coverage,
        &style_coverage,
        &accessibility_coverage,
        &terminal_coverage,
        &data_coverage,
    );

    let total_possible = total_dimension_count();
    let covered_count = ui_coverage.len()
        + state_coverage.len()
        + effect_coverage.len()
        + style_coverage.len()
        + accessibility_coverage.len()
        + terminal_coverage.len()
        + data_coverage.len();

    let total_dim_sum: usize = annotations
        .iter()
        .map(|a| a.complexity_score.dimension_count)
        .sum();
    let avg_dims = if total > 0 {
        total_dim_sum as f64 / total as f64
    } else {
        0.0
    };

    CoverageReport {
        ui_coverage,
        state_coverage,
        effect_coverage,
        style_coverage,
        accessibility_coverage,
        terminal_coverage,
        data_coverage,
        blind_spots,
        overrepresented,
        stats: CoverageStats {
            total_fixtures: total,
            total_dimensions_possible: total_possible,
            total_dimensions_covered: covered_count,
            coverage_percentage: if total_possible > 0 {
                covered_count as f64 / total_possible as f64 * 100.0
            } else {
                0.0
            },
            average_dimensions_per_fixture: avg_dims,
            tier_distribution: tier_dist,
        },
    }
}

/// Annotate all entries in a manifest and compute coverage.
pub fn analyze_corpus_coverage(
    manifest: &CorpusManifest,
) -> (Vec<FixtureAnnotation>, CoverageReport) {
    let annotations: Vec<_> = manifest
        .entries
        .values()
        .filter(|e| e.active)
        .map(annotate_entry)
        .collect();
    let report = compute_coverage(&annotations);
    (annotations, report)
}

// ── Helpers ──────────────────────────────────────────────────────────────

fn total_dimension_count() -> usize {
    // Sum of all enum variant counts.
    14 + 12 + 12 + 10 + 9 + 9 + 7 // ui + state + effect + style + a11y + terminal + data
}

fn classify_tier(dimension_count: usize) -> ComplexityTier {
    if dimension_count < 5 {
        ComplexityTier::Basic
    } else if dimension_count < 15 {
        ComplexityTier::Intermediate
    } else if dimension_count < 30 {
        ComplexityTier::Advanced
    } else {
        ComplexityTier::Comprehensive
    }
}

/// All known dimension names per category.
fn all_ui_pattern_names() -> Vec<&'static str> {
    vec![
        "StaticContent",
        "ConditionalRender",
        "ListRender",
        "NestedComposition",
        "SlotPattern",
        "HigherOrderComponent",
        "RenderProps",
        "PortalModal",
        "ErrorBoundary",
        "SuspenseLazy",
        "FragmentMultiRoot",
        "RecursiveTree",
        "ForwardRef",
        "ContextProviderNesting",
    ]
}

fn all_state_pattern_names() -> Vec<&'static str> {
    vec![
        "LocalState",
        "Reducer",
        "ContextState",
        "ExternalStore",
        "ServerState",
        "UrlState",
        "FormState",
        "DerivedState",
        "RefState",
        "InteractingState",
        "OptimisticUpdate",
        "StateMachine",
    ]
}

fn all_effect_pattern_names() -> Vec<&'static str> {
    vec![
        "MountFetch",
        "DependencyFetch",
        "DomManipulation",
        "EventListener",
        "TimerInterval",
        "Subscription",
        "LocalStorageSync",
        "EffectCleanup",
        "LayoutEffect",
        "DebouncedEffect",
        "WebSocketConnection",
        "BrowserApi",
    ]
}

fn all_style_pattern_names() -> Vec<&'static str> {
    vec![
        "InlineStyle",
        "CssModules",
        "CssInJs",
        "UtilityClasses",
        "ThemeSystem",
        "DynamicStyling",
        "CssVariables",
        "ResponsiveDesign",
        "Animation",
        "GlobalStyles",
    ]
}

fn all_accessibility_pattern_names() -> Vec<&'static str> {
    vec![
        "AriaAttributes",
        "KeyboardNavigation",
        "FocusManagement",
        "ScreenReaderText",
        "SkipNavigation",
        "ColorContrast",
        "SemanticHtml",
        "LiveRegions",
        "ReducedMotion",
    ]
}

fn all_terminal_pattern_names() -> Vec<&'static str> {
    vec![
        "AlternateScreen",
        "MouseInput",
        "KeyboardInput",
        "ColorOutput",
        "UnicodeGrapheme",
        "TerminalResize",
        "ScrollbackPreservation",
        "CursorManipulation",
        "ClipboardIntegration",
    ]
}

fn all_data_pattern_names() -> Vec<&'static str> {
    vec![
        "PropsDrilling",
        "EventBubbling",
        "UnidirectionalFlow",
        "BidirectionalBinding",
        "RenderCallbackChain",
        "CodeSplitting",
        "ServerSideData",
    ]
}

fn find_blind_spots(
    ui: &BTreeMap<String, usize>,
    state: &BTreeMap<String, usize>,
    effect: &BTreeMap<String, usize>,
    style: &BTreeMap<String, usize>,
    a11y: &BTreeMap<String, usize>,
    terminal: &BTreeMap<String, usize>,
    data: &BTreeMap<String, usize>,
) -> Vec<BlindSpot> {
    let mut spots = Vec::new();

    let check = |names: &[&str],
                 map: &BTreeMap<String, usize>,
                 category: &str,
                 spots: &mut Vec<BlindSpot>| {
        for name in names {
            if !map.contains_key(*name) {
                let impact = classify_blind_spot_impact(category, name);
                spots.push(BlindSpot {
                    category: category.to_string(),
                    dimension: name.to_string(),
                    impact,
                });
            }
        }
    };

    check(&all_ui_pattern_names(), ui, "ui", &mut spots);
    check(&all_state_pattern_names(), state, "state", &mut spots);
    check(&all_effect_pattern_names(), effect, "effect", &mut spots);
    check(&all_style_pattern_names(), style, "style", &mut spots);
    check(
        &all_accessibility_pattern_names(),
        a11y,
        "accessibility",
        &mut spots,
    );
    check(
        &all_terminal_pattern_names(),
        terminal,
        "terminal",
        &mut spots,
    );
    check(&all_data_pattern_names(), data, "data", &mut spots);

    spots
}

fn classify_blind_spot_impact(category: &str, dimension: &str) -> BlindSpotImpact {
    // High-impact patterns are common in production apps.
    let high_impact = [
        ("ui", "ConditionalRender"),
        ("ui", "ListRender"),
        ("ui", "NestedComposition"),
        ("state", "LocalState"),
        ("state", "ExternalStore"),
        ("state", "FormState"),
        ("effect", "MountFetch"),
        ("effect", "EventListener"),
        ("effect", "EffectCleanup"),
        ("style", "CssModules"),
        ("style", "UtilityClasses"),
        ("accessibility", "AriaAttributes"),
        ("accessibility", "KeyboardNavigation"),
        ("terminal", "KeyboardInput"),
        ("data", "EventBubbling"),
        ("data", "UnidirectionalFlow"),
    ];

    let medium_impact = [
        ("ui", "PortalModal"),
        ("ui", "ErrorBoundary"),
        ("state", "Reducer"),
        ("state", "ContextState"),
        ("state", "ServerState"),
        ("effect", "TimerInterval"),
        ("effect", "DomManipulation"),
        ("style", "ThemeSystem"),
        ("style", "DynamicStyling"),
        ("terminal", "MouseInput"),
        ("terminal", "ColorOutput"),
    ];

    if high_impact.contains(&(category, dimension)) {
        BlindSpotImpact::High
    } else if medium_impact.contains(&(category, dimension)) {
        BlindSpotImpact::Medium
    } else {
        BlindSpotImpact::Low
    }
}

#[allow(clippy::too_many_arguments)]
fn find_overrepresented(
    total: usize,
    ui: &BTreeMap<String, usize>,
    state: &BTreeMap<String, usize>,
    effect: &BTreeMap<String, usize>,
    style: &BTreeMap<String, usize>,
    a11y: &BTreeMap<String, usize>,
    terminal: &BTreeMap<String, usize>,
    data: &BTreeMap<String, usize>,
) -> Vec<OverrepresentedDimension> {
    if total == 0 {
        return Vec::new();
    }

    let threshold = 0.7; // >70% of fixtures have this pattern
    let mut over = Vec::new();

    let check = |map: &BTreeMap<String, usize>,
                 category: &str,
                 over: &mut Vec<OverrepresentedDimension>| {
        for (dim, count) in map {
            let pct = *count as f64 / total as f64;
            if pct > threshold {
                over.push(OverrepresentedDimension {
                    category: category.to_string(),
                    dimension: dim.clone(),
                    count: *count,
                    percentage: pct * 100.0,
                });
            }
        }
    };

    check(ui, "ui", &mut over);
    check(state, "state", &mut over);
    check(effect, "effect", &mut over);
    check(style, "style", &mut over);
    check(a11y, "accessibility", &mut over);
    check(terminal, "terminal", &mut over);
    check(data, "data", &mut over);

    over
}

// ── Tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::corpus::{CorpusProvenance, ProvenanceSourceType};

    fn make_entry(slug: &str, complexity: Vec<ComplexityTag>, features: Vec<&str>) -> CorpusEntry {
        CorpusEntry {
            slug: slug.to_string(),
            description: format!("Test {slug}"),
            source_url: format!("https://test/{slug}"),
            pinned_commit: "abc123".to_string(),
            license: "MIT".to_string(),
            license_verified: true,
            provenance: CorpusProvenance {
                added_by: "test".to_string(),
                added_at: "2026-01-01T00:00:00Z".to_string(),
                rationale: "test".to_string(),
                source_type: ProvenanceSourceType::Synthetic,
                attribution_notes: None,
            },
            complexity_tags: complexity,
            feature_tags: features.into_iter().map(String::from).collect(),
            expected_metrics: None,
            active: true,
        }
    }

    #[test]
    fn trivial_entry_has_static_content() {
        let entry = make_entry("trivial", vec![ComplexityTag::Trivial], vec![]);
        let ann = annotate_entry(&entry);
        assert!(ann.ui_patterns.contains(&UiPattern::StaticContent));
        assert_eq!(ann.complexity_score.tier, ComplexityTier::Basic);
    }

    #[test]
    fn large_entry_has_composition_patterns() {
        let entry = make_entry("large", vec![ComplexityTag::Large], vec![]);
        let ann = annotate_entry(&entry);
        assert!(ann.ui_patterns.contains(&UiPattern::NestedComposition));
        assert!(ann.ui_patterns.contains(&UiPattern::ConditionalRender));
        assert!(ann.ui_patterns.contains(&UiPattern::ListRender));
    }

    #[test]
    fn global_state_tag_infers_external_store() {
        let entry = make_entry("store", vec![ComplexityTag::GlobalState], vec![]);
        let ann = annotate_entry(&entry);
        assert!(ann.state_patterns.contains(&StatePattern::ExternalStore));
        assert!(ann.state_patterns.contains(&StatePattern::DerivedState));
    }

    #[test]
    fn feature_tags_infer_patterns() {
        let entry = make_entry("feat", vec![], vec!["hooks", "reducer", "context", "fetch"]);
        let ann = annotate_entry(&entry);
        assert!(ann.state_patterns.contains(&StatePattern::LocalState));
        assert!(ann.state_patterns.contains(&StatePattern::Reducer));
        assert!(ann.state_patterns.contains(&StatePattern::ContextState));
        assert!(ann.effect_patterns.contains(&EffectPattern::MountFetch));
    }

    #[test]
    fn style_feature_tags() {
        let entry = make_entry(
            "style",
            vec![],
            vec!["tailwind", "css-module", "styled-components"],
        );
        let ann = annotate_entry(&entry);
        assert!(ann.style_patterns.contains(&StylePattern::UtilityClasses));
        assert!(ann.style_patterns.contains(&StylePattern::CssModules));
        assert!(ann.style_patterns.contains(&StylePattern::CssInJs));
    }

    #[test]
    fn accessibility_tag_infers_a11y_patterns() {
        let entry = make_entry("a11y", vec![ComplexityTag::Accessibility], vec![]);
        let ann = annotate_entry(&entry);
        assert!(
            ann.accessibility_patterns
                .contains(&AccessibilityPattern::AriaAttributes)
        );
        assert!(
            ann.accessibility_patterns
                .contains(&AccessibilityPattern::KeyboardNavigation)
        );
    }

    #[test]
    fn realtime_tag_infers_effects() {
        let entry = make_entry("rt", vec![ComplexityTag::RealTime], vec![]);
        let ann = annotate_entry(&entry);
        assert!(
            ann.effect_patterns
                .contains(&EffectPattern::WebSocketConnection)
        );
        assert!(ann.effect_patterns.contains(&EffectPattern::Subscription));
    }

    #[test]
    fn complexity_tier_classification() {
        assert_eq!(classify_tier(0), ComplexityTier::Basic);
        assert_eq!(classify_tier(4), ComplexityTier::Basic);
        assert_eq!(classify_tier(5), ComplexityTier::Intermediate);
        assert_eq!(classify_tier(14), ComplexityTier::Intermediate);
        assert_eq!(classify_tier(15), ComplexityTier::Advanced);
        assert_eq!(classify_tier(29), ComplexityTier::Advanced);
        assert_eq!(classify_tier(30), ComplexityTier::Comprehensive);
    }

    #[test]
    fn total_dimensions_consistent() {
        let total = total_dimension_count();
        let sum = all_ui_pattern_names().len()
            + all_state_pattern_names().len()
            + all_effect_pattern_names().len()
            + all_style_pattern_names().len()
            + all_accessibility_pattern_names().len()
            + all_terminal_pattern_names().len()
            + all_data_pattern_names().len();
        assert_eq!(total, sum);
    }

    #[test]
    fn coverage_report_counts_correctly() {
        let e1 = make_entry(
            "a",
            vec![ComplexityTag::Large, ComplexityTag::GlobalState],
            vec!["hooks"],
        );
        let e2 = make_entry("b", vec![ComplexityTag::Small], vec!["fetch"]);
        let anns = vec![annotate_entry(&e1), annotate_entry(&e2)];
        let report = compute_coverage(&anns);
        assert_eq!(report.stats.total_fixtures, 2);
        assert!(report.stats.total_dimensions_covered > 0);
        assert!(report.stats.coverage_percentage > 0.0);
    }

    #[test]
    fn blind_spots_detected() {
        let entry = make_entry("minimal", vec![ComplexityTag::Trivial], vec![]);
        let anns = vec![annotate_entry(&entry)];
        let report = compute_coverage(&anns);
        // With only StaticContent, most dimensions should be blind spots.
        assert!(!report.blind_spots.is_empty());
        assert!(
            report
                .blind_spots
                .iter()
                .any(|b| b.impact == BlindSpotImpact::High)
        );
    }

    #[test]
    fn overrepresented_detected_when_all_same() {
        let entries: Vec<_> = (0..10)
            .map(|i| make_entry(&format!("e{i}"), vec![ComplexityTag::Large], vec![]))
            .collect();
        let anns: Vec<_> = entries.iter().map(annotate_entry).collect();
        let report = compute_coverage(&anns);
        // All 10 fixtures have the same patterns, so they're all >70%.
        assert!(!report.overrepresented.is_empty());
    }

    #[test]
    fn no_overrepresented_with_diverse_corpus() {
        let e1 = make_entry("a", vec![ComplexityTag::Trivial], vec![]);
        let e2 = make_entry("b", vec![ComplexityTag::GlobalState], vec![]);
        let e3 = make_entry("c", vec![ComplexityTag::RealTime], vec![]);
        let e4 = make_entry("d", vec![ComplexityTag::Accessibility], vec![]);
        let anns: Vec<_> = [&e1, &e2, &e3, &e4]
            .iter()
            .map(|e| annotate_entry(e))
            .collect();
        let report = compute_coverage(&anns);
        // With diverse entries, no single pattern should exceed 70%.
        // StaticContent only in 'a' (25%), etc.
        let over_ui: Vec<_> = report
            .overrepresented
            .iter()
            .filter(|o| o.category == "ui")
            .collect();
        assert!(
            over_ui.is_empty(),
            "unexpected overrepresented UI patterns: {:?}",
            over_ui
        );
    }

    #[test]
    fn analyze_corpus_coverage_integration() {
        let mut entries = BTreeMap::new();
        let e = make_entry(
            "test",
            vec![ComplexityTag::Medium, ComplexityTag::CustomHooks],
            vec!["hooks", "context"],
        );
        entries.insert(e.slug.clone(), e);

        let hash = crate::corpus::CorpusManifest::compute_hash(&entries);
        let manifest = CorpusManifest {
            schema_version: "v1".to_string(),
            updated_at: "now".to_string(),
            manifest_hash: hash,
            entries,
        };

        let (anns, report) = analyze_corpus_coverage(&manifest);
        assert_eq!(anns.len(), 1);
        assert!(report.stats.total_dimensions_covered > 0);
    }

    #[test]
    fn annotation_serialization_roundtrip() {
        let entry = make_entry(
            "rt",
            vec![ComplexityTag::Large, ComplexityTag::RealTime],
            vec!["hooks"],
        );
        let ann = annotate_entry(&entry);
        let json = serde_json::to_string(&ann).unwrap();
        let back: FixtureAnnotation = serde_json::from_str(&json).unwrap();
        assert_eq!(ann.slug, back.slug);
        assert_eq!(
            ann.complexity_score.dimension_count,
            back.complexity_score.dimension_count
        );
    }

    #[test]
    fn blind_spot_impact_classification() {
        assert_eq!(
            classify_blind_spot_impact("ui", "ConditionalRender"),
            BlindSpotImpact::High
        );
        assert_eq!(
            classify_blind_spot_impact("ui", "PortalModal"),
            BlindSpotImpact::Medium
        );
        assert_eq!(
            classify_blind_spot_impact("ui", "RecursiveTree"),
            BlindSpotImpact::Low
        );
    }

    #[test]
    fn inactive_entries_excluded_from_analysis() {
        let mut entries = BTreeMap::new();
        let e = make_entry("active", vec![ComplexityTag::Small], vec![]);
        entries.insert(e.slug.clone(), e);

        let mut inactive = make_entry("inactive", vec![ComplexityTag::Large], vec![]);
        inactive.active = false;
        entries.insert(inactive.slug.clone(), inactive);

        let hash = crate::corpus::CorpusManifest::compute_hash(&entries);
        let manifest = CorpusManifest {
            schema_version: "v1".to_string(),
            updated_at: "now".to_string(),
            manifest_hash: hash,
            entries,
        };

        let (anns, report) = analyze_corpus_coverage(&manifest);
        assert_eq!(anns.len(), 1); // Only active entry.
        assert_eq!(report.stats.total_fixtures, 1);
    }
}
