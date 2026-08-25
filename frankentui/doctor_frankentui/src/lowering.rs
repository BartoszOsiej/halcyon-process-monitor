//! Lowering pipeline: extracted semantic facts → canonical [`MigrationIr`].
//!
//! Consumes the output of:
//! - [`composition_semantics::extract_composition_semantics`] → view tree
//! - [`style_semantics::extract_style_semantics`] → style intent + design tokens
//! - [`state_effects::build_project_state_model`] → state graph, effects, capabilities
//!
//! Produces a fully-validated [`MigrationIr`] via [`IrBuilder`] with:
//! - Deterministic symbol resolution and stable node identities
//! - Complete provenance back to source locations
//! - Structured diagnostics for unresolved or partial constructs

use std::collections::{BTreeMap, BTreeSet};

use crate::composition_semantics::{self, CompositionSemanticsResult};
use crate::migration_ir::{
    self, AccessibilityEntry, DerivedState, EffectDecl, EffectKind, EventDecl, EventKind,
    EventTransition, IrBuilder, IrNodeId, IrWarning, MigrationIr, Provenance, StateScope,
    StateVariable,
};
use crate::state_effects::{
    self, EffectClassification, EventStateTransition, ProjectStateModel, StateVarScope,
};
use crate::style_semantics::{self, StyleSemanticsResult, StyleWarningKind};
use crate::tsx_parser::ProjectParse;

// ── Public API ──────────────────────────────────────────────────────────

/// Configuration for the lowering pipeline.
#[derive(Debug, Clone)]
pub struct LoweringConfig {
    /// Identifier for this lowering run.
    pub run_id: String,
    /// Source project name or path.
    pub source_project: String,
}

/// Result of the lowering pipeline.
#[derive(Debug)]
pub struct LoweringResult {
    /// The canonical IR.
    pub ir: MigrationIr,
    /// Diagnostics emitted during lowering (non-fatal).
    pub diagnostics: Vec<LoweringDiagnostic>,
}

/// A structured diagnostic from the lowering process.
#[derive(Debug, Clone)]
pub struct LoweringDiagnostic {
    pub code: String,
    pub severity: DiagnosticSeverity,
    pub message: String,
    pub provenance: Option<Provenance>,
}

/// Severity of a lowering diagnostic.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DiagnosticSeverity {
    Warning,
    Info,
}

/// Lower all extracted semantic facts into a canonical `MigrationIr`.
///
/// This is the main entry point for the lowering pipeline.
pub fn lower_to_ir(
    config: &LoweringConfig,
    project: &ProjectParse,
    composition: &CompositionSemanticsResult,
    styles: &StyleSemanticsResult,
    state_model: &ProjectStateModel,
) -> LoweringResult {
    let mut builder = IrBuilder::new(config.run_id.clone(), config.source_project.clone());
    let mut diagnostics = Vec::new();

    builder.set_source_file_count(project.files.len());

    // Phase 1: View tree (composition semantics → ViewTree)
    lower_view_tree(&mut builder, composition, &mut diagnostics);

    // Phase 2: State graph (state effects → StateGraph)
    let state_id_map = lower_state_graph(&mut builder, state_model, &mut diagnostics);

    // Phase 3: Events and transitions
    lower_events(&mut builder, state_model, &state_id_map, &mut diagnostics);

    // Phase 4: Effects
    lower_effects(&mut builder, state_model, &state_id_map, &mut diagnostics);

    // Phase 5: Style intent (style semantics → StyleIntent)
    lower_style_intent(&mut builder, styles, &mut diagnostics);

    // Phase 6: Capabilities
    lower_capabilities(&mut builder, state_model);

    // Phase 7: Accessibility (from style + composition hints)
    lower_accessibility(&mut builder, styles, composition, &mut diagnostics);

    // Phase 8: Propagate warnings from extraction layers
    propagate_extraction_warnings(&mut builder, composition, styles, &mut diagnostics);

    let ir = builder.build();

    LoweringResult { ir, diagnostics }
}

/// Lower from a `ProjectParse` by running all extraction phases first.
///
/// Convenience function that chains extraction → lowering.
pub fn lower_project(config: &LoweringConfig, project: &ProjectParse) -> LoweringResult {
    let composition = composition_semantics::extract_composition_semantics(project);
    let styles = style_semantics::extract_style_semantics(project);
    let state_model =
        state_effects::build_project_state_model(&project.files, &project.file_contents);

    lower_to_ir(config, project, &composition, &styles, &state_model)
}

// ── Phase 1: View Tree ──────────────────────────────────────────────────

fn lower_view_tree(
    builder: &mut IrBuilder,
    composition: &CompositionSemanticsResult,
    diagnostics: &mut Vec<LoweringDiagnostic>,
) {
    let view_tree = composition_semantics::to_view_tree(composition);

    for root_id in &view_tree.roots {
        builder.add_root(root_id.clone());
    }

    for (id, node) in &view_tree.nodes {
        builder.add_view_node(node.clone());

        // Emit diagnostic for nodes with no provenance file info.
        if node.provenance.file.is_empty() {
            diagnostics.push(LoweringDiagnostic {
                code: "L001".to_string(),
                severity: DiagnosticSeverity::Warning,
                message: format!(
                    "View node '{name}' ({id}) has empty provenance file",
                    name = node.name
                ),
                provenance: Some(node.provenance.clone()),
            });
        }
    }

    if view_tree.roots.is_empty() && !composition.component_tree.nodes.is_empty() {
        diagnostics.push(LoweringDiagnostic {
            code: "L002".to_string(),
            severity: DiagnosticSeverity::Info,
            message: format!(
                "View tree has {} nodes but no roots — all components may be non-root",
                composition.component_tree.nodes.len()
            ),
            provenance: None,
        });
    }
}

// ── Phase 2: State Graph ────────────────────────────────────────────────

/// Maps `(component_name, variable_name)` → `IrNodeId` for cross-reference.
type StateIdMap = BTreeMap<(String, String), IrNodeId>;

fn lower_state_graph(
    builder: &mut IrBuilder,
    state_model: &ProjectStateModel,
    diagnostics: &mut Vec<LoweringDiagnostic>,
) -> StateIdMap {
    let mut state_id_map = StateIdMap::new();
    let mut context_provider_ids: BTreeMap<(String, String), Vec<IrNodeId>> = BTreeMap::new();
    let mut context_consumer_ids: BTreeMap<(String, String), Vec<IrNodeId>> = BTreeMap::new();

    for (comp_name, comp_model) in &state_model.components {
        // State variables
        for (state_idx, var) in comp_model.state_vars.iter().enumerate() {
            let id_content = format!(
                "state:{}:{}:{}:{}:{}",
                comp_model.file, comp_name, var.line, var.name, state_idx
            );
            let id = migration_ir::make_node_id(id_content.as_bytes());

            state_id_map.insert((comp_name.clone(), var.name.clone()), id.clone());

            // Also map setter name for lookup from event transitions.
            if let Some(setter) = &var.setter {
                state_id_map.insert((comp_name.clone(), setter.clone()), id.clone());
            }

            let scope = map_state_scope(&var.scope);

            builder.add_state_variable(StateVariable {
                id: id.clone(),
                name: var.name.clone(),
                scope,
                type_annotation: var.type_hint.clone(),
                initial_value: var.initial_value.clone(),
                readers: BTreeSet::new(),
                writers: BTreeSet::new(),
                provenance: Provenance {
                    file: comp_model.file.clone(),
                    line: var.line,
                    column: None,
                    source_name: Some(format!("{}::{}", comp_name, var.name)),
                    policy_category: Some("state".to_string()),
                },
            });
        }

        for (provider_idx, provider) in comp_model.context_providers.iter().enumerate() {
            let id_content = format!(
                "context_provider:{}:{}:{}:{}:{}",
                comp_model.file, comp_name, provider.line, provider.context_name, provider_idx
            );
            let id = migration_ir::make_node_id(id_content.as_bytes());
            context_provider_ids
                .entry((comp_name.clone(), provider.context_name.clone()))
                .or_default()
                .push(id.clone());

            builder.add_state_variable(StateVariable {
                id,
                name: provider.context_name.clone(),
                scope: StateScope::Context,
                type_annotation: None,
                initial_value: provider.value_expression.clone(),
                readers: BTreeSet::new(),
                writers: BTreeSet::new(),
                provenance: Provenance {
                    file: comp_model.file.clone(),
                    line: provider.line,
                    column: None,
                    source_name: Some(format!("{}::{}", comp_name, provider.context_name)),
                    policy_category: Some("context".to_string()),
                },
            });
        }

        for (consumer_idx, consumer) in comp_model.context_consumers.iter().enumerate() {
            let id_content = format!(
                "context_consumer:{}:{}:{}:{}:{}",
                comp_model.file, comp_name, consumer.line, consumer.context_name, consumer_idx
            );
            let id = migration_ir::make_node_id(id_content.as_bytes());
            let display_name = consumer
                .binding
                .clone()
                .unwrap_or_else(|| consumer.context_name.clone());
            context_consumer_ids
                .entry((comp_name.clone(), consumer.context_name.clone()))
                .or_default()
                .push(id.clone());
            state_id_map.insert(
                (comp_name.clone(), consumer.context_name.clone()),
                id.clone(),
            );
            if let Some(binding) = &consumer.binding {
                state_id_map.insert((comp_name.clone(), binding.clone()), id.clone());
            }

            builder.add_state_variable(StateVariable {
                id,
                name: display_name.clone(),
                scope: StateScope::Context,
                type_annotation: None,
                initial_value: None,
                readers: BTreeSet::new(),
                writers: BTreeSet::new(),
                provenance: Provenance {
                    file: comp_model.file.clone(),
                    line: consumer.line,
                    column: None,
                    source_name: Some(format!("{}::{}", comp_name, display_name)),
                    policy_category: Some("context".to_string()),
                },
            });
        }

        // Derived state (useMemo, useCallback)
        for (derived_idx, derived) in comp_model.derived.iter().enumerate() {
            let name = derived.name.as_deref().unwrap_or("anonymous_derived");
            let id_content = format!(
                "derived:{}:{}:{}:{}:{}",
                comp_model.file, comp_name, derived.line, name, derived_idx
            );
            let id = migration_ir::make_node_id(id_content.as_bytes());

            // Resolve dependency IDs.
            let dep_ids: BTreeSet<IrNodeId> = derived
                .dependencies
                .iter()
                .filter_map(|dep_name| {
                    state_id_map
                        .get(&(comp_name.clone(), dep_name.clone()))
                        .cloned()
                })
                .collect();

            if dep_ids.len() < derived.dependencies.len() {
                let unresolved: Vec<_> = derived
                    .dependencies
                    .iter()
                    .filter(|d| !state_id_map.contains_key(&(comp_name.clone(), (*d).clone())))
                    .collect();
                diagnostics.push(LoweringDiagnostic {
                    code: "L010".to_string(),
                    severity: DiagnosticSeverity::Warning,
                    message: format!(
                        "Derived computation '{}' in {} has unresolved deps: {:?}",
                        name, comp_name, unresolved
                    ),
                    provenance: Some(Provenance {
                        file: comp_model.file.clone(),
                        line: derived.line,
                        column: None,
                        source_name: Some(format!("{}::{}", comp_name, name)),
                        policy_category: Some("state".to_string()),
                    }),
                });
            }

            builder.add_derived_state(DerivedState {
                id,
                name: name.to_string(),
                dependencies: dep_ids,
                expression_snippet: derived.expression_snippet.clone(),
                provenance: Provenance {
                    file: comp_model.file.clone(),
                    line: derived.line,
                    column: None,
                    source_name: Some(format!("{}::{}", comp_name, name)),
                    policy_category: Some("derived".to_string()),
                },
            });
        }

        // Context provider → consumer data flow edges.
    }

    // Add data flow edges from context graph.
    for edge in &state_model.context_graph {
        // Provider→consumer is an implicit data flow via context.
        let provider_key = (edge.provider_component.clone(), edge.context_name.clone());
        let consumer_key = (edge.consumer_component.clone(), edge.context_name.clone());

        if let (Some(from_ids), Some(to_ids)) = (
            context_provider_ids.get(&provider_key),
            context_consumer_ids.get(&consumer_key),
        ) {
            for from_id in from_ids {
                for to_id in to_ids {
                    builder.add_data_flow(from_id.clone(), to_id.clone());
                }
            }
        }
    }

    state_id_map
}

fn map_state_scope(scope: &StateVarScope) -> StateScope {
    match scope {
        StateVarScope::Local => StateScope::Local,
        StateVarScope::Reducer => StateScope::Local,
        StateVarScope::Ref => StateScope::Local,
        StateVarScope::Context => StateScope::Context,
        StateVarScope::ExternalStore => StateScope::Global,
        StateVarScope::Url => StateScope::Route,
        StateVarScope::Server => StateScope::Server,
    }
}

// ── Phase 3: Events ─────────────────────────────────────────────────────

fn lower_events(
    builder: &mut IrBuilder,
    state_model: &ProjectStateModel,
    state_id_map: &StateIdMap,
    diagnostics: &mut Vec<LoweringDiagnostic>,
) {
    for (comp_name, comp_model) in &state_model.components {
        for (event_idx, transition) in comp_model.event_transitions.iter().enumerate() {
            let event_id = make_event_id(&comp_model.file, comp_name, transition, event_idx);

            let kind = classify_event_kind(&transition.event_name);

            builder.add_event(EventDecl {
                id: event_id.clone(),
                name: transition.event_name.clone(),
                kind,
                source_node: None,
                payload_type: None,
                provenance: Provenance {
                    file: comp_model.file.clone(),
                    line: transition.line,
                    column: None,
                    source_name: transition.handler_name.clone(),
                    policy_category: Some("event".to_string()),
                },
            });

            // Create transitions for each state write.
            for state_write in &transition.state_writes {
                let target_id = state_id_map.get(&(comp_name.clone(), state_write.clone()));

                if let Some(target) = target_id {
                    builder.add_transition(EventTransition {
                        event_id: event_id.clone(),
                        target_state: target.clone(),
                        action_snippet: format!(
                            "{}({})",
                            state_write,
                            transition.handler_name.as_deref().unwrap_or("handler")
                        ),
                        guards: Vec::new(),
                    });
                } else {
                    diagnostics.push(LoweringDiagnostic {
                        code: "L020".to_string(),
                        severity: DiagnosticSeverity::Warning,
                        message: format!(
                            "Event '{}' in {} writes to '{}' but target state not found",
                            transition.event_name, comp_name, state_write
                        ),
                        provenance: Some(Provenance {
                            file: comp_model.file.clone(),
                            line: transition.line,
                            column: None,
                            source_name: transition.handler_name.clone(),
                            policy_category: Some("event".to_string()),
                        }),
                    });
                }
            }
        }
    }
}

fn make_event_id(
    file: &str,
    comp_name: &str,
    transition: &EventStateTransition,
    event_idx: usize,
) -> IrNodeId {
    let content = format!(
        "event:{}:{}:{}:{}:{}",
        file, comp_name, transition.event_name, transition.line, event_idx
    );
    migration_ir::make_node_id(content.as_bytes())
}

fn classify_event_kind(event_name: &str) -> EventKind {
    let name = event_name.to_lowercase();
    let is_user_input = name.strip_prefix("on").is_some_and(|suffix| {
        suffix.starts_with("click")
            || suffix.starts_with("mouse")
            || suffix.starts_with("key")
            || suffix.starts_with("touch")
            || suffix.starts_with("pointer")
            || suffix.starts_with("drag")
            || suffix.starts_with("drop")
            || suffix.starts_with("input")
            || suffix.starts_with("change")
            || suffix.starts_with("submit")
            || suffix.starts_with("focus")
            || suffix.starts_with("blur")
            || suffix.starts_with("scroll")
    });
    if is_user_input {
        return EventKind::UserInput;
    }

    if name.contains("mount") || name.contains("unmount") || name.contains("update") {
        return EventKind::Lifecycle;
    }

    if name.contains("timer") || name.contains("interval") || name.contains("timeout") {
        return EventKind::Timer;
    }

    if name.contains("fetch") || name.contains("response") || name.contains("request") {
        return EventKind::Network;
    }

    EventKind::Custom
}

// ── Phase 4: Effects ────────────────────────────────────────────────────

fn lower_effects(
    builder: &mut IrBuilder,
    state_model: &ProjectStateModel,
    state_id_map: &StateIdMap,
    diagnostics: &mut Vec<LoweringDiagnostic>,
) {
    for (comp_name, comp_model) in &state_model.components {
        for (idx, effect) in comp_model.effects.iter().enumerate() {
            let id_content = format!(
                "effect:{}:{}:{}:{}:{}",
                comp_model.file, comp_name, effect.hook, effect.line, idx
            );
            let id = migration_ir::make_node_id(id_content.as_bytes());

            let kind = map_effect_kind(&effect.kind);

            // Resolve dependency IDs.
            let dep_ids: BTreeSet<IrNodeId> = effect
                .dependencies
                .iter()
                .filter_map(|dep| state_id_map.get(&(comp_name.clone(), dep.clone())).cloned())
                .collect();

            let read_ids: BTreeSet<IrNodeId> = effect
                .reads
                .iter()
                .filter_map(|r| state_id_map.get(&(comp_name.clone(), r.clone())).cloned())
                .collect();

            let write_ids: BTreeSet<IrNodeId> = effect
                .writes
                .iter()
                .filter_map(|w| state_id_map.get(&(comp_name.clone(), w.clone())).cloned())
                .collect();

            // Emit diagnostic for unresolved reads/writes.
            let total_refs = effect.reads.len() + effect.writes.len();
            let resolved_refs = read_ids.len() + write_ids.len();
            if resolved_refs < total_refs {
                diagnostics.push(LoweringDiagnostic {
                    code: "L030".to_string(),
                    severity: DiagnosticSeverity::Warning,
                    message: format!(
                        "Effect #{} ({}) in {} has {} unresolved state references",
                        idx,
                        effect.hook,
                        comp_name,
                        total_refs - resolved_refs
                    ),
                    provenance: Some(Provenance {
                        file: comp_model.file.clone(),
                        line: effect.line,
                        column: None,
                        source_name: Some(format!("{}::effect#{}", comp_name, idx)),
                        policy_category: Some("effect".to_string()),
                    }),
                });
            }

            builder.add_effect(EffectDecl {
                id,
                name: format!("{}::effect#{}", comp_name, idx),
                kind,
                dependencies: dep_ids,
                has_cleanup: effect.has_cleanup,
                reads: read_ids,
                writes: write_ids,
                provenance: Provenance {
                    file: comp_model.file.clone(),
                    line: effect.line,
                    column: None,
                    source_name: Some(format!("{}::{}", comp_name, effect.hook)),
                    policy_category: Some("effect".to_string()),
                },
            });
        }
    }
}

fn map_effect_kind(classification: &EffectClassification) -> EffectKind {
    match classification {
        EffectClassification::DataFetch => EffectKind::Network,
        EffectClassification::DomManipulation => EffectKind::Dom,
        EffectClassification::EventListener => EffectKind::Subscription,
        EffectClassification::Timer => EffectKind::Timer,
        EffectClassification::Subscription => EffectKind::Subscription,
        EffectClassification::Sync => EffectKind::Storage,
        EffectClassification::Telemetry => EffectKind::Telemetry,
        EffectClassification::Unknown => EffectKind::Other,
    }
}

// ── Phase 5: Style Intent ───────────────────────────────────────────────

fn lower_style_intent(
    builder: &mut IrBuilder,
    styles: &StyleSemanticsResult,
    _diagnostics: &mut Vec<LoweringDiagnostic>,
) {
    let intent = style_semantics::to_style_intent(styles);

    for token in intent.tokens.values() {
        builder.add_style_token(token.clone());
    }

    for (node_id, layout) in &intent.layouts {
        builder.add_layout(node_id.clone(), layout.clone());
    }

    for theme in &intent.themes {
        builder.add_theme(theme.clone());
    }
}

// ── Phase 6: Capabilities ───────────────────────────────────────────────

fn lower_capabilities(builder: &mut IrBuilder, state_model: &ProjectStateModel) {
    for cap in &state_model.required_capabilities {
        builder.require_capability(cap.clone());
    }

    for cap in &state_model.optional_capabilities {
        builder.optional_capability(cap.clone());
    }

    for assumption in &state_model.platform_assumptions {
        builder.add_platform_assumption(assumption.clone());
    }
}

// ── Phase 7: Accessibility ──────────────────────────────────────────────

fn lower_accessibility(
    builder: &mut IrBuilder,
    styles: &StyleSemanticsResult,
    composition: &CompositionSemanticsResult,
    diagnostics: &mut Vec<LoweringDiagnostic>,
) {
    let a11y_meta = style_semantics::accessibility_meta(styles);

    // Add entries for components with accessibility-relevant style properties.
    for id in &a11y_meta.components_with_colors {
        builder.add_accessibility(AccessibilityEntry {
            node_id: id.clone(),
            role: None,
            label: None,
            description: Some("Has explicit color declarations — verify contrast".to_string()),
            keyboard_shortcut: None,
            focus_order: None,
            live_region: None,
        });
    }

    // Check composition tree for interactive elements without accessibility hints.
    for (id, node) in &composition.component_tree.nodes {
        let has_interactive_props = node.props_contract.iter().any(|p| p.is_callback);
        if has_interactive_props {
            builder.add_accessibility(AccessibilityEntry {
                node_id: id.clone(),
                role: Some("interactive".to_string()),
                label: None,
                description: Some(format!(
                    "Component '{}' has callback props — may need keyboard support",
                    node.component_name
                )),
                keyboard_shortcut: None,
                focus_order: None,
                live_region: None,
            });

            diagnostics.push(LoweringDiagnostic {
                code: "L040".to_string(),
                severity: DiagnosticSeverity::Info,
                message: format!(
                    "Interactive component '{}' should define ARIA role and keyboard handling",
                    node.component_name
                ),
                provenance: Some(Provenance {
                    file: node.source_file.clone(),
                    line: node.line_start,
                    column: None,
                    source_name: Some(node.component_name.clone()),
                    policy_category: Some("accessibility".to_string()),
                }),
            });
        }
    }
}

// ── Phase 8: Warning propagation ────────────────────────────────────────

fn propagate_extraction_warnings(
    builder: &mut IrBuilder,
    composition: &CompositionSemanticsResult,
    styles: &StyleSemanticsResult,
    diagnostics: &mut Vec<LoweringDiagnostic>,
) {
    // Composition warnings
    for w in &composition.warnings {
        builder.add_warning(IrWarning {
            code: format!("CS-{}", w.code),
            message: w.message.clone(),
            provenance: Some(Provenance {
                file: w.file.clone(),
                line: w.line.unwrap_or(0),
                column: None,
                source_name: None,
                policy_category: Some("composition".to_string()),
            }),
        });
        diagnostics.push(LoweringDiagnostic {
            code: format!("CS-{}", w.code),
            severity: DiagnosticSeverity::Warning,
            message: w.message.clone(),
            provenance: Some(Provenance {
                file: w.file.clone(),
                line: w.line.unwrap_or(0),
                column: None,
                source_name: None,
                policy_category: Some("composition".to_string()),
            }),
        });
    }

    // Style warnings
    for w in &styles.warnings {
        builder.add_warning(IrWarning {
            code: format!("SS-{}", style_warning_code(&w.kind)),
            message: w.message.clone(),
            provenance: w.provenance.clone(),
        });
    }
}

fn style_warning_code(kind: &StyleWarningKind) -> &'static str {
    match kind {
        StyleWarningKind::PrecedenceConflict => "PREC",
        StyleWarningKind::UnresolvedClassRef => "UCLS",
        StyleWarningKind::UnresolvedToken => "UTOK",
        StyleWarningKind::InlineOverride => "INOV",
        StyleWarningKind::HardcodedColor => "HCOL",
        StyleWarningKind::AccessibilityConcern => "A11Y",
    }
}

// ── Tests ───────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::migration_ir::validate_ir;
    use crate::tsx_parser::{
        ComponentDecl, ComponentKind, FileParse, HookCall, JsxElement, JsxProp,
    };
    use std::collections::BTreeSet;

    fn test_config() -> LoweringConfig {
        LoweringConfig {
            run_id: "test-run-001".to_string(),
            source_project: "test-project".to_string(),
        }
    }

    fn make_project(files: Vec<(&str, FileParse)>) -> ProjectParse {
        ProjectParse {
            files: files.into_iter().map(|(k, v)| (k.to_string(), v)).collect(),
            file_contents: BTreeMap::new(),
            symbol_table: BTreeMap::new(),
            component_count: 0,
            hook_usage_count: 0,
            type_count: 0,
            diagnostics: Vec::new(),
            external_imports: BTreeSet::new(),
        }
    }

    fn make_file(path: &str) -> FileParse {
        FileParse {
            file: path.to_string(),
            components: Vec::new(),
            hooks: Vec::new(),
            jsx_elements: Vec::new(),
            types: Vec::new(),
            symbols: Vec::new(),
            diagnostics: Vec::new(),
        }
    }

    fn make_component_file(path: &str) -> FileParse {
        FileParse {
            file: path.to_string(),
            components: vec![ComponentDecl {
                name: "App".to_string(),
                kind: ComponentKind::FunctionComponent,
                is_default_export: true,
                is_named_export: false,
                props_type: None,
                hooks: vec![
                    HookCall {
                        name: "useState".to_string(),
                        binding: Some("count, setCount".to_string()),
                        args_snippet: "0".to_string(),
                        line: 5,
                    },
                    HookCall {
                        name: "useEffect".to_string(),
                        binding: None,
                        args_snippet: "() => { document.title = `Count: ${count}` }, [count]"
                            .to_string(),
                        line: 8,
                    },
                ],
                event_handlers: vec![crate::tsx_parser::EventHandler {
                    event_name: "onClick".to_string(),
                    handler_name: Some("handleClick".to_string()),
                    is_inline: false,
                    line: 12,
                }],
                line: 1,
            }],
            hooks: Vec::new(),
            jsx_elements: vec![
                JsxElement {
                    tag: "div".to_string(),
                    is_component: false,
                    is_fragment: false,
                    is_self_closing: false,
                    props: vec![
                        JsxProp {
                            name: "className".to_string(),
                            is_spread: false,
                            value_snippet: Some("\"container\"".to_string()),
                        },
                        JsxProp {
                            name: "style".to_string(),
                            is_spread: false,
                            value_snippet: Some("{{ display: 'flex', color: '#333' }}".to_string()),
                        },
                    ],
                    line: 15,
                },
                JsxElement {
                    tag: "button".to_string(),
                    is_component: false,
                    is_fragment: false,
                    is_self_closing: false,
                    props: vec![JsxProp {
                        name: "onClick".to_string(),
                        is_spread: false,
                        value_snippet: Some("{handleClick}".to_string()),
                    }],
                    line: 16,
                },
            ],
            types: Vec::new(),
            symbols: Vec::new(),
            diagnostics: Vec::new(),
        }
    }

    fn make_nested_component_file(path: &str) -> FileParse {
        FileParse {
            file: path.to_string(),
            components: vec![
                ComponentDecl {
                    name: "App".to_string(),
                    kind: ComponentKind::FunctionComponent,
                    is_default_export: true,
                    is_named_export: false,
                    props_type: None,
                    hooks: Vec::new(),
                    event_handlers: Vec::new(),
                    line: 1,
                },
                ComponentDecl {
                    name: "Counter".to_string(),
                    kind: ComponentKind::FunctionComponent,
                    is_default_export: false,
                    is_named_export: true,
                    props_type: None,
                    hooks: Vec::new(),
                    event_handlers: Vec::new(),
                    line: 20,
                },
            ],
            hooks: Vec::new(),
            jsx_elements: vec![JsxElement {
                tag: "Counter".to_string(),
                is_component: true,
                is_fragment: false,
                is_self_closing: true,
                props: Vec::new(),
                line: 8,
            }],
            types: Vec::new(),
            symbols: Vec::new(),
            diagnostics: Vec::new(),
        }
    }

    // ── Basic pipeline ──────────────────────────────────────────────────

    #[test]
    fn empty_project_produces_valid_ir() {
        let project = make_project(vec![]);
        let result = lower_project(&test_config(), &project);
        let errors = validate_ir(&result.ir);
        assert!(errors.is_empty(), "Validation errors: {:?}", errors);
        assert_eq!(result.ir.schema_version, "migration-ir-v1");
    }

    #[test]
    fn single_file_project_lowers() {
        let project = make_project(vec![("src/App.tsx", make_file("src/App.tsx"))]);
        let result = lower_project(&test_config(), &project);
        let errors = validate_ir(&result.ir);
        assert!(errors.is_empty(), "Validation errors: {:?}", errors);
        assert_eq!(result.ir.metadata.source_file_count, 1);
    }

    #[test]
    fn component_file_produces_complete_ir() {
        let project = make_project(vec![("src/App.tsx", make_component_file("src/App.tsx"))]);
        let result = lower_project(&test_config(), &project);
        let errors = validate_ir(&result.ir);
        assert!(errors.is_empty(), "Validation errors: {:?}", errors);

        // Should have view nodes from composition semantics.
        assert!(!result.ir.view_tree.nodes.is_empty(), "Expected view nodes");

        // Should have state variables.
        assert!(
            !result.ir.state_graph.variables.is_empty(),
            "Expected state variables"
        );

        // Should have effects.
        assert!(
            !result.ir.effect_registry.effects.is_empty(),
            "Expected effects"
        );

        // Should have style intent (tokens or layouts).
        assert!(
            !result.ir.style_intent.layouts.is_empty() || !result.ir.style_intent.tokens.is_empty(),
            "Expected style data"
        );
    }

    #[test]
    fn same_file_nested_components_produce_valid_view_edges() {
        let project = make_project(vec![(
            "src/App.tsx",
            make_nested_component_file("src/App.tsx"),
        )]);
        let result = lower_project(&test_config(), &project);
        let errors = validate_ir(&result.ir);
        assert!(errors.is_empty(), "Validation errors: {:?}", errors);

        let app = result
            .ir
            .view_tree
            .nodes
            .values()
            .find(|node| node.name == "App")
            .expect("App view node");
        let counter_id = result
            .ir
            .view_tree
            .nodes
            .iter()
            .find(|(_, node)| node.name == "Counter")
            .map(|(id, _)| id.clone())
            .expect("Counter view node");

        assert_eq!(app.children, vec![counter_id]);
        assert_eq!(result.ir.view_tree.roots, vec![app.id.clone()]);
    }

    // ── View tree lowering ──────────────────────────────────────────────

    #[test]
    fn view_tree_preserves_roots() {
        let project = make_project(vec![("src/App.tsx", make_component_file("src/App.tsx"))]);
        let composition = composition_semantics::extract_composition_semantics(&project);
        let styles = style_semantics::extract_style_semantics(&project);
        let state_model =
            state_effects::build_project_state_model(&project.files, &project.file_contents);

        let result = lower_to_ir(
            &test_config(),
            &project,
            &composition,
            &styles,
            &state_model,
        );

        // Roots in IR should match composition.
        let comp_tree = composition_semantics::to_view_tree(&composition);
        assert_eq!(result.ir.view_tree.roots.len(), comp_tree.roots.len());
    }

    // ── State graph lowering ────────────────────────────────────────────

    #[test]
    fn state_scope_mapping() {
        assert_eq!(map_state_scope(&StateVarScope::Local), StateScope::Local);
        assert_eq!(map_state_scope(&StateVarScope::Reducer), StateScope::Local);
        assert_eq!(map_state_scope(&StateVarScope::Ref), StateScope::Local);
        assert_eq!(
            map_state_scope(&StateVarScope::Context),
            StateScope::Context
        );
        assert_eq!(
            map_state_scope(&StateVarScope::ExternalStore),
            StateScope::Global
        );
        assert_eq!(map_state_scope(&StateVarScope::Url), StateScope::Route);
        assert_eq!(map_state_scope(&StateVarScope::Server), StateScope::Server);
    }

    // ── Event classification ────────────────────────────────────────────

    #[test]
    fn event_kind_classification() {
        assert_eq!(classify_event_kind("onClick"), EventKind::UserInput);
        assert_eq!(classify_event_kind("onKeyDown"), EventKind::UserInput);
        assert_eq!(classify_event_kind("onChange"), EventKind::UserInput);
        assert_eq!(classify_event_kind("onSubmit"), EventKind::UserInput);
        assert_eq!(classify_event_kind("onMouseEnter"), EventKind::UserInput);
        assert_eq!(classify_event_kind("onScroll"), EventKind::UserInput);
        assert_eq!(
            classify_event_kind("componentDidMount"),
            EventKind::Lifecycle
        );
        assert_eq!(classify_event_kind("timerTick"), EventKind::Timer);
        assert_eq!(classify_event_kind("fetchData"), EventKind::Network);
        assert_eq!(classify_event_kind("customAction"), EventKind::Custom);
    }

    // ── Effect kind mapping ─────────────────────────────────────────────

    #[test]
    fn effect_kind_mapping() {
        assert_eq!(
            map_effect_kind(&EffectClassification::DataFetch),
            EffectKind::Network
        );
        assert_eq!(
            map_effect_kind(&EffectClassification::DomManipulation),
            EffectKind::Dom
        );
        assert_eq!(
            map_effect_kind(&EffectClassification::Timer),
            EffectKind::Timer
        );
        assert_eq!(
            map_effect_kind(&EffectClassification::Sync),
            EffectKind::Storage
        );
        assert_eq!(
            map_effect_kind(&EffectClassification::Telemetry),
            EffectKind::Telemetry
        );
        assert_eq!(
            map_effect_kind(&EffectClassification::Unknown),
            EffectKind::Other
        );
    }

    // ── Determinism ─────────────────────────────────────────────────────

    #[test]
    fn lowering_is_deterministic() {
        let project = make_project(vec![("src/App.tsx", make_component_file("src/App.tsx"))]);

        let result1 = lower_project(&test_config(), &project);
        let result2 = lower_project(&test_config(), &project);

        // Structural determinism: same number of nodes, variables, effects.
        assert_eq!(
            result1.ir.view_tree.nodes.len(),
            result2.ir.view_tree.nodes.len(),
            "View tree node count differs across runs"
        );
        assert_eq!(
            result1.ir.state_graph.variables.len(),
            result2.ir.state_graph.variables.len(),
            "State variable count differs across runs"
        );
        assert_eq!(
            result1.ir.effect_registry.effects.len(),
            result2.ir.effect_registry.effects.len(),
            "Effect count differs across runs"
        );
        // Node IDs must be identical (content-addressable).
        let ids1: BTreeSet<_> = result1.ir.view_tree.nodes.keys().collect();
        let ids2: BTreeSet<_> = result2.ir.view_tree.nodes.keys().collect();
        assert_eq!(ids1, ids2, "View tree node IDs differ across runs");
    }

    // ── Diagnostics ─────────────────────────────────────────────────────

    #[test]
    fn diagnostics_for_empty_view_tree() {
        // Empty file with no components → L002 info diagnostic.
        let project = make_project(vec![("src/empty.tsx", make_file("src/empty.tsx"))]);
        let result = lower_project(&test_config(), &project);

        // No view nodes means no L002 either (only triggers if nodes exist but no roots).
        assert!(
            result.diagnostics.is_empty() || result.diagnostics.iter().all(|d| d.code != "L002")
        );
    }

    // ── Config ──────────────────────────────────────────────────────────

    #[test]
    fn config_propagates_to_ir() {
        let config = LoweringConfig {
            run_id: "custom-run-42".to_string(),
            source_project: "my-react-app".to_string(),
        };
        let project = make_project(vec![]);
        let result = lower_project(&config, &project);
        assert_eq!(result.ir.run_id, "custom-run-42");
        assert_eq!(result.ir.source_project, "my-react-app");
    }

    // ── Serialization ───────────────────────────────────────────────────

    #[test]
    fn ir_serializes_to_json() {
        let project = make_project(vec![("src/App.tsx", make_component_file("src/App.tsx"))]);
        let result = lower_project(&test_config(), &project);
        let json = serde_json::to_string(&result.ir).unwrap();
        assert!(!json.is_empty());

        // Roundtrip.
        let deserialized: MigrationIr = serde_json::from_str(&json).unwrap();
        assert_eq!(result.ir.schema_version, deserialized.schema_version);
        assert_eq!(result.ir.run_id, deserialized.run_id);
    }

    // ── Multi-component ─────────────────────────────────────────────────

    #[test]
    fn multi_component_file() {
        let mut file = make_component_file("src/App.tsx");
        file.components.push(ComponentDecl {
            name: "Header".to_string(),
            kind: ComponentKind::FunctionComponent,
            is_default_export: false,
            is_named_export: true,
            props_type: Some("HeaderProps".to_string()),
            hooks: vec![HookCall {
                name: "useContext".to_string(),
                binding: Some("theme".to_string()),
                args_snippet: "ThemeContext".to_string(),
                line: 20,
            }],
            event_handlers: Vec::new(),
            line: 18,
        });

        let project = make_project(vec![("src/App.tsx", file)]);
        let result = lower_project(&test_config(), &project);
        let errors = validate_ir(&result.ir);
        assert!(errors.is_empty(), "Validation errors: {:?}", errors);

        // Should have at least one state var (from the App component's useState).
        assert!(
            !result.ir.state_graph.variables.is_empty(),
            "Expected state variables from multi-component file"
        );
    }

    // ── Capability forwarding ───────────────────────────────────────────

    #[test]
    fn capabilities_forwarded_from_state_model() {
        let project = make_project(vec![("src/App.tsx", make_component_file("src/App.tsx"))]);
        let composition = composition_semantics::extract_composition_semantics(&project);
        let styles = style_semantics::extract_style_semantics(&project);
        let state_model =
            state_effects::build_project_state_model(&project.files, &project.file_contents);

        let result = lower_to_ir(
            &test_config(),
            &project,
            &composition,
            &styles,
            &state_model,
        );

        // Capability sets should match state model.
        assert_eq!(
            result.ir.capabilities.required,
            state_model.required_capabilities
        );
        assert_eq!(
            result.ir.capabilities.optional,
            state_model.optional_capabilities
        );
    }

    #[test]
    fn context_provider_consumer_data_flow_is_lowered() {
        let provider_src = r#"
function App() {
    return <ThemeContext.Provider value={theme}><ThemeButton /></ThemeContext.Provider>;
}
"#;
        let consumer_src = r#"
function ThemeButton() {
    const theme = useContext(ThemeContext);
    return <button />;
}
"#;

        let mut files = BTreeMap::new();
        files.insert(
            "src/App.tsx".to_string(),
            crate::tsx_parser::parse_file(provider_src, "src/App.tsx"),
        );
        files.insert(
            "src/ThemeButton.tsx".to_string(),
            crate::tsx_parser::parse_file(consumer_src, "src/ThemeButton.tsx"),
        );
        let mut file_contents = BTreeMap::new();
        file_contents.insert("src/App.tsx".to_string(), provider_src.to_string());
        file_contents.insert("src/ThemeButton.tsx".to_string(), consumer_src.to_string());
        let project = ProjectParse {
            files,
            file_contents,
            symbol_table: BTreeMap::new(),
            component_count: 2,
            hook_usage_count: 1,
            type_count: 0,
            diagnostics: Vec::new(),
            external_imports: BTreeSet::new(),
        };

        let result = lower_project(&test_config(), &project);

        let provider_id = result
            .ir
            .state_graph
            .variables
            .iter()
            .find(|(_, var)| {
                var.scope == StateScope::Context
                    && var.name == "ThemeContext"
                    && var.provenance.file == "src/App.tsx"
            })
            .map(|(id, _)| id.clone())
            .expect("provider context state");
        let consumer_id = result
            .ir
            .state_graph
            .variables
            .iter()
            .find(|(_, var)| {
                var.scope == StateScope::Context
                    && var.name == "theme"
                    && var.provenance.file == "src/ThemeButton.tsx"
            })
            .map(|(id, _)| id.clone())
            .expect("consumer context state");

        assert!(
            result
                .ir
                .state_graph
                .data_flow
                .get(&provider_id)
                .is_some_and(|targets| targets.contains(&consumer_id)),
            "expected provider to feed consumer through context data flow"
        );
    }

    #[test]
    fn same_component_context_provider_and_consumer_stay_distinct_without_self_edge() {
        let src = r#"
function ThemeBridge() {
    const theme = useContext(ThemeContext);
    return <ThemeContext.Provider value={theme}><button /></ThemeContext.Provider>;
}
"#;

        let mut files = BTreeMap::new();
        files.insert(
            "src/ThemeBridge.tsx".to_string(),
            crate::tsx_parser::parse_file(src, "src/ThemeBridge.tsx"),
        );
        let mut file_contents = BTreeMap::new();
        file_contents.insert("src/ThemeBridge.tsx".to_string(), src.to_string());
        let project = ProjectParse {
            files,
            file_contents,
            symbol_table: BTreeMap::new(),
            component_count: 1,
            hook_usage_count: 1,
            type_count: 0,
            diagnostics: Vec::new(),
            external_imports: BTreeSet::new(),
        };

        let result = lower_project(&test_config(), &project);

        let provider_id = result
            .ir
            .state_graph
            .variables
            .iter()
            .find(|(_, var)| {
                var.scope == StateScope::Context
                    && var.name == "ThemeContext"
                    && var.provenance.file == "src/ThemeBridge.tsx"
            })
            .map(|(id, _)| id.clone())
            .expect("provider context state");
        let consumer_id = result
            .ir
            .state_graph
            .variables
            .iter()
            .find(|(_, var)| {
                var.scope == StateScope::Context
                    && var.name == "theme"
                    && var.provenance.file == "src/ThemeBridge.tsx"
            })
            .map(|(id, _)| id.clone())
            .expect("consumer context state");

        assert_ne!(provider_id, consumer_id);
        assert!(
            !result
                .ir
                .state_graph
                .data_flow
                .get(&provider_id)
                .is_some_and(|targets| targets.contains(&consumer_id)),
            "same-component provider should not feed a useContext call in that component"
        );
    }

    #[test]
    fn duplicate_context_consumers_in_one_component_keep_distinct_state_nodes() {
        let provider_src = r#"
function App() {
    return <ThemeContext.Provider value={theme}><ThemePanel /></ThemeContext.Provider>;
}
"#;
        let consumer_src = r#"
function ThemePanel() {
    const theme = useContext(ThemeContext);
    const fallbackTheme = useContext(ThemeContext);
    return <section />;
}
"#;

        let mut files = BTreeMap::new();
        files.insert(
            "src/App.tsx".to_string(),
            crate::tsx_parser::parse_file(provider_src, "src/App.tsx"),
        );
        files.insert(
            "src/ThemePanel.tsx".to_string(),
            crate::tsx_parser::parse_file(consumer_src, "src/ThemePanel.tsx"),
        );
        let mut file_contents = BTreeMap::new();
        file_contents.insert("src/App.tsx".to_string(), provider_src.to_string());
        file_contents.insert("src/ThemePanel.tsx".to_string(), consumer_src.to_string());
        let project = ProjectParse {
            files,
            file_contents,
            symbol_table: BTreeMap::new(),
            component_count: 2,
            hook_usage_count: 2,
            type_count: 0,
            diagnostics: Vec::new(),
            external_imports: BTreeSet::new(),
        };

        let result = lower_project(&test_config(), &project);

        let consumer_vars = result
            .ir
            .state_graph
            .variables
            .iter()
            .filter(|(_, var)| {
                var.scope == StateScope::Context && var.provenance.file == "src/ThemePanel.tsx"
            })
            .map(|(id, var)| (id.clone(), var.name.clone()))
            .collect::<Vec<_>>();

        assert_eq!(consumer_vars.len(), 2);
        assert!(
            consumer_vars.iter().any(|(_, name)| name == "theme"),
            "expected a context node for the first useContext binding"
        );
        assert!(
            consumer_vars
                .iter()
                .any(|(_, name)| name == "fallbackTheme"),
            "expected a distinct context node for the second useContext binding"
        );
        assert_ne!(consumer_vars[0].0, consumer_vars[1].0);

        let provider_id = result
            .ir
            .state_graph
            .variables
            .iter()
            .find(|(_, var)| {
                var.scope == StateScope::Context
                    && var.name == "ThemeContext"
                    && var.provenance.file == "src/App.tsx"
            })
            .map(|(id, _)| id.clone())
            .expect("provider context state");

        let targets = result
            .ir
            .state_graph
            .data_flow
            .get(&provider_id)
            .expect("provider data flow edges");
        for (consumer_id, _) in &consumer_vars {
            assert!(
                targets.contains(consumer_id),
                "expected provider to feed each context consumer binding"
            );
        }
    }

    #[test]
    fn same_line_unbound_state_hooks_keep_distinct_state_nodes() {
        let src = r#"
function App() { useRef(null); useRef(null); return <div />; }
"#;

        let project = ProjectParse {
            files: BTreeMap::from([(
                "src/App.tsx".to_string(),
                crate::tsx_parser::parse_file(src, "src/App.tsx"),
            )]),
            file_contents: BTreeMap::from([("src/App.tsx".to_string(), src.to_string())]),
            symbol_table: BTreeMap::new(),
            component_count: 1,
            hook_usage_count: 2,
            type_count: 0,
            diagnostics: Vec::new(),
            external_imports: BTreeSet::new(),
        };

        let result = lower_project(&test_config(), &project);
        let ref_vars = result
            .ir
            .state_graph
            .variables
            .values()
            .filter(|var| var.provenance.file == "src/App.tsx" && var.name == "ref")
            .count();

        assert_eq!(ref_vars, 2, "each unbound useRef should survive lowering");
    }

    #[test]
    fn same_line_anonymous_derived_hooks_keep_distinct_nodes() {
        let src = r#"
function App() { useMemo(() => 1, []); useMemo(() => 2, []); return <div />; }
"#;

        let project = ProjectParse {
            files: BTreeMap::from([(
                "src/App.tsx".to_string(),
                crate::tsx_parser::parse_file(src, "src/App.tsx"),
            )]),
            file_contents: BTreeMap::from([("src/App.tsx".to_string(), src.to_string())]),
            symbol_table: BTreeMap::new(),
            component_count: 1,
            hook_usage_count: 2,
            type_count: 0,
            diagnostics: Vec::new(),
            external_imports: BTreeSet::new(),
        };

        let result = lower_project(&test_config(), &project);
        assert_eq!(
            result.ir.state_graph.derived.len(),
            2,
            "same-line anonymous useMemo hooks should not overwrite each other"
        );
    }

    #[test]
    fn same_line_duplicate_events_keep_distinct_event_nodes() {
        let src = r#"
function App() { return <><button onClick={handleA} /><button onClick={handleB} /></>; }
"#;

        let project = ProjectParse {
            files: BTreeMap::from([(
                "src/App.tsx".to_string(),
                crate::tsx_parser::parse_file(src, "src/App.tsx"),
            )]),
            file_contents: BTreeMap::from([("src/App.tsx".to_string(), src.to_string())]),
            symbol_table: BTreeMap::new(),
            component_count: 1,
            hook_usage_count: 0,
            type_count: 0,
            diagnostics: Vec::new(),
            external_imports: BTreeSet::new(),
        };

        let result = lower_project(&test_config(), &project);
        assert_eq!(
            result.ir.event_catalog.events.len(),
            2,
            "same-line event handlers should each produce an event node"
        );
    }

    #[test]
    fn same_line_duplicate_effects_keep_distinct_effect_nodes() {
        let src = r#"
function App() { useEffect(() => {}, []); useEffect(() => {}, []); return <div />; }
"#;

        let project = ProjectParse {
            files: BTreeMap::from([(
                "src/App.tsx".to_string(),
                crate::tsx_parser::parse_file(src, "src/App.tsx"),
            )]),
            file_contents: BTreeMap::from([("src/App.tsx".to_string(), src.to_string())]),
            symbol_table: BTreeMap::new(),
            component_count: 1,
            hook_usage_count: 2,
            type_count: 0,
            diagnostics: Vec::new(),
            external_imports: BTreeSet::new(),
        };

        let result = lower_project(&test_config(), &project);
        assert_eq!(
            result.ir.effect_registry.effects.len(),
            2,
            "same-line useEffect hooks should not overwrite each other"
        );
    }
}
