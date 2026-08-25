//! IR normalization passes for the canonical [`MigrationIr`].
//!
//! Each pass is idempotent and order-stable: applying a pass to an already-
//! normalized IR produces no changes.  Passes are composed via
//! [`normalize`], which applies them in a fixed deterministic order.
//!
//! ## Pass inventory
//!
//! | Pass | Purpose |
//! |------|---------|
//! | `canonicalize_ordering` | Sort all maps and lists by stable IDs |
//! | `desugar_fragments` | Hoist children of empty fragments into parents |
//! | `prune_dead_state` | Remove unreferenced state variables |
//! | `prune_dead_events` | Remove events with no transitions |
//! | `merge_duplicate_tokens` | Deduplicate identical design tokens |
//! | `normalize_provenance` | Strip trailing slashes, normalize file paths |

use std::collections::{BTreeMap, BTreeSet};

use crate::migration_ir::{
    EffectRegistry, EventCatalog, IrNodeId, MigrationIr, StateGraph, StyleIntent, ViewNodeKind,
    ViewTree, compute_integrity_hash,
};

// ── Public API ──────────────────────────────────────────────────────────

/// Apply all normalization passes to an IR in deterministic order.
///
/// Returns the number of mutations performed (0 = already normalized).
pub fn normalize(ir: &mut MigrationIr) -> NormalizationReport {
    let mut report = NormalizationReport::default();

    report.ordering_changes += canonicalize_ordering(ir);
    report.fragments_desugared += desugar_fragments(&mut ir.view_tree);
    report.dead_state_pruned += prune_dead_state(&mut ir.state_graph, &ir.effect_registry);
    report.dead_events_pruned += prune_dead_events(&mut ir.event_catalog, &ir.state_graph);
    report.tokens_merged += merge_duplicate_tokens(&mut ir.style_intent);
    report.provenance_normalized += normalize_provenance(&mut ir.view_tree);

    // Update metadata counts and recompute integrity hash.
    ir.metadata.total_nodes = ir.view_tree.nodes.len();
    ir.metadata.total_state_vars = ir.state_graph.variables.len();
    ir.metadata.total_events = ir.event_catalog.events.len();
    ir.metadata.total_effects = ir.effect_registry.effects.len();
    if ir.metadata.integrity_hash.is_some() {
        ir.metadata.integrity_hash = Some(compute_integrity_hash(ir));
    }

    report
}

/// Summary of mutations performed by normalization.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct NormalizationReport {
    pub ordering_changes: usize,
    pub fragments_desugared: usize,
    pub dead_state_pruned: usize,
    pub dead_events_pruned: usize,
    pub tokens_merged: usize,
    pub provenance_normalized: usize,
}

impl NormalizationReport {
    /// Total number of mutations across all passes.
    pub fn total(&self) -> usize {
        self.ordering_changes
            + self.fragments_desugared
            + self.dead_state_pruned
            + self.dead_events_pruned
            + self.tokens_merged
            + self.provenance_normalized
    }

    /// True if no mutations were needed.
    pub fn is_clean(&self) -> bool {
        self.total() == 0
    }
}

// ── Pass 1: Canonical ordering ──────────────────────────────────────────

/// Sort children lists in all view nodes by ID for deterministic output.
fn canonicalize_ordering(ir: &mut MigrationIr) -> usize {
    let mut changes = 0;

    // Sort view node children.
    for node in ir.view_tree.nodes.values_mut() {
        let mut sorted = node.children.clone();
        sorted.sort();
        if sorted != node.children {
            node.children = sorted;
            changes += 1;
        }
    }

    // Sort roots.
    let mut sorted_roots = ir.view_tree.roots.clone();
    sorted_roots.sort();
    if sorted_roots != ir.view_tree.roots {
        ir.view_tree.roots = sorted_roots;
        changes += 1;
    }

    // Sort event transition lists by event ID.
    ir.event_catalog.transitions.sort_by(|a, b| {
        a.event_id
            .cmp(&b.event_id)
            .then_with(|| a.target_state.cmp(&b.target_state))
    });

    changes
}

// ── Pass 2: Desugar fragments ───────────────────────────────────────────

/// Hoist children of empty, nameless fragments into their parent nodes.
///
/// A fragment with no props, no conditions, and no slots is purely
/// structural — its children should live directly under the parent.
fn desugar_fragments(tree: &mut ViewTree) -> usize {
    let mut fragments_to_remove: Vec<IrNodeId> = Vec::new();

    // Identify fragments eligible for removal.
    let fragment_ids: Vec<IrNodeId> = tree
        .nodes
        .iter()
        .filter(|(_, node)| {
            node.kind == ViewNodeKind::Fragment
                && node.props.is_empty()
                && node.conditions.is_empty()
                && node.slots.is_empty()
        })
        .map(|(id, _)| id.clone())
        .collect();

    if fragment_ids.is_empty() {
        return 0;
    }

    // Build parent map: child_id → parent_id.
    let parent_map = build_parent_map(tree);

    for frag_id in &fragment_ids {
        if let Some(frag_node) = tree.nodes.get(frag_id) {
            let frag_children = frag_node.children.clone();

            if let Some(parent_id) = parent_map.get(frag_id) {
                // Replace fragment reference in parent with fragment's children.
                if let Some(parent) = tree.nodes.get_mut(parent_id) {
                    let pos = parent.children.iter().position(|c| c == frag_id);
                    if let Some(idx) = pos {
                        parent.children.remove(idx);
                        for (i, child) in frag_children.iter().enumerate() {
                            parent.children.insert(idx + i, child.clone());
                        }
                    }
                }
                fragments_to_remove.push(frag_id.clone());
            } else if tree.roots.contains(frag_id) {
                // Fragment is a root — replace in roots list.
                let pos = tree.roots.iter().position(|r| r == frag_id);
                if let Some(idx) = pos {
                    tree.roots.remove(idx);
                    for (i, child) in frag_children.iter().enumerate() {
                        tree.roots.insert(idx + i, child.clone());
                    }
                }
                fragments_to_remove.push(frag_id.clone());
            }
        }
    }

    // Remove desugared fragment nodes.
    for id in &fragments_to_remove {
        tree.nodes.remove(id);
    }

    fragments_to_remove.len()
}

fn build_parent_map(tree: &ViewTree) -> BTreeMap<IrNodeId, IrNodeId> {
    let mut map = BTreeMap::new();
    for (parent_id, node) in &tree.nodes {
        for child_id in &node.children {
            map.insert(child_id.clone(), parent_id.clone());
        }
    }
    map
}

// ── Pass 3: Prune dead state ────────────────────────────────────────────

/// Remove state variables that are never read by any view node, effect,
/// or event transition.
///
/// A state variable is "dead" if its `readers` set is empty AND no effect
/// reads it AND no event transitions target it.
fn prune_dead_state(state: &mut StateGraph, effects: &EffectRegistry) -> usize {
    // Collect all state IDs referenced by effects.
    let effect_refs: BTreeSet<IrNodeId> = effects
        .effects
        .values()
        .flat_map(|e| {
            e.reads
                .iter()
                .chain(e.writes.iter())
                .chain(e.dependencies.iter())
        })
        .cloned()
        .collect();

    // Collect state IDs referenced as derived deps.
    let derived_refs: BTreeSet<IrNodeId> = state
        .derived
        .values()
        .flat_map(|d| d.dependencies.iter())
        .cloned()
        .collect();

    // Collect state IDs referenced in data flow.
    let flow_refs: BTreeSet<IrNodeId> = state
        .data_flow
        .iter()
        .flat_map(|(from, tos)| std::iter::once(from).chain(tos.iter()))
        .cloned()
        .collect();

    let all_refs: BTreeSet<IrNodeId> = effect_refs
        .union(&derived_refs)
        .cloned()
        .collect::<BTreeSet<_>>()
        .union(&flow_refs)
        .cloned()
        .collect();

    let dead: Vec<IrNodeId> = state
        .variables
        .iter()
        .filter(|(id, var)| {
            var.readers.is_empty() && var.writers.is_empty() && !all_refs.contains(*id)
        })
        .map(|(id, _)| id.clone())
        .collect();

    let count = dead.len();
    for id in &dead {
        state.variables.remove(id);
    }

    count
}

// ── Pass 4: Prune dead events ───────────────────────────────────────────

/// Remove events that have no associated transitions and no valid
/// target state.
fn prune_dead_events(catalog: &mut EventCatalog, state: &StateGraph) -> usize {
    // Events that have at least one transition.
    let events_with_transitions: BTreeSet<IrNodeId> = catalog
        .transitions
        .iter()
        .map(|t| t.event_id.clone())
        .collect();

    let dead: Vec<IrNodeId> = catalog
        .events
        .keys()
        .filter(|id| !events_with_transitions.contains(*id))
        .cloned()
        .collect();

    let count = dead.len();
    for id in &dead {
        catalog.events.remove(id);
    }

    // Also remove transitions to nonexistent state.
    let before = catalog.transitions.len();
    catalog
        .transitions
        .retain(|t| state.variables.contains_key(&t.target_state));
    let transitions_removed = before - catalog.transitions.len();

    count + transitions_removed
}

// ── Pass 5: Merge duplicate tokens ──────────────────────────────────────

/// Deduplicate design tokens with identical category + value, keeping
/// the first occurrence by name ordering.
fn merge_duplicate_tokens(style: &mut StyleIntent) -> usize {
    let mut seen: BTreeMap<(String, String), String> = BTreeMap::new();
    let mut duplicates = Vec::new();

    for (name, token) in &style.tokens {
        let key = (format!("{:?}", token.category), token.value.clone());
        if let std::collections::btree_map::Entry::Vacant(e) = seen.entry(key) {
            e.insert(name.clone());
        } else {
            duplicates.push(name.clone());
        }
    }

    let count = duplicates.len();
    for name in &duplicates {
        style.tokens.remove(name);
    }

    count
}

// ── Pass 6: Normalize provenance ────────────────────────────────────────

/// Normalize file paths in provenance: strip leading `./`, normalize
/// separators, trim whitespace.
fn normalize_provenance(tree: &mut ViewTree) -> usize {
    let mut changes = 0;

    for node in tree.nodes.values_mut() {
        let normalized = normalize_file_path(&node.provenance.file);
        if normalized != node.provenance.file {
            node.provenance.file = normalized;
            changes += 1;
        }
    }

    changes
}

fn normalize_file_path(path: &str) -> String {
    let trimmed = path.trim();
    let no_prefix = trimmed.strip_prefix("./").unwrap_or(trimmed);
    // Normalize backslashes to forward slashes.
    no_prefix.replace('\\', "/")
}

// ── Tests ───────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lowering::{LoweringConfig, lower_project};
    use crate::migration_ir::{
        EventDecl, EventKind, EventTransition, Provenance, StateScope, StateVariable, StyleToken,
        TokenCategory, ViewNode, validate_ir,
    };
    use crate::tsx_parser::{
        ComponentDecl, ComponentKind, FileParse, HookCall, JsxElement, JsxProp, ProjectParse,
    };
    use std::collections::BTreeSet;

    fn test_config() -> LoweringConfig {
        LoweringConfig {
            run_id: "test-norm".to_string(),
            source_project: "test-project".to_string(),
        }
    }

    fn make_view_node(id: &str, name: &str, kind: ViewNodeKind, children: &[&str]) -> ViewNode {
        ViewNode {
            id: IrNodeId(id.to_string()),
            kind,
            name: name.to_string(),
            children: children.iter().map(|s| IrNodeId(s.to_string())).collect(),
            props: Vec::new(),
            slots: Vec::new(),
            conditions: Vec::new(),
            provenance: Provenance {
                file: "src/App.tsx".to_string(),
                line: 1,
                column: None,
                source_name: Some(name.to_string()),
                policy_category: None,
            },
        }
    }

    fn make_ir_with_tree(nodes: Vec<ViewNode>, roots: Vec<&str>) -> MigrationIr {
        let mut tree = ViewTree {
            roots: roots.iter().map(|s| IrNodeId(s.to_string())).collect(),
            nodes: BTreeMap::new(),
        };
        for node in nodes {
            tree.nodes.insert(node.id.clone(), node);
        }

        MigrationIr {
            schema_version: "migration-ir-v1".to_string(),
            run_id: "test".to_string(),
            source_project: "test".to_string(),
            view_tree: tree,
            state_graph: StateGraph {
                variables: BTreeMap::new(),
                derived: BTreeMap::new(),
                data_flow: BTreeMap::new(),
            },
            event_catalog: EventCatalog {
                events: BTreeMap::new(),
                transitions: Vec::new(),
            },
            effect_registry: EffectRegistry {
                effects: BTreeMap::new(),
            },
            style_intent: StyleIntent {
                tokens: BTreeMap::new(),
                layouts: BTreeMap::new(),
                themes: Vec::new(),
            },
            capabilities: crate::migration_ir::CapabilityProfile {
                required: BTreeSet::new(),
                optional: BTreeSet::new(),
                platform_assumptions: Vec::new(),
            },
            accessibility: crate::migration_ir::AccessibilityMap {
                entries: BTreeMap::new(),
            },
            metadata: crate::migration_ir::IrMetadata {
                created_at: "2026-01-01T00:00:00Z".to_string(),
                source_file_count: 1,
                total_nodes: 0,
                total_state_vars: 0,
                total_events: 0,
                total_effects: 0,
                warnings: Vec::new(),
                integrity_hash: None,
            },
        }
    }

    // ── Idempotence ─────────────────────────────────────────────────────

    #[test]
    fn normalize_is_idempotent() {
        let mut ir = make_ir_with_tree(
            vec![
                make_view_node("c", "C", ViewNodeKind::Element, &[]),
                make_view_node("a", "A", ViewNodeKind::Component, &["c"]),
                make_view_node("b", "B", ViewNodeKind::Element, &[]),
            ],
            vec!["b", "a"],
        );

        let r1 = normalize(&mut ir);
        let r2 = normalize(&mut ir);

        assert!(r2.is_clean(), "Second normalize should be clean: {:?}", r2);
        // First may or may not have changes depending on input.
        let _ = r1;
    }

    // ── Canonical ordering ──────────────────────────────────────────────

    #[test]
    fn children_sorted_by_id() {
        let mut ir = make_ir_with_tree(
            vec![
                make_view_node(
                    "parent",
                    "Parent",
                    ViewNodeKind::Component,
                    &["z-child", "a-child"],
                ),
                make_view_node("z-child", "Z", ViewNodeKind::Element, &[]),
                make_view_node("a-child", "A", ViewNodeKind::Element, &[]),
            ],
            vec!["parent"],
        );

        let report = normalize(&mut ir);
        let parent = &ir.view_tree.nodes[&IrNodeId("parent".to_string())];
        assert_eq!(
            parent.children,
            vec![
                IrNodeId("a-child".to_string()),
                IrNodeId("z-child".to_string())
            ]
        );
        assert!(report.ordering_changes > 0);
    }

    #[test]
    fn roots_sorted() {
        let mut ir = make_ir_with_tree(
            vec![
                make_view_node("z-root", "Z", ViewNodeKind::Component, &[]),
                make_view_node("a-root", "A", ViewNodeKind::Component, &[]),
            ],
            vec!["z-root", "a-root"],
        );

        normalize(&mut ir);
        assert_eq!(
            ir.view_tree.roots,
            vec![
                IrNodeId("a-root".to_string()),
                IrNodeId("z-root".to_string())
            ]
        );
    }

    // ── Fragment desugaring ─────────────────────────────────────────────

    #[test]
    fn empty_fragment_desugared() {
        let mut ir = make_ir_with_tree(
            vec![
                make_view_node("parent", "Parent", ViewNodeKind::Component, &["frag"]),
                make_view_node(
                    "frag",
                    "Fragment",
                    ViewNodeKind::Fragment,
                    &["child-a", "child-b"],
                ),
                make_view_node("child-a", "A", ViewNodeKind::Element, &[]),
                make_view_node("child-b", "B", ViewNodeKind::Element, &[]),
            ],
            vec!["parent"],
        );

        let report = normalize(&mut ir);
        assert_eq!(report.fragments_desugared, 1);

        // Fragment should be removed.
        assert!(
            !ir.view_tree
                .nodes
                .contains_key(&IrNodeId("frag".to_string()))
        );

        // Children should be hoisted to parent.
        let parent = &ir.view_tree.nodes[&IrNodeId("parent".to_string())];
        assert!(parent.children.contains(&IrNodeId("child-a".to_string())));
        assert!(parent.children.contains(&IrNodeId("child-b".to_string())));
    }

    #[test]
    fn fragment_with_conditions_kept() {
        let mut frag = make_view_node("frag", "Fragment", ViewNodeKind::Fragment, &["child"]);
        frag.conditions.push(crate::migration_ir::RenderCondition {
            kind: crate::migration_ir::ConditionKind::Guard,
            expression_snippet: "isVisible".to_string(),
            state_dependencies: Vec::new(),
        });

        let mut ir = make_ir_with_tree(
            vec![
                make_view_node("parent", "Parent", ViewNodeKind::Component, &["frag"]),
                frag,
                make_view_node("child", "Child", ViewNodeKind::Element, &[]),
            ],
            vec!["parent"],
        );

        let report = normalize(&mut ir);
        assert_eq!(report.fragments_desugared, 0);
        assert!(
            ir.view_tree
                .nodes
                .contains_key(&IrNodeId("frag".to_string()))
        );
    }

    #[test]
    fn root_fragment_desugared() {
        let mut ir = make_ir_with_tree(
            vec![
                make_view_node(
                    "frag",
                    "Fragment",
                    ViewNodeKind::Fragment,
                    &["child-a", "child-b"],
                ),
                make_view_node("child-a", "A", ViewNodeKind::Element, &[]),
                make_view_node("child-b", "B", ViewNodeKind::Element, &[]),
            ],
            vec!["frag"],
        );

        normalize(&mut ir);
        assert!(!ir.view_tree.roots.contains(&IrNodeId("frag".to_string())));
        assert!(
            ir.view_tree
                .roots
                .contains(&IrNodeId("child-a".to_string()))
        );
        assert!(
            ir.view_tree
                .roots
                .contains(&IrNodeId("child-b".to_string()))
        );
    }

    // ── Dead state pruning ──────────────────────────────────────────────

    #[test]
    fn unreferenced_state_pruned() {
        let mut ir = make_ir_with_tree(vec![], vec![]);
        ir.state_graph.variables.insert(
            IrNodeId("dead-var".to_string()),
            StateVariable {
                id: IrNodeId("dead-var".to_string()),
                name: "deadVar".to_string(),
                scope: StateScope::Local,
                type_annotation: None,
                initial_value: None,
                readers: BTreeSet::new(),
                writers: BTreeSet::new(),
                provenance: Provenance {
                    file: "test.tsx".to_string(),
                    line: 1,
                    column: None,
                    source_name: None,
                    policy_category: None,
                },
            },
        );

        let report = normalize(&mut ir);
        assert_eq!(report.dead_state_pruned, 1);
        assert!(ir.state_graph.variables.is_empty());
    }

    #[test]
    fn referenced_state_kept() {
        let mut ir = make_ir_with_tree(vec![], vec![]);
        let var_id = IrNodeId("used-var".to_string());
        ir.state_graph.variables.insert(
            var_id.clone(),
            StateVariable {
                id: var_id.clone(),
                name: "usedVar".to_string(),
                scope: StateScope::Local,
                type_annotation: None,
                initial_value: None,
                readers: BTreeSet::from([IrNodeId("reader".to_string())]),
                writers: BTreeSet::new(),
                provenance: Provenance {
                    file: "test.tsx".to_string(),
                    line: 1,
                    column: None,
                    source_name: None,
                    policy_category: None,
                },
            },
        );

        let report = normalize(&mut ir);
        assert_eq!(report.dead_state_pruned, 0);
        assert!(ir.state_graph.variables.contains_key(&var_id));
    }

    // ── Dead event pruning ──────────────────────────────────────────────

    #[test]
    fn event_without_transitions_pruned() {
        let mut ir = make_ir_with_tree(vec![], vec![]);
        ir.event_catalog.events.insert(
            IrNodeId("dead-event".to_string()),
            EventDecl {
                id: IrNodeId("dead-event".to_string()),
                name: "onClick".to_string(),
                kind: EventKind::UserInput,
                source_node: None,
                payload_type: None,
                provenance: Provenance {
                    file: "test.tsx".to_string(),
                    line: 1,
                    column: None,
                    source_name: None,
                    policy_category: None,
                },
            },
        );

        let report = normalize(&mut ir);
        assert_eq!(report.dead_events_pruned, 1);
        assert!(ir.event_catalog.events.is_empty());
    }

    #[test]
    fn event_with_transition_kept() {
        let mut ir = make_ir_with_tree(vec![], vec![]);
        let event_id = IrNodeId("live-event".to_string());
        let state_id = IrNodeId("target-state".to_string());

        ir.event_catalog.events.insert(
            event_id.clone(),
            EventDecl {
                id: event_id.clone(),
                name: "onClick".to_string(),
                kind: EventKind::UserInput,
                source_node: None,
                payload_type: None,
                provenance: Provenance {
                    file: "test.tsx".to_string(),
                    line: 1,
                    column: None,
                    source_name: None,
                    policy_category: None,
                },
            },
        );
        ir.event_catalog.transitions.push(EventTransition {
            event_id: event_id.clone(),
            target_state: state_id.clone(),
            action_snippet: "setState(...)".to_string(),
            guards: Vec::new(),
        });
        ir.state_graph.variables.insert(
            state_id.clone(),
            StateVariable {
                id: state_id,
                name: "count".to_string(),
                scope: StateScope::Local,
                type_annotation: None,
                initial_value: None,
                readers: BTreeSet::new(),
                writers: BTreeSet::from([event_id.clone()]),
                provenance: Provenance {
                    file: "test.tsx".to_string(),
                    line: 1,
                    column: None,
                    source_name: None,
                    policy_category: None,
                },
            },
        );

        let report = normalize(&mut ir);
        assert_eq!(report.dead_events_pruned, 0);
        assert!(ir.event_catalog.events.contains_key(&event_id));
    }

    // ── Token deduplication ─────────────────────────────────────────────

    #[test]
    fn duplicate_tokens_merged() {
        let mut ir = make_ir_with_tree(vec![], vec![]);
        ir.style_intent.tokens.insert(
            "color-primary".to_string(),
            StyleToken {
                name: "color-primary".to_string(),
                category: TokenCategory::Color,
                value: "#1976d2".to_string(),
                provenance: None,
            },
        );
        ir.style_intent.tokens.insert(
            "primary-color".to_string(),
            StyleToken {
                name: "primary-color".to_string(),
                category: TokenCategory::Color,
                value: "#1976d2".to_string(),
                provenance: None,
            },
        );

        let report = normalize(&mut ir);
        assert_eq!(report.tokens_merged, 1);
        assert_eq!(ir.style_intent.tokens.len(), 1);
    }

    #[test]
    fn different_tokens_kept() {
        let mut ir = make_ir_with_tree(vec![], vec![]);
        ir.style_intent.tokens.insert(
            "color-primary".to_string(),
            StyleToken {
                name: "color-primary".to_string(),
                category: TokenCategory::Color,
                value: "#1976d2".to_string(),
                provenance: None,
            },
        );
        ir.style_intent.tokens.insert(
            "color-secondary".to_string(),
            StyleToken {
                name: "color-secondary".to_string(),
                category: TokenCategory::Color,
                value: "#dc004e".to_string(),
                provenance: None,
            },
        );

        let report = normalize(&mut ir);
        assert_eq!(report.tokens_merged, 0);
        assert_eq!(ir.style_intent.tokens.len(), 2);
    }

    // ── Provenance normalization ────────────────────────────────────────

    #[test]
    fn file_path_normalized() {
        assert_eq!(normalize_file_path("./src/App.tsx"), "src/App.tsx");
        assert_eq!(normalize_file_path("src\\App.tsx"), "src/App.tsx");
        assert_eq!(normalize_file_path("  ./src/App.tsx  "), "src/App.tsx");
        assert_eq!(normalize_file_path("src/App.tsx"), "src/App.tsx");
    }

    #[test]
    fn provenance_paths_cleaned() {
        let mut ir = make_ir_with_tree(
            vec![make_view_node("n1", "App", ViewNodeKind::Component, &[])],
            vec!["n1"],
        );
        ir.view_tree
            .nodes
            .get_mut(&IrNodeId("n1".to_string()))
            .unwrap()
            .provenance
            .file = "./src/App.tsx".to_string();

        let report = normalize(&mut ir);
        assert_eq!(report.provenance_normalized, 1);
        assert_eq!(
            ir.view_tree.nodes[&IrNodeId("n1".to_string())]
                .provenance
                .file,
            "src/App.tsx"
        );
    }

    // ── Integration with lowering pipeline ──────────────────────────────

    #[test]
    fn normalize_lowered_ir_is_valid() {
        let project = ProjectParse {
            files: vec![(
                "src/App.tsx".to_string(),
                FileParse {
                    file: "src/App.tsx".to_string(),
                    components: vec![ComponentDecl {
                        name: "App".to_string(),
                        kind: ComponentKind::FunctionComponent,
                        is_default_export: true,
                        is_named_export: false,
                        props_type: None,
                        hooks: vec![HookCall {
                            name: "useState".to_string(),
                            binding: Some("count, setCount".to_string()),
                            args_snippet: "0".to_string(),
                            line: 3,
                        }],
                        event_handlers: Vec::new(),
                        line: 1,
                    }],
                    hooks: Vec::new(),
                    jsx_elements: vec![JsxElement {
                        tag: "div".to_string(),
                        is_component: false,
                        is_fragment: false,
                        is_self_closing: false,
                        props: vec![JsxProp {
                            name: "className".to_string(),
                            is_spread: false,
                            value_snippet: Some("\"app\"".to_string()),
                        }],
                        line: 5,
                    }],
                    types: Vec::new(),
                    symbols: Vec::new(),
                    diagnostics: Vec::new(),
                },
            )]
            .into_iter()
            .collect(),
            file_contents: BTreeMap::new(),
            symbol_table: BTreeMap::new(),
            component_count: 1,
            hook_usage_count: 1,
            type_count: 0,
            diagnostics: Vec::new(),
            external_imports: BTreeSet::new(),
        };

        let result = lower_project(&test_config(), &project);
        let mut ir = result.ir;

        let _report = normalize(&mut ir);
        // Second pass should be clean.
        let report2 = normalize(&mut ir);
        assert!(
            report2.is_clean(),
            "Second normalize not clean: {:?}",
            report2
        );

        let errors = validate_ir(&ir);
        assert!(
            errors.is_empty(),
            "Validation errors after normalize: {:?}",
            errors
        );
    }

    // ── Report helpers ──────────────────────────────────────────────────

    #[test]
    fn report_total_and_clean() {
        let empty = NormalizationReport::default();
        assert_eq!(empty.total(), 0);
        assert!(empty.is_clean());

        let nonempty = NormalizationReport {
            ordering_changes: 2,
            ..Default::default()
        };
        assert_eq!(nonempty.total(), 2);
        assert!(!nonempty.is_clean());
    }
}
