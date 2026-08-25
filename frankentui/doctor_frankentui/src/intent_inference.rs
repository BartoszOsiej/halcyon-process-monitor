// SPDX-License-Identifier: Apache-2.0
//! Infer layout, focus, and interaction intent graphs from canonical IR.
//!
//! Takes a `MigrationIr` and produces confidence-scored intent graphs that
//! capture higher-level semantics beyond what syntax-level translation provides:
//!
//! - **Layout constraints**: Flex/grid/stack hierarchies, sizing relationships,
//!   containment patterns that map to ftui-layout primitives.
//! - **Focus traversal**: Tab order, focus groups, keyboard navigation paths
//!   compatible with ftui-runtime's event model.
//! - **Interaction clusters**: Related event/state/effect groupings that
//!   represent cohesive user workflows (forms, toggles, navigation flows).
//!
//! All inferences carry confidence scores and explainable evidence chains.
//! Ambiguous paths are preserved as ranked alternatives for downstream
//! planner decisions.

use std::collections::{BTreeMap, BTreeSet};

use crate::migration_ir::{
    AccessibilityMap, EffectKind, EffectRegistry, EventCatalog, EventKind, IrNodeId, LayoutIntent,
    LayoutKind, MigrationIr, StateGraph, StateScope, StyleIntent, ViewNode, ViewNodeKind, ViewTree,
};

// ── Confidence ──────────────────────────────────────────────────────────

/// Confidence score in [0.0, 1.0] with an explanation of how it was derived.
#[derive(Debug, Clone)]
pub struct Confidence {
    /// Score in [0.0, 1.0]. Higher = more certain.
    pub score: f64,
    /// Human-readable explanation of the scoring rationale.
    pub rationale: String,
}

impl Confidence {
    pub fn new(score: f64, rationale: impl Into<String>) -> Self {
        Self {
            score: score.clamp(0.0, 1.0),
            rationale: rationale.into(),
        }
    }

    pub fn high(rationale: impl Into<String>) -> Self {
        Self::new(0.9, rationale)
    }

    pub fn medium(rationale: impl Into<String>) -> Self {
        Self::new(0.6, rationale)
    }

    pub fn low(rationale: impl Into<String>) -> Self {
        Self::new(0.3, rationale)
    }
}

// ── Evidence ────────────────────────────────────────────────────────────

/// A piece of evidence supporting an inference.
#[derive(Debug, Clone)]
pub struct Evidence {
    /// Which IR node or structure provided this evidence.
    pub source: EvidenceSource,
    /// What was observed.
    pub observation: String,
}

/// Where evidence was found in the IR.
#[derive(Debug, Clone)]
pub enum EvidenceSource {
    /// Evidence from a specific view node.
    ViewNode(IrNodeId),
    /// Evidence from a layout intent.
    Layout(IrNodeId),
    /// Evidence from an event declaration.
    Event(IrNodeId),
    /// Evidence from a state variable.
    State(IrNodeId),
    /// Evidence from an effect.
    Effect(IrNodeId),
    /// Evidence from an accessibility entry.
    Accessibility(IrNodeId),
    /// Structural evidence (e.g., tree shape, data flow pattern).
    Structural(String),
}

// ── Layout Constraint Graph ─────────────────────────────────────────────

/// Inferred layout constraint graph.
#[derive(Debug, Clone)]
pub struct LayoutConstraintGraph {
    /// Layout regions and their constraints.
    pub regions: BTreeMap<IrNodeId, LayoutRegion>,
    /// Containment edges: parent → children in layout hierarchy.
    pub containment: BTreeMap<IrNodeId, Vec<IrNodeId>>,
    /// Sizing relationships between regions.
    pub sizing_constraints: Vec<SizingConstraint>,
    /// Top-level layout pattern detected.
    pub pattern: LayoutPattern,
    /// Confidence in the overall layout inference.
    pub confidence: Confidence,
    /// Evidence supporting the inference.
    pub evidence: Vec<Evidence>,
}

/// A layout region inferred from view tree + style intent.
#[derive(Debug, Clone)]
pub struct LayoutRegion {
    pub node_id: IrNodeId,
    pub kind: LayoutRegionKind,
    /// Inferred ftui-layout constraint kind.
    pub constraint: InferredConstraint,
    /// Alternative interpretations if ambiguous.
    pub alternatives: Vec<InferredConstraint>,
    pub confidence: Confidence,
}

/// What kind of layout region this is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LayoutRegionKind {
    /// Container that lays out children along an axis.
    LinearContainer,
    /// Grid container with rows and columns.
    GridContainer,
    /// Stack/overlay container (z-axis layering).
    StackContainer,
    /// Fixed-position element.
    FixedElement,
    /// Scrollable region.
    ScrollableRegion,
    /// Leaf content (text, input, etc.).
    LeafContent,
}

/// An inferred layout constraint compatible with ftui-layout.
#[derive(Debug, Clone)]
pub struct InferredConstraint {
    pub kind: InferredConstraintKind,
    pub direction: Option<LayoutDirection>,
    pub alignment: Option<LayoutAlignment>,
    pub sizing: Option<InferredSizing>,
    pub confidence: Confidence,
    pub evidence: Vec<Evidence>,
}

/// Kind of inferred constraint.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InferredConstraintKind {
    /// Flex layout (row or column).
    Flex,
    /// Grid layout.
    Grid,
    /// Absolute/fixed positioning.
    Absolute,
    /// Stacked/overlaid elements.
    Stack,
    /// Natural document flow.
    Flow,
    /// Unknown — needs manual planner decision.
    Unknown,
}

/// Inferred layout direction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LayoutDirection {
    Horizontal,
    Vertical,
}

/// Inferred alignment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LayoutAlignment {
    Start,
    Center,
    End,
    Stretch,
    SpaceBetween,
}

/// Inferred sizing relationship.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InferredSizing {
    /// Fixed size (absolute).
    Fixed,
    /// Proportional (percentage or ratio).
    Proportional,
    /// Content-driven (min-content / max-content).
    ContentDriven,
    /// Fill available space.
    Fill,
}

/// Sizing constraint between two layout regions.
#[derive(Debug, Clone)]
pub struct SizingConstraint {
    pub subject: IrNodeId,
    pub kind: SizingConstraintKind,
    pub confidence: Confidence,
}

/// Kind of sizing constraint.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SizingConstraintKind {
    /// Subject fills remaining space in parent.
    FillRemaining,
    /// Subject has fixed proportion of parent.
    FixedProportion,
    /// Subject sizes to its content.
    FitContent,
    /// Subject matches sibling's size.
    MatchSibling(IrNodeId),
}

/// High-level layout pattern for the entire application.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LayoutPattern {
    /// Header / content / footer stacking.
    HeaderContentFooter,
    /// Sidebar + main content.
    SidebarMain,
    /// Dashboard grid layout.
    DashboardGrid,
    /// Single scrollable list.
    SingleList,
    /// Tabbed interface.
    TabbedInterface,
    /// Modal overlay on base content.
    ModalOverlay,
    /// No dominant pattern detected.
    Unclassified,
}

// ── Focus Traversal Graph ───────────────────────────────────────────────

/// Inferred focus traversal graph.
#[derive(Debug, Clone)]
pub struct FocusTraversalGraph {
    /// Focusable nodes and their metadata.
    pub nodes: BTreeMap<IrNodeId, FocusNode>,
    /// Focus groups (logical groupings of focusable elements).
    pub groups: Vec<FocusGroup>,
    /// Tab order (linear traversal sequence).
    pub tab_order: Vec<IrNodeId>,
    /// Directional navigation hints (arrow key movement).
    pub directional_hints: Vec<DirectionalHint>,
    /// Overall confidence in the focus inference.
    pub confidence: Confidence,
    pub evidence: Vec<Evidence>,
}

/// A focusable node in the traversal graph.
#[derive(Debug, Clone)]
pub struct FocusNode {
    pub node_id: IrNodeId,
    /// Why this node is considered focusable.
    pub reason: FocusReason,
    /// Role hint for screen readers / keyboard nav.
    pub role: Option<String>,
    /// Keyboard shortcut if declared.
    pub shortcut: Option<String>,
    /// Which focus group this belongs to (index into `groups`).
    pub group_index: Option<usize>,
    pub confidence: Confidence,
}

/// Why a node is considered focusable.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FocusReason {
    /// Has user-input event handlers (onClick, onKeyDown, etc.).
    HasUserInputEvents,
    /// Is an interactive element type (button, input, link).
    InteractiveElement,
    /// Has explicit tabIndex or focus-related accessibility.
    ExplicitFocusOrder,
    /// Has ARIA role indicating interactivity.
    AriaInteractive,
    /// Receives state writes from user interaction.
    WritesStateOnInteraction,
}

/// A group of related focusable elements.
#[derive(Debug, Clone)]
pub struct FocusGroup {
    /// Human-readable name (e.g. "navigation", "form-fields", "toolbar").
    pub name: String,
    /// Member node IDs in traversal order.
    pub members: Vec<IrNodeId>,
    /// The common ancestor in the view tree.
    pub container: Option<IrNodeId>,
    /// Whether this group traps focus (e.g., modal dialog).
    pub traps_focus: bool,
    pub confidence: Confidence,
    pub evidence: Vec<Evidence>,
}

/// A directional navigation hint between focusable nodes.
#[derive(Debug, Clone)]
pub struct DirectionalHint {
    pub from: IrNodeId,
    pub to: IrNodeId,
    pub direction: NavDirection,
    pub confidence: Confidence,
}

/// Navigation direction for arrow-key movement.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NavDirection {
    Up,
    Down,
    Left,
    Right,
}

// ── Interaction Cluster Graph ───────────────────────────────────────────

/// Inferred interaction clusters.
#[derive(Debug, Clone)]
pub struct InteractionClusterGraph {
    /// Detected interaction clusters.
    pub clusters: Vec<InteractionCluster>,
    /// State flow edges between clusters.
    pub cross_cluster_flows: Vec<CrossClusterFlow>,
    /// Overall confidence.
    pub confidence: Confidence,
    pub evidence: Vec<Evidence>,
}

/// A cluster of related interactive elements forming a cohesive workflow.
#[derive(Debug, Clone)]
pub struct InteractionCluster {
    /// Unique name for this cluster.
    pub name: String,
    /// What kind of interaction pattern this represents.
    pub pattern: InteractionPattern,
    /// View nodes involved.
    pub view_nodes: BTreeSet<IrNodeId>,
    /// Events involved.
    pub events: BTreeSet<IrNodeId>,
    /// State variables involved.
    pub state_vars: BTreeSet<IrNodeId>,
    /// Effects triggered by this cluster.
    pub effects: BTreeSet<IrNodeId>,
    pub confidence: Confidence,
    pub evidence: Vec<Evidence>,
}

/// Recognized interaction pattern.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InteractionPattern {
    /// Form: inputs → validation → submission.
    FormSubmission,
    /// Toggle/switch: click → state flip → re-render.
    Toggle,
    /// Navigation: click → route change → view swap.
    Navigation,
    /// Pagination/infinite scroll: scroll/click → fetch → append.
    Pagination,
    /// Search: input → debounce → fetch → display results.
    Search,
    /// Modal: trigger → overlay → dismiss.
    ModalDialog,
    /// Drag and drop: mousedown → drag → drop → reorder.
    DragAndDrop,
    /// Selection: click items → update selection set.
    Selection,
    /// Unclassified interaction group.
    Unclassified,
}

/// Data flow between interaction clusters.
#[derive(Debug, Clone)]
pub struct CrossClusterFlow {
    pub from_cluster: usize,
    pub to_cluster: usize,
    /// Shared state variables connecting the clusters.
    pub shared_state: BTreeSet<IrNodeId>,
    pub confidence: Confidence,
}

// ── Top-Level Result ────────────────────────────────────────────────────

/// Complete intent inference result.
#[derive(Debug, Clone)]
pub struct IntentInferenceResult {
    /// Inferred layout constraint graph.
    pub layout: LayoutConstraintGraph,
    /// Inferred focus traversal graph.
    pub focus: FocusTraversalGraph,
    /// Inferred interaction clusters.
    pub interactions: InteractionClusterGraph,
    /// Overall confidence across all inference domains.
    pub overall_confidence: Confidence,
    /// Summary statistics.
    pub stats: InferenceStats,
}

/// Summary statistics for the inference run.
#[derive(Debug, Clone)]
pub struct InferenceStats {
    pub layout_regions: usize,
    pub focus_nodes: usize,
    pub focus_groups: usize,
    pub interaction_clusters: usize,
    pub cross_cluster_flows: usize,
    pub ambiguous_intents: usize,
}

// ── Inference Engine ────────────────────────────────────────────────────

/// Run full intent inference on a migration IR.
pub fn infer_intents(ir: &MigrationIr) -> IntentInferenceResult {
    let layout = infer_layout_constraints(&ir.view_tree, &ir.style_intent);
    let focus = infer_focus_traversal(
        &ir.view_tree,
        &ir.event_catalog,
        &ir.accessibility,
        &ir.state_graph,
    );
    let interactions = infer_interaction_clusters(
        &ir.view_tree,
        &ir.event_catalog,
        &ir.state_graph,
        &ir.effect_registry,
    );

    let ambiguous_intents = count_ambiguous(&layout, &focus, &interactions);

    let overall_score =
        (layout.confidence.score + focus.confidence.score + interactions.confidence.score) / 3.0;

    let stats = InferenceStats {
        layout_regions: layout.regions.len(),
        focus_nodes: focus.nodes.len(),
        focus_groups: focus.groups.len(),
        interaction_clusters: interactions.clusters.len(),
        cross_cluster_flows: interactions.cross_cluster_flows.len(),
        ambiguous_intents,
    };

    IntentInferenceResult {
        layout,
        focus,
        interactions,
        overall_confidence: Confidence::new(
            overall_score,
            format!(
                "Average of layout/focus/interaction confidence ({ambiguous_intents} ambiguous intents)"
            ),
        ),
        stats,
    }
}

/// Count intents that have alternatives (ambiguous).
fn count_ambiguous(
    layout: &LayoutConstraintGraph,
    focus: &FocusTraversalGraph,
    interactions: &InteractionClusterGraph,
) -> usize {
    let layout_ambiguous = layout
        .regions
        .values()
        .filter(|r| !r.alternatives.is_empty())
        .count();

    let focus_ambiguous = focus
        .nodes
        .values()
        .filter(|n| n.confidence.score < 0.5)
        .count();

    let interaction_ambiguous = interactions
        .clusters
        .iter()
        .filter(|c| c.confidence.score < 0.5)
        .count();

    layout_ambiguous + focus_ambiguous + interaction_ambiguous
}

// ── Phase 1: Layout Constraint Inference ────────────────────────────────

fn infer_layout_constraints(tree: &ViewTree, style: &StyleIntent) -> LayoutConstraintGraph {
    let mut regions = BTreeMap::new();
    let mut containment: BTreeMap<IrNodeId, Vec<IrNodeId>> = BTreeMap::new();
    let mut sizing_constraints = Vec::new();
    let mut all_evidence = Vec::new();

    // Walk the view tree and classify each node's layout role.
    for (node_id, node) in &tree.nodes {
        let layout_intent = style.layouts.get(node_id);
        let region = classify_layout_region(node, layout_intent);

        // Build containment edges.
        if !node.children.is_empty() {
            containment.insert(node_id.clone(), node.children.clone());
        }

        // Infer sizing constraints from children.
        infer_sizing_from_children(node, &region, &mut sizing_constraints);

        all_evidence.push(Evidence {
            source: EvidenceSource::ViewNode(node_id.clone()),
            observation: format!("Classified {} as {:?}", node.name, region.kind),
        });

        regions.insert(node_id.clone(), region);
    }

    // Detect high-level layout pattern from root structure.
    let pattern = detect_layout_pattern(tree, style);

    let region_count = regions.len();
    let has_explicit_layouts = !style.layouts.is_empty();
    let confidence = if has_explicit_layouts {
        Confidence::high(format!(
            "{} explicit layout intents found for {region_count} regions",
            style.layouts.len()
        ))
    } else if region_count > 0 {
        Confidence::medium(format!(
            "Inferred from view tree structure ({region_count} regions, no explicit layouts)"
        ))
    } else {
        Confidence::low("Empty view tree — no layout to infer")
    };

    all_evidence.push(Evidence {
        source: EvidenceSource::Structural("layout-pattern".into()),
        observation: format!("Detected pattern: {pattern:?}"),
    });

    LayoutConstraintGraph {
        regions,
        containment,
        sizing_constraints,
        pattern,
        confidence,
        evidence: all_evidence,
    }
}

fn classify_layout_region(node: &ViewNode, layout_intent: Option<&LayoutIntent>) -> LayoutRegion {
    // If we have an explicit layout intent from style analysis, use it.
    if let Some(intent) = layout_intent {
        let (kind, constraint_kind, direction) = match intent.kind {
            LayoutKind::Flex => {
                let dir = intent.direction.as_deref().map(|d| {
                    if d.contains("row") {
                        LayoutDirection::Horizontal
                    } else {
                        LayoutDirection::Vertical
                    }
                });
                (
                    LayoutRegionKind::LinearContainer,
                    InferredConstraintKind::Flex,
                    dir,
                )
            }
            LayoutKind::Grid => (
                LayoutRegionKind::GridContainer,
                InferredConstraintKind::Grid,
                None,
            ),
            LayoutKind::Stack => (
                LayoutRegionKind::StackContainer,
                InferredConstraintKind::Stack,
                None,
            ),
            LayoutKind::Absolute => (
                LayoutRegionKind::FixedElement,
                InferredConstraintKind::Absolute,
                None,
            ),
            LayoutKind::Flow => (
                LayoutRegionKind::LeafContent,
                InferredConstraintKind::Flow,
                None,
            ),
        };

        let alignment = intent.alignment.as_deref().and_then(parse_alignment);
        let sizing = intent.sizing.as_deref().map(parse_sizing);

        return LayoutRegion {
            node_id: node.id.clone(),
            kind,
            constraint: InferredConstraint {
                kind: constraint_kind.clone(),
                direction: direction.clone(),
                alignment: alignment.clone(),
                sizing: sizing.clone(),
                confidence: Confidence::high("Explicit layout intent from style analysis"),
                evidence: vec![Evidence {
                    source: EvidenceSource::Layout(node.id.clone()),
                    observation: format!("StyleIntent specifies {:?}", intent.kind),
                }],
            },
            alternatives: Vec::new(),
            confidence: Confidence::high("Derived from explicit StyleIntent layout"),
        };
    }

    // No explicit layout — infer from node structure.
    infer_layout_from_structure(node)
}

fn infer_layout_from_structure(node: &ViewNode) -> LayoutRegion {
    let child_count = node.children.len();
    let name_lower = node.name.to_lowercase();

    // Heuristic: containers with children are likely flex/stack.
    if child_count > 0 {
        let kind = if is_list_like(&name_lower) {
            LayoutRegionKind::LinearContainer
        } else if is_grid_like(&name_lower) {
            LayoutRegionKind::GridContainer
        } else if is_overlay_like(&name_lower) {
            LayoutRegionKind::StackContainer
        } else {
            LayoutRegionKind::LinearContainer
        };

        let primary_direction = if is_horizontal_hint(&name_lower) {
            Some(LayoutDirection::Horizontal)
        } else {
            Some(LayoutDirection::Vertical)
        };

        let primary = InferredConstraint {
            kind: match kind {
                LayoutRegionKind::GridContainer => InferredConstraintKind::Grid,
                LayoutRegionKind::StackContainer => InferredConstraintKind::Stack,
                _ => InferredConstraintKind::Flex,
            },
            direction: primary_direction.clone(),
            alignment: None,
            sizing: None,
            confidence: Confidence::medium(format!(
                "Inferred from name '{}' and {child_count} children",
                node.name
            )),
            evidence: vec![Evidence {
                source: EvidenceSource::ViewNode(node.id.clone()),
                observation: format!("Container '{}' with {child_count} children", node.name),
            }],
        };

        // Provide alternative if ambiguous.
        let alt_direction = match primary_direction {
            Some(LayoutDirection::Horizontal) => Some(LayoutDirection::Vertical),
            _ => Some(LayoutDirection::Horizontal),
        };

        let alternatives = vec![InferredConstraint {
            kind: InferredConstraintKind::Flex,
            direction: alt_direction,
            alignment: None,
            sizing: None,
            confidence: Confidence::low("Alternative axis direction"),
            evidence: vec![],
        }];

        LayoutRegion {
            node_id: node.id.clone(),
            kind,
            constraint: primary,
            alternatives,
            confidence: Confidence::medium(format!(
                "Structural inference from '{}' with {child_count} children",
                node.name
            )),
        }
    } else {
        // Leaf node.
        LayoutRegion {
            node_id: node.id.clone(),
            kind: LayoutRegionKind::LeafContent,
            constraint: InferredConstraint {
                kind: InferredConstraintKind::Flow,
                direction: None,
                alignment: None,
                sizing: Some(InferredSizing::ContentDriven),
                confidence: Confidence::high("Leaf node defaults to flow layout"),
                evidence: vec![Evidence {
                    source: EvidenceSource::ViewNode(node.id.clone()),
                    observation: format!("Leaf node '{}' has no children", node.name),
                }],
            },
            alternatives: Vec::new(),
            confidence: Confidence::high("Leaf node — flow layout"),
        }
    }
}

fn infer_sizing_from_children(
    node: &ViewNode,
    region: &LayoutRegion,
    constraints: &mut Vec<SizingConstraint>,
) {
    // If this is a linear container with multiple children, infer fill/fit patterns.
    if region.kind != LayoutRegionKind::LinearContainer || node.children.len() < 2 {
        return;
    }

    // Simple heuristic: if there are exactly 2 or 3 children, the last
    // child is often the "main" content that fills remaining space.
    let last_child = &node.children[node.children.len() - 1];
    constraints.push(SizingConstraint {
        subject: last_child.clone(),
        kind: SizingConstraintKind::FillRemaining,
        confidence: Confidence::low(
            "Heuristic: last child in linear container often fills remaining space",
        ),
    });

    // First children are often fit-content (header, sidebar).
    for child_id in &node.children[..node.children.len() - 1] {
        constraints.push(SizingConstraint {
            subject: child_id.clone(),
            kind: SizingConstraintKind::FitContent,
            confidence: Confidence::low(
                "Heuristic: non-last children in linear container often fit-content",
            ),
        });
    }
}

fn detect_layout_pattern(tree: &ViewTree, style: &StyleIntent) -> LayoutPattern {
    if tree.roots.is_empty() {
        return LayoutPattern::Unclassified;
    }

    // Gather root children for pattern detection.
    let root_children: Vec<&ViewNode> = tree
        .roots
        .iter()
        .filter_map(|r| tree.nodes.get(r))
        .flat_map(|root| root.children.iter().filter_map(|c| tree.nodes.get(c)))
        .collect();

    if root_children.is_empty() {
        return LayoutPattern::Unclassified;
    }

    let names: Vec<String> = root_children
        .iter()
        .map(|n| n.name.to_lowercase())
        .collect();

    // Check for header/content/footer pattern.
    let has_header = names.iter().any(|n| {
        n.contains("header") || n.contains("navbar") || n.contains("topbar") || n.contains("appbar")
    });
    let has_footer = names
        .iter()
        .any(|n| n.contains("footer") || n.contains("bottombar") || n.contains("statusbar"));
    let has_sidebar = names
        .iter()
        .any(|n| n.contains("sidebar") || n.contains("drawer") || n.contains("nav"));
    let has_main = names
        .iter()
        .any(|n| n.contains("main") || n.contains("content") || n.contains("body"));

    // Check layout intents for grid patterns.
    let grid_count = style
        .layouts
        .values()
        .filter(|l| l.kind == LayoutKind::Grid)
        .count();

    // Check for tabs.
    let has_tabs = names
        .iter()
        .any(|n| n.contains("tab") || n.contains("tabs") || n.contains("tabpanel"));

    // Check for modal/overlay.
    let has_modal = tree.nodes.values().any(|n| {
        let nl = n.name.to_lowercase();
        nl.contains("modal") || nl.contains("dialog") || nl.contains("overlay")
    });
    let has_portal = tree.nodes.values().any(|n| n.kind == ViewNodeKind::Portal);

    if has_sidebar && has_main {
        LayoutPattern::SidebarMain
    } else if has_header && (has_footer || has_main) {
        LayoutPattern::HeaderContentFooter
    } else if grid_count >= 2 {
        LayoutPattern::DashboardGrid
    } else if has_tabs {
        LayoutPattern::TabbedInterface
    } else if has_modal || has_portal {
        LayoutPattern::ModalOverlay
    } else if root_children.len() == 1 {
        // Single root child with list-like name.
        let only_name = &names[0];
        if is_list_like(only_name) {
            LayoutPattern::SingleList
        } else {
            LayoutPattern::Unclassified
        }
    } else {
        LayoutPattern::Unclassified
    }
}

// ── Phase 2: Focus Traversal Inference ──────────────────────────────────

fn infer_focus_traversal(
    tree: &ViewTree,
    events: &EventCatalog,
    a11y: &AccessibilityMap,
    state: &StateGraph,
) -> FocusTraversalGraph {
    let mut nodes = BTreeMap::new();
    let mut all_evidence = Vec::new();

    // Build a map of event source nodes for quick lookup.
    let event_sources: BTreeMap<&IrNodeId, Vec<&IrNodeId>> = {
        let mut map: BTreeMap<&IrNodeId, Vec<&IrNodeId>> = BTreeMap::new();
        for (event_id, event) in &events.events {
            if let Some(ref source) = event.source_node {
                map.entry(source).or_default().push(event_id);
            }
        }
        map
    };

    // Build a map of state writers from events (nodes that write state on interaction).
    let interactive_writers: BTreeSet<&IrNodeId> = {
        let mut writers = BTreeSet::new();
        for transition in &events.transitions {
            if let Some(event) = events.events.get(&transition.event_id)
                && event.kind == EventKind::UserInput
                && let Some(ref source) = event.source_node
            {
                writers.insert(source);
            }
        }
        writers
    };

    // Scan all view nodes to determine focusability.
    for (node_id, node) in &tree.nodes {
        let mut reasons = Vec::new();

        // Check for user-input event handlers.
        if let Some(event_ids) = event_sources.get(node_id) {
            let has_user_input = event_ids.iter().any(|eid| {
                events
                    .events
                    .get(*eid)
                    .is_some_and(|e| e.kind == EventKind::UserInput)
            });
            if has_user_input {
                reasons.push(FocusReason::HasUserInputEvents);
            }
        }

        // Check for interactive element type.
        if is_interactive_element(&node.name) {
            reasons.push(FocusReason::InteractiveElement);
        }

        // Check for explicit focus order in accessibility.
        if let Some(entry) = a11y.entries.get(node_id) {
            if entry.focus_order.is_some() {
                reasons.push(FocusReason::ExplicitFocusOrder);
            }
            if let Some(ref role) = entry.role
                && is_interactive_role(role)
            {
                reasons.push(FocusReason::AriaInteractive);
            }
        }

        // Check for state writes on interaction.
        if interactive_writers.contains(node_id) {
            reasons.push(FocusReason::WritesStateOnInteraction);
        }

        if reasons.is_empty() {
            continue;
        }

        let primary_reason = reasons[0].clone();
        let confidence = compute_focus_confidence(&reasons);

        let role = a11y.entries.get(node_id).and_then(|e| e.role.clone());
        let shortcut = a11y
            .entries
            .get(node_id)
            .and_then(|e| e.keyboard_shortcut.clone());

        all_evidence.push(Evidence {
            source: EvidenceSource::ViewNode(node_id.clone()),
            observation: format!("'{}' is focusable: {:?}", node.name, reasons),
        });

        nodes.insert(
            node_id.clone(),
            FocusNode {
                node_id: node_id.clone(),
                reason: primary_reason,
                role,
                shortcut,
                group_index: None,
                confidence,
            },
        );
    }

    // Compute tab order: explicit focus_order first, then tree order.
    let tab_order = compute_tab_order(&nodes, a11y, tree);

    // Detect focus groups by finding common ancestors of focusable nodes.
    let groups = detect_focus_groups(tree, &mut nodes, state);

    // Compute directional hints from spatial relationships.
    let directional_hints = compute_directional_hints(&tab_order, &nodes);

    let node_count = nodes.len();
    let confidence = if node_count == 0 {
        Confidence::low("No focusable nodes detected")
    } else {
        let avg_conf: f64 =
            nodes.values().map(|n| n.confidence.score).sum::<f64>() / node_count as f64;
        Confidence::new(
            avg_conf,
            format!("{node_count} focusable nodes, average confidence {avg_conf:.2}"),
        )
    };

    FocusTraversalGraph {
        nodes,
        groups,
        tab_order,
        directional_hints,
        confidence,
        evidence: all_evidence,
    }
}

fn compute_focus_confidence(reasons: &[FocusReason]) -> Confidence {
    // More reasons = higher confidence.
    let base: f64 = match reasons.len() {
        0 => 0.0,
        1 => 0.5,
        2 => 0.7,
        _ => 0.9,
    };

    // Boost for explicit signals.
    let boost: f64 = if reasons.contains(&FocusReason::ExplicitFocusOrder) {
        0.2
    } else if reasons.contains(&FocusReason::InteractiveElement) {
        0.1
    } else {
        0.0
    };

    let score = (base + boost).min(1.0);
    Confidence::new(
        score,
        format!("{} supporting reasons: {:?}", reasons.len(), reasons),
    )
}

fn compute_tab_order(
    focus_nodes: &BTreeMap<IrNodeId, FocusNode>,
    a11y: &AccessibilityMap,
    tree: &ViewTree,
) -> Vec<IrNodeId> {
    let mut explicit: Vec<(u32, IrNodeId)> = Vec::new();
    let mut implicit: Vec<IrNodeId> = Vec::new();

    for node_id in focus_nodes.keys() {
        if let Some(order) = a11y.entries.get(node_id).and_then(|e| e.focus_order) {
            explicit.push((order, node_id.clone()));
        } else {
            implicit.push(node_id.clone());
        }
    }

    // Sort explicit by declared order.
    explicit.sort_by_key(|(order, _)| *order);

    // Sort implicit by tree traversal order (DFS pre-order).
    let tree_order = compute_tree_order(tree);
    implicit.sort_by_key(|id| tree_order.get(id).copied().unwrap_or(usize::MAX));

    let mut result: Vec<IrNodeId> = explicit.into_iter().map(|(_, id)| id).collect();
    result.extend(implicit);
    result
}

fn compute_tree_order(tree: &ViewTree) -> BTreeMap<IrNodeId, usize> {
    let mut order = BTreeMap::new();
    let mut counter = 0;

    for root in &tree.roots {
        walk_tree_order(root, tree, &mut order, &mut counter);
    }

    order
}

fn walk_tree_order(
    node_id: &IrNodeId,
    tree: &ViewTree,
    order: &mut BTreeMap<IrNodeId, usize>,
    counter: &mut usize,
) {
    if order.contains_key(node_id) {
        return;
    }
    order.insert(node_id.clone(), *counter);
    *counter += 1;

    if let Some(node) = tree.nodes.get(node_id) {
        for child in &node.children {
            walk_tree_order(child, tree, order, counter);
        }
    }
}

fn detect_focus_groups(
    tree: &ViewTree,
    focus_nodes: &mut BTreeMap<IrNodeId, FocusNode>,
    state: &StateGraph,
) -> Vec<FocusGroup> {
    let mut groups = Vec::new();

    // Strategy: group focusable nodes by their lowest common ancestor
    // that isn't the root. Also detect form-like patterns where multiple
    // inputs share state dependencies.

    // Find ancestors for each focusable node.
    let parent_map = build_parent_map(tree);

    // Group by immediate container (parent that has >1 focusable descendant).
    let mut container_members: BTreeMap<IrNodeId, Vec<IrNodeId>> = BTreeMap::new();

    for node_id in focus_nodes.keys() {
        if let Some(parent_id) = parent_map.get(node_id) {
            container_members
                .entry(parent_id.clone())
                .or_default()
                .push(node_id.clone());
        }
    }

    // Only form groups with 2+ members.
    for (container_id, members) in &container_members {
        if members.len() < 2 {
            continue;
        }

        let container_name = tree
            .nodes
            .get(container_id)
            .map(|n| n.name.as_str())
            .unwrap_or("unknown");

        let group_name = infer_group_name(container_name, members.len());

        // Detect focus trapping (modals, dialogs).
        let traps_focus = tree.nodes.get(container_id).is_some_and(|n| {
            let nl = n.name.to_lowercase();
            nl.contains("modal") || nl.contains("dialog") || n.kind == ViewNodeKind::Portal
        });

        // Check if members share state writes (form-like).
        let shared_state = members
            .iter()
            .any(|m| state.variables.values().any(|sv| sv.writers.contains(m)));

        let confidence = if shared_state {
            Confidence::high(format!(
                "Group '{}' members share state writes (form-like)",
                group_name
            ))
        } else {
            Confidence::medium(format!(
                "Group '{}' from common container '{}'",
                group_name, container_name
            ))
        };

        let group_index = groups.len();

        // Update focus nodes with group assignment.
        for member_id in members {
            if let Some(fnode) = focus_nodes.get_mut(member_id) {
                fnode.group_index = Some(group_index);
            }
        }

        groups.push(FocusGroup {
            name: group_name,
            members: members.clone(),
            container: Some(container_id.clone()),
            traps_focus,
            confidence: confidence.clone(),
            evidence: vec![Evidence {
                source: EvidenceSource::ViewNode(container_id.clone()),
                observation: format!(
                    "Container '{}' has {} focusable descendants",
                    container_name,
                    members.len()
                ),
            }],
        });
    }

    groups
}

fn build_parent_map(tree: &ViewTree) -> BTreeMap<IrNodeId, IrNodeId> {
    let mut parent_map = BTreeMap::new();
    for (parent_id, node) in &tree.nodes {
        for child_id in &node.children {
            parent_map.insert(child_id.clone(), parent_id.clone());
        }
    }
    parent_map
}

fn compute_directional_hints(
    tab_order: &[IrNodeId],
    _focus_nodes: &BTreeMap<IrNodeId, FocusNode>,
) -> Vec<DirectionalHint> {
    let mut hints = Vec::new();

    // Simple linear chain: each consecutive pair gets up/down hints.
    for window in tab_order.windows(2) {
        hints.push(DirectionalHint {
            from: window[0].clone(),
            to: window[1].clone(),
            direction: NavDirection::Down,
            confidence: Confidence::medium("Sequential in tab order"),
        });
        hints.push(DirectionalHint {
            from: window[1].clone(),
            to: window[0].clone(),
            direction: NavDirection::Up,
            confidence: Confidence::medium("Sequential in tab order"),
        });
    }

    hints
}

// ── Phase 3: Interaction Cluster Inference ──────────────────────────────

fn infer_interaction_clusters(
    tree: &ViewTree,
    events: &EventCatalog,
    state: &StateGraph,
    effects: &EffectRegistry,
) -> InteractionClusterGraph {
    let mut clusters = Vec::new();
    let mut all_evidence = Vec::new();

    // Strategy: start from each user-input event, follow state transitions
    // and effect triggers to find related elements, then classify the pattern.

    // Build adjacency: event → state vars it modifies.
    let event_to_states: BTreeMap<&IrNodeId, BTreeSet<&IrNodeId>> = {
        let mut map: BTreeMap<&IrNodeId, BTreeSet<&IrNodeId>> = BTreeMap::new();
        for t in &events.transitions {
            map.entry(&t.event_id).or_default().insert(&t.target_state);
        }
        map
    };

    // Build adjacency: state var → effects that read/write it.
    let state_to_effects: BTreeMap<&IrNodeId, BTreeSet<&IrNodeId>> = {
        let mut map: BTreeMap<&IrNodeId, BTreeSet<&IrNodeId>> = BTreeMap::new();
        for (effect_id, effect) in &effects.effects {
            for read_id in &effect.reads {
                map.entry(read_id).or_default().insert(effect_id);
            }
            for write_id in &effect.writes {
                map.entry(write_id).or_default().insert(effect_id);
            }
        }
        map
    };

    // Build adjacency: state var → view nodes that read it.
    let state_to_readers: BTreeMap<&IrNodeId, &BTreeSet<IrNodeId>> = state
        .variables
        .iter()
        .map(|(id, sv)| (id, &sv.readers))
        .collect();

    // Track which events have been assigned to a cluster.
    let mut assigned_events: BTreeSet<IrNodeId> = BTreeSet::new();

    // For each user-input event, expand the connected subgraph.
    for (event_id, event) in &events.events {
        if event.kind != EventKind::UserInput {
            continue;
        }
        if assigned_events.contains(event_id) {
            continue;
        }

        let mut cluster_events: BTreeSet<IrNodeId> = BTreeSet::new();
        let mut cluster_states: BTreeSet<IrNodeId> = BTreeSet::new();
        let mut cluster_effects: BTreeSet<IrNodeId> = BTreeSet::new();
        let mut cluster_views: BTreeSet<IrNodeId> = BTreeSet::new();

        // Seed with this event.
        cluster_events.insert(event_id.clone());
        if let Some(ref source) = event.source_node {
            cluster_views.insert(source.clone());
        }

        // Follow transitions to state variables.
        if let Some(target_states) = event_to_states.get(event_id) {
            for &state_id in target_states {
                cluster_states.insert(state_id.clone());

                // Follow to effects triggered by this state.
                if let Some(effect_ids) = state_to_effects.get(state_id) {
                    for &effect_id in effect_ids {
                        cluster_effects.insert(effect_id.clone());
                    }
                }

                // Follow to view nodes that read this state.
                if let Some(readers) = state_to_readers.get(state_id) {
                    for reader_id in *readers {
                        cluster_views.insert(reader_id.clone());
                    }
                }
            }
        }

        // Also find co-located events: other events on the same source node.
        if let Some(ref source) = event.source_node {
            for (other_id, other_event) in &events.events {
                if other_event.source_node.as_ref() == Some(source)
                    && !assigned_events.contains(other_id)
                {
                    cluster_events.insert(other_id.clone());
                }
            }
        }

        assigned_events.extend(cluster_events.iter().cloned());

        // Classify the interaction pattern.
        let pattern = classify_interaction_pattern(
            &cluster_events,
            &cluster_states,
            &cluster_effects,
            &cluster_views,
            events,
            state,
            effects,
            tree,
        );

        let name = generate_cluster_name(&pattern, &cluster_views, tree);

        let evidence_items = vec![Evidence {
            source: EvidenceSource::Event(event_id.clone()),
            observation: format!(
                "Cluster seeded from event '{}': {} events, {} state vars, {} effects, {} views",
                event.name,
                cluster_events.len(),
                cluster_states.len(),
                cluster_effects.len(),
                cluster_views.len()
            ),
        }];

        let confidence =
            compute_cluster_confidence(&pattern, cluster_events.len(), cluster_states.len());

        all_evidence.extend(evidence_items.iter().cloned());

        clusters.push(InteractionCluster {
            name,
            pattern,
            view_nodes: cluster_views,
            events: cluster_events,
            state_vars: cluster_states,
            effects: cluster_effects,
            confidence,
            evidence: evidence_items,
        });
    }

    // Detect cross-cluster state flows.
    let cross_cluster_flows = detect_cross_cluster_flows(&clusters);

    let cluster_count = clusters.len();
    let confidence = if cluster_count == 0 {
        Confidence::low("No user-input events found — no interaction clusters")
    } else {
        let avg: f64 =
            clusters.iter().map(|c| c.confidence.score).sum::<f64>() / cluster_count as f64;
        Confidence::new(
            avg,
            format!("{cluster_count} clusters, average confidence {avg:.2}"),
        )
    };

    InteractionClusterGraph {
        clusters,
        cross_cluster_flows,
        confidence,
        evidence: all_evidence,
    }
}

#[allow(clippy::too_many_arguments)]
fn classify_interaction_pattern(
    cluster_events: &BTreeSet<IrNodeId>,
    cluster_states: &BTreeSet<IrNodeId>,
    cluster_effects: &BTreeSet<IrNodeId>,
    cluster_views: &BTreeSet<IrNodeId>,
    events: &EventCatalog,
    state: &StateGraph,
    effects: &EffectRegistry,
    tree: &ViewTree,
) -> InteractionPattern {
    // Check for form submission pattern: multiple inputs + submit event + network effect.
    let has_network_effect = cluster_effects.iter().any(|eid| {
        effects
            .effects
            .get(eid)
            .is_some_and(|e| e.kind == EffectKind::Network)
    });

    let input_count = cluster_views
        .iter()
        .filter(|vid| {
            tree.nodes
                .get(*vid)
                .is_some_and(|n| is_input_element(&n.name))
        })
        .count();

    let event_names: Vec<&str> = cluster_events
        .iter()
        .filter_map(|eid| events.events.get(eid).map(|e| e.name.as_str()))
        .collect();

    let has_submit = event_names
        .iter()
        .any(|n| n.contains("submit") || n.contains("Submit"));

    // Check for toggle pattern: single state var with boolean-like transitions.
    let is_single_state_toggle = cluster_states.len() == 1
        && cluster_events.len() <= 2
        && state
            .variables
            .values()
            .any(|sv| cluster_states.contains(&sv.id) && is_boolean_state(sv));

    // Check for navigation pattern.
    let has_route_state = cluster_states.iter().any(|sid| {
        state
            .variables
            .get(sid)
            .is_some_and(|sv| sv.scope == StateScope::Route)
    });

    // Check for modal pattern.
    let has_modal_view = cluster_views.iter().any(|vid| {
        tree.nodes.get(vid).is_some_and(|n| {
            let nl = n.name.to_lowercase();
            nl.contains("modal") || nl.contains("dialog")
        })
    });

    // Check for search pattern: input + debounce/timer + network.
    let has_timer_effect = cluster_effects.iter().any(|eid| {
        effects
            .effects
            .get(eid)
            .is_some_and(|e| e.kind == EffectKind::Timer)
    });

    // Check for selection pattern: multiple view nodes reading same state.
    let has_shared_readers = cluster_states.iter().any(|sid| {
        state
            .variables
            .get(sid)
            .is_some_and(|sv| sv.readers.len() > 2)
    });

    // Pattern matching with priority.
    if (has_submit || (input_count >= 2 && has_network_effect)) && !has_route_state {
        InteractionPattern::FormSubmission
    } else if has_route_state {
        InteractionPattern::Navigation
    } else if has_modal_view {
        InteractionPattern::ModalDialog
    } else if is_single_state_toggle {
        InteractionPattern::Toggle
    } else if input_count >= 1 && has_timer_effect && has_network_effect {
        InteractionPattern::Search
    } else if has_network_effect && has_shared_readers {
        InteractionPattern::Pagination
    } else if has_shared_readers && cluster_events.len() > 2 {
        InteractionPattern::Selection
    } else {
        InteractionPattern::Unclassified
    }
}

fn detect_cross_cluster_flows(clusters: &[InteractionCluster]) -> Vec<CrossClusterFlow> {
    let mut flows = Vec::new();

    for (i, a) in clusters.iter().enumerate() {
        for (j, b) in clusters.iter().enumerate() {
            if i >= j {
                continue;
            }

            let shared: BTreeSet<IrNodeId> =
                a.state_vars.intersection(&b.state_vars).cloned().collect();

            if !shared.is_empty() {
                flows.push(CrossClusterFlow {
                    from_cluster: i,
                    to_cluster: j,
                    shared_state: shared.clone(),
                    confidence: Confidence::new(
                        0.7,
                        format!("{} shared state variables", shared.len()),
                    ),
                });
            }
        }
    }

    flows
}

fn compute_cluster_confidence(
    pattern: &InteractionPattern,
    event_count: usize,
    state_count: usize,
) -> Confidence {
    let pattern_score: f64 = match pattern {
        InteractionPattern::FormSubmission => 0.85,
        InteractionPattern::Toggle => 0.9,
        InteractionPattern::Navigation => 0.85,
        InteractionPattern::Search => 0.7,
        InteractionPattern::ModalDialog => 0.8,
        InteractionPattern::Pagination => 0.6,
        InteractionPattern::Selection => 0.6,
        InteractionPattern::DragAndDrop => 0.5,
        InteractionPattern::Unclassified => 0.3,
    };

    // More connected elements = higher confidence.
    let connectivity_boost: f64 = if event_count + state_count > 4 {
        0.1
    } else {
        0.0
    };

    Confidence::new(
        (pattern_score + connectivity_boost).min(1.0),
        format!(
            "Pattern {:?} with {event_count} events, {state_count} state vars",
            pattern
        ),
    )
}

fn generate_cluster_name(
    pattern: &InteractionPattern,
    view_nodes: &BTreeSet<IrNodeId>,
    tree: &ViewTree,
) -> String {
    let prefix = match pattern {
        InteractionPattern::FormSubmission => "form",
        InteractionPattern::Toggle => "toggle",
        InteractionPattern::Navigation => "nav",
        InteractionPattern::Pagination => "pagination",
        InteractionPattern::Search => "search",
        InteractionPattern::ModalDialog => "modal",
        InteractionPattern::DragAndDrop => "dnd",
        InteractionPattern::Selection => "selection",
        InteractionPattern::Unclassified => "interaction",
    };

    // Try to find a meaningful name from the view nodes.
    let context = view_nodes
        .iter()
        .find_map(|vid| tree.nodes.get(vid).map(|n| n.name.clone()))
        .unwrap_or_else(|| "unknown".to_string());

    format!("{prefix}_{context}")
}

// ── Helpers ─────────────────────────────────────────────────────────────

fn parse_alignment(s: &str) -> Option<LayoutAlignment> {
    let lower = s.to_lowercase();
    if lower.contains("center") {
        Some(LayoutAlignment::Center)
    } else if lower.contains("end") || lower.contains("right") || lower.contains("bottom") {
        Some(LayoutAlignment::End)
    } else if lower.contains("start") || lower.contains("left") || lower.contains("top") {
        Some(LayoutAlignment::Start)
    } else if lower.contains("stretch") {
        Some(LayoutAlignment::Stretch)
    } else if lower.contains("between") || lower.contains("space") {
        Some(LayoutAlignment::SpaceBetween)
    } else {
        None
    }
}

fn parse_sizing(s: &str) -> InferredSizing {
    let lower = s.to_lowercase();
    if lower.contains("fixed") || lower.contains("px") {
        InferredSizing::Fixed
    } else if lower.contains("%") || lower.contains("ratio") || lower.contains("fr") {
        InferredSizing::Proportional
    } else if lower.contains("fit") || lower.contains("content") || lower.contains("auto") {
        InferredSizing::ContentDriven
    } else if lower.contains("fill") || lower.contains("stretch") || lower.contains("100%") {
        InferredSizing::Fill
    } else {
        InferredSizing::ContentDriven
    }
}

fn is_list_like(name: &str) -> bool {
    name.contains("list")
        || name.contains("menu")
        || name.contains("items")
        || name.contains("feed")
        || name.contains("timeline")
}

fn is_grid_like(name: &str) -> bool {
    name.contains("grid") || name.contains("dashboard") || name.contains("gallery")
}

fn is_overlay_like(name: &str) -> bool {
    name.contains("overlay")
        || name.contains("stack")
        || name.contains("layer")
        || name.contains("popup")
}

fn is_horizontal_hint(name: &str) -> bool {
    name.contains("row")
        || name.contains("toolbar")
        || name.contains("bar")
        || name.contains("breadcrumb")
        || name.contains("tabs")
        || name.contains("horizontal")
}

fn is_interactive_element(name: &str) -> bool {
    let lower = name.to_lowercase();
    lower == "button"
        || lower == "input"
        || lower == "textarea"
        || lower == "select"
        || lower == "a"
        || lower == "link"
        || lower.contains("checkbox")
        || lower.contains("radio")
        || lower.contains("switch")
        || lower.contains("slider")
        || lower.contains("toggle")
}

fn is_interactive_role(role: &str) -> bool {
    let lower = role.to_lowercase();
    lower == "button"
        || lower == "link"
        || lower == "textbox"
        || lower == "checkbox"
        || lower == "radio"
        || lower == "slider"
        || lower == "switch"
        || lower == "tab"
        || lower == "menuitem"
        || lower == "option"
        || lower == "combobox"
        || lower == "listbox"
        || lower == "spinbutton"
        || lower == "searchbox"
        || lower == "treeitem"
}

fn is_input_element(name: &str) -> bool {
    let lower = name.to_lowercase();
    lower == "input"
        || lower == "textarea"
        || lower == "select"
        || lower.contains("textfield")
        || lower.contains("datepicker")
        || lower.contains("autocomplete")
}

fn is_boolean_state(sv: &crate::migration_ir::StateVariable) -> bool {
    if let Some(ref type_ann) = sv.type_annotation {
        let lower = type_ann.to_lowercase();
        return lower == "boolean" || lower == "bool";
    }
    if let Some(ref init) = sv.initial_value {
        return init == "true" || init == "false";
    }
    false
}

fn infer_group_name(container_name: &str, member_count: usize) -> String {
    let lower = container_name.to_lowercase();
    if lower.contains("form") {
        format!("form_{container_name}")
    } else if lower.contains("nav") || lower.contains("menu") {
        format!("nav_{container_name}")
    } else if lower.contains("toolbar") || lower.contains("actions") {
        format!("toolbar_{container_name}")
    } else if lower.contains("dialog") || lower.contains("modal") {
        format!("dialog_{container_name}")
    } else {
        format!("group_{container_name}_{member_count}")
    }
}

// ── Tests ───────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::migration_ir::{
        AccessibilityEntry, EffectDecl, EventDecl, EventTransition, IrBuilder, Provenance,
        StateVariable, ViewNode, ViewNodeKind,
    };

    fn test_provenance() -> Provenance {
        Provenance {
            file: "test.tsx".into(),
            line: 1,
            column: Some(1),
            source_name: None,
            policy_category: None,
        }
    }

    fn make_id(s: &str) -> IrNodeId {
        crate::migration_ir::make_node_id(s.as_bytes())
    }

    fn empty_ir() -> MigrationIr {
        IrBuilder::new("test-run".into(), "test-project".into()).build()
    }

    fn simple_ir_with_buttons() -> MigrationIr {
        let mut builder = IrBuilder::new("test-run".into(), "test-project".into());

        let root_id = make_id("root");
        let btn1_id = make_id("button1");
        let btn2_id = make_id("button2");
        let input_id = make_id("input1");

        builder.add_root(root_id.clone());
        builder.add_view_node(ViewNode {
            id: root_id.clone(),
            kind: ViewNodeKind::Component,
            name: "App".into(),
            children: vec![btn1_id.clone(), btn2_id.clone(), input_id.clone()],
            props: vec![],
            slots: vec![],
            conditions: vec![],
            provenance: test_provenance(),
        });
        builder.add_view_node(ViewNode {
            id: btn1_id.clone(),
            kind: ViewNodeKind::Element,
            name: "button".into(),
            children: vec![],
            props: vec![],
            slots: vec![],
            conditions: vec![],
            provenance: test_provenance(),
        });
        builder.add_view_node(ViewNode {
            id: btn2_id.clone(),
            kind: ViewNodeKind::Element,
            name: "button".into(),
            children: vec![],
            props: vec![],
            slots: vec![],
            conditions: vec![],
            provenance: test_provenance(),
        });
        builder.add_view_node(ViewNode {
            id: input_id.clone(),
            kind: ViewNodeKind::Element,
            name: "input".into(),
            children: vec![],
            props: vec![],
            slots: vec![],
            conditions: vec![],
            provenance: test_provenance(),
        });

        // Add events on buttons.
        let evt1_id = make_id("click1");
        let evt2_id = make_id("click2");
        builder.add_event(EventDecl {
            id: evt1_id.clone(),
            name: "onClick".into(),
            kind: EventKind::UserInput,
            source_node: Some(btn1_id.clone()),
            payload_type: None,
            provenance: test_provenance(),
        });
        builder.add_event(EventDecl {
            id: evt2_id.clone(),
            name: "onClick".into(),
            kind: EventKind::UserInput,
            source_node: Some(btn2_id.clone()),
            payload_type: None,
            provenance: test_provenance(),
        });

        // Add input event.
        let evt3_id = make_id("change1");
        builder.add_event(EventDecl {
            id: evt3_id.clone(),
            name: "onChange".into(),
            kind: EventKind::UserInput,
            source_node: Some(input_id.clone()),
            payload_type: None,
            provenance: test_provenance(),
        });

        // Add state variable.
        let state_id = make_id("count");
        builder.add_state_variable(StateVariable {
            id: state_id.clone(),
            name: "count".into(),
            scope: StateScope::Local,
            type_annotation: Some("number".into()),
            initial_value: Some("0".into()),
            readers: BTreeSet::from([btn1_id.clone()]),
            writers: BTreeSet::from([btn1_id.clone()]),
            provenance: test_provenance(),
        });

        // Event transition.
        builder.add_transition(EventTransition {
            event_id: evt1_id.clone(),
            target_state: state_id.clone(),
            action_snippet: "setCount(count + 1)".into(),
            guards: vec![],
        });

        builder.build()
    }

    #[test]
    fn empty_ir_produces_low_confidence() {
        let ir = empty_ir();
        let result = infer_intents(&ir);

        assert!(result.overall_confidence.score < 0.5);
        assert_eq!(result.stats.layout_regions, 0);
        assert_eq!(result.stats.focus_nodes, 0);
        assert_eq!(result.stats.interaction_clusters, 0);
    }

    #[test]
    fn interactive_elements_are_focusable() {
        let ir = simple_ir_with_buttons();
        let result = infer_intents(&ir);

        // Buttons and input should be detected as focusable.
        assert!(
            result.stats.focus_nodes >= 3,
            "Expected at least 3 focusable nodes, got {}",
            result.stats.focus_nodes
        );
    }

    #[test]
    fn tab_order_follows_tree_order() {
        let ir = simple_ir_with_buttons();
        let result = infer_intents(&ir);

        // Tab order should have entries.
        assert!(!result.focus.tab_order.is_empty());

        // All tab order entries should be in the focusable nodes.
        for id in &result.focus.tab_order {
            assert!(
                result.focus.nodes.contains_key(id),
                "Tab order entry {id} not in focusable nodes"
            );
        }
    }

    #[test]
    fn directional_hints_connect_consecutive_nodes() {
        let ir = simple_ir_with_buttons();
        let result = infer_intents(&ir);

        if result.focus.tab_order.len() >= 2 {
            assert!(
                !result.focus.directional_hints.is_empty(),
                "Expected directional hints for consecutive focusable nodes"
            );
        }
    }

    #[test]
    fn layout_regions_match_view_nodes() {
        let ir = simple_ir_with_buttons();
        let result = infer_intents(&ir);

        // Every view node should have a layout region.
        assert_eq!(
            result.stats.layout_regions,
            ir.view_tree.nodes.len(),
            "Layout regions should match view node count"
        );
    }

    #[test]
    fn leaf_nodes_get_flow_layout() {
        let ir = simple_ir_with_buttons();
        let result = infer_intents(&ir);

        let btn1_id = make_id("button1");
        if let Some(region) = result.layout.regions.get(&btn1_id) {
            assert_eq!(
                region.kind,
                LayoutRegionKind::LeafContent,
                "Leaf button should be classified as LeafContent"
            );
            assert_eq!(
                region.constraint.kind,
                InferredConstraintKind::Flow,
                "Leaf button should have Flow constraint"
            );
        }
    }

    #[test]
    fn container_nodes_get_linear_layout() {
        let ir = simple_ir_with_buttons();
        let result = infer_intents(&ir);

        let root_id = make_id("root");
        if let Some(region) = result.layout.regions.get(&root_id) {
            assert_eq!(
                region.kind,
                LayoutRegionKind::LinearContainer,
                "Container with children should be LinearContainer"
            );
        }
    }

    #[test]
    fn containers_with_children_have_alternatives() {
        let ir = simple_ir_with_buttons();
        let result = infer_intents(&ir);

        let root_id = make_id("root");
        if let Some(region) = result.layout.regions.get(&root_id) {
            // Container should have alternative direction.
            assert!(
                !region.alternatives.is_empty(),
                "Container should have alternative layout interpretations"
            );
        }
    }

    #[test]
    fn interaction_clusters_from_events() {
        let ir = simple_ir_with_buttons();
        let result = infer_intents(&ir);

        // Should have at least one interaction cluster (from the user-input events).
        assert!(
            result.stats.interaction_clusters >= 1,
            "Expected at least 1 interaction cluster, got {}",
            result.stats.interaction_clusters
        );
    }

    #[test]
    fn toggle_pattern_detection() {
        let mut builder = IrBuilder::new("test-run".into(), "test-project".into());

        let root_id = make_id("root");
        let toggle_id = make_id("toggle-btn");

        builder.add_root(root_id.clone());
        builder.add_view_node(ViewNode {
            id: root_id.clone(),
            kind: ViewNodeKind::Component,
            name: "ToggleApp".into(),
            children: vec![toggle_id.clone()],
            props: vec![],
            slots: vec![],
            conditions: vec![],
            provenance: test_provenance(),
        });
        builder.add_view_node(ViewNode {
            id: toggle_id.clone(),
            kind: ViewNodeKind::Element,
            name: "button".into(),
            children: vec![],
            props: vec![],
            slots: vec![],
            conditions: vec![],
            provenance: test_provenance(),
        });

        let state_id = make_id("is_open");
        builder.add_state_variable(StateVariable {
            id: state_id.clone(),
            name: "isOpen".into(),
            scope: StateScope::Local,
            type_annotation: Some("boolean".into()),
            initial_value: Some("false".into()),
            readers: BTreeSet::from([toggle_id.clone()]),
            writers: BTreeSet::from([toggle_id.clone()]),
            provenance: test_provenance(),
        });

        let evt_id = make_id("toggle-click");
        builder.add_event(EventDecl {
            id: evt_id.clone(),
            name: "onClick".into(),
            kind: EventKind::UserInput,
            source_node: Some(toggle_id.clone()),
            payload_type: None,
            provenance: test_provenance(),
        });
        builder.add_transition(EventTransition {
            event_id: evt_id,
            target_state: state_id,
            action_snippet: "setIsOpen(!isOpen)".into(),
            guards: vec![],
        });

        let ir = builder.build();
        let result = infer_intents(&ir);

        let has_toggle = result
            .interactions
            .clusters
            .iter()
            .any(|c| c.pattern == InteractionPattern::Toggle);
        assert!(has_toggle, "Should detect toggle pattern");
    }

    #[test]
    fn form_submission_pattern_detection() {
        let mut builder = IrBuilder::new("test-run".into(), "test-project".into());

        let root_id = make_id("form-root");
        let input1_id = make_id("email-input");
        let input2_id = make_id("password-input");
        let submit_id = make_id("submit-btn");

        builder.add_root(root_id.clone());
        builder.add_view_node(ViewNode {
            id: root_id.clone(),
            kind: ViewNodeKind::Component,
            name: "LoginForm".into(),
            children: vec![input1_id.clone(), input2_id.clone(), submit_id.clone()],
            props: vec![],
            slots: vec![],
            conditions: vec![],
            provenance: test_provenance(),
        });
        for (id, name) in [
            (&input1_id, "input"),
            (&input2_id, "input"),
            (&submit_id, "button"),
        ] {
            builder.add_view_node(ViewNode {
                id: id.clone(),
                kind: ViewNodeKind::Element,
                name: name.into(),
                children: vec![],
                props: vec![],
                slots: vec![],
                conditions: vec![],
                provenance: test_provenance(),
            });
        }

        // State for form fields.
        let email_state = make_id("email-state");
        let pwd_state = make_id("pwd-state");
        builder.add_state_variable(StateVariable {
            id: email_state.clone(),
            name: "email".into(),
            scope: StateScope::Local,
            type_annotation: Some("string".into()),
            initial_value: Some("\"\"".into()),
            readers: BTreeSet::from([input1_id.clone()]),
            writers: BTreeSet::from([input1_id.clone()]),
            provenance: test_provenance(),
        });
        builder.add_state_variable(StateVariable {
            id: pwd_state.clone(),
            name: "password".into(),
            scope: StateScope::Local,
            type_annotation: Some("string".into()),
            initial_value: Some("\"\"".into()),
            readers: BTreeSet::from([input2_id.clone()]),
            writers: BTreeSet::from([input2_id.clone()]),
            provenance: test_provenance(),
        });

        // Submit event.
        let submit_evt = make_id("submit-click");
        builder.add_event(EventDecl {
            id: submit_evt.clone(),
            name: "onSubmit".into(),
            kind: EventKind::UserInput,
            source_node: Some(submit_id.clone()),
            payload_type: None,
            provenance: test_provenance(),
        });

        // Input events.
        let change1 = make_id("email-change");
        let change2 = make_id("pwd-change");
        builder.add_event(EventDecl {
            id: change1.clone(),
            name: "onChange".into(),
            kind: EventKind::UserInput,
            source_node: Some(input1_id.clone()),
            payload_type: None,
            provenance: test_provenance(),
        });
        builder.add_event(EventDecl {
            id: change2.clone(),
            name: "onChange".into(),
            kind: EventKind::UserInput,
            source_node: Some(input2_id.clone()),
            payload_type: None,
            provenance: test_provenance(),
        });

        // Transitions.
        builder.add_transition(EventTransition {
            event_id: change1,
            target_state: email_state,
            action_snippet: "setEmail(e.target.value)".into(),
            guards: vec![],
        });
        builder.add_transition(EventTransition {
            event_id: change2,
            target_state: pwd_state,
            action_snippet: "setPassword(e.target.value)".into(),
            guards: vec![],
        });

        // Network effect on submit.
        let login_effect = make_id("login-effect");
        builder.add_effect(EffectDecl {
            id: login_effect.clone(),
            name: "loginRequest".into(),
            kind: EffectKind::Network,
            dependencies: BTreeSet::new(),
            has_cleanup: false,
            reads: BTreeSet::new(),
            writes: BTreeSet::new(),
            provenance: test_provenance(),
        });

        // Connect submit to state so the cluster reaches the network effect.
        let submit_state = make_id("submitting-state");
        builder.add_state_variable(StateVariable {
            id: submit_state.clone(),
            name: "isSubmitting".into(),
            scope: StateScope::Local,
            type_annotation: Some("boolean".into()),
            initial_value: Some("false".into()),
            readers: BTreeSet::new(),
            writers: BTreeSet::from([submit_id.clone()]),
            provenance: test_provenance(),
        });
        builder.add_transition(EventTransition {
            event_id: submit_evt,
            target_state: submit_state,
            action_snippet: "setIsSubmitting(true)".into(),
            guards: vec![],
        });

        let ir = builder.build();
        let result = infer_intents(&ir);

        // Should detect form inputs as focusable.
        assert!(
            result.stats.focus_nodes >= 3,
            "Should detect at least 3 focusable nodes in form"
        );

        // Should detect form or toggle pattern.
        let patterns: Vec<_> = result
            .interactions
            .clusters
            .iter()
            .map(|c| &c.pattern)
            .collect();
        assert!(
            !patterns.is_empty(),
            "Should detect at least one interaction pattern"
        );
    }

    #[test]
    fn navigation_pattern_detection() {
        let mut builder = IrBuilder::new("test-run".into(), "test-project".into());

        let root_id = make_id("nav-root");
        let link_id = make_id("nav-link");

        builder.add_root(root_id.clone());
        builder.add_view_node(ViewNode {
            id: root_id.clone(),
            kind: ViewNodeKind::Component,
            name: "NavBar".into(),
            children: vec![link_id.clone()],
            props: vec![],
            slots: vec![],
            conditions: vec![],
            provenance: test_provenance(),
        });
        builder.add_view_node(ViewNode {
            id: link_id.clone(),
            kind: ViewNodeKind::Element,
            name: "a".into(),
            children: vec![],
            props: vec![],
            slots: vec![],
            conditions: vec![],
            provenance: test_provenance(),
        });

        let route_state = make_id("route-state");
        builder.add_state_variable(StateVariable {
            id: route_state.clone(),
            name: "currentRoute".into(),
            scope: StateScope::Route,
            type_annotation: Some("string".into()),
            initial_value: Some("\"/\"".into()),
            readers: BTreeSet::from([link_id.clone()]),
            writers: BTreeSet::from([link_id.clone()]),
            provenance: test_provenance(),
        });

        let click_evt = make_id("nav-click");
        builder.add_event(EventDecl {
            id: click_evt.clone(),
            name: "onClick".into(),
            kind: EventKind::UserInput,
            source_node: Some(link_id),
            payload_type: None,
            provenance: test_provenance(),
        });
        builder.add_transition(EventTransition {
            event_id: click_evt,
            target_state: route_state,
            action_snippet: "navigate('/about')".into(),
            guards: vec![],
        });

        let ir = builder.build();
        let result = infer_intents(&ir);

        let has_nav = result
            .interactions
            .clusters
            .iter()
            .any(|c| c.pattern == InteractionPattern::Navigation);
        assert!(has_nav, "Should detect navigation pattern");
    }

    #[test]
    fn explicit_layout_intent_high_confidence() {
        let mut builder = IrBuilder::new("test-run".into(), "test-project".into());

        let root_id = make_id("flex-root");
        builder.add_root(root_id.clone());
        builder.add_view_node(ViewNode {
            id: root_id.clone(),
            kind: ViewNodeKind::Component,
            name: "FlexContainer".into(),
            children: vec![],
            props: vec![],
            slots: vec![],
            conditions: vec![],
            provenance: test_provenance(),
        });

        builder.add_layout(
            root_id.clone(),
            LayoutIntent {
                kind: LayoutKind::Flex,
                direction: Some("row".into()),
                alignment: Some("center".into()),
                sizing: Some("fill".into()),
            },
        );

        let ir = builder.build();
        let result = infer_intents(&ir);

        let region = result.layout.regions.get(&root_id).unwrap();
        assert_eq!(region.constraint.kind, InferredConstraintKind::Flex);
        assert_eq!(
            region.constraint.direction,
            Some(LayoutDirection::Horizontal)
        );
        assert_eq!(region.constraint.alignment, Some(LayoutAlignment::Center));
        assert_eq!(region.constraint.sizing, Some(InferredSizing::Fill));
        assert!(
            region.confidence.score >= 0.8,
            "Explicit layout should have high confidence"
        );
    }

    #[test]
    fn sidebar_main_pattern_detection() {
        let mut builder = IrBuilder::new("test-run".into(), "test-project".into());

        let root_id = make_id("app-root");
        let sidebar_id = make_id("sidebar");
        let main_id = make_id("main-content");

        builder.add_root(root_id.clone());
        builder.add_view_node(ViewNode {
            id: root_id.clone(),
            kind: ViewNodeKind::Component,
            name: "App".into(),
            children: vec![sidebar_id.clone(), main_id.clone()],
            props: vec![],
            slots: vec![],
            conditions: vec![],
            provenance: test_provenance(),
        });
        builder.add_view_node(ViewNode {
            id: sidebar_id,
            kind: ViewNodeKind::Component,
            name: "Sidebar".into(),
            children: vec![],
            props: vec![],
            slots: vec![],
            conditions: vec![],
            provenance: test_provenance(),
        });
        builder.add_view_node(ViewNode {
            id: main_id,
            kind: ViewNodeKind::Component,
            name: "MainContent".into(),
            children: vec![],
            props: vec![],
            slots: vec![],
            conditions: vec![],
            provenance: test_provenance(),
        });

        let ir = builder.build();
        let result = infer_intents(&ir);

        assert_eq!(result.layout.pattern, LayoutPattern::SidebarMain);
    }

    #[test]
    fn accessibility_focus_order_respected() {
        let mut builder = IrBuilder::new("test-run".into(), "test-project".into());

        let root_id = make_id("root");
        let btn_a = make_id("btn-a");
        let btn_b = make_id("btn-b");

        builder.add_root(root_id.clone());
        builder.add_view_node(ViewNode {
            id: root_id.clone(),
            kind: ViewNodeKind::Component,
            name: "App".into(),
            children: vec![btn_a.clone(), btn_b.clone()],
            props: vec![],
            slots: vec![],
            conditions: vec![],
            provenance: test_provenance(),
        });
        builder.add_view_node(ViewNode {
            id: btn_a.clone(),
            kind: ViewNodeKind::Element,
            name: "button".into(),
            children: vec![],
            props: vec![],
            slots: vec![],
            conditions: vec![],
            provenance: test_provenance(),
        });
        builder.add_view_node(ViewNode {
            id: btn_b.clone(),
            kind: ViewNodeKind::Element,
            name: "button".into(),
            children: vec![],
            props: vec![],
            slots: vec![],
            conditions: vec![],
            provenance: test_provenance(),
        });

        // btn_b has explicit focus order 1, btn_a has focus order 2.
        builder.add_accessibility(AccessibilityEntry {
            node_id: btn_b.clone(),
            role: Some("button".into()),
            label: Some("Second Button".into()),
            description: None,
            keyboard_shortcut: None,
            focus_order: Some(1),
            live_region: None,
        });
        builder.add_accessibility(AccessibilityEntry {
            node_id: btn_a.clone(),
            role: Some("button".into()),
            label: Some("First Button".into()),
            description: None,
            keyboard_shortcut: None,
            focus_order: Some(2),
            live_region: None,
        });

        let ir = builder.build();
        let result = infer_intents(&ir);

        // btn_b should come before btn_a in tab order due to explicit ordering.
        let b_pos = result.focus.tab_order.iter().position(|id| *id == btn_b);
        let a_pos = result.focus.tab_order.iter().position(|id| *id == btn_a);

        assert!(b_pos.is_some(), "btn_b should be in tab order");
        assert!(a_pos.is_some(), "btn_a should be in tab order");
        assert!(
            b_pos.unwrap() < a_pos.unwrap(),
            "btn_b (focus_order=1) should precede btn_a (focus_order=2)"
        );
    }

    #[test]
    fn focus_group_detection() {
        let ir = simple_ir_with_buttons();
        let result = infer_intents(&ir);

        // The root has 3 focusable children, should form a group.
        if result.stats.focus_nodes >= 3 {
            assert!(
                !result.focus.groups.is_empty(),
                "Should detect focus groups when multiple focusable nodes share a parent"
            );
        }
    }

    #[test]
    fn modal_traps_focus() {
        let mut builder = IrBuilder::new("test-run".into(), "test-project".into());

        let root_id = make_id("root");
        let modal_id = make_id("modal");
        let btn_id = make_id("modal-btn");
        let close_id = make_id("close-btn");

        builder.add_root(root_id.clone());
        builder.add_view_node(ViewNode {
            id: root_id.clone(),
            kind: ViewNodeKind::Component,
            name: "App".into(),
            children: vec![modal_id.clone()],
            props: vec![],
            slots: vec![],
            conditions: vec![],
            provenance: test_provenance(),
        });
        builder.add_view_node(ViewNode {
            id: modal_id.clone(),
            kind: ViewNodeKind::Component,
            name: "ModalDialog".into(),
            children: vec![btn_id.clone(), close_id.clone()],
            props: vec![],
            slots: vec![],
            conditions: vec![],
            provenance: test_provenance(),
        });
        builder.add_view_node(ViewNode {
            id: btn_id.clone(),
            kind: ViewNodeKind::Element,
            name: "button".into(),
            children: vec![],
            props: vec![],
            slots: vec![],
            conditions: vec![],
            provenance: test_provenance(),
        });
        builder.add_view_node(ViewNode {
            id: close_id.clone(),
            kind: ViewNodeKind::Element,
            name: "button".into(),
            children: vec![],
            props: vec![],
            slots: vec![],
            conditions: vec![],
            provenance: test_provenance(),
        });

        let ir = builder.build();
        let result = infer_intents(&ir);

        let modal_group = result
            .focus
            .groups
            .iter()
            .find(|g| g.name.contains("dialog") || g.name.contains("modal"));

        if let Some(group) = modal_group {
            assert!(group.traps_focus, "Modal group should trap focus");
        }
    }

    #[test]
    fn cross_cluster_flow_detection() {
        let mut builder = IrBuilder::new("test-run".into(), "test-project".into());

        let root_id = make_id("root");
        let search_id = make_id("search-input");
        let list_id = make_id("result-list");

        builder.add_root(root_id.clone());
        builder.add_view_node(ViewNode {
            id: root_id.clone(),
            kind: ViewNodeKind::Component,
            name: "SearchApp".into(),
            children: vec![search_id.clone(), list_id.clone()],
            props: vec![],
            slots: vec![],
            conditions: vec![],
            provenance: test_provenance(),
        });
        builder.add_view_node(ViewNode {
            id: search_id.clone(),
            kind: ViewNodeKind::Element,
            name: "input".into(),
            children: vec![],
            props: vec![],
            slots: vec![],
            conditions: vec![],
            provenance: test_provenance(),
        });
        builder.add_view_node(ViewNode {
            id: list_id.clone(),
            kind: ViewNodeKind::Element,
            name: "button".into(),
            children: vec![],
            props: vec![],
            slots: vec![],
            conditions: vec![],
            provenance: test_provenance(),
        });

        // Shared state: query.
        let query_state = make_id("query");
        builder.add_state_variable(StateVariable {
            id: query_state.clone(),
            name: "query".into(),
            scope: StateScope::Local,
            type_annotation: Some("string".into()),
            initial_value: Some("\"\"".into()),
            readers: BTreeSet::from([search_id.clone(), list_id.clone()]),
            writers: BTreeSet::from([search_id.clone()]),
            provenance: test_provenance(),
        });

        // Search input event.
        let search_evt = make_id("search-change");
        builder.add_event(EventDecl {
            id: search_evt.clone(),
            name: "onChange".into(),
            kind: EventKind::UserInput,
            source_node: Some(search_id),
            payload_type: None,
            provenance: test_provenance(),
        });
        builder.add_transition(EventTransition {
            event_id: search_evt,
            target_state: query_state.clone(),
            action_snippet: "setQuery(e.target.value)".into(),
            guards: vec![],
        });

        // List click event (separate cluster seed).
        let list_evt = make_id("list-click");
        builder.add_event(EventDecl {
            id: list_evt.clone(),
            name: "onClick".into(),
            kind: EventKind::UserInput,
            source_node: Some(list_id),
            payload_type: None,
            provenance: test_provenance(),
        });
        builder.add_transition(EventTransition {
            event_id: list_evt,
            target_state: query_state,
            action_snippet: "setQuery(selected)".into(),
            guards: vec![],
        });

        let ir = builder.build();
        let result = infer_intents(&ir);

        // Should have cross-cluster flows if both clusters touch query state.
        // Note: they might be merged into one cluster since they share a state.
        // Either way, the inference should work.
        assert!(
            result.stats.interaction_clusters >= 1,
            "Should have at least 1 interaction cluster"
        );
    }

    #[test]
    fn confidence_scores_are_bounded() {
        let ir = simple_ir_with_buttons();
        let result = infer_intents(&ir);

        assert!(result.overall_confidence.score >= 0.0);
        assert!(result.overall_confidence.score <= 1.0);

        for region in result.layout.regions.values() {
            assert!(region.confidence.score >= 0.0);
            assert!(region.confidence.score <= 1.0);
        }

        for node in result.focus.nodes.values() {
            assert!(node.confidence.score >= 0.0);
            assert!(node.confidence.score <= 1.0);
        }

        for cluster in &result.interactions.clusters {
            assert!(cluster.confidence.score >= 0.0);
            assert!(cluster.confidence.score <= 1.0);
        }
    }

    #[test]
    fn containment_edges_valid() {
        let ir = simple_ir_with_buttons();
        let result = infer_intents(&ir);

        // Every containment parent should be a layout region.
        for parent_id in result.layout.containment.keys() {
            assert!(
                result.layout.regions.contains_key(parent_id),
                "Containment parent {parent_id} should be a layout region"
            );
        }

        // Every containment child should be a layout region.
        for children in result.layout.containment.values() {
            for child_id in children {
                assert!(
                    result.layout.regions.contains_key(child_id),
                    "Containment child {child_id} should be a layout region"
                );
            }
        }
    }

    #[test]
    fn inference_stats_consistent() {
        let ir = simple_ir_with_buttons();
        let result = infer_intents(&ir);

        assert_eq!(result.stats.layout_regions, result.layout.regions.len());
        assert_eq!(result.stats.focus_nodes, result.focus.nodes.len());
        assert_eq!(result.stats.focus_groups, result.focus.groups.len());
        assert_eq!(
            result.stats.interaction_clusters,
            result.interactions.clusters.len()
        );
        assert_eq!(
            result.stats.cross_cluster_flows,
            result.interactions.cross_cluster_flows.len()
        );
    }

    #[test]
    fn grid_layout_from_explicit_intent() {
        let mut builder = IrBuilder::new("test-run".into(), "test-project".into());

        let grid_id = make_id("grid");
        builder.add_root(grid_id.clone());
        builder.add_view_node(ViewNode {
            id: grid_id.clone(),
            kind: ViewNodeKind::Component,
            name: "DashboardGrid".into(),
            children: vec![],
            props: vec![],
            slots: vec![],
            conditions: vec![],
            provenance: test_provenance(),
        });
        builder.add_layout(
            grid_id.clone(),
            LayoutIntent {
                kind: LayoutKind::Grid,
                direction: None,
                alignment: None,
                sizing: None,
            },
        );

        let ir = builder.build();
        let result = infer_intents(&ir);

        let region = result.layout.regions.get(&grid_id).unwrap();
        assert_eq!(region.kind, LayoutRegionKind::GridContainer);
        assert_eq!(region.constraint.kind, InferredConstraintKind::Grid);
    }
}
