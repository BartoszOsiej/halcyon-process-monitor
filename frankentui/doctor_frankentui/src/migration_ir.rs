// SPDX-License-Identifier: Apache-2.0
//! Canonical migration IR schema and invariants.
//!
//! Defines a versioned, syntax-independent intermediate representation
//! capturing the semantic essence of an OpenTUI application for migration
//! to FrankenTUI. Covers view tree, state graph, event transitions,
//! effects, style intent, capabilities, and accessibility semantics.
//!
//! # Invariants
//!
//! 1. **Acyclic ownership**: The view tree is strictly acyclic (parent→child).
//! 2. **Deterministic ordering**: Children, events, and effects are ordered
//!    by their stable IDs, producing identical IR from identical input.
//! 3. **Stable IDs**: Every node has a content-addressable `IrNodeId` that
//!    is invariant across re-parses of the same source.
//! 4. **Provenance links**: Every IR node links back to its source location
//!    for diagnostics and traceability.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

// ── Schema Version ───────────────────────────────────────────────────────

/// Current IR schema version (semver). Breaking changes increment the major.
pub const IR_SCHEMA_VERSION: &str = "migration-ir-v1";

// ── Identifiers ──────────────────────────────────────────────────────────

/// Content-addressable identifier for an IR node.
/// Computed as SHA-256 prefix of the node's canonical serialization.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct IrNodeId(pub String);

impl std::fmt::Display for IrNodeId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// Generate a stable node ID from content bytes.
pub fn make_node_id(content: &[u8]) -> IrNodeId {
    let hash = Sha256::digest(content);
    let hex_str = hex_encode(hash.as_slice());
    IrNodeId(format!("ir-{}", &hex_str[..16]))
}

/// Encode bytes as hex string.
fn hex_encode(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

// ── Provenance ───────────────────────────────────────────────────────────

/// Link from an IR node back to its source location.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Provenance {
    /// Relative file path within the snapshot.
    pub file: String,
    /// 1-based line number.
    pub line: usize,
    /// Optional column.
    pub column: Option<usize>,
    /// Original source construct name (e.g. component name, hook name).
    pub source_name: Option<String>,
    /// Transformation policy category from the contract.
    pub policy_category: Option<String>,
}

// ── Root IR ──────────────────────────────────────────────────────────────

/// The complete migration IR for a project.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MigrationIr {
    /// Schema version tag.
    pub schema_version: String,
    /// Unique run identifier.
    pub run_id: String,
    /// Source project identifier.
    pub source_project: String,
    /// View tree (acyclic component hierarchy).
    pub view_tree: ViewTree,
    /// State graph (state variables and derived computations).
    pub state_graph: StateGraph,
    /// Event catalog (user/system events and their transitions).
    pub event_catalog: EventCatalog,
    /// Effect registry (side effects and their scheduling).
    pub effect_registry: EffectRegistry,
    /// Style intent (visual semantics independent of CSS).
    pub style_intent: StyleIntent,
    /// Capability profile (platform/runtime requirements).
    pub capabilities: CapabilityProfile,
    /// Accessibility semantics.
    pub accessibility: AccessibilityMap,
    /// IR-level metadata and diagnostics.
    pub metadata: IrMetadata,
}

/// IR-level metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IrMetadata {
    pub created_at: String,
    pub source_file_count: usize,
    pub total_nodes: usize,
    pub total_state_vars: usize,
    pub total_events: usize,
    pub total_effects: usize,
    pub warnings: Vec<IrWarning>,
    /// Content hash of the entire IR for integrity checking.
    pub integrity_hash: Option<String>,
}

/// A non-fatal issue detected during IR construction.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IrWarning {
    pub code: String,
    pub message: String,
    pub provenance: Option<Provenance>,
}

// ── View Tree ────────────────────────────────────────────────────────────

/// The component hierarchy as an acyclic tree.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ViewTree {
    pub roots: Vec<IrNodeId>,
    pub nodes: BTreeMap<IrNodeId, ViewNode>,
}

/// A single node in the view tree.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ViewNode {
    pub id: IrNodeId,
    pub kind: ViewNodeKind,
    pub name: String,
    /// Ordered child node IDs (deterministic ordering).
    pub children: Vec<IrNodeId>,
    /// Props contract (input parameters).
    pub props: Vec<PropDecl>,
    /// Slots (render props / children insertion points).
    pub slots: Vec<SlotDecl>,
    /// Conditional rendering predicates.
    pub conditions: Vec<RenderCondition>,
    /// Source provenance.
    pub provenance: Provenance,
}

/// What kind of view node this is.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ViewNodeKind {
    /// A reusable component (function or class).
    Component,
    /// A primitive element (div, span, button, etc.).
    Element,
    /// A fragment (grouping without DOM node).
    Fragment,
    /// A portal (renders outside normal hierarchy).
    Portal,
    /// A provider (context/state distribution).
    Provider,
    /// A consumer (context/state subscription).
    Consumer,
    /// A route (navigation boundary).
    Route,
}

/// A prop declaration on a view node.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PropDecl {
    pub name: String,
    pub type_annotation: Option<String>,
    pub optional: bool,
    pub default_value: Option<String>,
    pub is_callback: bool,
}

/// A slot (insertion point for children).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SlotDecl {
    pub name: String,
    pub accepts: SlotAccepts,
}

/// What a slot accepts.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum SlotAccepts {
    /// Regular children.
    Children,
    /// Render prop (function as child).
    RenderProp,
    /// Named slot.
    Named(String),
}

/// A conditional rendering predicate.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RenderCondition {
    pub kind: ConditionKind,
    pub expression_snippet: String,
    pub state_dependencies: Vec<IrNodeId>,
}

/// Kind of conditional rendering.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ConditionKind {
    /// Boolean guard (if/ternary).
    Guard,
    /// List rendering (map/forEach).
    List,
    /// Switch/match pattern.
    Switch,
    /// Lazy/suspense boundary.
    Suspense,
}

// ── State Graph ──────────────────────────────────────────────────────────

/// The state model: variables, derived computations, and data flow.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StateGraph {
    pub variables: BTreeMap<IrNodeId, StateVariable>,
    pub derived: BTreeMap<IrNodeId, DerivedState>,
    /// Edges: state_id → set of dependent state_ids.
    pub data_flow: BTreeMap<IrNodeId, BTreeSet<IrNodeId>>,
}

/// A state variable (local or shared).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StateVariable {
    pub id: IrNodeId,
    pub name: String,
    pub scope: StateScope,
    pub type_annotation: Option<String>,
    pub initial_value: Option<String>,
    /// Which view nodes read this state.
    pub readers: BTreeSet<IrNodeId>,
    /// Which events/effects write this state.
    pub writers: BTreeSet<IrNodeId>,
    pub provenance: Provenance,
}

/// Scope of a state variable.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum StateScope {
    /// Component-local (useState, useReducer).
    Local,
    /// Shared via context.
    Context,
    /// Global store (Redux, Zustand, etc.).
    Global,
    /// URL/routing state.
    Route,
    /// Server state (React Query, SWR).
    Server,
}

/// A derived computation (useMemo, selector, computed).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DerivedState {
    pub id: IrNodeId,
    pub name: String,
    pub dependencies: BTreeSet<IrNodeId>,
    pub expression_snippet: String,
    pub provenance: Provenance,
}

// ── Event Catalog ────────────────────────────────────────────────────────

/// The event model: user interactions and system events.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EventCatalog {
    pub events: BTreeMap<IrNodeId, EventDecl>,
    /// Event → state transitions it triggers.
    pub transitions: Vec<EventTransition>,
}

/// An event declaration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EventDecl {
    pub id: IrNodeId,
    pub name: String,
    pub kind: EventKind,
    /// View node that sources this event.
    pub source_node: Option<IrNodeId>,
    /// Payload type.
    pub payload_type: Option<String>,
    pub provenance: Provenance,
}

/// Kind of event.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum EventKind {
    /// User interaction (click, keypress, etc.).
    UserInput,
    /// Lifecycle (mount, unmount, update).
    Lifecycle,
    /// Timer/interval.
    Timer,
    /// Network response.
    Network,
    /// Custom/application event.
    Custom,
}

/// A state transition triggered by an event.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EventTransition {
    pub event_id: IrNodeId,
    pub target_state: IrNodeId,
    pub action_snippet: String,
    pub guards: Vec<String>,
}

// ── Effect Registry ──────────────────────────────────────────────────────

/// The effect model: side effects and their scheduling.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EffectRegistry {
    pub effects: BTreeMap<IrNodeId, EffectDecl>,
}

/// An effect declaration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EffectDecl {
    pub id: IrNodeId,
    pub name: String,
    pub kind: EffectKind,
    /// Dependencies that trigger re-execution.
    pub dependencies: BTreeSet<IrNodeId>,
    /// Cleanup function present?
    pub has_cleanup: bool,
    /// State variables this effect reads.
    pub reads: BTreeSet<IrNodeId>,
    /// State variables this effect writes.
    pub writes: BTreeSet<IrNodeId>,
    pub provenance: Provenance,
}

/// Kind of side effect.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum EffectKind {
    /// DOM manipulation.
    Dom,
    /// Network request (fetch, XHR, WebSocket).
    Network,
    /// Timer (setTimeout, setInterval, requestAnimationFrame).
    Timer,
    /// Storage (localStorage, sessionStorage, IndexedDB).
    Storage,
    /// Subscription (event listener, observable).
    Subscription,
    /// Process spawn (child process, worker).
    Process,
    /// Logging/analytics.
    Telemetry,
    /// Other/unknown.
    Other,
}

// ── Style Intent ─────────────────────────────────────────────────────────

/// Visual semantics independent of CSS implementation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StyleIntent {
    pub tokens: BTreeMap<String, StyleToken>,
    pub layouts: BTreeMap<IrNodeId, LayoutIntent>,
    pub themes: Vec<ThemeDecl>,
}

/// A design token.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StyleToken {
    pub name: String,
    pub category: TokenCategory,
    pub value: String,
    pub provenance: Option<Provenance>,
}

/// Category of design token.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum TokenCategory {
    Color,
    Spacing,
    Typography,
    Border,
    Shadow,
    Animation,
    Breakpoint,
    ZIndex,
}

/// Layout intent for a view node.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LayoutIntent {
    pub kind: LayoutKind,
    pub direction: Option<String>,
    pub alignment: Option<String>,
    pub sizing: Option<String>,
}

/// Kind of layout.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum LayoutKind {
    Flex,
    Grid,
    Absolute,
    Stack,
    Flow,
}

/// A theme declaration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThemeDecl {
    pub name: String,
    pub tokens: BTreeMap<String, String>,
    pub is_default: bool,
}

// ── Capability Profile ───────────────────────────────────────────────────

/// Platform and runtime requirements for the application.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CapabilityProfile {
    pub required: BTreeSet<Capability>,
    pub optional: BTreeSet<Capability>,
    pub platform_assumptions: Vec<PlatformAssumption>,
}

/// A capability requirement.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Capability {
    /// Mouse/pointer input.
    MouseInput,
    /// Keyboard input.
    KeyboardInput,
    /// Touch input.
    TouchInput,
    /// Network access.
    NetworkAccess,
    /// File system access.
    FileSystem,
    /// Clipboard access.
    Clipboard,
    /// Timer/scheduling.
    Timers,
    /// Alternate screen mode.
    AlternateScreen,
    /// TrueColor (24-bit).
    TrueColor,
    /// Unicode/grapheme rendering.
    Unicode,
    /// Inline mode (preserve scrollback).
    InlineMode,
    /// Process spawning.
    ProcessSpawn,
    /// Custom capability.
    Custom(String),
}

/// An assumption about the target platform.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlatformAssumption {
    pub assumption: String,
    pub evidence: String,
    pub impact_if_wrong: String,
}

// ── Accessibility ────────────────────────────────────────────────────────

/// Accessibility semantics map.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AccessibilityMap {
    pub entries: BTreeMap<IrNodeId, AccessibilityEntry>,
}

/// Accessibility annotation for a view node.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AccessibilityEntry {
    pub node_id: IrNodeId,
    pub role: Option<String>,
    pub label: Option<String>,
    pub description: Option<String>,
    pub keyboard_shortcut: Option<String>,
    pub focus_order: Option<u32>,
    pub live_region: Option<String>,
}

// ── Validation ───────────────────────────────────────────────────────────

/// Validation error for an IR instance.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IrValidationError {
    pub code: String,
    pub message: String,
    pub node_id: Option<IrNodeId>,
}

impl std::fmt::Display for IrValidationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if let Some(ref id) = self.node_id {
            write!(f, "[{}] {}: {}", self.code, id, self.message)
        } else {
            write!(f, "[{}] {}", self.code, self.message)
        }
    }
}

/// Validate an IR instance against all structural invariants.
///
/// Returns a list of validation errors (empty = valid).
pub fn validate_ir(ir: &MigrationIr) -> Vec<IrValidationError> {
    let mut errors = Vec::new();

    // 1. Schema version check.
    if ir.schema_version != IR_SCHEMA_VERSION {
        errors.push(IrValidationError {
            code: "V001".to_string(),
            message: format!(
                "Schema version mismatch: expected {IR_SCHEMA_VERSION}, got {}",
                ir.schema_version
            ),
            node_id: None,
        });
    }

    // 2. Acyclic ownership: no cycles in view tree.
    errors.extend(validate_acyclic_ownership(&ir.view_tree));

    // 3. Referential integrity: all child IDs exist in the nodes map.
    errors.extend(validate_referential_integrity(&ir.view_tree));

    // 4. Deterministic ordering: children are sorted by ID.
    errors.extend(validate_deterministic_ordering(&ir.view_tree));

    // 5. State graph references: all state variable refs resolve.
    errors.extend(validate_state_references(
        &ir.state_graph,
        &ir.event_catalog,
        &ir.effect_registry,
    ));

    // 6. Provenance completeness: every node has a valid provenance.
    errors.extend(validate_provenance(&ir.view_tree));

    // 7. Integrity hash (if present).
    if let Some(ref expected_hash) = ir.metadata.integrity_hash {
        let actual_hash = compute_integrity_hash(ir);
        if *expected_hash != actual_hash {
            errors.push(IrValidationError {
                code: "V007".to_string(),
                message: "Integrity hash mismatch".to_string(),
                node_id: None,
            });
        }
    }

    errors
}

/// Check that the view tree is acyclic.
fn validate_acyclic_ownership(tree: &ViewTree) -> Vec<IrValidationError> {
    let mut errors = Vec::new();
    let mut visited = BTreeSet::new();
    let mut on_stack = BTreeSet::new();

    for root in &tree.roots {
        check_cycle(root, tree, &mut visited, &mut on_stack, &mut errors);
    }

    errors
}

fn check_cycle(
    node_id: &IrNodeId,
    tree: &ViewTree,
    visited: &mut BTreeSet<IrNodeId>,
    on_stack: &mut BTreeSet<IrNodeId>,
    errors: &mut Vec<IrValidationError>,
) {
    if on_stack.contains(node_id) {
        errors.push(IrValidationError {
            code: "V002".to_string(),
            message: format!("Cycle detected in view tree at node {node_id}"),
            node_id: Some(node_id.clone()),
        });
        return;
    }
    if visited.contains(node_id) {
        return;
    }

    visited.insert(node_id.clone());
    on_stack.insert(node_id.clone());

    if let Some(node) = tree.nodes.get(node_id) {
        for child_id in &node.children {
            check_cycle(child_id, tree, visited, on_stack, errors);
        }
    }

    on_stack.remove(node_id);
}

/// Check that all child references resolve to existing nodes.
fn validate_referential_integrity(tree: &ViewTree) -> Vec<IrValidationError> {
    let mut errors = Vec::new();

    // Roots must exist.
    for root in &tree.roots {
        if !tree.nodes.contains_key(root) {
            errors.push(IrValidationError {
                code: "V003".to_string(),
                message: format!("Root node {root} not found in view tree"),
                node_id: Some(root.clone()),
            });
        }
    }

    // Children must exist.
    for (parent_id, node) in &tree.nodes {
        for child_id in &node.children {
            if !tree.nodes.contains_key(child_id) {
                errors.push(IrValidationError {
                    code: "V003".to_string(),
                    message: format!("Child node {child_id} referenced by {parent_id} not found"),
                    node_id: Some(child_id.clone()),
                });
            }
        }
    }

    errors
}

/// Check that children lists are deterministically ordered.
fn validate_deterministic_ordering(tree: &ViewTree) -> Vec<IrValidationError> {
    let mut errors = Vec::new();

    for (parent_id, node) in &tree.nodes {
        for window in node.children.windows(2) {
            if window[0] > window[1] {
                errors.push(IrValidationError {
                    code: "V004".to_string(),
                    message: format!(
                        "Children of {parent_id} not in sorted order: {} > {}",
                        window[0], window[1]
                    ),
                    node_id: Some(parent_id.clone()),
                });
                break;
            }
        }
    }

    errors
}

/// Check that state variable references in events and effects resolve.
fn validate_state_references(
    state: &StateGraph,
    events: &EventCatalog,
    effects: &EffectRegistry,
) -> Vec<IrValidationError> {
    let mut errors = Vec::new();
    let state_ids: BTreeSet<&IrNodeId> = state.variables.keys().collect();

    // Event transitions must target existing state.
    for transition in &events.transitions {
        if !state_ids.contains(&transition.target_state) {
            errors.push(IrValidationError {
                code: "V005".to_string(),
                message: format!(
                    "Event transition targets unknown state {}",
                    transition.target_state
                ),
                node_id: Some(transition.target_state.clone()),
            });
        }
    }

    // Effect read/write refs must resolve.
    for (effect_id, effect) in &effects.effects {
        for read_id in &effect.reads {
            if !state_ids.contains(read_id) {
                errors.push(IrValidationError {
                    code: "V005".to_string(),
                    message: format!("Effect {effect_id} reads unknown state {read_id}"),
                    node_id: Some(read_id.clone()),
                });
            }
        }
        for write_id in &effect.writes {
            if !state_ids.contains(write_id) {
                errors.push(IrValidationError {
                    code: "V005".to_string(),
                    message: format!("Effect {effect_id} writes unknown state {write_id}"),
                    node_id: Some(write_id.clone()),
                });
            }
        }
    }

    errors
}

/// Check that all view nodes have valid provenance.
fn validate_provenance(tree: &ViewTree) -> Vec<IrValidationError> {
    let mut errors = Vec::new();

    for (node_id, node) in &tree.nodes {
        if node.provenance.file.is_empty() {
            errors.push(IrValidationError {
                code: "V006".to_string(),
                message: format!("Node {node_id} missing provenance file"),
                node_id: Some(node_id.clone()),
            });
        }
        if node.provenance.line == 0 {
            errors.push(IrValidationError {
                code: "V006".to_string(),
                message: format!("Node {node_id} has zero line number in provenance"),
                node_id: Some(node_id.clone()),
            });
        }
    }

    errors
}

/// Compute an integrity hash over the IR content (excluding the hash field itself).
pub fn compute_integrity_hash(ir: &MigrationIr) -> String {
    // Create a copy without the hash for deterministic computation.
    let mut ir_copy = ir.clone();
    ir_copy.metadata.integrity_hash = None;

    let json = serde_json::to_string(&ir_copy).unwrap_or_default();
    let hash = Sha256::digest(json.as_bytes());
    hex_encode(hash.as_slice())
}

// ── Builder ──────────────────────────────────────────────────────────────

/// Builder for constructing a MigrationIr step by step.
pub struct IrBuilder {
    run_id: String,
    source_project: String,
    view_roots: Vec<IrNodeId>,
    view_nodes: BTreeMap<IrNodeId, ViewNode>,
    state_vars: BTreeMap<IrNodeId, StateVariable>,
    derived: BTreeMap<IrNodeId, DerivedState>,
    data_flow: BTreeMap<IrNodeId, BTreeSet<IrNodeId>>,
    events: BTreeMap<IrNodeId, EventDecl>,
    transitions: Vec<EventTransition>,
    effects: BTreeMap<IrNodeId, EffectDecl>,
    style_tokens: BTreeMap<String, StyleToken>,
    layouts: BTreeMap<IrNodeId, LayoutIntent>,
    themes: Vec<ThemeDecl>,
    capabilities_required: BTreeSet<Capability>,
    capabilities_optional: BTreeSet<Capability>,
    platform_assumptions: Vec<PlatformAssumption>,
    accessibility: BTreeMap<IrNodeId, AccessibilityEntry>,
    warnings: Vec<IrWarning>,
    source_file_count: usize,
}

impl IrBuilder {
    /// Create a new IR builder.
    pub fn new(run_id: String, source_project: String) -> Self {
        Self {
            run_id,
            source_project,
            view_roots: Vec::new(),
            view_nodes: BTreeMap::new(),
            state_vars: BTreeMap::new(),
            derived: BTreeMap::new(),
            data_flow: BTreeMap::new(),
            events: BTreeMap::new(),
            transitions: Vec::new(),
            effects: BTreeMap::new(),
            style_tokens: BTreeMap::new(),
            layouts: BTreeMap::new(),
            themes: Vec::new(),
            capabilities_required: BTreeSet::new(),
            capabilities_optional: BTreeSet::new(),
            platform_assumptions: Vec::new(),
            accessibility: BTreeMap::new(),
            warnings: Vec::new(),
            source_file_count: 0,
        }
    }

    /// Set the number of source files processed.
    pub fn set_source_file_count(&mut self, count: usize) {
        self.source_file_count = count;
    }

    /// Add a view tree root.
    pub fn add_root(&mut self, id: IrNodeId) {
        self.view_roots.push(id);
    }

    /// Add a view node.
    pub fn add_view_node(&mut self, node: ViewNode) {
        self.view_nodes.insert(node.id.clone(), node);
    }

    /// Add a state variable.
    pub fn add_state_variable(&mut self, var: StateVariable) {
        self.state_vars.insert(var.id.clone(), var);
    }

    /// Add a derived state computation.
    pub fn add_derived_state(&mut self, derived: DerivedState) {
        self.derived.insert(derived.id.clone(), derived);
    }

    /// Add a data flow edge.
    pub fn add_data_flow(&mut self, from: IrNodeId, to: IrNodeId) {
        self.data_flow.entry(from).or_default().insert(to);
    }

    /// Add an event declaration.
    pub fn add_event(&mut self, event: EventDecl) {
        self.events.insert(event.id.clone(), event);
    }

    /// Add an event transition.
    pub fn add_transition(&mut self, transition: EventTransition) {
        self.transitions.push(transition);
    }

    /// Add an effect declaration.
    pub fn add_effect(&mut self, effect: EffectDecl) {
        self.effects.insert(effect.id.clone(), effect);
    }

    /// Add a style token.
    pub fn add_style_token(&mut self, token: StyleToken) {
        self.style_tokens.insert(token.name.clone(), token);
    }

    /// Add a layout intent.
    pub fn add_layout(&mut self, node_id: IrNodeId, layout: LayoutIntent) {
        self.layouts.insert(node_id, layout);
    }

    /// Add a theme.
    pub fn add_theme(&mut self, theme: ThemeDecl) {
        self.themes.push(theme);
    }

    /// Add a required capability.
    pub fn require_capability(&mut self, cap: Capability) {
        self.capabilities_required.insert(cap);
    }

    /// Add an optional capability.
    pub fn optional_capability(&mut self, cap: Capability) {
        self.capabilities_optional.insert(cap);
    }

    /// Add a platform assumption.
    pub fn add_platform_assumption(&mut self, assumption: PlatformAssumption) {
        self.platform_assumptions.push(assumption);
    }

    /// Add an accessibility entry.
    pub fn add_accessibility(&mut self, entry: AccessibilityEntry) {
        self.accessibility.insert(entry.node_id.clone(), entry);
    }

    /// Add a warning.
    pub fn add_warning(&mut self, warning: IrWarning) {
        self.warnings.push(warning);
    }

    /// Build the final IR with integrity hash.
    pub fn build(self) -> MigrationIr {
        let total_nodes = self.view_nodes.len();
        let total_state_vars = self.state_vars.len();
        let total_events = self.events.len();
        let total_effects = self.effects.len();

        let mut ir = MigrationIr {
            schema_version: IR_SCHEMA_VERSION.to_string(),
            run_id: self.run_id,
            source_project: self.source_project,
            view_tree: ViewTree {
                roots: self.view_roots,
                nodes: self.view_nodes,
            },
            state_graph: StateGraph {
                variables: self.state_vars,
                derived: self.derived,
                data_flow: self.data_flow,
            },
            event_catalog: EventCatalog {
                events: self.events,
                transitions: self.transitions,
            },
            effect_registry: EffectRegistry {
                effects: self.effects,
            },
            style_intent: StyleIntent {
                tokens: self.style_tokens,
                layouts: self.layouts,
                themes: self.themes,
            },
            capabilities: CapabilityProfile {
                required: self.capabilities_required,
                optional: self.capabilities_optional,
                platform_assumptions: self.platform_assumptions,
            },
            accessibility: AccessibilityMap {
                entries: self.accessibility,
            },
            metadata: IrMetadata {
                created_at: chrono::Utc::now().to_rfc3339(),
                source_file_count: self.source_file_count,
                total_nodes,
                total_state_vars,
                total_events,
                total_effects,
                warnings: self.warnings,
                integrity_hash: None,
            },
        };

        // Compute and set the integrity hash.
        let hash = compute_integrity_hash(&ir);
        ir.metadata.integrity_hash = Some(hash);

        ir
    }
}

// ── Tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn test_provenance(file: &str, line: usize) -> Provenance {
        Provenance {
            file: file.to_string(),
            line,
            column: None,
            source_name: None,
            policy_category: None,
        }
    }

    fn build_minimal_ir() -> MigrationIr {
        let mut builder = IrBuilder::new("test-run".to_string(), "test-project".to_string());
        builder.set_source_file_count(1);

        let root_id = make_node_id(b"root-component");
        let child_id = make_node_id(b"child-component");

        builder.add_root(root_id.clone());
        builder.add_view_node(ViewNode {
            id: root_id.clone(),
            kind: ViewNodeKind::Component,
            name: "App".to_string(),
            children: vec![child_id.clone()],
            props: Vec::new(),
            slots: Vec::new(),
            conditions: Vec::new(),
            provenance: test_provenance("src/App.tsx", 1),
        });
        builder.add_view_node(ViewNode {
            id: child_id.clone(),
            kind: ViewNodeKind::Element,
            name: "div".to_string(),
            children: Vec::new(),
            props: Vec::new(),
            slots: Vec::new(),
            conditions: Vec::new(),
            provenance: test_provenance("src/App.tsx", 3),
        });

        let state_id = make_node_id(b"count-state");
        builder.add_state_variable(StateVariable {
            id: state_id.clone(),
            name: "count".to_string(),
            scope: StateScope::Local,
            type_annotation: Some("number".to_string()),
            initial_value: Some("0".to_string()),
            readers: BTreeSet::from([root_id.clone()]),
            writers: BTreeSet::new(),
            provenance: test_provenance("src/App.tsx", 2),
        });

        builder.require_capability(Capability::KeyboardInput);
        builder.add_platform_assumption(PlatformAssumption {
            assumption: "Browser DOM APIs are available at runtime".to_string(),
            evidence: "useEffect(() => window.addEventListener(...))".to_string(),
            impact_if_wrong: "Event wiring fails in non-DOM hosts".to_string(),
        });

        builder.build()
    }

    #[test]
    fn schema_version_is_correct() {
        let ir = build_minimal_ir();
        assert_eq!(ir.schema_version, IR_SCHEMA_VERSION);
    }

    #[test]
    fn valid_ir_passes_validation() {
        let ir = build_minimal_ir();
        let errors = validate_ir(&ir);
        assert!(errors.is_empty(), "Unexpected errors: {errors:?}");
    }

    #[test]
    fn wrong_schema_version_fails() {
        let mut ir = build_minimal_ir();
        ir.schema_version = "wrong-version".to_string();
        let errors = validate_ir(&ir);
        assert!(errors.iter().any(|e| e.code == "V001"));
    }

    #[test]
    fn cycle_in_view_tree_detected() {
        let mut ir = build_minimal_ir();
        let id_a = make_node_id(b"cycle-a");
        let id_b = make_node_id(b"cycle-b");

        ir.view_tree.roots = vec![id_a.clone()];
        ir.view_tree.nodes.clear();
        ir.view_tree.nodes.insert(
            id_a.clone(),
            ViewNode {
                id: id_a.clone(),
                kind: ViewNodeKind::Component,
                name: "A".to_string(),
                children: vec![id_b.clone()],
                props: Vec::new(),
                slots: Vec::new(),
                conditions: Vec::new(),
                provenance: test_provenance("a.tsx", 1),
            },
        );
        ir.view_tree.nodes.insert(
            id_b.clone(),
            ViewNode {
                id: id_b.clone(),
                kind: ViewNodeKind::Component,
                name: "B".to_string(),
                children: vec![id_a.clone()],
                props: Vec::new(),
                slots: Vec::new(),
                conditions: Vec::new(),
                provenance: test_provenance("b.tsx", 1),
            },
        );

        let errors = validate_ir(&ir);
        assert!(errors.iter().any(|e| e.code == "V002"));
    }

    #[test]
    fn missing_child_node_detected() {
        let mut ir = build_minimal_ir();
        let missing = IrNodeId("ir-missing".to_string());

        if let Some(root) = ir.view_tree.nodes.values_mut().find(|n| n.name == "App") {
            root.children.push(missing);
        }

        let errors = validate_ir(&ir);
        assert!(errors.iter().any(|e| e.code == "V003"));
    }

    #[test]
    fn unsorted_children_detected() {
        let mut ir = build_minimal_ir();
        let id_z = IrNodeId("ir-zzzz".to_string());
        let id_a = IrNodeId("ir-aaaa".to_string());

        // Add both nodes.
        ir.view_tree.nodes.insert(
            id_z.clone(),
            ViewNode {
                id: id_z.clone(),
                kind: ViewNodeKind::Element,
                name: "z".to_string(),
                children: Vec::new(),
                props: Vec::new(),
                slots: Vec::new(),
                conditions: Vec::new(),
                provenance: test_provenance("z.tsx", 1),
            },
        );
        ir.view_tree.nodes.insert(
            id_a.clone(),
            ViewNode {
                id: id_a.clone(),
                kind: ViewNodeKind::Element,
                name: "a".to_string(),
                children: Vec::new(),
                props: Vec::new(),
                slots: Vec::new(),
                conditions: Vec::new(),
                provenance: test_provenance("a.tsx", 1),
            },
        );

        // Set unsorted children on root.
        if let Some(root) = ir.view_tree.nodes.values_mut().find(|n| n.name == "App") {
            root.children = vec![id_z, id_a]; // wrong order
        }

        let errors = validate_ir(&ir);
        assert!(errors.iter().any(|e| e.code == "V004"));
    }

    #[test]
    fn event_transition_to_unknown_state_detected() {
        let mut ir = build_minimal_ir();
        ir.event_catalog.transitions.push(EventTransition {
            event_id: make_node_id(b"click"),
            target_state: IrNodeId("ir-nonexistent".to_string()),
            action_snippet: "setCount(c + 1)".to_string(),
            guards: Vec::new(),
        });

        let errors = validate_ir(&ir);
        assert!(errors.iter().any(|e| e.code == "V005"));
    }

    #[test]
    fn missing_provenance_file_detected() {
        let mut ir = build_minimal_ir();
        if let Some(node) = ir.view_tree.nodes.values_mut().next() {
            node.provenance.file = String::new();
        }

        let errors = validate_ir(&ir);
        assert!(errors.iter().any(|e| e.code == "V006"));
    }

    #[test]
    fn integrity_hash_is_computed() {
        let ir = build_minimal_ir();
        assert!(ir.metadata.integrity_hash.is_some());
        let hash = ir.metadata.integrity_hash.as_ref().unwrap();
        assert_eq!(hash.len(), 64); // SHA-256 hex
    }

    #[test]
    fn integrity_hash_is_deterministic() {
        let ir1 = build_minimal_ir();
        let ir2 = build_minimal_ir();
        // Hashes may differ due to timestamps, but the compute function
        // should be deterministic given the same input.
        let h1 = compute_integrity_hash(&ir1);
        let h2 = compute_integrity_hash(&ir1);
        assert_eq!(h1, h2);
        let _ = ir2; // used to verify construction doesn't panic
    }

    #[test]
    fn tampered_hash_fails_validation() {
        let mut ir = build_minimal_ir();
        ir.metadata.integrity_hash =
            Some("0000000000000000000000000000000000000000000000000000000000000000".to_string());
        let errors = validate_ir(&ir);
        assert!(errors.iter().any(|e| e.code == "V007"));
    }

    #[test]
    fn ir_serializes_to_json() {
        let ir = build_minimal_ir();
        let json = serde_json::to_string_pretty(&ir).unwrap();
        assert!(json.contains("migration-ir-v1"));
        assert!(json.contains("App"));

        // Round-trip.
        let parsed: MigrationIr = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.schema_version, ir.schema_version);
        assert_eq!(parsed.view_tree.nodes.len(), ir.view_tree.nodes.len());
    }

    #[test]
    fn node_id_generation_is_stable() {
        let id1 = make_node_id(b"test-content");
        let id2 = make_node_id(b"test-content");
        assert_eq!(id1, id2);
        assert!(id1.0.starts_with("ir-"));
        assert_eq!(id1.0.len(), 19); // "ir-" + 16 hex chars
    }

    #[test]
    fn different_content_produces_different_ids() {
        let id1 = make_node_id(b"content-a");
        let id2 = make_node_id(b"content-b");
        assert_ne!(id1, id2);
    }

    #[test]
    fn builder_creates_valid_ir() {
        let ir = build_minimal_ir();
        assert_eq!(ir.metadata.total_nodes, 2);
        assert_eq!(ir.metadata.total_state_vars, 1);
        assert_eq!(ir.metadata.source_file_count, 1);
        assert!(
            ir.capabilities
                .required
                .contains(&Capability::KeyboardInput)
        );
        assert!(!ir.capabilities.platform_assumptions.is_empty());
    }

    #[test]
    fn view_node_kinds_are_distinct() {
        let kinds = [
            ViewNodeKind::Component,
            ViewNodeKind::Element,
            ViewNodeKind::Fragment,
            ViewNodeKind::Portal,
            ViewNodeKind::Provider,
            ViewNodeKind::Consumer,
            ViewNodeKind::Route,
        ];
        for (i, a) in kinds.iter().enumerate() {
            for (j, b) in kinds.iter().enumerate() {
                if i == j {
                    assert_eq!(a, b);
                } else {
                    assert_ne!(a, b);
                }
            }
        }
    }

    #[test]
    fn state_scope_variants() {
        let scopes = [
            StateScope::Local,
            StateScope::Context,
            StateScope::Global,
            StateScope::Route,
            StateScope::Server,
        ];
        // Ensure distinct serialization.
        for (i, a) in scopes.iter().enumerate() {
            for (j, b) in scopes.iter().enumerate() {
                let sa = serde_json::to_string(a).unwrap();
                let sb = serde_json::to_string(b).unwrap();
                if i == j {
                    assert_eq!(sa, sb);
                } else {
                    assert_ne!(sa, sb);
                }
            }
        }
    }

    #[test]
    fn effect_read_to_unknown_state_detected() {
        let mut ir = build_minimal_ir();
        let effect_id = make_node_id(b"bad-effect");
        ir.effect_registry.effects.insert(
            effect_id.clone(),
            EffectDecl {
                id: effect_id,
                name: "badEffect".to_string(),
                kind: EffectKind::Network,
                dependencies: BTreeSet::new(),
                has_cleanup: false,
                reads: BTreeSet::from([IrNodeId("ir-nonexistent".to_string())]),
                writes: BTreeSet::new(),
                provenance: test_provenance("eff.ts", 1),
            },
        );

        let errors = validate_ir(&ir);
        assert!(errors.iter().any(|e| e.code == "V005"));
    }

    #[test]
    fn validation_error_display() {
        let err = IrValidationError {
            code: "V001".to_string(),
            message: "test error".to_string(),
            node_id: Some(IrNodeId("ir-abc".to_string())),
        };
        assert_eq!(format!("{err}"), "[V001] ir-abc: test error");

        let err_no_node = IrValidationError {
            code: "V001".to_string(),
            message: "test error".to_string(),
            node_id: None,
        };
        assert_eq!(format!("{err_no_node}"), "[V001] test error");
    }

    #[test]
    fn empty_ir_is_valid() {
        let builder = IrBuilder::new("run".to_string(), "proj".to_string());
        let ir = builder.build();
        let errors = validate_ir(&ir);
        assert!(errors.is_empty());
    }

    #[test]
    fn capability_ordering() {
        let mut caps = BTreeSet::new();
        caps.insert(Capability::Unicode);
        caps.insert(Capability::KeyboardInput);
        caps.insert(Capability::MouseInput);
        caps.insert(Capability::Custom("custom-cap".to_string()));

        // BTreeSet should maintain ordering.
        let ordered: Vec<_> = caps.iter().collect();
        assert!(ordered.len() == 4);
    }

    #[test]
    fn provenance_with_all_fields() {
        let p = Provenance {
            file: "src/App.tsx".to_string(),
            line: 42,
            column: Some(5),
            source_name: Some("App".to_string()),
            policy_category: Some("state".to_string()),
        };
        let json = serde_json::to_string(&p).unwrap();
        assert!(json.contains("42"));
        assert!(json.contains("App"));
        let parsed: Provenance = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.line, 42);
    }

    #[test]
    fn derived_state_in_graph() {
        let mut builder = IrBuilder::new("run".to_string(), "proj".to_string());

        let state_id = make_node_id(b"items");
        let derived_id = make_node_id(b"item-count");

        builder.add_state_variable(StateVariable {
            id: state_id.clone(),
            name: "items".to_string(),
            scope: StateScope::Local,
            type_annotation: Some("Item[]".to_string()),
            initial_value: Some("[]".to_string()),
            readers: BTreeSet::new(),
            writers: BTreeSet::new(),
            provenance: test_provenance("store.ts", 1),
        });

        builder.add_derived_state(DerivedState {
            id: derived_id.clone(),
            name: "itemCount".to_string(),
            dependencies: BTreeSet::from([state_id.clone()]),
            expression_snippet: "items.length".to_string(),
            provenance: test_provenance("store.ts", 5),
        });

        builder.add_data_flow(state_id, derived_id);

        let ir = builder.build();
        assert_eq!(ir.state_graph.derived.len(), 1);
        assert_eq!(ir.state_graph.data_flow.len(), 1);
    }
}
