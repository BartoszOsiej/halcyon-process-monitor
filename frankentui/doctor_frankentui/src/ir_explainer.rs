// SPDX-License-Identifier: Apache-2.0
//! IR explainer tooling: graph dumps, provenance traces, and pass diffs.
//!
//! Provides debuggability tools for developers and operators to inspect
//! migration IR graphs, trace provenance slices, and compare before/after
//! normalization passes with semantic-change classification.
//!
//! # Output Modes
//!
//! All explainer functions produce `ExplainerOutput` which can be rendered
//! as either human-readable text or machine-readable JSON, supporting both
//! interactive debugging and automated CI/certification pipelines.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::ir_normalize::{self, NormalizationReport};
use crate::migration_ir::{EffectKind, IrNodeId, MigrationIr, Provenance};

// ── Output Types ────────────────────────────────────────────────────────

/// Unified output from any explainer operation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExplainerOutput {
    /// Human-readable text representation.
    pub text: String,
    /// Machine-readable structured data.
    pub data: serde_json::Value,
    /// Output kind for downstream routing.
    pub kind: OutputKind,
}

/// What kind of explainer output this is.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum OutputKind {
    GraphDump,
    ProvenanceTrace,
    PassDiff,
    Summary,
}

// ── Graph Dump ──────────────────────────────────────────────────────────

/// Summary statistics for an IR graph.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GraphSummary {
    pub schema_version: String,
    pub run_id: String,
    pub source_project: String,
    pub view_nodes: usize,
    pub view_roots: usize,
    pub state_variables: usize,
    pub derived_computations: usize,
    pub data_flow_edges: usize,
    pub events: usize,
    pub transitions: usize,
    pub effects: usize,
    pub style_tokens: usize,
    pub themes: usize,
    pub capabilities_required: usize,
    pub capabilities_optional: usize,
    pub accessibility_entries: usize,
    pub warnings: usize,
}

/// Dump a full human-readable and machine-readable representation of an IR.
pub fn dump_graph(ir: &MigrationIr) -> ExplainerOutput {
    let summary = build_graph_summary(ir);
    let mut text = String::new();

    // Header.
    text.push_str(&format!("=== IR Graph Dump: {} ===\n", ir.source_project));
    text.push_str(&format!(
        "Schema: {}  Run: {}\n",
        ir.schema_version, ir.run_id
    ));
    text.push_str(&format!(
        "Files: {}  Warnings: {}\n\n",
        ir.metadata.source_file_count,
        ir.metadata.warnings.len()
    ));

    // View tree.
    text.push_str(&format!(
        "── View Tree ({} nodes, {} roots) ──\n",
        ir.view_tree.nodes.len(),
        ir.view_tree.roots.len()
    ));
    for root_id in &ir.view_tree.roots {
        dump_view_subtree(ir, root_id, 0, &mut text);
    }
    // Orphan nodes (not reachable from roots).
    let reachable = collect_reachable_nodes(ir);
    let orphans: Vec<_> = ir
        .view_tree
        .nodes
        .keys()
        .filter(|id| !reachable.contains(*id))
        .collect();
    if !orphans.is_empty() {
        text.push_str(&format!("  [orphan nodes: {}]\n", orphans.len()));
    }
    text.push('\n');

    // State graph.
    text.push_str(&format!(
        "── State Graph ({} vars, {} derived) ──\n",
        ir.state_graph.variables.len(),
        ir.state_graph.derived.len()
    ));
    for (id, var) in &ir.state_graph.variables {
        text.push_str(&format!(
            "  {} [{}] {:?} = {}\n",
            var.name,
            id,
            var.scope,
            var.initial_value.as_deref().unwrap_or("?")
        ));
    }
    for (id, derived) in &ir.state_graph.derived {
        text.push_str(&format!(
            "  derived:{} [{}] deps={}\n",
            derived.name,
            id,
            derived.dependencies.len()
        ));
    }
    text.push('\n');

    // Events.
    text.push_str(&format!(
        "── Events ({} events, {} transitions) ──\n",
        ir.event_catalog.events.len(),
        ir.event_catalog.transitions.len()
    ));
    for (id, event) in &ir.event_catalog.events {
        text.push_str(&format!("  {} [{}] {:?}\n", event.name, id, event.kind));
    }
    text.push('\n');

    // Effects.
    text.push_str(&format!(
        "── Effects ({}) ──\n",
        ir.effect_registry.effects.len()
    ));
    for (id, effect) in &ir.effect_registry.effects {
        text.push_str(&format!(
            "  {} [{}] {:?} cleanup={}\n",
            effect.name, id, effect.kind, effect.has_cleanup
        ));
    }
    text.push('\n');

    // Style intent.
    text.push_str(&format!(
        "── Style ({} tokens, {} layouts, {} themes) ──\n",
        ir.style_intent.tokens.len(),
        ir.style_intent.layouts.len(),
        ir.style_intent.themes.len()
    ));
    text.push('\n');

    // Capabilities.
    text.push_str(&format!(
        "── Capabilities (req={}, opt={}) ──\n",
        ir.capabilities.required.len(),
        ir.capabilities.optional.len()
    ));
    for cap in &ir.capabilities.required {
        text.push_str(&format!("  [required] {:?}\n", cap));
    }
    for cap in &ir.capabilities.optional {
        text.push_str(&format!("  [optional] {:?}\n", cap));
    }

    let data = serde_json::to_value(&summary).unwrap_or_default();

    ExplainerOutput {
        text,
        data,
        kind: OutputKind::GraphDump,
    }
}

fn build_graph_summary(ir: &MigrationIr) -> GraphSummary {
    let data_flow_edges: usize = ir.state_graph.data_flow.values().map(|s| s.len()).sum();

    GraphSummary {
        schema_version: ir.schema_version.clone(),
        run_id: ir.run_id.clone(),
        source_project: ir.source_project.clone(),
        view_nodes: ir.view_tree.nodes.len(),
        view_roots: ir.view_tree.roots.len(),
        state_variables: ir.state_graph.variables.len(),
        derived_computations: ir.state_graph.derived.len(),
        data_flow_edges,
        events: ir.event_catalog.events.len(),
        transitions: ir.event_catalog.transitions.len(),
        effects: ir.effect_registry.effects.len(),
        style_tokens: ir.style_intent.tokens.len(),
        themes: ir.style_intent.themes.len(),
        capabilities_required: ir.capabilities.required.len(),
        capabilities_optional: ir.capabilities.optional.len(),
        accessibility_entries: ir.accessibility.entries.len(),
        warnings: ir.metadata.warnings.len(),
    }
}

fn dump_view_subtree(ir: &MigrationIr, id: &IrNodeId, depth: usize, out: &mut String) {
    let indent = "  ".repeat(depth + 1);
    if let Some(node) = ir.view_tree.nodes.get(id) {
        out.push_str(&format!(
            "{}{} ({:?}) [{}]\n",
            indent, node.name, node.kind, id
        ));
        for child_id in &node.children {
            dump_view_subtree(ir, child_id, depth + 1, out);
        }
    } else {
        out.push_str(&format!("{}[missing: {}]\n", indent, id));
    }
}

fn collect_reachable_nodes(ir: &MigrationIr) -> BTreeSet<IrNodeId> {
    let mut reachable = BTreeSet::new();
    let mut stack: Vec<IrNodeId> = ir.view_tree.roots.clone();

    while let Some(id) = stack.pop() {
        if reachable.insert(id.clone())
            && let Some(node) = ir.view_tree.nodes.get(&id)
        {
            stack.extend(node.children.iter().cloned());
        }
    }

    reachable
}

// ── Provenance Trace ────────────────────────────────────────────────────

/// A provenance entry linking an IR construct to its source.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProvenanceEntry {
    pub node_id: String,
    pub construct_kind: String,
    pub construct_name: String,
    pub file: String,
    pub line: usize,
    pub column: Option<usize>,
    pub source_name: Option<String>,
    pub policy_category: Option<String>,
}

/// Trace provenance for all constructs in the IR, optionally filtered by file.
pub fn trace_provenance(ir: &MigrationIr, file_filter: Option<&str>) -> ExplainerOutput {
    let mut entries: Vec<ProvenanceEntry> = Vec::new();

    // View nodes.
    for (id, node) in &ir.view_tree.nodes {
        if matches_filter(&node.provenance, file_filter) {
            entries.push(provenance_entry(
                id,
                "view_node",
                &node.name,
                &node.provenance,
            ));
        }
    }

    // State variables.
    for (id, var) in &ir.state_graph.variables {
        if matches_filter(&var.provenance, file_filter) {
            entries.push(provenance_entry(
                id,
                "state_variable",
                &var.name,
                &var.provenance,
            ));
        }
    }

    // Derived state.
    for (id, derived) in &ir.state_graph.derived {
        if matches_filter(&derived.provenance, file_filter) {
            entries.push(provenance_entry(
                id,
                "derived_state",
                &derived.name,
                &derived.provenance,
            ));
        }
    }

    // Events.
    for (id, event) in &ir.event_catalog.events {
        if matches_filter(&event.provenance, file_filter) {
            entries.push(provenance_entry(
                id,
                "event",
                &event.name,
                &event.provenance,
            ));
        }
    }

    // Effects.
    for (id, effect) in &ir.effect_registry.effects {
        if matches_filter(&effect.provenance, file_filter) {
            entries.push(provenance_entry(
                id,
                "effect",
                &effect.name,
                &effect.provenance,
            ));
        }
    }

    // Sort by file, then line for stable output.
    entries.sort_by(|a, b| a.file.cmp(&b.file).then(a.line.cmp(&b.line)));

    let mut text = String::new();
    text.push_str(&format!(
        "=== Provenance Trace{} ===\n",
        file_filter
            .map(|f| format!(" (file: {f})"))
            .unwrap_or_default()
    ));
    text.push_str(&format!("{} entries\n\n", entries.len()));

    let mut current_file = String::new();
    for entry in &entries {
        if entry.file != current_file {
            current_file.clone_from(&entry.file);
            text.push_str(&format!("── {} ──\n", current_file));
        }
        text.push_str(&format!(
            "  L{:<5} {} {} [{}]\n",
            entry.line, entry.construct_kind, entry.construct_name, entry.node_id
        ));
    }

    let data = serde_json::to_value(&entries).unwrap_or_default();

    ExplainerOutput {
        text,
        data,
        kind: OutputKind::ProvenanceTrace,
    }
}

fn matches_filter(prov: &Provenance, filter: Option<&str>) -> bool {
    match filter {
        None => true,
        Some(f) => prov.file.contains(f),
    }
}

fn provenance_entry(id: &IrNodeId, kind: &str, name: &str, prov: &Provenance) -> ProvenanceEntry {
    ProvenanceEntry {
        node_id: id.to_string(),
        construct_kind: kind.to_string(),
        construct_name: name.to_string(),
        file: prov.file.clone(),
        line: prov.line,
        column: prov.column,
        source_name: prov.source_name.clone(),
        policy_category: prov.policy_category.clone(),
    }
}

// ── Pass Diffs ──────────────────────────────────────────────────────────

/// Classification of semantic changes from a normalization pass.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ChangeClass {
    /// Structural reordering (no semantic change).
    Ordering,
    /// Syntax desugaring (simplification, same semantics).
    Desugaring,
    /// Dead code/state removal (pruning).
    Pruning,
    /// Style/token deduplication.
    Deduplication,
    /// Provenance normalization (path cleanup).
    ProvenanceCleanup,
    /// Unknown or mixed change.
    Mixed,
}

/// A diff between two IR snapshots across a normalization pass.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PassDiff {
    /// Which pass was applied.
    pub pass_name: String,
    /// Classification of the changes.
    pub change_class: ChangeClass,
    /// Number of mutations.
    pub mutation_count: usize,
    /// Human-readable description of what changed.
    pub description: String,
    /// Specific mutations (for machine consumption).
    pub mutations: Vec<Mutation>,
}

/// A single mutation within a pass.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Mutation {
    pub target_kind: String,
    pub target_id: Option<String>,
    pub action: MutationAction,
    pub detail: String,
}

/// What kind of mutation occurred.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum MutationAction {
    Added,
    Removed,
    Reordered,
    Modified,
    Merged,
}

/// Result of computing pass diffs.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PassDiffResult {
    pub passes: Vec<PassDiff>,
    pub total_mutations: usize,
    pub normalization_report: NormalizationReportView,
}

/// Serializable view of NormalizationReport.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NormalizationReportView {
    pub ordering_changes: usize,
    pub fragments_desugared: usize,
    pub dead_state_pruned: usize,
    pub dead_events_pruned: usize,
    pub tokens_merged: usize,
    pub provenance_normalized: usize,
    pub total: usize,
}

/// Compute diffs for each normalization pass applied to an IR.
///
/// Takes a snapshot before normalization, applies it, and returns
/// structured diffs for each pass that made changes.
pub fn compute_pass_diffs(ir: &mut MigrationIr) -> ExplainerOutput {
    // Snapshot before.
    let before = snapshot_ir(ir);

    // Apply normalization.
    let report = ir_normalize::normalize(ir);

    // Snapshot after.
    let after = snapshot_ir(ir);

    // Build per-pass diffs.
    let mut passes = Vec::new();

    if report.ordering_changes > 0 {
        let mutations = diff_ordering(&before, &after);
        passes.push(PassDiff {
            pass_name: "canonicalize_ordering".to_string(),
            change_class: ChangeClass::Ordering,
            mutation_count: report.ordering_changes,
            description: format!("{} children/root lists reordered", report.ordering_changes),
            mutations,
        });
    }

    if report.fragments_desugared > 0 {
        passes.push(PassDiff {
            pass_name: "desugar_fragments".to_string(),
            change_class: ChangeClass::Desugaring,
            mutation_count: report.fragments_desugared,
            description: format!(
                "{} fragments desugared (children hoisted)",
                report.fragments_desugared
            ),
            mutations: diff_fragments(&before, &after),
        });
    }

    if report.dead_state_pruned > 0 {
        let mutations = diff_state_pruning(&before, &after);
        passes.push(PassDiff {
            pass_name: "prune_dead_state".to_string(),
            change_class: ChangeClass::Pruning,
            mutation_count: report.dead_state_pruned,
            description: format!(
                "{} unreferenced state variables removed",
                report.dead_state_pruned
            ),
            mutations,
        });
    }

    if report.dead_events_pruned > 0 {
        let mutations = diff_event_pruning(&before, &after);
        passes.push(PassDiff {
            pass_name: "prune_dead_events".to_string(),
            change_class: ChangeClass::Pruning,
            mutation_count: report.dead_events_pruned,
            description: format!(
                "{} transitionless events removed",
                report.dead_events_pruned
            ),
            mutations,
        });
    }

    if report.tokens_merged > 0 {
        passes.push(PassDiff {
            pass_name: "merge_duplicate_tokens".to_string(),
            change_class: ChangeClass::Deduplication,
            mutation_count: report.tokens_merged,
            description: format!("{} duplicate style tokens merged", report.tokens_merged),
            mutations: diff_tokens(&before, &after),
        });
    }

    if report.provenance_normalized > 0 {
        passes.push(PassDiff {
            pass_name: "normalize_provenance".to_string(),
            change_class: ChangeClass::ProvenanceCleanup,
            mutation_count: report.provenance_normalized,
            description: format!(
                "{} provenance paths normalized",
                report.provenance_normalized
            ),
            mutations: Vec::new(), // Path changes are too fine-grained to enumerate.
        });
    }

    let report_view = report_to_view(&report);
    let total_mutations = report.total();

    let result = PassDiffResult {
        passes: passes.clone(),
        total_mutations,
        normalization_report: report_view,
    };

    // Build text.
    let mut text = String::new();
    text.push_str("=== Normalization Pass Diffs ===\n");
    text.push_str(&format!("Total mutations: {}\n\n", total_mutations));

    for pass in &passes {
        text.push_str(&format!(
            "── {} ({:?}, {} mutations) ──\n",
            pass.pass_name, pass.change_class, pass.mutation_count
        ));
        text.push_str(&format!("  {}\n", pass.description));
        for m in &pass.mutations {
            text.push_str(&format!(
                "    {:?} {} {}\n",
                m.action, m.target_kind, m.detail
            ));
        }
        text.push('\n');
    }

    if passes.is_empty() {
        text.push_str("  No mutations — IR is already normalized.\n");
    }

    let data = serde_json::to_value(&result).unwrap_or_default();

    ExplainerOutput {
        text,
        data,
        kind: OutputKind::PassDiff,
    }
}

fn report_to_view(report: &NormalizationReport) -> NormalizationReportView {
    NormalizationReportView {
        ordering_changes: report.ordering_changes,
        fragments_desugared: report.fragments_desugared,
        dead_state_pruned: report.dead_state_pruned,
        dead_events_pruned: report.dead_events_pruned,
        tokens_merged: report.tokens_merged,
        provenance_normalized: report.provenance_normalized,
        total: report.total(),
    }
}

// ── Snapshot helpers (for diffing) ──────────────────────────────────────

/// Lightweight snapshot of IR structure for diffing.
#[derive(Clone)]
struct IrSnapshot {
    view_node_ids: BTreeSet<IrNodeId>,
    view_roots: Vec<IrNodeId>,
    children_order: BTreeMap<IrNodeId, Vec<IrNodeId>>,
    state_ids: BTreeSet<IrNodeId>,
    event_ids: BTreeSet<IrNodeId>,
    token_names: BTreeSet<String>,
}

fn snapshot_ir(ir: &MigrationIr) -> IrSnapshot {
    IrSnapshot {
        view_node_ids: ir.view_tree.nodes.keys().cloned().collect(),
        view_roots: ir.view_tree.roots.clone(),
        children_order: ir
            .view_tree
            .nodes
            .iter()
            .map(|(id, n)| (id.clone(), n.children.clone()))
            .collect(),
        state_ids: ir.state_graph.variables.keys().cloned().collect(),
        event_ids: ir.event_catalog.events.keys().cloned().collect(),
        token_names: ir.style_intent.tokens.keys().cloned().collect(),
    }
}

fn diff_ordering(before: &IrSnapshot, after: &IrSnapshot) -> Vec<Mutation> {
    let mut mutations = Vec::new();

    // Check root order changes.
    if before.view_roots != after.view_roots {
        mutations.push(Mutation {
            target_kind: "view_tree".to_string(),
            target_id: None,
            action: MutationAction::Reordered,
            detail: "Root list reordered".to_string(),
        });
    }

    // Check children order changes.
    for (id, before_children) in &before.children_order {
        if let Some(after_children) = after.children_order.get(id)
            && before_children != after_children
        {
            mutations.push(Mutation {
                target_kind: "view_node".to_string(),
                target_id: Some(id.to_string()),
                action: MutationAction::Reordered,
                detail: format!(
                    "Children of {} reordered ({} children)",
                    id,
                    after_children.len()
                ),
            });
        }
    }

    mutations
}

fn diff_fragments(before: &IrSnapshot, after: &IrSnapshot) -> Vec<Mutation> {
    let removed: Vec<_> = before
        .view_node_ids
        .difference(&after.view_node_ids)
        .collect();

    removed
        .into_iter()
        .map(|id| Mutation {
            target_kind: "view_node".to_string(),
            target_id: Some(id.to_string()),
            action: MutationAction::Removed,
            detail: format!("Fragment {} desugared (children hoisted)", id),
        })
        .collect()
}

fn diff_state_pruning(before: &IrSnapshot, after: &IrSnapshot) -> Vec<Mutation> {
    let pruned: Vec<_> = before.state_ids.difference(&after.state_ids).collect();

    pruned
        .into_iter()
        .map(|id| Mutation {
            target_kind: "state_variable".to_string(),
            target_id: Some(id.to_string()),
            action: MutationAction::Removed,
            detail: format!("Unreferenced state variable {} pruned", id),
        })
        .collect()
}

fn diff_event_pruning(before: &IrSnapshot, after: &IrSnapshot) -> Vec<Mutation> {
    let pruned: Vec<_> = before.event_ids.difference(&after.event_ids).collect();

    pruned
        .into_iter()
        .map(|id| Mutation {
            target_kind: "event".to_string(),
            target_id: Some(id.to_string()),
            action: MutationAction::Removed,
            detail: format!("Transitionless event {} pruned", id),
        })
        .collect()
}

fn diff_tokens(before: &IrSnapshot, after: &IrSnapshot) -> Vec<Mutation> {
    let removed: Vec<_> = before.token_names.difference(&after.token_names).collect();

    removed
        .into_iter()
        .map(|name| Mutation {
            target_kind: "style_token".to_string(),
            target_id: Some(name.clone()),
            action: MutationAction::Merged,
            detail: format!("Duplicate token '{}' merged", name),
        })
        .collect()
}

// ── IR Summary (for triage/certification) ───────────────────────────────

/// Compact summary for failure triage and certification reports.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TriageSummary {
    /// Overall health assessment.
    pub health: HealthStatus,
    /// Key metrics.
    pub metrics: GraphSummary,
    /// Issues requiring attention.
    pub issues: Vec<TriageIssue>,
}

/// Health status of the IR.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum HealthStatus {
    /// IR is clean and well-formed.
    Healthy,
    /// IR has minor issues (warnings, non-critical).
    Degraded,
    /// IR has significant issues.
    Unhealthy,
}

/// An issue found during triage.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TriageIssue {
    pub severity: TriageSeverity,
    pub category: String,
    pub message: String,
    pub affected_nodes: Vec<String>,
}

/// Severity of a triage issue.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum TriageSeverity {
    Info,
    Warning,
    Error,
}

/// Generate a triage summary for an IR instance.
pub fn triage_summary(ir: &MigrationIr) -> ExplainerOutput {
    let metrics = build_graph_summary(ir);
    let mut issues = Vec::new();

    // Check for orphan view nodes.
    let reachable = collect_reachable_nodes(ir);
    let orphans: Vec<_> = ir
        .view_tree
        .nodes
        .keys()
        .filter(|id| !reachable.contains(*id))
        .map(|id| id.to_string())
        .collect();
    if !orphans.is_empty() {
        issues.push(TriageIssue {
            severity: TriageSeverity::Warning,
            category: "view_tree".to_string(),
            message: format!(
                "{} view nodes are not reachable from any root",
                orphans.len()
            ),
            affected_nodes: orphans,
        });
    }

    // Check for effects without cleanup.
    let no_cleanup: Vec<_> = ir
        .effect_registry
        .effects
        .iter()
        .filter(|(_, e)| !e.has_cleanup && needs_cleanup(&e.kind))
        .map(|(id, _)| id.to_string())
        .collect();
    if !no_cleanup.is_empty() {
        issues.push(TriageIssue {
            severity: TriageSeverity::Warning,
            category: "effects".to_string(),
            message: format!(
                "{} subscription/timer effects lack cleanup (potential leak)",
                no_cleanup.len()
            ),
            affected_nodes: no_cleanup,
        });
    }

    // Check for empty provenance.
    let empty_prov: Vec<_> = ir
        .view_tree
        .nodes
        .iter()
        .filter(|(_, n)| n.provenance.file.is_empty())
        .map(|(id, _)| id.to_string())
        .collect();
    if !empty_prov.is_empty() {
        issues.push(TriageIssue {
            severity: TriageSeverity::Error,
            category: "provenance".to_string(),
            message: format!(
                "{} view nodes have empty provenance (untraceable)",
                empty_prov.len()
            ),
            affected_nodes: empty_prov,
        });
    }

    // Check for warnings in metadata.
    if !ir.metadata.warnings.is_empty() {
        issues.push(TriageIssue {
            severity: TriageSeverity::Info,
            category: "metadata".to_string(),
            message: format!(
                "{} IR warnings recorded during construction",
                ir.metadata.warnings.len()
            ),
            affected_nodes: Vec::new(),
        });
    }

    let health = if issues.iter().any(|i| i.severity == TriageSeverity::Error) {
        HealthStatus::Unhealthy
    } else if issues.iter().any(|i| i.severity == TriageSeverity::Warning) {
        HealthStatus::Degraded
    } else {
        HealthStatus::Healthy
    };

    let summary = TriageSummary {
        health: health.clone(),
        metrics,
        issues: issues.clone(),
    };

    let mut text = String::new();
    text.push_str("=== Triage Summary ===\n");
    text.push_str(&format!("Health: {:?}\n", health));
    text.push_str(&format!("Issues: {}\n\n", issues.len()));

    for issue in &issues {
        text.push_str(&format!(
            "  [{:?}] {}: {}\n",
            issue.severity, issue.category, issue.message
        ));
        if !issue.affected_nodes.is_empty() {
            let preview: Vec<_> = issue.affected_nodes.iter().take(3).collect();
            text.push_str(&format!("    affected: {:?}", preview));
            if issue.affected_nodes.len() > 3 {
                text.push_str(&format!(" (+{} more)", issue.affected_nodes.len() - 3));
            }
            text.push('\n');
        }
    }

    let data = serde_json::to_value(&summary).unwrap_or_default();

    ExplainerOutput {
        text,
        data,
        kind: OutputKind::Summary,
    }
}

fn needs_cleanup(kind: &EffectKind) -> bool {
    matches!(
        kind,
        EffectKind::Subscription | EffectKind::Timer | EffectKind::Process
    )
}

// ── Tests ───────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    use crate::migration_ir::{
        Capability, EffectDecl, EventDecl, EventKind, EventTransition, IrBuilder, StateScope,
        StateVariable, ViewNode, ViewNodeKind,
    };

    fn build_test_ir() -> MigrationIr {
        let mut builder = IrBuilder::new("test-run".to_string(), "test-project".to_string());
        builder.set_source_file_count(2);

        let root_id = crate::migration_ir::make_node_id(b"root");
        let child_id = crate::migration_ir::make_node_id(b"child");

        builder.add_root(root_id.clone());
        builder.add_view_node(ViewNode {
            id: root_id.clone(),
            kind: ViewNodeKind::Component,
            name: "App".to_string(),
            children: vec![child_id.clone()],
            props: Vec::new(),
            slots: Vec::new(),
            conditions: Vec::new(),
            provenance: Provenance {
                file: "src/App.tsx".to_string(),
                line: 1,
                column: None,
                source_name: Some("App".to_string()),
                policy_category: None,
            },
        });
        builder.add_view_node(ViewNode {
            id: child_id.clone(),
            kind: ViewNodeKind::Element,
            name: "div".to_string(),
            children: Vec::new(),
            props: Vec::new(),
            slots: Vec::new(),
            conditions: Vec::new(),
            provenance: Provenance {
                file: "src/App.tsx".to_string(),
                line: 5,
                column: None,
                source_name: None,
                policy_category: None,
            },
        });

        let state_id = crate::migration_ir::make_node_id(b"count");
        builder.add_state_variable(StateVariable {
            id: state_id.clone(),
            name: "count".to_string(),
            scope: StateScope::Local,
            type_annotation: Some("number".to_string()),
            initial_value: Some("0".to_string()),
            readers: BTreeSet::from([root_id.clone()]),
            writers: BTreeSet::new(),
            provenance: Provenance {
                file: "src/App.tsx".to_string(),
                line: 3,
                column: None,
                source_name: Some("App::count".to_string()),
                policy_category: Some("state".to_string()),
            },
        });

        let event_id = crate::migration_ir::make_node_id(b"click-event");
        builder.add_event(EventDecl {
            id: event_id.clone(),
            name: "onClick".to_string(),
            kind: EventKind::UserInput,
            source_node: Some(child_id.clone()),
            payload_type: None,
            provenance: Provenance {
                file: "src/App.tsx".to_string(),
                line: 8,
                column: None,
                source_name: Some("handleClick".to_string()),
                policy_category: Some("event".to_string()),
            },
        });
        builder.add_transition(EventTransition {
            event_id: event_id.clone(),
            target_state: state_id.clone(),
            action_snippet: "setCount(c + 1)".to_string(),
            guards: Vec::new(),
        });

        let effect_id = crate::migration_ir::make_node_id(b"timer-effect");
        builder.add_effect(EffectDecl {
            id: effect_id,
            name: "App::timer".to_string(),
            kind: EffectKind::Timer,
            dependencies: BTreeSet::new(),
            has_cleanup: false,
            reads: BTreeSet::new(),
            writes: BTreeSet::new(),
            provenance: Provenance {
                file: "src/App.tsx".to_string(),
                line: 10,
                column: None,
                source_name: Some("useEffect".to_string()),
                policy_category: Some("effect".to_string()),
            },
        });

        builder.require_capability(Capability::KeyboardInput);
        builder.require_capability(Capability::Timers);

        builder.build()
    }

    // ── Graph dump ─────────────────────────────────────────────────────

    #[test]
    fn graph_dump_produces_text_and_data() {
        let ir = build_test_ir();
        let output = dump_graph(&ir);

        assert_eq!(output.kind, OutputKind::GraphDump);
        assert!(output.text.contains("View Tree"));
        assert!(output.text.contains("App"));
        assert!(output.text.contains("State Graph"));
        assert!(output.text.contains("count"));
        assert!(output.text.contains("Effects"));
        assert!(!output.data.is_null());
    }

    #[test]
    fn graph_dump_shows_capabilities() {
        let ir = build_test_ir();
        let output = dump_graph(&ir);

        assert!(output.text.contains("Capabilities"));
        assert!(output.text.contains("KeyboardInput"));
        assert!(output.text.contains("Timers"));
    }

    #[test]
    fn graph_summary_counts_are_correct() {
        let ir = build_test_ir();
        let summary = build_graph_summary(&ir);

        assert_eq!(summary.view_nodes, 2);
        assert_eq!(summary.view_roots, 1);
        assert_eq!(summary.state_variables, 1);
        assert_eq!(summary.events, 1);
        assert_eq!(summary.transitions, 1);
        assert_eq!(summary.effects, 1);
        assert_eq!(summary.capabilities_required, 2);
    }

    #[test]
    fn graph_dump_empty_ir() {
        let builder = IrBuilder::new("empty".to_string(), "empty-project".to_string());
        let ir = builder.build();
        let output = dump_graph(&ir);

        assert!(output.text.contains("0 nodes"));
        assert!(output.text.contains("0 roots"));
    }

    // ── Provenance trace ───────────────────────────────────────────────

    #[test]
    fn provenance_trace_all_constructs() {
        let ir = build_test_ir();
        let output = trace_provenance(&ir, None);

        assert_eq!(output.kind, OutputKind::ProvenanceTrace);
        assert!(output.text.contains("src/App.tsx"));
        assert!(output.text.contains("view_node"));
        assert!(output.text.contains("state_variable"));
        assert!(output.text.contains("event"));
        assert!(output.text.contains("effect"));
    }

    #[test]
    fn provenance_trace_file_filter() {
        let ir = build_test_ir();
        let output = trace_provenance(&ir, Some("App.tsx"));

        // Should include entries from App.tsx.
        let entries: Vec<ProvenanceEntry> = serde_json::from_value(output.data).unwrap();
        assert!(!entries.is_empty());
        assert!(entries.iter().all(|e| e.file.contains("App.tsx")));
    }

    #[test]
    fn provenance_trace_nonexistent_filter() {
        let ir = build_test_ir();
        let output = trace_provenance(&ir, Some("nonexistent.tsx"));

        let entries: Vec<ProvenanceEntry> = serde_json::from_value(output.data).unwrap();
        assert!(entries.is_empty());
    }

    #[test]
    fn provenance_entries_sorted_by_file_and_line() {
        let ir = build_test_ir();
        let output = trace_provenance(&ir, None);
        let entries: Vec<ProvenanceEntry> = serde_json::from_value(output.data).unwrap();

        for window in entries.windows(2) {
            let cmp = window[0].file.cmp(&window[1].file);
            match cmp {
                std::cmp::Ordering::Less => {}
                std::cmp::Ordering::Equal => {
                    assert!(window[0].line <= window[1].line);
                }
                std::cmp::Ordering::Greater => {
                    panic!("Entries not sorted by file");
                }
            }
        }
    }

    // ── Pass diffs ─────────────────────────────────────────────────────

    #[test]
    fn pass_diffs_on_clean_ir() {
        let mut ir = build_test_ir();
        // Pre-normalize to make it clean.
        ir_normalize::normalize(&mut ir);

        let output = compute_pass_diffs(&mut ir);

        assert_eq!(output.kind, OutputKind::PassDiff);
        let result: PassDiffResult = serde_json::from_value(output.data).unwrap();
        assert_eq!(result.total_mutations, 0);
        assert!(result.passes.is_empty());
        assert!(output.text.contains("already normalized"));
    }

    #[test]
    fn pass_diffs_detect_pruning() {
        let mut ir = build_test_ir();

        // Add an unreferenced state variable (will be pruned).
        let orphan_state = crate::migration_ir::make_node_id(b"orphan-state");
        ir.state_graph.variables.insert(
            orphan_state.clone(),
            StateVariable {
                id: orphan_state,
                name: "unused".to_string(),
                scope: StateScope::Local,
                type_annotation: None,
                initial_value: None,
                readers: BTreeSet::new(),
                writers: BTreeSet::new(),
                provenance: Provenance {
                    file: "src/App.tsx".to_string(),
                    line: 20,
                    column: None,
                    source_name: None,
                    policy_category: None,
                },
            },
        );

        let output = compute_pass_diffs(&mut ir);
        let result: PassDiffResult = serde_json::from_value(output.data).unwrap();

        assert!(result.total_mutations > 0);
        assert!(
            result
                .passes
                .iter()
                .any(|p| p.pass_name == "prune_dead_state")
        );
        assert!(
            result
                .passes
                .iter()
                .any(|p| p.change_class == ChangeClass::Pruning)
        );
    }

    #[test]
    fn pass_diffs_stable_and_deterministic() {
        let make_ir = || {
            let mut ir = build_test_ir();
            // Add orphan state for pruning.
            let orphan = crate::migration_ir::make_node_id(b"orphan-state");
            ir.state_graph.variables.insert(
                orphan.clone(),
                StateVariable {
                    id: orphan,
                    name: "unused".to_string(),
                    scope: StateScope::Local,
                    type_annotation: None,
                    initial_value: None,
                    readers: BTreeSet::new(),
                    writers: BTreeSet::new(),
                    provenance: Provenance {
                        file: "src/App.tsx".to_string(),
                        line: 20,
                        column: None,
                        source_name: None,
                        policy_category: None,
                    },
                },
            );
            ir
        };

        let mut ir1 = make_ir();
        let mut ir2 = make_ir();
        let out1 = compute_pass_diffs(&mut ir1);
        let out2 = compute_pass_diffs(&mut ir2);

        let r1: PassDiffResult = serde_json::from_value(out1.data).unwrap();
        let r2: PassDiffResult = serde_json::from_value(out2.data).unwrap();

        assert_eq!(r1.total_mutations, r2.total_mutations);
        assert_eq!(r1.passes.len(), r2.passes.len());
    }

    // ── Triage summary ─────────────────────────────────────────────────

    #[test]
    fn triage_healthy_ir() {
        let ir = build_test_ir();
        let output = triage_summary(&ir);

        assert_eq!(output.kind, OutputKind::Summary);
        let summary: TriageSummary = serde_json::from_value(output.data).unwrap();

        // Timer without cleanup → Degraded, not Healthy.
        assert_eq!(summary.health, HealthStatus::Degraded);
    }

    #[test]
    fn triage_detects_missing_cleanup() {
        let ir = build_test_ir();
        let output = triage_summary(&ir);
        let summary: TriageSummary = serde_json::from_value(output.data).unwrap();

        assert!(
            summary
                .issues
                .iter()
                .any(|i| i.category == "effects" && i.message.contains("cleanup"))
        );
    }

    #[test]
    fn triage_detects_empty_provenance() {
        let mut ir = build_test_ir();

        // Corrupt provenance of a node.
        if let Some(node) = ir.view_tree.nodes.values_mut().next() {
            node.provenance.file = String::new();
        }

        let output = triage_summary(&ir);
        let summary: TriageSummary = serde_json::from_value(output.data).unwrap();

        assert_eq!(summary.health, HealthStatus::Unhealthy);
        assert!(summary.issues.iter().any(|i| i.category == "provenance"));
    }

    #[test]
    fn triage_empty_ir_is_healthy() {
        let builder = IrBuilder::new("empty".to_string(), "empty".to_string());
        let ir = builder.build();
        let output = triage_summary(&ir);
        let summary: TriageSummary = serde_json::from_value(output.data).unwrap();

        assert_eq!(summary.health, HealthStatus::Healthy);
        assert!(summary.issues.is_empty());
    }

    // ── Output kind ────────────────────────────────────────────────────

    #[test]
    fn output_kinds_serialize() {
        let kinds = [
            OutputKind::GraphDump,
            OutputKind::ProvenanceTrace,
            OutputKind::PassDiff,
            OutputKind::Summary,
        ];

        for kind in &kinds {
            let json = serde_json::to_string(kind).unwrap();
            let parsed: OutputKind = serde_json::from_str(&json).unwrap();
            assert_eq!(*kind, parsed);
        }
    }

    // ── Reachable nodes ────────────────────────────────────────────────

    #[test]
    fn reachable_nodes_from_roots() {
        let ir = build_test_ir();
        let reachable = collect_reachable_nodes(&ir);

        // Both root and child should be reachable.
        assert_eq!(reachable.len(), 2);
        for id in reachable {
            assert!(ir.view_tree.nodes.contains_key(&id));
        }
    }

    #[test]
    fn orphan_nodes_not_reachable() {
        let mut ir = build_test_ir();
        let orphan = crate::migration_ir::make_node_id(b"orphan-view");
        ir.view_tree.nodes.insert(
            orphan.clone(),
            ViewNode {
                id: orphan.clone(),
                kind: ViewNodeKind::Element,
                name: "orphan".to_string(),
                children: Vec::new(),
                props: Vec::new(),
                slots: Vec::new(),
                conditions: Vec::new(),
                provenance: Provenance {
                    file: "orphan.tsx".to_string(),
                    line: 1,
                    column: None,
                    source_name: None,
                    policy_category: None,
                },
            },
        );

        let reachable = collect_reachable_nodes(&ir);
        assert!(!reachable.contains(&orphan));
    }

    // ── Change class serialization ─────────────────────────────────────

    #[test]
    fn change_classes_roundtrip() {
        let classes = [
            ChangeClass::Ordering,
            ChangeClass::Desugaring,
            ChangeClass::Pruning,
            ChangeClass::Deduplication,
            ChangeClass::ProvenanceCleanup,
            ChangeClass::Mixed,
        ];

        for class in &classes {
            let json = serde_json::to_string(class).unwrap();
            let parsed: ChangeClass = serde_json::from_str(&json).unwrap();
            assert_eq!(*class, parsed);
        }
    }

    // ── Mutation actions ───────────────────────────────────────────────

    #[test]
    fn mutation_actions_roundtrip() {
        let actions = [
            MutationAction::Added,
            MutationAction::Removed,
            MutationAction::Reordered,
            MutationAction::Modified,
            MutationAction::Merged,
        ];

        for action in &actions {
            let json = serde_json::to_string(action).unwrap();
            let parsed: MutationAction = serde_json::from_str(&json).unwrap();
            assert_eq!(*action, parsed);
        }
    }
}
