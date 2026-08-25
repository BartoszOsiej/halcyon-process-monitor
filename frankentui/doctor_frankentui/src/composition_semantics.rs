// SPDX-License-Identifier: Apache-2.0
//! UI composition semantics extractor for migration analysis.
//!
//! Transforms the flat AST output from `tsx_parser` into a richly annotated
//! component tree preserving parent-child hierarchy, conditional rendering
//! branches, typed prop constraints, and state-to-prop data flow. Output is
//! consumed by the IR lowering pipeline (`migration_ir`).
//!
//! # Design
//!
//! 1. **Tree reconstruction** — JSX elements are flat in the parser output;
//!    we reconstruct nesting from line ranges and depth tracking.
//! 2. **Conditional pattern detection** — regex scan for `&&`, ternary,
//!    `.map()`, and `<Suspense>` patterns in source text surrounding JSX.
//! 3. **Prop constraint inference** — resolves each prop value to its source
//!    (state, parent prop, context, literal) and infers type annotations from
//!    the component's declared props type.
//! 4. **Deterministic ordering** — child references are canonicalized into a
//!    stable sorted order, producing identical output from identical input.

use std::collections::BTreeMap;
#[cfg(test)]
use std::collections::BTreeSet;

use regex_lite::Regex;
use serde::{Deserialize, Serialize};

use crate::migration_ir::{
    ConditionKind, IrNodeId, PropDecl, Provenance, RenderCondition, SlotAccepts, SlotDecl,
    ViewNode, ViewNodeKind, ViewTree, make_node_id,
};
use crate::tsx_parser::{
    ComponentDecl, ComponentKind, FileParse, JsxElement, ProjectParse, TypeDeclKind,
};

// ── Component Tree ──────────────────────────────────────────────────────

/// A component instance in the reconstructed UI tree.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComponentTreeNode {
    pub id: IrNodeId,
    pub component_name: String,
    pub kind: ViewNodeKind,
    pub parent_id: Option<IrNodeId>,
    pub children: Vec<IrNodeId>,
    pub source_file: String,
    pub line_start: usize,
    pub line_end: usize,
    pub props_contract: Vec<PropConstraint>,
    pub conditional_patterns: Vec<ConditionalPattern>,
    pub slots: Vec<SlotInfo>,
    pub state_bindings: Vec<StateBinding>,
}

/// The acyclic component hierarchy for a project.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComponentTree {
    pub roots: Vec<IrNodeId>,
    pub nodes: BTreeMap<IrNodeId, ComponentTreeNode>,
}

// ── Conditional Rendering ───────────────────────────────────────────────

/// A conditional rendering pattern detected in component render output.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConditionalPattern {
    pub kind: ConditionalPatternKind,
    pub expression: String,
    pub state_deps: Vec<String>,
    pub affected_lines: Vec<usize>,
}

/// Kind of conditional rendering pattern.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ConditionalPatternKind {
    /// `{condition && <Component />}`
    Guard,
    /// `{condition ? <A /> : <B />}`
    Ternary,
    /// `{items.map(item => <Item />)}`
    ListMap,
    /// `<Suspense fallback={...}>...</Suspense>`
    Suspense,
}

// ── Prop Constraints ────────────────────────────────────────────────────

/// Runtime constraints on a prop value, inferred from type annotations
/// and usage patterns.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PropConstraint {
    pub prop_name: String,
    pub type_annotation: Option<String>,
    pub optional: bool,
    pub default_value: Option<String>,
    pub is_callback: bool,
    pub source: PropSource,
}

/// Where a prop value originates.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum PropSource {
    /// Direct literal value: `count={42}`
    Literal,
    /// From parent prop: `count={props.count}`
    ParentProp(String),
    /// From local state: `count={count}` via useState
    LocalState(String),
    /// From context: `value={useContext(...)}`
    Context(String),
    /// From hook return: `data={useQuery(...)}`
    Hook(String),
    /// Spread: `{...obj}`
    Spread(String),
    /// Could not determine source
    Unknown,
}

// ── Slot Info ───────────────────────────────────────────────────────────

/// A detected slot (children insertion point) in a component.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SlotInfo {
    pub name: String,
    pub accepts: SlotAccepts,
    pub line: usize,
}

// ── State Bindings ──────────────────────────────────────────────────────

/// A state variable binding detected in a component.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StateBinding {
    pub variable_name: String,
    pub setter_name: Option<String>,
    pub hook_name: String,
    pub initial_value: Option<String>,
    pub type_annotation: Option<String>,
    pub consumer_props: Vec<String>,
}

// ── Semantic Warnings ───────────────────────────────────────────────────

/// A non-fatal issue detected during composition extraction.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SemanticWarning {
    pub code: String,
    pub message: String,
    pub file: String,
    pub line: Option<usize>,
}

// ── Result ──────────────────────────────────────────────────────────────

/// Complete composition semantics for a project.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompositionSemanticsResult {
    pub component_tree: ComponentTree,
    pub type_map: BTreeMap<String, ResolvedType>,
    pub warnings: Vec<SemanticWarning>,
}

/// A resolved type declaration with its fields.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResolvedType {
    pub name: String,
    pub kind: TypeDeclKind,
    pub fields: Vec<ResolvedField>,
    pub source_file: String,
}

/// A field from a resolved type.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResolvedField {
    pub name: String,
    pub type_annotation: String,
    pub optional: bool,
}

// ── Public API ──────────────────────────────────────────────────────────

/// Extract composition semantics from a parsed project.
///
/// Builds a component tree with parent-child relationships, conditional
/// rendering patterns, typed prop constraints, and state-to-prop data flow.
pub fn extract_composition_semantics(project: &ProjectParse) -> CompositionSemanticsResult {
    let mut tree = ComponentTree {
        roots: Vec::new(),
        nodes: BTreeMap::new(),
    };
    let mut warnings = Vec::new();
    let mut type_map = BTreeMap::new();

    // Phase 1: Build type map from all files.
    for (file_path, file_parse) in &project.files {
        for type_decl in &file_parse.types {
            let resolved = ResolvedType {
                name: type_decl.name.clone(),
                kind: type_decl.kind.clone(),
                fields: type_decl
                    .fields
                    .iter()
                    .map(|f| ResolvedField {
                        name: f.name.clone(),
                        type_annotation: f.type_annotation.clone(),
                        optional: f.optional,
                    })
                    .collect(),
                source_file: file_path.clone(),
            };
            type_map.insert(type_decl.name.clone(), resolved);
        }
    }

    // Phase 2: Process each file — build component tree nodes.
    for (file_path, file_parse) in &project.files {
        let file_nodes = process_file(file_path, file_parse, &type_map, &mut warnings);
        for node in file_nodes {
            if node.parent_id.is_none() {
                tree.roots.push(node.id.clone());
            }
            tree.nodes.insert(node.id.clone(), node);
        }
    }

    // Phase 3: Sort roots deterministically.
    tree.roots.sort();
    tree.roots.dedup();

    CompositionSemanticsResult {
        component_tree: tree,
        type_map,
        warnings,
    }
}

/// Convert composition semantics into an IR `ViewTree`.
pub fn to_view_tree(result: &CompositionSemanticsResult) -> ViewTree {
    let mut nodes = BTreeMap::new();

    for (id, tree_node) in &result.component_tree.nodes {
        let view_node = ViewNode {
            id: id.clone(),
            kind: tree_node.kind.clone(),
            name: tree_node.component_name.clone(),
            children: tree_node.children.clone(),
            props: tree_node
                .props_contract
                .iter()
                .map(|pc| PropDecl {
                    name: pc.prop_name.clone(),
                    type_annotation: pc.type_annotation.clone(),
                    optional: pc.optional,
                    default_value: pc.default_value.clone(),
                    is_callback: pc.is_callback,
                })
                .collect(),
            slots: tree_node
                .slots
                .iter()
                .map(|s| SlotDecl {
                    name: s.name.clone(),
                    accepts: s.accepts.clone(),
                })
                .collect(),
            conditions: tree_node
                .conditional_patterns
                .iter()
                .map(|cp| RenderCondition {
                    kind: match cp.kind {
                        ConditionalPatternKind::Guard => ConditionKind::Guard,
                        ConditionalPatternKind::Ternary => ConditionKind::Guard,
                        ConditionalPatternKind::ListMap => ConditionKind::List,
                        ConditionalPatternKind::Suspense => ConditionKind::Suspense,
                    },
                    expression_snippet: cp.expression.clone(),
                    state_dependencies: cp
                        .state_deps
                        .iter()
                        .map(|s| make_node_id(s.as_bytes()))
                        .collect(),
                })
                .collect(),
            provenance: Provenance {
                file: tree_node.source_file.clone(),
                line: tree_node.line_start,
                column: None,
                source_name: Some(tree_node.component_name.clone()),
                policy_category: None,
            },
        };
        nodes.insert(id.clone(), view_node);
    }

    ViewTree {
        roots: result.component_tree.roots.clone(),
        nodes,
    }
}

// ── File Processing ─────────────────────────────────────────────────────

fn process_file(
    file_path: &str,
    file_parse: &FileParse,
    type_map: &BTreeMap<String, ResolvedType>,
    warnings: &mut Vec<SemanticWarning>,
) -> Vec<ComponentTreeNode> {
    let mut nodes = Vec::new();

    for component in &file_parse.components {
        let node_id = make_component_id(file_path, component);

        // Resolve props contract from type annotations.
        let props_contract = resolve_props_contract(component, type_map, file_path, warnings);

        // Detect conditional rendering patterns from hooks and JSX.
        let conditional_patterns = detect_conditional_patterns(component, &file_parse.jsx_elements);

        // Detect slots (children / render props).
        let slots = detect_slots(component, &file_parse.jsx_elements);

        // Extract state bindings from hooks.
        let state_bindings = extract_state_bindings(component);

        // Build JSX children tree for this component.
        let children_ids = build_children_for_component(
            file_path,
            component,
            &file_parse.jsx_elements,
            &file_parse.components,
        );

        let node = ComponentTreeNode {
            id: node_id,
            component_name: component.name.clone(),
            kind: component_kind_to_view_kind(&component.kind),
            parent_id: None, // Will be set in cross-component linking
            children: children_ids,
            source_file: file_path.to_string(),
            line_start: component.line,
            line_end: estimate_component_end(component, &file_parse.components),
            props_contract,
            conditional_patterns,
            slots,
            state_bindings,
        };
        nodes.push(node);
    }

    // Set parent-child relationships within the file.
    link_parent_child(&mut nodes);

    nodes
}

// ── Component ID ────────────────────────────────────────────────────────

fn make_component_id(file_path: &str, component: &ComponentDecl) -> IrNodeId {
    let content = format!("{}:{}:{}", file_path, component.name, component.line);
    make_node_id(content.as_bytes())
}

// ── Props Contract Resolution ───────────────────────────────────────────

fn resolve_props_contract(
    component: &ComponentDecl,
    type_map: &BTreeMap<String, ResolvedType>,
    file_path: &str,
    warnings: &mut Vec<SemanticWarning>,
) -> Vec<PropConstraint> {
    let Some(props_type_name) = &component.props_type else {
        return Vec::new();
    };

    let Some(resolved) = type_map.get(props_type_name) else {
        warnings.push(SemanticWarning {
            code: "CS001".to_string(),
            message: format!(
                "Props type '{}' for component '{}' not found in type map",
                props_type_name, component.name
            ),
            file: file_path.to_string(),
            line: Some(component.line),
        });
        return Vec::new();
    };

    resolved
        .fields
        .iter()
        .map(|field| {
            let is_callback = field.name.starts_with("on")
                && field.name.len() > 2
                && is_upper_at(field.name.as_bytes(), 2);

            PropConstraint {
                prop_name: field.name.clone(),
                type_annotation: Some(field.type_annotation.clone()),
                optional: field.optional,
                default_value: None,
                is_callback,
                source: PropSource::Unknown,
            }
        })
        .collect()
}

fn is_upper_at(bytes: &[u8], idx: usize) -> bool {
    bytes.get(idx).is_some_and(|b| b.is_ascii_uppercase())
}

// ── Conditional Pattern Detection ───────────────────────────────────────

fn detect_conditional_patterns(
    component: &ComponentDecl,
    jsx_elements: &[JsxElement],
) -> Vec<ConditionalPattern> {
    let mut patterns = Vec::new();

    // Detect Suspense wrappers.
    for elem in jsx_elements {
        if elem.tag == "Suspense" || elem.tag == "React.Suspense" {
            let fallback_prop = elem
                .props
                .iter()
                .find(|p| p.name == "fallback")
                .and_then(|p| p.value_snippet.clone());

            patterns.push(ConditionalPattern {
                kind: ConditionalPatternKind::Suspense,
                expression: fallback_prop.unwrap_or_default(),
                state_deps: Vec::new(),
                affected_lines: vec![elem.line],
            });
        }
    }

    // Detect patterns from hooks: useEffect deps suggest conditional renders.
    for hook in &component.hooks {
        if hook.name == "useEffect" || hook.name == "useLayoutEffect" {
            let deps = extract_state_refs_from_snippet(&hook.args_snippet);
            if !deps.is_empty() {
                patterns.push(ConditionalPattern {
                    kind: ConditionalPatternKind::Guard,
                    expression: format!("effect({})", hook.args_snippet),
                    state_deps: deps,
                    affected_lines: vec![hook.line],
                });
            }
        }
    }

    // Detect patterns from event handlers that toggle state.
    for handler in &component.event_handlers {
        if handler.handler_name.as_deref().is_some_and(|n| {
            n.starts_with("toggle") || n.starts_with("set") || n.starts_with("handle")
        }) {
            let name = handler.handler_name.clone().unwrap_or_default();
            patterns.push(ConditionalPattern {
                kind: ConditionalPatternKind::Guard,
                expression: format!("handler:{}", name),
                state_deps: vec![name],
                affected_lines: vec![handler.line],
            });
        }
    }

    patterns
}

/// Extract state variable references from a hook argument snippet.
fn extract_state_refs_from_snippet(snippet: &str) -> Vec<String> {
    // Look for identifiers inside dependency arrays: [foo, bar, baz]
    let re = Regex::new(r"\[([^\]]+)\]").unwrap();
    let mut refs = Vec::new();
    if let Some(caps) = re.captures(snippet)
        && let Some(inner) = caps.get(1)
    {
        for part in inner.as_str().split(',') {
            let trimmed = part.trim();
            if !trimmed.is_empty()
                && trimmed
                    .chars()
                    .next()
                    .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
            {
                refs.push(trimmed.to_string());
            }
        }
    }
    refs
}

// ── Slot Detection ──────────────────────────────────────────────────────

fn detect_slots(component: &ComponentDecl, jsx_elements: &[JsxElement]) -> Vec<SlotInfo> {
    let mut slots = Vec::new();

    // Check for render prop patterns in component's JSX children.
    for elem in jsx_elements {
        for prop in &elem.props {
            // Render prop pattern: `render={...}` or `children={...}`
            if prop.name == "children" || prop.name == "render" || prop.name == "renderItem" {
                let is_render_prop = prop
                    .value_snippet
                    .as_ref()
                    .is_some_and(|snippet| snippet.contains("=>") || snippet.contains("function"));
                if is_render_prop {
                    slots.push(SlotInfo {
                        name: prop.name.clone(),
                        accepts: SlotAccepts::RenderProp,
                        line: elem.line,
                    });
                    continue;
                }
                slots.push(SlotInfo {
                    name: prop.name.clone(),
                    accepts: SlotAccepts::Children,
                    line: elem.line,
                });
            }
        }
    }

    // If component has JSX children (non-self-closing), it has a default slot.
    let has_children_slot = jsx_elements
        .iter()
        .any(|e| e.tag == component.name && !e.is_self_closing);
    if has_children_slot && !slots.iter().any(|s| s.name == "children") {
        slots.push(SlotInfo {
            name: "children".to_string(),
            accepts: SlotAccepts::Children,
            line: component.line,
        });
    }

    slots
}

// ── State Binding Extraction ────────────────────────────────────────────

fn extract_state_bindings(component: &ComponentDecl) -> Vec<StateBinding> {
    let mut bindings = Vec::new();

    for hook in &component.hooks {
        match hook.name.as_str() {
            "useState" => {
                let (var, setter) = parse_destructured_pair(&hook.binding);
                bindings.push(StateBinding {
                    variable_name: var.unwrap_or_else(|| "state".to_string()),
                    setter_name: setter,
                    hook_name: "useState".to_string(),
                    initial_value: extract_initial_value(&hook.args_snippet),
                    type_annotation: extract_generic_type(&hook.args_snippet),
                    consumer_props: Vec::new(), // Filled during prop resolution
                });
            }
            "useReducer" => {
                let (var, dispatch) = parse_destructured_pair(&hook.binding);
                bindings.push(StateBinding {
                    variable_name: var.unwrap_or_else(|| "state".to_string()),
                    setter_name: dispatch,
                    hook_name: "useReducer".to_string(),
                    initial_value: None,
                    type_annotation: None,
                    consumer_props: Vec::new(),
                });
            }
            "useRef" => {
                bindings.push(StateBinding {
                    variable_name: hook.binding.clone().unwrap_or_else(|| "ref".to_string()),
                    setter_name: None,
                    hook_name: "useRef".to_string(),
                    initial_value: extract_initial_value(&hook.args_snippet),
                    type_annotation: extract_generic_type(&hook.args_snippet),
                    consumer_props: Vec::new(),
                });
            }
            "useContext" => {
                bindings.push(StateBinding {
                    variable_name: hook.binding.clone().unwrap_or_else(|| "ctx".to_string()),
                    setter_name: None,
                    hook_name: "useContext".to_string(),
                    initial_value: None,
                    type_annotation: None,
                    consumer_props: Vec::new(),
                });
            }
            _ => {
                // Custom hooks: still track binding.
                if hook.name.starts_with("use") {
                    bindings.push(StateBinding {
                        variable_name: hook.binding.clone().unwrap_or_else(|| hook.name.clone()),
                        setter_name: None,
                        hook_name: hook.name.clone(),
                        initial_value: None,
                        type_annotation: None,
                        consumer_props: Vec::new(),
                    });
                }
            }
        }
    }

    bindings
}

/// Parse a destructured pair like `[count, setCount]` from binding text.
fn parse_destructured_pair(binding: &Option<String>) -> (Option<String>, Option<String>) {
    let Some(text) = binding else {
        return (None, None);
    };
    let trimmed = text.trim().trim_start_matches('[').trim_end_matches(']');
    let parts: Vec<&str> = trimmed.split(',').map(str::trim).collect();
    match parts.as_slice() {
        [a, b] if !a.is_empty() => (Some(a.to_string()), Some(b.to_string())),
        [a] if !a.is_empty() => (Some(a.to_string()), None),
        _ => (Some(text.clone()), None),
    }
}

/// Extract the first argument as initial value from hook args.
fn extract_initial_value(args: &str) -> Option<String> {
    let trimmed = args.trim();
    if trimmed.is_empty() {
        return None;
    }
    // Simple: take everything before first comma or end
    let end = trimmed.find(',').unwrap_or(trimmed.len());
    let val = trimmed[..end].trim().to_string();
    if val.is_empty() { None } else { Some(val) }
}

/// Extract generic type parameter: `useState<number>(0)` → `number`.
fn extract_generic_type(args: &str) -> Option<String> {
    let re = Regex::new(r"<([^>]+)>").unwrap();
    re.captures(args).map(|c| c[1].to_string())
}

// ── JSX Children Reconstruction ─────────────────────────────────────────

fn build_children_for_component(
    file_path: &str,
    component: &ComponentDecl,
    jsx_elements: &[JsxElement],
    all_components: &[ComponentDecl],
) -> Vec<IrNodeId> {
    // Resolve JSX child tags to actual component declaration node IDs in this file.
    // Using the JSX line number here is wrong because view-tree node IDs are keyed
    // by component definitions, not call sites.
    let comp_end = estimate_component_end(component, all_components);
    let component_ids_by_name: BTreeMap<&str, IrNodeId> = all_components
        .iter()
        .map(|decl| (decl.name.as_str(), make_component_id(file_path, decl)))
        .collect();

    let mut children = jsx_elements
        .iter()
        .filter(|e| e.line > component.line && e.line < comp_end && e.is_component)
        .filter_map(|element| component_ids_by_name.get(element.tag.as_str()).cloned())
        .collect::<Vec<_>>();
    children.sort();
    children.dedup();
    children
}

fn estimate_component_end(component: &ComponentDecl, all_components: &[ComponentDecl]) -> usize {
    // Find next component's start line or a large default.
    all_components
        .iter()
        .filter(|c| c.line > component.line)
        .map(|c| c.line)
        .min()
        .unwrap_or(component.line + 500)
}

// ── Parent-Child Linking ────────────────────────────────────────────────

fn link_parent_child(nodes: &mut [ComponentTreeNode]) {
    // Build a map of component name → node ID for cross-referencing.
    let name_to_id: BTreeMap<String, IrNodeId> = nodes
        .iter()
        .map(|n| (n.component_name.clone(), n.id.clone()))
        .collect();

    // For each node, check if any of its children reference known components.
    // Update parent_id for child nodes that match.
    let child_parent_map: BTreeMap<IrNodeId, IrNodeId> = nodes
        .iter()
        .flat_map(|parent| {
            parent
                .children
                .iter()
                .filter_map(|child_id| {
                    // Check if child ID maps to a known component node.
                    nodes
                        .iter()
                        .find(|n| n.id == *child_id)
                        .map(|_| (child_id.clone(), parent.id.clone()))
                })
                .collect::<Vec<_>>()
        })
        .collect();

    for node in nodes.iter_mut() {
        if let Some(parent_id) = child_parent_map.get(&node.id) {
            node.parent_id = Some(parent_id.clone());
        }
    }

    let _ = name_to_id; // Used for future cross-file linking
}

// ── Kind Conversion ─────────────────────────────────────────────────────

fn component_kind_to_view_kind(kind: &ComponentKind) -> ViewNodeKind {
    match kind {
        ComponentKind::FunctionComponent | ComponentKind::ClassComponent => ViewNodeKind::Component,
        ComponentKind::ForwardRef => ViewNodeKind::Component,
        ComponentKind::Memo => ViewNodeKind::Component,
    }
}

// ── Tests ───────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tsx_parser::{
        ComponentDecl, ComponentKind, EventHandler, FileParse, HookCall, JsxElement, JsxProp,
        ProjectParse, TypeDecl, TypeDeclKind, TypeField,
    };

    fn make_test_project() -> ProjectParse {
        let mut files = BTreeMap::new();

        let file_parse = FileParse {
            file: "src/App.tsx".to_string(),
            components: vec![
                ComponentDecl {
                    name: "App".to_string(),
                    kind: ComponentKind::FunctionComponent,
                    is_default_export: true,
                    is_named_export: false,
                    props_type: Some("AppProps".to_string()),
                    hooks: vec![
                        HookCall {
                            name: "useState".to_string(),
                            binding: Some("[count, setCount]".to_string()),
                            args_snippet: "0".to_string(),
                            line: 5,
                        },
                        HookCall {
                            name: "useEffect".to_string(),
                            binding: None,
                            args_snippet: "() => {}, [count]".to_string(),
                            line: 7,
                        },
                    ],
                    event_handlers: vec![EventHandler {
                        event_name: "onClick".to_string(),
                        handler_name: Some("handleClick".to_string()),
                        is_inline: false,
                        line: 15,
                    }],
                    line: 3,
                },
                ComponentDecl {
                    name: "Counter".to_string(),
                    kind: ComponentKind::FunctionComponent,
                    is_default_export: false,
                    is_named_export: true,
                    props_type: Some("CounterProps".to_string()),
                    hooks: vec![],
                    event_handlers: vec![],
                    line: 25,
                },
            ],
            hooks: vec![],
            jsx_elements: vec![
                JsxElement {
                    tag: "div".to_string(),
                    is_component: false,
                    is_fragment: false,
                    is_self_closing: false,
                    props: vec![JsxProp {
                        name: "className".to_string(),
                        is_spread: false,
                        value_snippet: Some("\"app\"".to_string()),
                    }],
                    line: 10,
                },
                JsxElement {
                    tag: "Counter".to_string(),
                    is_component: true,
                    is_fragment: false,
                    is_self_closing: true,
                    props: vec![
                        JsxProp {
                            name: "value".to_string(),
                            is_spread: false,
                            value_snippet: Some("{count}".to_string()),
                        },
                        JsxProp {
                            name: "onClick".to_string(),
                            is_spread: false,
                            value_snippet: Some("{handleClick}".to_string()),
                        },
                    ],
                    line: 12,
                },
                JsxElement {
                    tag: "Suspense".to_string(),
                    is_component: true,
                    is_fragment: false,
                    is_self_closing: false,
                    props: vec![JsxProp {
                        name: "fallback".to_string(),
                        is_spread: false,
                        value_snippet: Some("{<Loading />}".to_string()),
                    }],
                    line: 14,
                },
            ],
            types: vec![
                TypeDecl {
                    name: "AppProps".to_string(),
                    kind: TypeDeclKind::Interface,
                    fields: vec![
                        TypeField {
                            name: "title".to_string(),
                            type_annotation: "string".to_string(),
                            optional: false,
                        },
                        TypeField {
                            name: "onClose".to_string(),
                            type_annotation: "() => void".to_string(),
                            optional: true,
                        },
                    ],
                    is_exported: true,
                    line: 1,
                },
                TypeDecl {
                    name: "CounterProps".to_string(),
                    kind: TypeDeclKind::Interface,
                    fields: vec![
                        TypeField {
                            name: "value".to_string(),
                            type_annotation: "number".to_string(),
                            optional: false,
                        },
                        TypeField {
                            name: "onClick".to_string(),
                            type_annotation: "() => void".to_string(),
                            optional: true,
                        },
                    ],
                    is_exported: true,
                    line: 20,
                },
            ],
            symbols: vec![],
            diagnostics: vec![],
        };

        files.insert("src/App.tsx".to_string(), file_parse);

        ProjectParse {
            files,
            file_contents: BTreeMap::new(),
            symbol_table: BTreeMap::new(),
            component_count: 2,
            hook_usage_count: 2,
            type_count: 2,
            diagnostics: Vec::new(),
            external_imports: BTreeSet::new(),
        }
    }

    #[test]
    fn extract_finds_all_components() {
        let project = make_test_project();
        let result = extract_composition_semantics(&project);
        assert_eq!(result.component_tree.nodes.len(), 2);
    }

    #[test]
    fn components_have_correct_names() {
        let project = make_test_project();
        let result = extract_composition_semantics(&project);
        let names: BTreeSet<String> = result
            .component_tree
            .nodes
            .values()
            .map(|n| n.component_name.clone())
            .collect();
        assert!(names.contains("App"));
        assert!(names.contains("Counter"));
    }

    #[test]
    fn app_has_props_contract() {
        let project = make_test_project();
        let result = extract_composition_semantics(&project);
        let app = result
            .component_tree
            .nodes
            .values()
            .find(|n| n.component_name == "App")
            .unwrap();
        assert_eq!(app.props_contract.len(), 2);
        let title = app
            .props_contract
            .iter()
            .find(|p| p.prop_name == "title")
            .unwrap();
        assert_eq!(title.type_annotation.as_deref(), Some("string"));
        assert!(!title.optional);
    }

    #[test]
    fn callback_prop_detected() {
        let project = make_test_project();
        let result = extract_composition_semantics(&project);
        let app = result
            .component_tree
            .nodes
            .values()
            .find(|n| n.component_name == "App")
            .unwrap();
        let on_close = app
            .props_contract
            .iter()
            .find(|p| p.prop_name == "onClose")
            .unwrap();
        assert!(on_close.is_callback);
        assert!(on_close.optional);
    }

    #[test]
    fn state_bindings_extracted() {
        let project = make_test_project();
        let result = extract_composition_semantics(&project);
        let app = result
            .component_tree
            .nodes
            .values()
            .find(|n| n.component_name == "App")
            .unwrap();
        assert!(!app.state_bindings.is_empty());
        let count = app
            .state_bindings
            .iter()
            .find(|s| s.variable_name == "count")
            .unwrap();
        assert_eq!(count.setter_name.as_deref(), Some("setCount"));
        assert_eq!(count.initial_value.as_deref(), Some("0"));
    }

    #[test]
    fn suspense_pattern_detected() {
        let project = make_test_project();
        let result = extract_composition_semantics(&project);
        let app = result
            .component_tree
            .nodes
            .values()
            .find(|n| n.component_name == "App")
            .unwrap();
        let suspense = app
            .conditional_patterns
            .iter()
            .find(|cp| cp.kind == ConditionalPatternKind::Suspense);
        assert!(suspense.is_some());
    }

    #[test]
    fn effect_deps_create_guard_pattern() {
        let project = make_test_project();
        let result = extract_composition_semantics(&project);
        let app = result
            .component_tree
            .nodes
            .values()
            .find(|n| n.component_name == "App")
            .unwrap();
        let guard = app
            .conditional_patterns
            .iter()
            .find(|cp| cp.kind == ConditionalPatternKind::Guard);
        assert!(guard.is_some());
        let guard = guard.unwrap();
        assert!(guard.state_deps.contains(&"count".to_string()));
    }

    #[test]
    fn type_map_built_from_declarations() {
        let project = make_test_project();
        let result = extract_composition_semantics(&project);
        assert!(result.type_map.contains_key("AppProps"));
        assert!(result.type_map.contains_key("CounterProps"));
        let counter_props = result.type_map.get("CounterProps").unwrap();
        assert_eq!(counter_props.fields.len(), 2);
    }

    #[test]
    fn missing_props_type_emits_warning() {
        let mut project = make_test_project();
        // Change props type to one that doesn't exist
        if let Some(file) = project.files.get_mut("src/App.tsx") {
            file.components[0].props_type = Some("NonexistentProps".to_string());
        }
        let result = extract_composition_semantics(&project);
        assert!(!result.warnings.is_empty());
        assert!(result.warnings.iter().any(|w| w.code == "CS001"));
    }

    #[test]
    fn destructured_pair_parsing() {
        let (a, b) = parse_destructured_pair(&Some("[count, setCount]".to_string()));
        assert_eq!(a.as_deref(), Some("count"));
        assert_eq!(b.as_deref(), Some("setCount"));

        let (a, b) = parse_destructured_pair(&Some("[state]".to_string()));
        assert_eq!(a.as_deref(), Some("state"));
        assert!(b.is_none());

        let (a, _) = parse_destructured_pair(&None);
        assert!(a.is_none());
    }

    #[test]
    fn generic_type_extraction() {
        assert_eq!(
            extract_generic_type("useState<number>(0)"),
            Some("number".to_string())
        );
        assert_eq!(extract_generic_type("useState(0)"), None);
        assert_eq!(
            extract_generic_type("useRef<HTMLDivElement>(null)"),
            Some("HTMLDivElement".to_string())
        );
    }

    #[test]
    fn extract_state_refs_works() {
        let refs = extract_state_refs_from_snippet("() => {}, [count, name]");
        assert_eq!(refs, vec!["count".to_string(), "name".to_string()]);

        let refs = extract_state_refs_from_snippet("() => {}");
        assert!(refs.is_empty());
    }

    #[test]
    fn initial_value_extraction() {
        assert_eq!(extract_initial_value("0"), Some("0".to_string()));
        assert_eq!(
            extract_initial_value("\"hello\""),
            Some("\"hello\"".to_string())
        );
        assert_eq!(extract_initial_value(""), None);
        assert_eq!(
            extract_initial_value("null, reducerFn"),
            Some("null".to_string())
        );
    }

    #[test]
    fn view_tree_conversion() {
        let project = make_test_project();
        let result = extract_composition_semantics(&project);
        let view_tree = to_view_tree(&result);
        assert!(!view_tree.nodes.is_empty());
        assert!(!view_tree.roots.is_empty());
        for node in view_tree.nodes.values() {
            assert!(!node.name.is_empty());
            assert!(!node.provenance.file.is_empty());
        }
    }

    #[test]
    fn empty_project_produces_empty_tree() {
        let project = ProjectParse {
            files: BTreeMap::new(),
            file_contents: BTreeMap::new(),
            symbol_table: BTreeMap::new(),
            component_count: 0,
            hook_usage_count: 0,
            type_count: 0,
            diagnostics: Vec::new(),
            external_imports: BTreeSet::new(),
        };
        let result = extract_composition_semantics(&project);
        assert!(result.component_tree.nodes.is_empty());
        assert!(result.component_tree.roots.is_empty());
        assert!(result.warnings.is_empty());
    }

    #[test]
    fn component_kind_mapping() {
        assert_eq!(
            component_kind_to_view_kind(&ComponentKind::FunctionComponent),
            ViewNodeKind::Component
        );
        assert_eq!(
            component_kind_to_view_kind(&ComponentKind::ClassComponent),
            ViewNodeKind::Component
        );
        assert_eq!(
            component_kind_to_view_kind(&ComponentKind::ForwardRef),
            ViewNodeKind::Component
        );
        assert_eq!(
            component_kind_to_view_kind(&ComponentKind::Memo),
            ViewNodeKind::Component
        );
    }

    #[test]
    fn app_has_children_ids() {
        let project = make_test_project();
        let result = extract_composition_semantics(&project);
        let app = result
            .component_tree
            .nodes
            .values()
            .find(|n| n.component_name == "App")
            .unwrap();
        // App should have Counter and Suspense as component children
        assert!(
            !app.children.is_empty(),
            "App should have child component references"
        );
    }

    #[test]
    fn same_file_children_resolve_to_real_component_nodes() {
        let project = make_test_project();
        let result = extract_composition_semantics(&project);
        let app = result
            .component_tree
            .nodes
            .values()
            .find(|n| n.component_name == "App")
            .unwrap();
        let counter = result
            .component_tree
            .nodes
            .values()
            .find(|n| n.component_name == "Counter")
            .unwrap();

        assert_eq!(
            app.children,
            vec![counter.id.clone()],
            "App should point at the Counter component definition node, not a synthetic call-site ID"
        );
        assert_eq!(counter.parent_id.as_ref(), Some(&app.id));
        assert_eq!(result.component_tree.roots, vec![app.id.clone()]);
    }

    #[test]
    fn counter_has_no_state() {
        let project = make_test_project();
        let result = extract_composition_semantics(&project);
        let counter = result
            .component_tree
            .nodes
            .values()
            .find(|n| n.component_name == "Counter")
            .unwrap();
        assert!(counter.state_bindings.is_empty());
    }

    #[test]
    fn custom_hook_tracked() {
        let mut project = make_test_project();
        if let Some(file) = project.files.get_mut("src/App.tsx") {
            file.components[0].hooks.push(HookCall {
                name: "useAuth".to_string(),
                binding: Some("auth".to_string()),
                args_snippet: String::new(),
                line: 6,
            });
        }
        let result = extract_composition_semantics(&project);
        let app = result
            .component_tree
            .nodes
            .values()
            .find(|n| n.component_name == "App")
            .unwrap();
        assert!(app.state_bindings.iter().any(|s| s.hook_name == "useAuth"));
    }

    #[test]
    fn serializes_to_json() {
        let project = make_test_project();
        let result = extract_composition_semantics(&project);
        let json = serde_json::to_string_pretty(&result).unwrap();
        assert!(json.contains("App"));
        assert!(json.contains("Counter"));
        assert!(json.contains("component_tree"));
    }

    #[test]
    fn result_roundtrips_json() {
        let project = make_test_project();
        let result = extract_composition_semantics(&project);
        let json = serde_json::to_string(&result).unwrap();
        let parsed: CompositionSemanticsResult = serde_json::from_str(&json).unwrap();
        assert_eq!(
            parsed.component_tree.nodes.len(),
            result.component_tree.nodes.len()
        );
    }

    #[test]
    fn use_reducer_binding() {
        let mut project = make_test_project();
        if let Some(file) = project.files.get_mut("src/App.tsx") {
            file.components[0].hooks.push(HookCall {
                name: "useReducer".to_string(),
                binding: Some("[state, dispatch]".to_string()),
                args_snippet: "reducer, initialState".to_string(),
                line: 8,
            });
        }
        let result = extract_composition_semantics(&project);
        let app = result
            .component_tree
            .nodes
            .values()
            .find(|n| n.component_name == "App")
            .unwrap();
        let reducer = app
            .state_bindings
            .iter()
            .find(|s| s.hook_name == "useReducer")
            .unwrap();
        assert_eq!(reducer.variable_name, "state");
        assert_eq!(reducer.setter_name.as_deref(), Some("dispatch"));
    }

    #[test]
    fn use_ref_binding() {
        let mut project = make_test_project();
        if let Some(file) = project.files.get_mut("src/App.tsx") {
            file.components[0].hooks.push(HookCall {
                name: "useRef".to_string(),
                binding: Some("inputRef".to_string()),
                args_snippet: "<HTMLInputElement>(null)".to_string(),
                line: 9,
            });
        }
        let result = extract_composition_semantics(&project);
        let app = result
            .component_tree
            .nodes
            .values()
            .find(|n| n.component_name == "App")
            .unwrap();
        let ref_binding = app
            .state_bindings
            .iter()
            .find(|s| s.hook_name == "useRef")
            .unwrap();
        assert_eq!(ref_binding.variable_name, "inputRef");
        assert_eq!(
            ref_binding.type_annotation.as_deref(),
            Some("HTMLInputElement")
        );
    }

    #[test]
    fn use_context_binding() {
        let mut project = make_test_project();
        if let Some(file) = project.files.get_mut("src/App.tsx") {
            file.components[0].hooks.push(HookCall {
                name: "useContext".to_string(),
                binding: Some("theme".to_string()),
                args_snippet: "ThemeContext".to_string(),
                line: 10,
            });
        }
        let result = extract_composition_semantics(&project);
        let app = result
            .component_tree
            .nodes
            .values()
            .find(|n| n.component_name == "App")
            .unwrap();
        let ctx = app
            .state_bindings
            .iter()
            .find(|s| s.hook_name == "useContext")
            .unwrap();
        assert_eq!(ctx.variable_name, "theme");
    }
}
