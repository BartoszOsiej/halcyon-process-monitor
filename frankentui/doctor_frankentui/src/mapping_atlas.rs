// SPDX-License-Identifier: Apache-2.0
//! IR-to-FrankenTUI construct mapping atlas.
//!
//! Defines canonical mappings from `MigrationIr` constructs to FrankenTUI
//! runtime primitives (`Model`/`update`/`view`, `Cmd`, `Subscription`,
//! layout constraints, widgets, style). Each mapping entry carries:
//!
//! - Policy class (`Exact`, `Approximate`, `ExtendFtui`, `Unsupported`)
//! - Preconditions that must hold for the mapping to apply
//! - Failure modes when preconditions are violated
//! - Remediation strategy for human-guided resolution
//!
//! The atlas is versioned and consumed by the translation planner and
//! certification reporting.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::migration_ir::{EffectKind, EventKind, LayoutKind, StateScope, ViewNodeKind};
use crate::semantic_contract::{TransformationHandlingClass, TransformationRiskLevel};

// ── Atlas Version ───────────────────────────────────────────────────────

/// Current atlas schema version.
pub const ATLAS_VERSION: &str = "mapping-atlas-v3";

/// Previous atlas versions compatible with this one.
pub const ATLAS_COMPAT: &[&str] = &["mapping-atlas-v2", "mapping-atlas-v1"];

/// Compatibility notes for previous atlas versions.
pub const ATLAS_COMPAT_NOTES: &[(&str, &str)] = &[
    (
        "mapping-atlas-v2",
        "EffectKind::Process now maps to ProcessSubscription/Cmd::task fallback and legacy process gaps can auto-resolve.",
    ),
    (
        "mapping-atlas-v1",
        "v1 style/layout signatures remain loadable through v2+v3 compatibility shims.",
    ),
];

// ── Core Types ──────────────────────────────────────────────────────────

/// The complete mapping atlas.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MappingAtlas {
    /// Atlas schema version.
    pub version: String,
    /// View node kind mappings.
    pub view_mappings: BTreeMap<String, MappingEntry>,
    /// State scope mappings.
    pub state_mappings: BTreeMap<String, MappingEntry>,
    /// Event kind mappings.
    pub event_mappings: BTreeMap<String, MappingEntry>,
    /// Effect kind mappings.
    pub effect_mappings: BTreeMap<String, MappingEntry>,
    /// Layout kind mappings.
    pub layout_mappings: BTreeMap<String, MappingEntry>,
    /// Style/theme mappings.
    pub style_mappings: BTreeMap<String, MappingEntry>,
    /// Accessibility mappings.
    pub accessibility_mappings: BTreeMap<String, MappingEntry>,
    /// Capability mappings.
    pub capability_mappings: BTreeMap<String, MappingEntry>,
}

/// A single mapping entry from an IR construct to a FrankenTUI target.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MappingEntry {
    /// Source construct signature (e.g. "ViewNodeKind::Component").
    pub source_signature: String,
    /// Policy class from the transformation contract.
    pub policy: TransformationHandlingClass,
    /// Risk level of this mapping.
    pub risk: TransformationRiskLevel,
    /// Target FrankenTUI construct (e.g. "Model impl struct").
    pub target: FtuiTarget,
    /// Preconditions that must hold for this mapping to apply.
    pub preconditions: Vec<Precondition>,
    /// Known failure modes when mapping doesn't work cleanly.
    pub failure_modes: Vec<FailureMode>,
    /// Strategy for human-guided resolution of mapping issues.
    pub remediation: RemediationStrategy,
    /// Category for grouping in reports.
    pub category: MappingCategory,
}

/// The target FrankenTUI construct for a mapping.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FtuiTarget {
    /// Target construct name (e.g. "Model", "Cmd::Task", `"Subscription<M>"`).
    pub construct: String,
    /// Target crate (e.g. "ftui-runtime", "ftui-layout", "ftui-widgets").
    pub crate_name: String,
    /// Brief description of how the mapping works.
    pub description: String,
}

/// A precondition for a mapping to apply.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Precondition {
    /// What must be true.
    pub condition: String,
    /// What happens if not satisfied.
    pub on_violation: String,
}

/// A known failure mode for a mapping.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FailureMode {
    /// Description of the failure scenario.
    pub scenario: String,
    /// How to detect this failure.
    pub detection: String,
    /// Impact on the translated output.
    pub impact: String,
}

/// Strategy for resolving mapping issues.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RemediationStrategy {
    /// Suggested approach.
    pub approach: String,
    /// Whether this can be automated.
    pub automatable: bool,
    /// Estimated effort level.
    pub effort: EffortLevel,
}

/// Effort level for remediation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum EffortLevel {
    Trivial,
    Low,
    Medium,
    High,
}

/// Category for grouping mappings in reports.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum MappingCategory {
    View,
    State,
    Event,
    Effect,
    Layout,
    Style,
    Accessibility,
    Capability,
}

// ── Atlas Builder ───────────────────────────────────────────────────────

/// Build the canonical mapping atlas with all known construct mappings.
pub fn build_atlas() -> MappingAtlas {
    MappingAtlas {
        version: ATLAS_VERSION.to_string(),
        view_mappings: build_view_mappings(),
        state_mappings: build_state_mappings(),
        event_mappings: build_event_mappings(),
        effect_mappings: build_effect_mappings(),
        layout_mappings: build_layout_mappings(),
        style_mappings: build_style_mappings(),
        accessibility_mappings: build_accessibility_mappings(),
        capability_mappings: build_capability_mappings(),
    }
}

/// Look up a mapping entry by construct signature across all categories.
pub fn lookup<'a>(atlas: &'a MappingAtlas, signature: &str) -> Option<&'a MappingEntry> {
    atlas
        .view_mappings
        .get(signature)
        .or_else(|| atlas.state_mappings.get(signature))
        .or_else(|| atlas.event_mappings.get(signature))
        .or_else(|| atlas.effect_mappings.get(signature))
        .or_else(|| atlas.layout_mappings.get(signature))
        .or_else(|| atlas.style_mappings.get(signature))
        .or_else(|| atlas.accessibility_mappings.get(signature))
        .or_else(|| atlas.capability_mappings.get(signature))
}

/// Get all mappings with a specific policy class.
pub fn by_policy(atlas: &MappingAtlas, policy: TransformationHandlingClass) -> Vec<&MappingEntry> {
    all_entries(atlas).filter(|e| e.policy == policy).collect()
}

/// Get all mappings with a specific risk level.
pub fn by_risk(atlas: &MappingAtlas, risk: TransformationRiskLevel) -> Vec<&MappingEntry> {
    all_entries(atlas).filter(|e| e.risk == risk).collect()
}

/// Get all mappings in a specific category.
pub fn by_category(atlas: &MappingAtlas, category: MappingCategory) -> Vec<&MappingEntry> {
    all_entries(atlas)
        .filter(|e| e.category == category)
        .collect()
}

/// Compute atlas statistics for reporting.
pub fn atlas_stats(atlas: &MappingAtlas) -> AtlasStats {
    let all: Vec<&MappingEntry> = all_entries(atlas).collect();
    let total = all.len();

    let exact = all
        .iter()
        .filter(|e| e.policy == TransformationHandlingClass::Exact)
        .count();
    let approximate = all
        .iter()
        .filter(|e| e.policy == TransformationHandlingClass::Approximate)
        .count();
    let extend = all
        .iter()
        .filter(|e| e.policy == TransformationHandlingClass::ExtendFtui)
        .count();
    let unsupported = all
        .iter()
        .filter(|e| e.policy == TransformationHandlingClass::Unsupported)
        .count();

    let automatable = all.iter().filter(|e| e.remediation.automatable).count();

    AtlasStats {
        total,
        exact,
        approximate,
        extend,
        unsupported,
        automatable,
        coverage_ratio: if total > 0 {
            (exact + approximate) as f64 / total as f64
        } else {
            0.0
        },
    }
}

/// Atlas statistics.
#[derive(Debug, Clone)]
pub struct AtlasStats {
    pub total: usize,
    pub exact: usize,
    pub approximate: usize,
    pub extend: usize,
    pub unsupported: usize,
    pub automatable: usize,
    /// Ratio of (exact + approximate) / total.
    pub coverage_ratio: f64,
}

/// Check if an atlas version is compatible with this one.
pub fn is_compatible(version: &str) -> bool {
    version == ATLAS_VERSION || ATLAS_COMPAT.contains(&version)
}

/// Result of re-evaluating a gap against a newer atlas.
#[derive(Debug, Clone)]
pub struct GapReevaluation {
    /// Gap signatures that are now resolved (mapping exists and is not Unsupported).
    pub resolved: Vec<String>,
    /// Gap signatures that remain unresolved.
    pub remaining: Vec<String>,
    /// Gap signatures that improved (e.g. ExtendFtui → Approximate).
    pub improved: Vec<String>,
}

/// Re-evaluate a set of gap signatures against the current atlas.
///
/// Takes signatures from previously emitted gap tickets and checks whether
/// the current atlas now has better mappings for them.
pub fn reevaluate_gaps(atlas: &MappingAtlas, gap_signatures: &[String]) -> GapReevaluation {
    let mut resolved = Vec::new();
    let mut remaining = Vec::new();
    let mut improved = Vec::new();

    for sig in gap_signatures {
        match lookup(atlas, sig) {
            Some(entry) => match entry.policy {
                TransformationHandlingClass::Exact | TransformationHandlingClass::Approximate => {
                    resolved.push(sig.clone());
                }
                TransformationHandlingClass::ExtendFtui => {
                    improved.push(sig.clone());
                }
                TransformationHandlingClass::Unsupported => {
                    remaining.push(sig.clone());
                }
            },
            None => {
                remaining.push(sig.clone());
            }
        }
    }

    GapReevaluation {
        resolved,
        remaining,
        improved,
    }
}

/// Iterate over all entries in all categories.
fn all_entries(atlas: &MappingAtlas) -> impl Iterator<Item = &MappingEntry> {
    atlas
        .view_mappings
        .values()
        .chain(atlas.state_mappings.values())
        .chain(atlas.event_mappings.values())
        .chain(atlas.effect_mappings.values())
        .chain(atlas.layout_mappings.values())
        .chain(atlas.style_mappings.values())
        .chain(atlas.accessibility_mappings.values())
        .chain(atlas.capability_mappings.values())
}

// ── View Mappings ───────────────────────────────────────────────────────

fn build_view_mappings() -> BTreeMap<String, MappingEntry> {
    let mut m = BTreeMap::new();

    m.insert(
        sig_view(ViewNodeKind::Component),
        MappingEntry {
            source_signature: sig_view(ViewNodeKind::Component),
            policy: TransformationHandlingClass::Exact,
            risk: TransformationRiskLevel::Low,
            target: FtuiTarget {
                construct: "Model impl struct".into(),
                crate_name: "ftui-runtime".into(),
                description:
                    "Each component becomes a Model implementation with update/view methods".into(),
            },
            preconditions: vec![Precondition {
                condition: "Component has well-defined props interface".into(),
                on_violation: "Generate stub Model with TODO markers".into(),
            }],
            failure_modes: vec![FailureMode {
                scenario: "Component uses render props or higher-order patterns".into(),
                detection: "SlotDecl with RenderProp accept type".into(),
                impact: "May require manual restructuring of composition".into(),
            }],
            remediation: RemediationStrategy {
                approach: "Direct mapping: function component → Model struct with Message enum"
                    .into(),
                automatable: true,
                effort: EffortLevel::Low,
            },
            category: MappingCategory::View,
        },
    );

    m.insert(
        sig_view(ViewNodeKind::Element),
        MappingEntry {
            source_signature: sig_view(ViewNodeKind::Element),
            policy: TransformationHandlingClass::Approximate,
            risk: TransformationRiskLevel::Medium,
            target: FtuiTarget {
                construct: "Widget render call".into(),
                crate_name: "ftui-widgets".into(),
                description: "HTML elements map to ftui widget primitives (Block, Paragraph, List, etc.)".into(),
            },
            preconditions: vec![
                Precondition {
                    condition: "Element has a known ftui widget equivalent".into(),
                    on_violation: "Fall back to Block with inner text content".into(),
                },
            ],
            failure_modes: vec![
                FailureMode {
                    scenario: "Complex HTML element with no TUI equivalent (canvas, video, iframe)".into(),
                    detection: "Element name not in known widget mapping table".into(),
                    impact: "Content may be omitted or replaced with placeholder".into(),
                },
            ],
            remediation: RemediationStrategy {
                approach: "Map common elements: div→Block, span→Span, button→Button widget, input→Input widget, ul/ol→List, table→Table".into(),
                automatable: true,
                effort: EffortLevel::Medium,
            },
            category: MappingCategory::View,
        },
    );

    m.insert(
        sig_view(ViewNodeKind::Fragment),
        MappingEntry {
            source_signature: sig_view(ViewNodeKind::Fragment),
            policy: TransformationHandlingClass::Exact,
            risk: TransformationRiskLevel::Low,
            target: FtuiTarget {
                construct: "Inline children (no wrapper)".into(),
                crate_name: "ftui-runtime".into(),
                description: "Fragments are transparent — children rendered directly in parent"
                    .into(),
            },
            preconditions: vec![],
            failure_modes: vec![],
            remediation: RemediationStrategy {
                approach: "Hoist fragment children to parent during translation".into(),
                automatable: true,
                effort: EffortLevel::Trivial,
            },
            category: MappingCategory::View,
        },
    );

    m.insert(
        sig_view(ViewNodeKind::Portal),
        MappingEntry {
            source_signature: sig_view(ViewNodeKind::Portal),
            policy: TransformationHandlingClass::Approximate,
            risk: TransformationRiskLevel::Medium,
            target: FtuiTarget {
                construct: "Overlay/popup layer".into(),
                crate_name: "ftui-widgets".into(),
                description: "Portals become overlay widgets rendered on top of main content"
                    .into(),
            },
            preconditions: vec![Precondition {
                condition: "Portal renders to a bounded overlay area".into(),
                on_violation: "Render inline with z-index simulation".into(),
            }],
            failure_modes: vec![FailureMode {
                scenario: "Portal targets outside terminal viewport".into(),
                detection: "Portal target resolves to non-terminal container".into(),
                impact: "Content clipped to terminal bounds".into(),
            }],
            remediation: RemediationStrategy {
                approach: "Use ftui overlay/popup widget with focus trapping".into(),
                automatable: true,
                effort: EffortLevel::Medium,
            },
            category: MappingCategory::View,
        },
    );

    m.insert(
        sig_view(ViewNodeKind::Provider),
        MappingEntry {
            source_signature: sig_view(ViewNodeKind::Provider),
            policy: TransformationHandlingClass::Exact,
            risk: TransformationRiskLevel::Low,
            target: FtuiTarget {
                construct: "Shared state via Model fields".into(),
                crate_name: "ftui-runtime".into(),
                description: "Context providers become shared state fields on parent Model".into(),
            },
            preconditions: vec![Precondition {
                condition: "Provider value is serializable state".into(),
                on_violation: "Wrap in Arc<Mutex<>> for shared access".into(),
            }],
            failure_modes: vec![],
            remediation: RemediationStrategy {
                approach: "Lift provider state to nearest common ancestor Model struct".into(),
                automatable: true,
                effort: EffortLevel::Low,
            },
            category: MappingCategory::View,
        },
    );

    m.insert(
        sig_view(ViewNodeKind::Consumer),
        MappingEntry {
            source_signature: sig_view(ViewNodeKind::Consumer),
            policy: TransformationHandlingClass::Exact,
            risk: TransformationRiskLevel::Low,
            target: FtuiTarget {
                construct: "Model field access".into(),
                crate_name: "ftui-runtime".into(),
                description:
                    "Context consumers become reads from parent Model's shared state fields".into(),
            },
            preconditions: vec![],
            failure_modes: vec![],
            remediation: RemediationStrategy {
                approach: "Replace useContext() with direct field access on Model".into(),
                automatable: true,
                effort: EffortLevel::Trivial,
            },
            category: MappingCategory::View,
        },
    );

    m.insert(
        sig_view(ViewNodeKind::Route),
        MappingEntry {
            source_signature: sig_view(ViewNodeKind::Route),
            policy: TransformationHandlingClass::Approximate,
            risk: TransformationRiskLevel::High,
            target: FtuiTarget {
                construct: "Screen/page switching via Model state".into(),
                crate_name: "ftui-runtime".into(),
                description: "Routes become an enum-driven screen selector in update/view".into(),
            },
            preconditions: vec![Precondition {
                condition: "Routes form a finite set of known screens".into(),
                on_violation: "Generate dynamic dispatch with fallback screen".into(),
            }],
            failure_modes: vec![FailureMode {
                scenario: "Deep nested routing with URL parameters".into(),
                detection: "Route node has parameterized path patterns".into(),
                impact: "URL parameter extraction requires manual mapping".into(),
            }],
            remediation: RemediationStrategy {
                approach: "Create Screen enum, route state variable, and match in view()".into(),
                automatable: true,
                effort: EffortLevel::High,
            },
            category: MappingCategory::View,
        },
    );

    m
}

// ── State Mappings ──────────────────────────────────────────────────────

fn build_state_mappings() -> BTreeMap<String, MappingEntry> {
    let mut m = BTreeMap::new();

    m.insert(
        sig_state(StateScope::Local),
        MappingEntry {
            source_signature: sig_state(StateScope::Local),
            policy: TransformationHandlingClass::Exact,
            risk: TransformationRiskLevel::Low,
            target: FtuiTarget {
                construct: "Model struct field".into(),
                crate_name: "ftui-runtime".into(),
                description: "useState/useReducer → field on Model struct, updated in update()"
                    .into(),
            },
            preconditions: vec![],
            failure_modes: vec![],
            remediation: RemediationStrategy {
                approach: "Direct 1:1 mapping: state variable → Model field".into(),
                automatable: true,
                effort: EffortLevel::Trivial,
            },
            category: MappingCategory::State,
        },
    );

    m.insert(
        sig_state(StateScope::Context),
        MappingEntry {
            source_signature: sig_state(StateScope::Context),
            policy: TransformationHandlingClass::Exact,
            risk: TransformationRiskLevel::Low,
            target: FtuiTarget {
                construct: "Shared Model field (parent struct)".into(),
                crate_name: "ftui-runtime".into(),
                description: "Context state → field on common ancestor Model, passed as props"
                    .into(),
            },
            preconditions: vec![Precondition {
                condition: "Context has identifiable provider ancestor".into(),
                on_violation: "Hoist to root Model as global field".into(),
            }],
            failure_modes: vec![],
            remediation: RemediationStrategy {
                approach: "Lift to ancestor Model, pass down via view() parameters".into(),
                automatable: true,
                effort: EffortLevel::Low,
            },
            category: MappingCategory::State,
        },
    );

    m.insert(
        sig_state(StateScope::Global),
        MappingEntry {
            source_signature: sig_state(StateScope::Global),
            policy: TransformationHandlingClass::Approximate,
            risk: TransformationRiskLevel::Medium,
            target: FtuiTarget {
                construct: "Root Model field or StateRegistry".into(),
                crate_name: "ftui-runtime".into(),
                description:
                    "Redux/Zustand global store → root Model fields with message-based updates"
                        .into(),
            },
            preconditions: vec![Precondition {
                condition: "Global state access patterns are traceable".into(),
                on_violation: "Wrap in opaque store field with accessor messages".into(),
            }],
            failure_modes: vec![FailureMode {
                scenario: "Middleware or selectors with complex derived state".into(),
                detection: "Multiple DerivedState nodes depending on global scope".into(),
                impact: "May need manual restructuring of state access".into(),
            }],
            remediation: RemediationStrategy {
                approach: "Flatten store into Model fields; selectors become computed getters"
                    .into(),
                automatable: true,
                effort: EffortLevel::Medium,
            },
            category: MappingCategory::State,
        },
    );

    m.insert(
        sig_state(StateScope::Route),
        MappingEntry {
            source_signature: sig_state(StateScope::Route),
            policy: TransformationHandlingClass::Approximate,
            risk: TransformationRiskLevel::Medium,
            target: FtuiTarget {
                construct: "Screen enum + current_screen field".into(),
                crate_name: "ftui-runtime".into(),
                description: "URL/route state → enum variant field on Model".into(),
            },
            preconditions: vec![Precondition {
                condition: "Route transitions map to enum variants".into(),
                on_violation: "Use String-based route with match fallback".into(),
            }],
            failure_modes: vec![FailureMode {
                scenario: "Dynamic route segments with runtime parameters".into(),
                detection: "Route state has non-constant initial_value".into(),
                impact: "Parameter extraction requires manual mapping".into(),
            }],
            remediation: RemediationStrategy {
                approach: "Generate Screen enum from known routes; params as enum fields".into(),
                automatable: true,
                effort: EffortLevel::Medium,
            },
            category: MappingCategory::State,
        },
    );

    m.insert(
        sig_state(StateScope::Server),
        MappingEntry {
            source_signature: sig_state(StateScope::Server),
            policy: TransformationHandlingClass::Approximate,
            risk: TransformationRiskLevel::High,
            target: FtuiTarget {
                construct: "Cmd::Task + loading/error state fields".into(),
                crate_name: "ftui-runtime".into(),
                description:
                    "React Query/SWR → Cmd::Task for fetch + Model fields for cache/status".into(),
            },
            preconditions: vec![Precondition {
                condition: "Server state has identifiable fetch URL/key".into(),
                on_violation: "Generate placeholder Cmd::Task with TODO".into(),
            }],
            failure_modes: vec![FailureMode {
                scenario: "Automatic refetching, stale-while-revalidate, optimistic updates".into(),
                detection: "Multiple effect dependencies on same server state".into(),
                impact: "Cache invalidation logic requires manual implementation".into(),
            }],
            remediation: RemediationStrategy {
                approach: "Generate fetch command + loading/data/error fields; manual cache logic"
                    .into(),
                automatable: false,
                effort: EffortLevel::High,
            },
            category: MappingCategory::State,
        },
    );

    m
}

// ── Event Mappings ──────────────────────────────────────────────────────

fn build_event_mappings() -> BTreeMap<String, MappingEntry> {
    let mut m = BTreeMap::new();

    m.insert(
        sig_event(EventKind::UserInput),
        MappingEntry {
            source_signature: sig_event(EventKind::UserInput),
            policy: TransformationHandlingClass::Exact,
            risk: TransformationRiskLevel::Low,
            target: FtuiTarget {
                construct: "Message enum variant + update() match arm".into(),
                crate_name: "ftui-runtime".into(),
                description: "onClick/onKeyDown → Message variant dispatched via From<Event>"
                    .into(),
            },
            preconditions: vec![Precondition {
                condition: "Event has identifiable source node in view tree".into(),
                on_violation: "Generate catch-all event handler".into(),
            }],
            failure_modes: vec![],
            remediation: RemediationStrategy {
                approach: "Map event handler → Message variant; handler body → update() arm".into(),
                automatable: true,
                effort: EffortLevel::Low,
            },
            category: MappingCategory::Event,
        },
    );

    m.insert(
        sig_event(EventKind::Lifecycle),
        MappingEntry {
            source_signature: sig_event(EventKind::Lifecycle),
            policy: TransformationHandlingClass::Approximate,
            risk: TransformationRiskLevel::Medium,
            target: FtuiTarget {
                construct: "Model::init() or Subscription lifecycle".into(),
                crate_name: "ftui-runtime".into(),
                description: "componentDidMount → init(); unmount → subscription cleanup".into(),
            },
            preconditions: vec![],
            failure_modes: vec![FailureMode {
                scenario: "componentDidUpdate with complex prev/next prop comparison".into(),
                detection: "Lifecycle event with multiple state dependencies".into(),
                impact: "May require manual diff logic in update()".into(),
            }],
            remediation: RemediationStrategy {
                approach: "Mount → init(); update → message-driven; unmount → subscription stop"
                    .into(),
                automatable: true,
                effort: EffortLevel::Medium,
            },
            category: MappingCategory::Event,
        },
    );

    m.insert(
        sig_event(EventKind::Timer),
        MappingEntry {
            source_signature: sig_event(EventKind::Timer),
            policy: TransformationHandlingClass::Exact,
            risk: TransformationRiskLevel::Low,
            target: FtuiTarget {
                construct: "Cmd::tick() or Every subscription".into(),
                crate_name: "ftui-runtime".into(),
                description:
                    "setTimeout/setInterval → Cmd::tick(duration) or Every::new(interval, msg)"
                        .into(),
            },
            preconditions: vec![],
            failure_modes: vec![],
            remediation: RemediationStrategy {
                approach: "One-shot → Cmd::tick(); repeating → Every subscription".into(),
                automatable: true,
                effort: EffortLevel::Trivial,
            },
            category: MappingCategory::Event,
        },
    );

    m.insert(
        sig_event(EventKind::Network),
        MappingEntry {
            source_signature: sig_event(EventKind::Network),
            policy: TransformationHandlingClass::Approximate,
            risk: TransformationRiskLevel::Medium,
            target: FtuiTarget {
                construct: "Cmd::Task message via channel".into(),
                crate_name: "ftui-runtime".into(),
                description: "Network response events → message from Cmd::Task completion".into(),
            },
            preconditions: vec![],
            failure_modes: vec![FailureMode {
                scenario: "WebSocket bidirectional communication".into(),
                detection: "Event kind is Network with continuous trigger".into(),
                impact: "Requires custom Subscription implementation".into(),
            }],
            remediation: RemediationStrategy {
                approach: "Request/response → Cmd::Task; streaming → custom Subscription".into(),
                automatable: true,
                effort: EffortLevel::Medium,
            },
            category: MappingCategory::Event,
        },
    );

    m.insert(
        sig_event(EventKind::Custom),
        MappingEntry {
            source_signature: sig_event(EventKind::Custom),
            policy: TransformationHandlingClass::Approximate,
            risk: TransformationRiskLevel::Medium,
            target: FtuiTarget {
                construct: "Custom Message variant".into(),
                crate_name: "ftui-runtime".into(),
                description: "Custom events → additional Message enum variants with explicit dispatch".into(),
            },
            preconditions: vec![],
            failure_modes: vec![
                FailureMode {
                    scenario: "Event bus pattern with dynamic event names".into(),
                    detection: "Custom event with no static source node".into(),
                    impact: "May require runtime event name matching".into(),
                },
            ],
            remediation: RemediationStrategy {
                approach: "Generate Message variant per custom event; manual dispatch in update()".into(),
                automatable: false,
                effort: EffortLevel::Medium,
            },
            category: MappingCategory::Event,
        },
    );

    m
}

// ── Effect Mappings ─────────────────────────────────────────────────────

fn build_effect_mappings() -> BTreeMap<String, MappingEntry> {
    let mut m = BTreeMap::new();

    m.insert(
        sig_effect(EffectKind::Network),
        MappingEntry {
            source_signature: sig_effect(EffectKind::Network),
            policy: TransformationHandlingClass::Approximate,
            risk: TransformationRiskLevel::Medium,
            target: FtuiTarget {
                construct: "Cmd::task(|| fetch(...))".into(),
                crate_name: "ftui-runtime".into(),
                description: "Network effects → background task command returning data message"
                    .into(),
            },
            preconditions: vec![Precondition {
                condition: "Network endpoint is known or configurable".into(),
                on_violation: "Generate placeholder with TODO URL".into(),
            }],
            failure_modes: vec![FailureMode {
                scenario: "Request depends on browser APIs (fetch with credentials, CORS)".into(),
                detection: "Effect reads browser-specific state".into(),
                impact: "Must replace with terminal-compatible HTTP client".into(),
            }],
            remediation: RemediationStrategy {
                approach: "Replace fetch/XHR with reqwest or ureq in Cmd::task closure".into(),
                automatable: true,
                effort: EffortLevel::Medium,
            },
            category: MappingCategory::Effect,
        },
    );

    m.insert(
        sig_effect(EffectKind::Timer),
        MappingEntry {
            source_signature: sig_effect(EffectKind::Timer),
            policy: TransformationHandlingClass::Exact,
            risk: TransformationRiskLevel::Low,
            target: FtuiTarget {
                construct: "Every subscription or Cmd::tick()".into(),
                crate_name: "ftui-runtime".into(),
                description: "Timer effects → Every<M> subscription (repeating) or Cmd::tick (one-shot)".into(),
            },
            preconditions: vec![],
            failure_modes: vec![],
            remediation: RemediationStrategy {
                approach: "setInterval → Every::new(duration, || Msg::Tick); setTimeout → Cmd::tick(duration)".into(),
                automatable: true,
                effort: EffortLevel::Trivial,
            },
            category: MappingCategory::Effect,
        },
    );

    m.insert(
        sig_effect(EffectKind::Dom),
        MappingEntry {
            source_signature: sig_effect(EffectKind::Dom),
            policy: TransformationHandlingClass::Unsupported,
            risk: TransformationRiskLevel::High,
            target: FtuiTarget {
                construct: "N/A (no DOM in terminal)".into(),
                crate_name: "ftui-runtime".into(),
                description: "DOM manipulation has no direct terminal equivalent".into(),
            },
            preconditions: vec![],
            failure_modes: vec![
                FailureMode {
                    scenario: "Direct DOM measurement (getBoundingClientRect, scrollHeight)".into(),
                    detection: "Effect reads DOM state".into(),
                    impact: "Must be replaced with terminal buffer queries".into(),
                },
            ],
            remediation: RemediationStrategy {
                approach: "Replace DOM reads with buffer/frame queries; remove DOM writes (view handles rendering)".into(),
                automatable: false,
                effort: EffortLevel::High,
            },
            category: MappingCategory::Effect,
        },
    );

    m.insert(
        sig_effect(EffectKind::Storage),
        MappingEntry {
            source_signature: sig_effect(EffectKind::Storage),
            policy: TransformationHandlingClass::Approximate,
            risk: TransformationRiskLevel::Low,
            target: FtuiTarget {
                construct: "Cmd::save_state() / Cmd::restore_state()".into(),
                crate_name: "ftui-runtime".into(),
                description: "localStorage/sessionStorage → StateRegistry persistence commands"
                    .into(),
            },
            preconditions: vec![Precondition {
                condition: "Storage keys map to serializable Model fields".into(),
                on_violation: "Generate manual file I/O in Cmd::task".into(),
            }],
            failure_modes: vec![],
            remediation: RemediationStrategy {
                approach:
                    "Map to Cmd::save_state()/restore_state() for simple KV; file I/O for complex"
                        .into(),
                automatable: true,
                effort: EffortLevel::Low,
            },
            category: MappingCategory::Effect,
        },
    );

    m.insert(
        sig_effect(EffectKind::Subscription),
        MappingEntry {
            source_signature: sig_effect(EffectKind::Subscription),
            policy: TransformationHandlingClass::Exact,
            risk: TransformationRiskLevel::Low,
            target: FtuiTarget {
                construct: "Subscription<M> trait impl".into(),
                crate_name: "ftui-runtime".into(),
                description: "Event listeners/observables → custom Subscription implementation"
                    .into(),
            },
            preconditions: vec![],
            failure_modes: vec![],
            remediation: RemediationStrategy {
                approach: "Implement Subscription trait: id(), run(sender, stop)".into(),
                automatable: true,
                effort: EffortLevel::Low,
            },
            category: MappingCategory::Effect,
        },
    );

    m.insert(
        sig_effect(EffectKind::Process),
        MappingEntry {
            source_signature: sig_effect(EffectKind::Process),
            policy: TransformationHandlingClass::Approximate,
            risk: TransformationRiskLevel::Medium,
            target: FtuiTarget {
                construct: "ProcessSubscription<M> (or Cmd::task fallback)".into(),
                crate_name: "ftui-runtime".into(),
                description:
                    "Worker/child process → ProcessSubscription when spawn is available, otherwise Cmd::task fallback"
                        .into(),
            },
            preconditions: vec![Precondition {
                condition: "Process spawn is available in target environment".into(),
                on_violation: "Fall back to in-process task".into(),
            }],
            failure_modes: vec![FailureMode {
                scenario: "Web Worker API not available in terminal context".into(),
                detection: "Process effect targets browser Worker API".into(),
                impact: "Must replace with std::process::Command or thread".into(),
            }],
            remediation: RemediationStrategy {
                approach:
                    "Prefer ProcessSubscription::new(...); if unavailable, map to bounded Cmd::task in-process worker".into(),
                automatable: true,
                effort: EffortLevel::Medium,
            },
            category: MappingCategory::Effect,
        },
    );

    m.insert(
        sig_effect(EffectKind::Telemetry),
        MappingEntry {
            source_signature: sig_effect(EffectKind::Telemetry),
            policy: TransformationHandlingClass::Approximate,
            risk: TransformationRiskLevel::Low,
            target: FtuiTarget {
                construct: "Cmd::log() or tracing macro".into(),
                crate_name: "ftui-runtime".into(),
                description: "Analytics/logging → Cmd::log() for scrollback or tracing::info!()"
                    .into(),
            },
            preconditions: vec![],
            failure_modes: vec![],
            remediation: RemediationStrategy {
                approach: "Replace console.log → Cmd::log(); analytics → tracing events".into(),
                automatable: true,
                effort: EffortLevel::Trivial,
            },
            category: MappingCategory::Effect,
        },
    );

    m.insert(
        sig_effect(EffectKind::Other),
        MappingEntry {
            source_signature: sig_effect(EffectKind::Other),
            policy: TransformationHandlingClass::Unsupported,
            risk: TransformationRiskLevel::High,
            target: FtuiTarget {
                construct: "Manual implementation required".into(),
                crate_name: "ftui-runtime".into(),
                description: "Unknown effects require manual analysis and mapping".into(),
            },
            preconditions: vec![],
            failure_modes: vec![FailureMode {
                scenario: "Effect uses browser-specific APIs with no terminal equivalent".into(),
                detection: "Effect kind is Other with no cleanup or state interaction".into(),
                impact: "Effect may need to be dropped or substantially rewritten".into(),
            }],
            remediation: RemediationStrategy {
                approach: "Manual analysis required; generate TODO stub with provenance link"
                    .into(),
                automatable: false,
                effort: EffortLevel::High,
            },
            category: MappingCategory::Effect,
        },
    );

    m
}

// ── Layout Mappings ─────────────────────────────────────────────────────

fn build_layout_mappings() -> BTreeMap<String, MappingEntry> {
    let mut m = BTreeMap::new();

    m.insert(
        sig_layout(LayoutKind::Flex),
        MappingEntry {
            source_signature: sig_layout(LayoutKind::Flex),
            policy: TransformationHandlingClass::Exact,
            risk: TransformationRiskLevel::Low,
            target: FtuiTarget {
                construct: "Layout::horizontal() / Layout::vertical() with Constraint".into(),
                crate_name: "ftui-layout".into(),
                description: "Flexbox → ftui-layout constraints with direction and sizing".into(),
            },
            preconditions: vec![],
            failure_modes: vec![
                FailureMode {
                    scenario: "flex-wrap causing multi-line flow".into(),
                    detection: "Flex layout with wrap enabled".into(),
                    impact: "Terminal has no native wrapping flex; may need manual chunking".into(),
                },
            ],
            remediation: RemediationStrategy {
                approach: "Map flex-direction to Layout direction; flex-grow to Percentage/Min constraints".into(),
                automatable: true,
                effort: EffortLevel::Low,
            },
            category: MappingCategory::Layout,
        },
    );

    m.insert(
        sig_layout(LayoutKind::Grid),
        MappingEntry {
            source_signature: sig_layout(LayoutKind::Grid),
            policy: TransformationHandlingClass::Approximate,
            risk: TransformationRiskLevel::Medium,
            target: FtuiTarget {
                construct: "Nested Layout with row/column constraints".into(),
                crate_name: "ftui-layout".into(),
                description: "CSS Grid → nested horizontal/vertical layouts with fixed constraints"
                    .into(),
            },
            preconditions: vec![Precondition {
                condition: "Grid template is static (not responsive)".into(),
                on_violation: "Approximate with equal-width columns".into(),
            }],
            failure_modes: vec![FailureMode {
                scenario: "Grid with auto-placement, named areas, or span".into(),
                detection: "Complex grid template in style intent".into(),
                impact: "Auto-placement not supported; items placed sequentially".into(),
            }],
            remediation: RemediationStrategy {
                approach: "Convert grid rows/columns to nested Layout constraints".into(),
                automatable: true,
                effort: EffortLevel::Medium,
            },
            category: MappingCategory::Layout,
        },
    );

    m.insert(
        sig_layout(LayoutKind::Absolute),
        MappingEntry {
            source_signature: sig_layout(LayoutKind::Absolute),
            policy: TransformationHandlingClass::Approximate,
            risk: TransformationRiskLevel::High,
            target: FtuiTarget {
                construct: "Rect-based positioning in view()".into(),
                crate_name: "ftui-render".into(),
                description: "Absolute positioning → explicit Rect coordinates in view rendering"
                    .into(),
            },
            preconditions: vec![Precondition {
                condition: "Absolute positions are within terminal bounds".into(),
                on_violation: "Clamp to terminal dimensions".into(),
            }],
            failure_modes: vec![FailureMode {
                scenario: "Absolute elements overlapping with unpredictable z-order".into(),
                detection: "Multiple absolute elements in same container".into(),
                impact: "Rendering order determines visual stacking (last wins)".into(),
            }],
            remediation: RemediationStrategy {
                approach: "Convert CSS pixels to terminal cells; render at explicit Rect".into(),
                automatable: false,
                effort: EffortLevel::High,
            },
            category: MappingCategory::Layout,
        },
    );

    m.insert(
        sig_layout(LayoutKind::Stack),
        MappingEntry {
            source_signature: sig_layout(LayoutKind::Stack),
            policy: TransformationHandlingClass::Exact,
            risk: TransformationRiskLevel::Low,
            target: FtuiTarget {
                construct: "Layered rendering in view() (draw order)".into(),
                crate_name: "ftui-render".into(),
                description:
                    "Stack/z-axis layering → sequential render calls with later overwriting earlier"
                        .into(),
            },
            preconditions: vec![],
            failure_modes: vec![],
            remediation: RemediationStrategy {
                approach: "Render stack children in order; last child is topmost".into(),
                automatable: true,
                effort: EffortLevel::Trivial,
            },
            category: MappingCategory::Layout,
        },
    );

    m.insert(
        sig_layout(LayoutKind::Flow),
        MappingEntry {
            source_signature: sig_layout(LayoutKind::Flow),
            policy: TransformationHandlingClass::Exact,
            risk: TransformationRiskLevel::Low,
            target: FtuiTarget {
                construct: "Paragraph / text wrapping".into(),
                crate_name: "ftui-text".into(),
                description: "Normal document flow → Paragraph widget with text wrapping".into(),
            },
            preconditions: vec![],
            failure_modes: vec![],
            remediation: RemediationStrategy {
                approach: "Wrap text content in Paragraph widget with line wrapping".into(),
                automatable: true,
                effort: EffortLevel::Trivial,
            },
            category: MappingCategory::Layout,
        },
    );

    m
}

// ── Style Mappings ──────────────────────────────────────────────────────

fn build_style_mappings() -> BTreeMap<String, MappingEntry> {
    let mut m = BTreeMap::new();

    m.insert(
        "StyleToken::Color".into(),
        MappingEntry {
            source_signature: "StyleToken::Color".into(),
            policy: TransformationHandlingClass::Approximate,
            risk: TransformationRiskLevel::Low,
            target: FtuiTarget {
                construct: "Style::fg() / Style::bg() with Color".into(),
                crate_name: "ftui-style".into(),
                description: "CSS colors → ftui Color (Rgb, Indexed, or named)".into(),
            },
            preconditions: vec![Precondition {
                condition: "Terminal supports TrueColor or 256 colors".into(),
                on_violation: "Downgrade to nearest ANSI color".into(),
            }],
            failure_modes: vec![],
            remediation: RemediationStrategy {
                approach: "Parse CSS color → map to Color::Rgb or nearest indexed color".into(),
                automatable: true,
                effort: EffortLevel::Low,
            },
            category: MappingCategory::Style,
        },
    );

    m.insert(
        "StyleToken::Typography".into(),
        MappingEntry {
            source_signature: "StyleToken::Typography".into(),
            policy: TransformationHandlingClass::Approximate,
            risk: TransformationRiskLevel::Medium,
            target: FtuiTarget {
                construct: "Style::add_modifier() with Modifier flags".into(),
                crate_name: "ftui-style".into(),
                description: "Font styles → terminal modifiers (Bold, Italic, Underlined, etc.)".into(),
            },
            preconditions: vec![],
            failure_modes: vec![
                FailureMode {
                    scenario: "Font family, size, or weight has no terminal equivalent".into(),
                    detection: "Typography token references specific font metrics".into(),
                    impact: "Font information lost; only bold/italic/underline preserved".into(),
                },
            ],
            remediation: RemediationStrategy {
                approach: "Map font-weight:bold → Bold, font-style:italic → Italic, text-decoration → Underlined".into(),
                automatable: true,
                effort: EffortLevel::Low,
            },
            category: MappingCategory::Style,
        },
    );

    m.insert(
        "StyleToken::Spacing".into(),
        MappingEntry {
            source_signature: "StyleToken::Spacing".into(),
            policy: TransformationHandlingClass::Approximate,
            risk: TransformationRiskLevel::Medium,
            target: FtuiTarget {
                construct: "Margin/Padding via layout Constraint or Block margins".into(),
                crate_name: "ftui-layout".into(),
                description: "CSS spacing → terminal cell-based margins and padding".into(),
            },
            preconditions: vec![],
            failure_modes: vec![FailureMode {
                scenario: "Sub-cell spacing (e.g., 3px margin in a terminal)".into(),
                detection: "Spacing value smaller than one terminal cell".into(),
                impact: "Rounded to nearest cell boundary; may collapse to zero".into(),
            }],
            remediation: RemediationStrategy {
                approach: "Convert px/rem/em to cell units; round to nearest integer".into(),
                automatable: true,
                effort: EffortLevel::Low,
            },
            category: MappingCategory::Style,
        },
    );

    m.insert(
        "StyleToken::Border".into(),
        MappingEntry {
            source_signature: "StyleToken::Border".into(),
            policy: TransformationHandlingClass::Exact,
            risk: TransformationRiskLevel::Low,
            target: FtuiTarget {
                construct: "Block::bordered() with BorderType".into(),
                crate_name: "ftui-widgets".into(),
                description: "CSS borders → Block widget borders using box-drawing characters"
                    .into(),
            },
            preconditions: vec![],
            failure_modes: vec![],
            remediation: RemediationStrategy {
                approach: "Map border-style to BorderType (Plain, Rounded, Double, Thick)".into(),
                automatable: true,
                effort: EffortLevel::Trivial,
            },
            category: MappingCategory::Style,
        },
    );

    m.insert(
        "ThemeDecl".into(),
        MappingEntry {
            source_signature: "ThemeDecl".into(),
            policy: TransformationHandlingClass::Approximate,
            risk: TransformationRiskLevel::Medium,
            target: FtuiTarget {
                construct: "Theme struct with token overrides".into(),
                crate_name: "ftui-style".into(),
                description: "CSS themes → ftui Theme with resolved color/style tokens".into(),
            },
            preconditions: vec![],
            failure_modes: vec![FailureMode {
                scenario: "CSS custom properties with runtime evaluation".into(),
                detection: "Theme tokens reference var() or calc()".into(),
                impact: "Dynamic tokens resolved at build time; runtime switching limited".into(),
            }],
            remediation: RemediationStrategy {
                approach:
                    "Resolve theme tokens at translation time; generate Theme struct per variant"
                        .into(),
                automatable: true,
                effort: EffortLevel::Medium,
            },
            category: MappingCategory::Style,
        },
    );

    // ── Token categories not yet covered ──

    m.insert(
        "StyleToken::Shadow".into(),
        MappingEntry {
            source_signature: "StyleToken::Shadow".into(),
            policy: TransformationHandlingClass::Approximate,
            risk: TransformationRiskLevel::Medium,
            target: FtuiTarget {
                construct: "Dim/reverse attribute or border emphasis".into(),
                crate_name: "ftui-style".into(),
                description: "Box/text shadows → approximated via Dim attribute or border emphasis"
                    .into(),
            },
            preconditions: vec![],
            failure_modes: vec![FailureMode {
                scenario: "Complex multi-layer shadows with offsets".into(),
                detection: "Shadow token has offset-x/offset-y values".into(),
                impact: "Shadow depth lost; only presence/absence preserved".into(),
            }],
            remediation: RemediationStrategy {
                approach: "Map shadow presence to Dim/Reverse modifier or thicker border".into(),
                automatable: true,
                effort: EffortLevel::Low,
            },
            category: MappingCategory::Style,
        },
    );

    m.insert(
        "StyleToken::Animation".into(),
        MappingEntry {
            source_signature: "StyleToken::Animation".into(),
            policy: TransformationHandlingClass::ExtendFtui,
            risk: TransformationRiskLevel::High,
            target: FtuiTarget {
                construct: "Subscription-driven style updates".into(),
                crate_name: "ftui-runtime".into(),
                description:
                    "CSS animations → timer-based Subscription that updates Model style state"
                        .into(),
            },
            preconditions: vec![Precondition {
                condition: "Animation is tick-driven (not continuous GPU)".into(),
                on_violation: "Drop animation; emit static final-state style".into(),
            }],
            failure_modes: vec![FailureMode {
                scenario: "60fps keyframe animation exceeds terminal refresh rate".into(),
                detection: "Animation duration < 100ms per frame".into(),
                impact: "Animation may appear choppy or be dropped entirely".into(),
            }],
            remediation: RemediationStrategy {
                approach: "Convert keyframes to discrete style states driven by tick Subscription"
                    .into(),
                automatable: false,
                effort: EffortLevel::High,
            },
            category: MappingCategory::Style,
        },
    );

    m.insert(
        "StyleToken::Breakpoint".into(),
        MappingEntry {
            source_signature: "StyleToken::Breakpoint".into(),
            policy: TransformationHandlingClass::Approximate,
            risk: TransformationRiskLevel::Medium,
            target: FtuiTarget {
                construct: "Terminal size query in view()".into(),
                crate_name: "ftui-runtime".into(),
                description:
                    "CSS media breakpoints → terminal width/height checks in view function".into(),
            },
            preconditions: vec![],
            failure_modes: vec![FailureMode {
                scenario: "Breakpoint uses non-size media features (e.g., color-scheme)".into(),
                detection: "Breakpoint condition references non-dimensional query".into(),
                impact: "Non-size breakpoints ignored; layout fixed to default".into(),
            }],
            remediation: RemediationStrategy {
                approach: "Map width/height breakpoints to terminal cols/rows thresholds".into(),
                automatable: true,
                effort: EffortLevel::Low,
            },
            category: MappingCategory::Style,
        },
    );

    m.insert(
        "StyleToken::ZIndex".into(),
        MappingEntry {
            source_signature: "StyleToken::ZIndex".into(),
            policy: TransformationHandlingClass::Approximate,
            risk: TransformationRiskLevel::Medium,
            target: FtuiTarget {
                construct: "Overlay render order in view()".into(),
                crate_name: "ftui-render".into(),
                description: "z-index → overlay rendering order (later renders on top)".into(),
            },
            preconditions: vec![],
            failure_modes: vec![FailureMode {
                scenario: "Complex stacking contexts with negative z-index".into(),
                detection: "z-index value is negative or involves nested stacking contexts".into(),
                impact: "Layer order may not match browser rendering exactly".into(),
            }],
            remediation: RemediationStrategy {
                approach: "Normalize z-index values to sequential overlay ordering".into(),
                automatable: true,
                effort: EffortLevel::Medium,
            },
            category: MappingCategory::Style,
        },
    );

    // ── CSS-like parity properties (added v2) ──

    m.insert(
        "StyleProp::TextTransform".into(),
        MappingEntry {
            source_signature: "StyleProp::TextTransform".into(),
            policy: TransformationHandlingClass::Exact,
            risk: TransformationRiskLevel::Low,
            target: FtuiTarget {
                construct: "TextTransform enum".into(),
                crate_name: "ftui-style".into(),
                description: "text-transform → TextTransform (Uppercase/Lowercase/Capitalize)"
                    .into(),
            },
            preconditions: vec![],
            failure_modes: vec![],
            remediation: RemediationStrategy {
                approach: "Direct 1:1 mapping to TextTransform enum variants".into(),
                automatable: true,
                effort: EffortLevel::Trivial,
            },
            category: MappingCategory::Style,
        },
    );

    m.insert(
        "StyleProp::TextOverflow".into(),
        MappingEntry {
            source_signature: "StyleProp::TextOverflow".into(),
            policy: TransformationHandlingClass::Exact,
            risk: TransformationRiskLevel::Low,
            target: FtuiTarget {
                construct: "TextOverflow enum".into(),
                crate_name: "ftui-style".into(),
                description: "text-overflow → TextOverflow (Clip/Ellipsis/Indicator)".into(),
            },
            preconditions: vec![],
            failure_modes: vec![],
            remediation: RemediationStrategy {
                approach: "Direct 1:1 mapping to TextOverflow enum variants".into(),
                automatable: true,
                effort: EffortLevel::Trivial,
            },
            category: MappingCategory::Style,
        },
    );

    m.insert(
        "StyleProp::Overflow".into(),
        MappingEntry {
            source_signature: "StyleProp::Overflow".into(),
            policy: TransformationHandlingClass::Exact,
            risk: TransformationRiskLevel::Low,
            target: FtuiTarget {
                construct: "Overflow enum".into(),
                crate_name: "ftui-style".into(),
                description: "overflow → Overflow (Visible/Hidden/Scroll/Auto)".into(),
            },
            preconditions: vec![Precondition {
                condition: "Scroll overflow requires scrollable container support".into(),
                on_violation: "Fall back to Hidden overflow".into(),
            }],
            failure_modes: vec![],
            remediation: RemediationStrategy {
                approach: "Direct mapping; Scroll variant uses scrollable widget wrapper".into(),
                automatable: true,
                effort: EffortLevel::Low,
            },
            category: MappingCategory::Style,
        },
    );

    m.insert(
        "StyleProp::WhiteSpace".into(),
        MappingEntry {
            source_signature: "StyleProp::WhiteSpace".into(),
            policy: TransformationHandlingClass::Exact,
            risk: TransformationRiskLevel::Low,
            target: FtuiTarget {
                construct: "WhiteSpaceMode enum".into(),
                crate_name: "ftui-style".into(),
                description: "white-space → WhiteSpaceMode (Normal/Pre/PreWrap/PreLine/NoWrap)"
                    .into(),
            },
            preconditions: vec![],
            failure_modes: vec![],
            remediation: RemediationStrategy {
                approach: "Direct 1:1 mapping to WhiteSpaceMode enum variants".into(),
                automatable: true,
                effort: EffortLevel::Trivial,
            },
            category: MappingCategory::Style,
        },
    );

    m.insert(
        "StyleProp::TextAlign".into(),
        MappingEntry {
            source_signature: "StyleProp::TextAlign".into(),
            policy: TransformationHandlingClass::Exact,
            risk: TransformationRiskLevel::Low,
            target: FtuiTarget {
                construct: "TextAlign enum".into(),
                crate_name: "ftui-style".into(),
                description: "text-align → TextAlign (Left/Right/Center/Justify)".into(),
            },
            preconditions: vec![],
            failure_modes: vec![],
            remediation: RemediationStrategy {
                approach: "Direct 1:1 mapping to TextAlign enum variants".into(),
                automatable: true,
                effort: EffortLevel::Trivial,
            },
            category: MappingCategory::Style,
        },
    );

    m.insert(
        "StyleProp::LineClamp".into(),
        MappingEntry {
            source_signature: "StyleProp::LineClamp".into(),
            policy: TransformationHandlingClass::Exact,
            risk: TransformationRiskLevel::Low,
            target: FtuiTarget {
                construct: "LineClamp struct".into(),
                crate_name: "ftui-style".into(),
                description: "-webkit-line-clamp → LineClamp with max_lines".into(),
            },
            preconditions: vec![],
            failure_modes: vec![],
            remediation: RemediationStrategy {
                approach: "Direct mapping: clamp value → LineClamp::new(n)".into(),
                automatable: true,
                effort: EffortLevel::Trivial,
            },
            category: MappingCategory::Style,
        },
    );

    m
}

// ── Accessibility Mappings ──────────────────────────────────────────────

fn build_accessibility_mappings() -> BTreeMap<String, MappingEntry> {
    let mut m = BTreeMap::new();

    m.insert(
        "AccessibilityEntry::role".into(),
        MappingEntry {
            source_signature: "AccessibilityEntry::role".into(),
            policy: TransformationHandlingClass::Approximate,
            risk: TransformationRiskLevel::Medium,
            target: FtuiTarget {
                construct: "HitRegion semantic tag + widget type selection".into(),
                crate_name: "ftui-render".into(),
                description: "ARIA roles → HitRegion types and widget selection heuristics".into(),
            },
            preconditions: vec![],
            failure_modes: vec![FailureMode {
                scenario: "Complex ARIA patterns (combobox, tree, grid) with no TUI equivalent"
                    .into(),
                detection: "Role maps to composite ARIA widget".into(),
                impact: "Simplified to nearest TUI interaction pattern".into(),
            }],
            remediation: RemediationStrategy {
                approach:
                    "Map ARIA roles to TUI widgets: button→Button, textbox→Input, listbox→List"
                        .into(),
                automatable: true,
                effort: EffortLevel::Medium,
            },
            category: MappingCategory::Accessibility,
        },
    );

    m.insert(
        "AccessibilityEntry::focus_order".into(),
        MappingEntry {
            source_signature: "AccessibilityEntry::focus_order".into(),
            policy: TransformationHandlingClass::Exact,
            risk: TransformationRiskLevel::Low,
            target: FtuiTarget {
                construct: "Tab-order in event dispatch".into(),
                crate_name: "ftui-runtime".into(),
                description: "tabIndex → focus traversal order in keyboard event routing".into(),
            },
            preconditions: vec![],
            failure_modes: vec![],
            remediation: RemediationStrategy {
                approach: "Preserve tabIndex values in focus traversal graph".into(),
                automatable: true,
                effort: EffortLevel::Trivial,
            },
            category: MappingCategory::Accessibility,
        },
    );

    m.insert(
        "AccessibilityEntry::keyboard_shortcut".into(),
        MappingEntry {
            source_signature: "AccessibilityEntry::keyboard_shortcut".into(),
            policy: TransformationHandlingClass::Exact,
            risk: TransformationRiskLevel::Low,
            target: FtuiTarget {
                construct: "KeyEvent match in update()".into(),
                crate_name: "ftui-runtime".into(),
                description: "accesskey → keyboard shortcut handler in Model::update()".into(),
            },
            preconditions: vec![Precondition {
                condition: "Shortcut key combination is available in terminal".into(),
                on_violation: "Remap to alternative key combination".into(),
            }],
            failure_modes: vec![FailureMode {
                scenario: "Shortcut conflicts with terminal emulator key binding".into(),
                detection: "Key combination matches known terminal shortcuts".into(),
                impact: "Shortcut may not reach application".into(),
            }],
            remediation: RemediationStrategy {
                approach: "Map to KeyEvent match arm; check for terminal conflicts".into(),
                automatable: true,
                effort: EffortLevel::Low,
            },
            category: MappingCategory::Accessibility,
        },
    );

    m.insert(
        "AccessibilityEntry::live_region".into(),
        MappingEntry {
            source_signature: "AccessibilityEntry::live_region".into(),
            policy: TransformationHandlingClass::Unsupported,
            risk: TransformationRiskLevel::Medium,
            target: FtuiTarget {
                construct: "N/A (no screen reader integration in terminal)".into(),
                crate_name: "ftui-runtime".into(),
                description: "ARIA live regions have no terminal equivalent".into(),
            },
            preconditions: vec![],
            failure_modes: vec![FailureMode {
                scenario: "Dynamic content updates that should be announced".into(),
                detection: "live_region is 'assertive' or 'polite'".into(),
                impact: "Screen reader announcements not available in terminal".into(),
            }],
            remediation: RemediationStrategy {
                approach: "Preserve as metadata; emit Cmd::log() for important updates".into(),
                automatable: false,
                effort: EffortLevel::Low,
            },
            category: MappingCategory::Accessibility,
        },
    );

    m
}

// ── Capability Mappings ─────────────────────────────────────────────────

fn build_capability_mappings() -> BTreeMap<String, MappingEntry> {
    let mut m = BTreeMap::new();

    for (cap_name, policy, risk, target_desc) in [
        (
            "Capability::MouseInput",
            TransformationHandlingClass::Exact,
            TransformationRiskLevel::Low,
            "Cmd::set_mouse_capture(true) + MouseEvent handling",
        ),
        (
            "Capability::KeyboardInput",
            TransformationHandlingClass::Exact,
            TransformationRiskLevel::Low,
            "KeyEvent handling in Model::update() via From<Event>",
        ),
        (
            "Capability::TouchInput",
            TransformationHandlingClass::Unsupported,
            TransformationRiskLevel::High,
            "N/A (no touch input in terminal)",
        ),
        (
            "Capability::NetworkAccess",
            TransformationHandlingClass::Exact,
            TransformationRiskLevel::Low,
            "Cmd::task() with reqwest/ureq HTTP client",
        ),
        (
            "Capability::FileSystem",
            TransformationHandlingClass::Exact,
            TransformationRiskLevel::Low,
            "Cmd::task() with std::fs operations",
        ),
        (
            "Capability::Clipboard",
            TransformationHandlingClass::Approximate,
            TransformationRiskLevel::Medium,
            "OSC 52 clipboard via terminal escape sequences",
        ),
        (
            "Capability::Timers",
            TransformationHandlingClass::Exact,
            TransformationRiskLevel::Low,
            "Every subscription or Cmd::tick()",
        ),
        (
            "Capability::AlternateScreen",
            TransformationHandlingClass::Exact,
            TransformationRiskLevel::Low,
            "TerminalSession with alternate screen mode",
        ),
        (
            "Capability::TrueColor",
            TransformationHandlingClass::Exact,
            TransformationRiskLevel::Low,
            "Color::Rgb in ftui-style (with capability detection)",
        ),
        (
            "Capability::Unicode",
            TransformationHandlingClass::Exact,
            TransformationRiskLevel::Low,
            "unicode-width crate for grapheme rendering",
        ),
        (
            "Capability::InlineMode",
            TransformationHandlingClass::Exact,
            TransformationRiskLevel::Low,
            "TerminalSession inline mode (scrollback preserved)",
        ),
        (
            "Capability::ProcessSpawn",
            TransformationHandlingClass::Exact,
            TransformationRiskLevel::Low,
            "std::process::Command in Cmd::task()",
        ),
    ] {
        m.insert(
            cap_name.into(),
            MappingEntry {
                source_signature: cap_name.into(),
                policy,
                risk,
                target: FtuiTarget {
                    construct: target_desc.into(),
                    crate_name: "ftui-runtime".into(),
                    description: format!("{cap_name} → {target_desc}"),
                },
                preconditions: vec![],
                failure_modes: vec![],
                remediation: RemediationStrategy {
                    approach: format!("Direct mapping: {cap_name}"),
                    automatable: policy != TransformationHandlingClass::Unsupported,
                    effort: if risk == TransformationRiskLevel::Low {
                        EffortLevel::Trivial
                    } else {
                        EffortLevel::Medium
                    },
                },
                category: MappingCategory::Capability,
            },
        );
    }

    m
}

// ── Signature Helpers ───────────────────────────────────────────────────

fn sig_view(kind: ViewNodeKind) -> String {
    format!("ViewNodeKind::{kind:?}")
}

fn sig_state(scope: StateScope) -> String {
    format!("StateScope::{scope:?}")
}

fn sig_event(kind: EventKind) -> String {
    format!("EventKind::{kind:?}")
}

fn sig_effect(kind: EffectKind) -> String {
    format!("EffectKind::{kind:?}")
}

fn sig_layout(kind: LayoutKind) -> String {
    format!("LayoutKind::{kind:?}")
}

// ── Tests ───────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn atlas_builds_without_panic() {
        let atlas = build_atlas();
        assert_eq!(atlas.version, ATLAS_VERSION);
    }

    #[test]
    fn all_view_node_kinds_covered() {
        let atlas = build_atlas();
        let kinds = [
            ViewNodeKind::Component,
            ViewNodeKind::Element,
            ViewNodeKind::Fragment,
            ViewNodeKind::Portal,
            ViewNodeKind::Provider,
            ViewNodeKind::Consumer,
            ViewNodeKind::Route,
        ];
        for kind in kinds {
            let sig = sig_view(kind.clone());
            assert!(
                atlas.view_mappings.contains_key(&sig),
                "Missing view mapping for {sig}"
            );
        }
    }

    #[test]
    fn all_state_scopes_covered() {
        let atlas = build_atlas();
        let scopes = [
            StateScope::Local,
            StateScope::Context,
            StateScope::Global,
            StateScope::Route,
            StateScope::Server,
        ];
        for scope in scopes {
            let sig = sig_state(scope.clone());
            assert!(
                atlas.state_mappings.contains_key(&sig),
                "Missing state mapping for {sig}"
            );
        }
    }

    #[test]
    fn all_event_kinds_covered() {
        let atlas = build_atlas();
        let kinds = [
            EventKind::UserInput,
            EventKind::Lifecycle,
            EventKind::Timer,
            EventKind::Network,
            EventKind::Custom,
        ];
        for kind in kinds {
            let sig = sig_event(kind.clone());
            assert!(
                atlas.event_mappings.contains_key(&sig),
                "Missing event mapping for {sig}"
            );
        }
    }

    #[test]
    fn all_effect_kinds_covered() {
        let atlas = build_atlas();
        let kinds = [
            EffectKind::Network,
            EffectKind::Timer,
            EffectKind::Dom,
            EffectKind::Storage,
            EffectKind::Subscription,
            EffectKind::Process,
            EffectKind::Telemetry,
            EffectKind::Other,
        ];
        for kind in kinds {
            let sig = sig_effect(kind.clone());
            assert!(
                atlas.effect_mappings.contains_key(&sig),
                "Missing effect mapping for {sig}"
            );
        }
    }

    #[test]
    fn all_layout_kinds_covered() {
        let atlas = build_atlas();
        let kinds = [
            LayoutKind::Flex,
            LayoutKind::Grid,
            LayoutKind::Absolute,
            LayoutKind::Stack,
            LayoutKind::Flow,
        ];
        for kind in kinds {
            let sig = sig_layout(kind.clone());
            assert!(
                atlas.layout_mappings.contains_key(&sig),
                "Missing layout mapping for {sig}"
            );
        }
    }

    #[test]
    fn lookup_finds_entries_across_categories() {
        let atlas = build_atlas();

        assert!(lookup(&atlas, "ViewNodeKind::Component").is_some());
        assert!(lookup(&atlas, "StateScope::Local").is_some());
        assert!(lookup(&atlas, "EventKind::UserInput").is_some());
        assert!(lookup(&atlas, "EffectKind::Network").is_some());
        assert!(lookup(&atlas, "LayoutKind::Flex").is_some());
        assert!(lookup(&atlas, "StyleToken::Color").is_some());
        assert!(lookup(&atlas, "AccessibilityEntry::role").is_some());
        assert!(lookup(&atlas, "Capability::MouseInput").is_some());
    }

    #[test]
    fn lookup_returns_none_for_unknown() {
        let atlas = build_atlas();
        assert!(lookup(&atlas, "NonExistent::Thing").is_none());
    }

    #[test]
    fn by_policy_filters_correctly() {
        let atlas = build_atlas();

        let exact = by_policy(&atlas, TransformationHandlingClass::Exact);
        assert!(!exact.is_empty());
        for entry in &exact {
            assert_eq!(entry.policy, TransformationHandlingClass::Exact);
        }

        let unsupported = by_policy(&atlas, TransformationHandlingClass::Unsupported);
        for entry in &unsupported {
            assert_eq!(entry.policy, TransformationHandlingClass::Unsupported);
        }
    }

    #[test]
    fn by_category_filters_correctly() {
        let atlas = build_atlas();

        let view = by_category(&atlas, MappingCategory::View);
        assert_eq!(view.len(), atlas.view_mappings.len());

        let state = by_category(&atlas, MappingCategory::State);
        assert_eq!(state.len(), atlas.state_mappings.len());
    }

    #[test]
    fn atlas_stats_sum_to_total() {
        let atlas = build_atlas();
        let stats = atlas_stats(&atlas);

        assert_eq!(
            stats.exact + stats.approximate + stats.extend + stats.unsupported,
            stats.total,
            "Policy counts should sum to total"
        );
    }

    #[test]
    fn coverage_ratio_bounded() {
        let atlas = build_atlas();
        let stats = atlas_stats(&atlas);

        assert!(stats.coverage_ratio >= 0.0);
        assert!(stats.coverage_ratio <= 1.0);
    }

    #[test]
    fn dom_effect_is_unsupported() {
        let atlas = build_atlas();
        let entry = lookup(&atlas, "EffectKind::Dom").unwrap();
        assert_eq!(entry.policy, TransformationHandlingClass::Unsupported);
        assert_eq!(entry.risk, TransformationRiskLevel::High);
    }

    #[test]
    fn local_state_is_exact() {
        let atlas = build_atlas();
        let entry = lookup(&atlas, "StateScope::Local").unwrap();
        assert_eq!(entry.policy, TransformationHandlingClass::Exact);
        assert_eq!(entry.risk, TransformationRiskLevel::Low);
        assert!(entry.remediation.automatable);
    }

    #[test]
    fn component_maps_to_model() {
        let atlas = build_atlas();
        let entry = lookup(&atlas, "ViewNodeKind::Component").unwrap();
        assert!(entry.target.construct.contains("Model"));
        assert_eq!(entry.target.crate_name, "ftui-runtime");
    }

    #[test]
    fn flex_maps_to_layout() {
        let atlas = build_atlas();
        let entry = lookup(&atlas, "LayoutKind::Flex").unwrap();
        assert!(entry.target.construct.contains("Layout"));
        assert_eq!(entry.target.crate_name, "ftui-layout");
    }

    #[test]
    fn touch_input_unsupported() {
        let atlas = build_atlas();
        let entry = lookup(&atlas, "Capability::TouchInput").unwrap();
        assert_eq!(entry.policy, TransformationHandlingClass::Unsupported);
        assert!(!entry.remediation.automatable);
    }

    #[test]
    fn all_entries_have_valid_category() {
        let atlas = build_atlas();
        for entry in all_entries(&atlas) {
            // Just verify we can access category without panic.
            let _ = entry.category;
            assert!(!entry.source_signature.is_empty());
            assert!(!entry.target.construct.is_empty());
        }
    }

    #[test]
    fn atlas_has_reasonable_coverage() {
        let atlas = build_atlas();
        let stats = atlas_stats(&atlas);

        // Should have a meaningful number of entries.
        assert!(
            stats.total >= 30,
            "Atlas should have at least 30 entries, got {}",
            stats.total
        );

        // Most mappings should be automatable.
        assert!(
            stats.automatable as f64 / stats.total as f64 > 0.5,
            "More than half of mappings should be automatable"
        );
    }

    #[test]
    fn version_is_set() {
        let atlas = build_atlas();
        assert!(atlas.version.starts_with("mapping-atlas-v"));
    }

    #[test]
    fn version_is_v3() {
        assert_eq!(ATLAS_VERSION, "mapping-atlas-v3");
    }

    #[test]
    fn v1_and_v2_are_compatible() {
        assert!(is_compatible("mapping-atlas-v1"));
        assert!(is_compatible("mapping-atlas-v2"));
        assert!(is_compatible("mapping-atlas-v3"));
        assert!(!is_compatible("mapping-atlas-v99"));
    }

    #[test]
    fn compatibility_notes_cover_prior_versions() {
        let versions: Vec<&str> = ATLAS_COMPAT_NOTES
            .iter()
            .map(|(version, _)| *version)
            .collect();
        assert!(versions.contains(&"mapping-atlas-v2"));
        assert!(versions.contains(&"mapping-atlas-v1"));
    }

    #[test]
    fn parity_style_props_exist() {
        let atlas = build_atlas();
        let props = [
            "StyleProp::TextTransform",
            "StyleProp::TextOverflow",
            "StyleProp::Overflow",
            "StyleProp::WhiteSpace",
            "StyleProp::TextAlign",
            "StyleProp::LineClamp",
        ];
        for sig in props {
            assert!(
                lookup(&atlas, sig).is_some(),
                "Missing parity style mapping for {sig}"
            );
        }
    }

    #[test]
    fn parity_style_props_are_exact() {
        let atlas = build_atlas();
        for sig in [
            "StyleProp::TextTransform",
            "StyleProp::TextOverflow",
            "StyleProp::WhiteSpace",
            "StyleProp::TextAlign",
            "StyleProp::LineClamp",
        ] {
            let entry = lookup(&atlas, sig).unwrap();
            assert_eq!(
                entry.policy,
                TransformationHandlingClass::Exact,
                "{sig} should be Exact"
            );
            assert!(entry.remediation.automatable, "{sig} should be automatable");
        }
    }

    #[test]
    fn all_token_categories_covered() {
        let atlas = build_atlas();
        let sigs = [
            "StyleToken::Color",
            "StyleToken::Typography",
            "StyleToken::Spacing",
            "StyleToken::Border",
            "StyleToken::Shadow",
            "StyleToken::Animation",
            "StyleToken::Breakpoint",
            "StyleToken::ZIndex",
        ];
        for sig in sigs {
            assert!(
                lookup(&atlas, sig).is_some(),
                "Missing token category mapping for {sig}"
            );
        }
    }

    #[test]
    fn animation_requires_extension() {
        let atlas = build_atlas();
        let entry = lookup(&atlas, "StyleToken::Animation").unwrap();
        assert_eq!(entry.policy, TransformationHandlingClass::ExtendFtui);
        assert!(!entry.remediation.automatable);
    }

    #[test]
    fn reevaluate_resolves_new_mappings() {
        let atlas = build_atlas();
        let gaps = vec![
            "StyleProp::TextTransform".to_string(), // now Exact → resolved
            "EffectKind::Process".to_string(),      // now Approximate → resolved
            "StyleToken::Animation".to_string(),    // ExtendFtui → improved
            "NonExistent::Thing".to_string(),       // not in atlas → remaining
        ];
        let result = reevaluate_gaps(&atlas, &gaps);
        assert_eq!(
            result.resolved,
            vec!["StyleProp::TextTransform", "EffectKind::Process"]
        );
        assert_eq!(result.improved, vec!["StyleToken::Animation"]);
        assert_eq!(result.remaining, vec!["NonExistent::Thing"]);
    }

    #[test]
    fn reevaluate_empty_gaps_is_empty() {
        let atlas = build_atlas();
        let result = reevaluate_gaps(&atlas, &[]);
        assert!(result.resolved.is_empty());
        assert!(result.remaining.is_empty());
        assert!(result.improved.is_empty());
    }

    #[test]
    fn atlas_total_entries_is_stable_in_v3() {
        let atlas = build_atlas();
        let stats = atlas_stats(&atlas);
        // v1 had 46 entries; v2 added 10 (4 token cats + 6 style props), v3 keeps those.
        assert!(
            stats.total >= 56,
            "Expected at least 56 entries in v3, got {}",
            stats.total
        );
    }
}
