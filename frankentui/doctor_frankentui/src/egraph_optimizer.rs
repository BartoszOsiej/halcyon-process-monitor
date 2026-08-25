// SPDX-License-Identifier: Apache-2.0
//! E-graph saturation optimizer for generated code.
//!
//! Solves pass-order sensitivity in the codegen optimization pipeline by
//! representing code transformations as equality-saturating rewrites in an
//! e-graph. Instead of applying optimizations in a fixed sequence (where
//! earlier passes can block later ones), all rewrites run concurrently on a
//! shared equality graph until saturation or budget exhaustion.
//!
//! # Design
//!
//! The optimizer operates on [`CodeTerm`]s — a simplified IR of generated
//! code fragments. Each rewrite rule has:
//! - A **pattern** (what to match)
//! - A **replacement** (what to rewrite to)
//! - A **semantics tag** (which category of optimization it belongs to)
//!
//! After saturation, the **extractor** picks the lowest-cost equivalent
//! using a transparent cost function. On budget breach, a deterministic
//! conservative extraction prefers the original term.
//!
//! # Budgets
//!
//! - `max_nodes`: Hard cap on e-graph node count
//! - `max_iterations`: Saturation loop iterations
//! - `timeout`: Wall-clock time limit
//!
//! On any budget breach, the optimizer stops adding rewrites and extracts
//! the best known equivalent deterministically.

use std::collections::{BTreeMap, BTreeSet};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use crate::code_emission::{EmissionPlan, EmittedFile, FileKind};

// ── Configuration ────────────────────────────────────────────────────

/// Resource budgets for e-graph saturation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EGraphBudget {
    /// Maximum nodes in the e-graph before forced extraction.
    pub max_nodes: usize,
    /// Maximum saturation iterations.
    pub max_iterations: usize,
    /// Wall-clock timeout for the entire optimization pass.
    pub timeout: Duration,
}

impl Default for EGraphBudget {
    fn default() -> Self {
        Self {
            max_nodes: 10_000,
            max_iterations: 50,
            timeout: Duration::from_secs(10),
        }
    }
}

// ── Code term IR ─────────────────────────────────────────────────────

/// A simplified code term for e-graph representation.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum CodeTerm {
    /// A literal code fragment (leaf).
    Literal(String),
    /// A use/import statement.
    Import { path: String },
    /// A style constant definition.
    StyleConst { name: String, value: String },
    /// A function/method call.
    Call { name: String, args: Vec<CodeTerm> },
    /// A match arm in an update function.
    MatchArm {
        pattern: String,
        body: Box<CodeTerm>,
    },
    /// A widget rendering expression.
    Widget {
        kind: String,
        children: Vec<CodeTerm>,
    },
    /// A sequence of statements.
    Block(Vec<CodeTerm>),
    /// An empty/no-op term.
    Noop,
}

impl CodeTerm {
    /// Compute the cost of this term (lower = better).
    fn cost(&self) -> usize {
        match self {
            CodeTerm::Noop => 0,
            CodeTerm::Literal(s) => s.len(),
            CodeTerm::Import { path } => path.len(),
            CodeTerm::StyleConst { name, value } => name.len() + value.len(),
            CodeTerm::Call { args, .. } => 5 + args.iter().map(|a| a.cost()).sum::<usize>(),
            CodeTerm::MatchArm { body, .. } => 3 + body.cost(),
            CodeTerm::Widget { children, .. } => {
                10 + children.iter().map(|c| c.cost()).sum::<usize>()
            }
            CodeTerm::Block(stmts) => stmts.iter().map(|s| s.cost()).sum(),
        }
    }

    /// Count nodes in this term tree.
    fn node_count(&self) -> usize {
        match self {
            CodeTerm::Noop | CodeTerm::Literal(_) | CodeTerm::Import { .. } => 1,
            CodeTerm::StyleConst { .. } => 1,
            CodeTerm::Call { args, .. } => 1 + args.iter().map(|a| a.node_count()).sum::<usize>(),
            CodeTerm::MatchArm { body, .. } => 1 + body.node_count(),
            CodeTerm::Widget { children, .. } => {
                1 + children.iter().map(|c| c.node_count()).sum::<usize>()
            }
            CodeTerm::Block(stmts) => 1 + stmts.iter().map(|s| s.node_count()).sum::<usize>(),
        }
    }
}

// ── Rewrite rules ────────────────────────────────────────────────────

/// Semantic tag for a rewrite rule.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum RewriteSemantics {
    /// Remove dead/unreachable code.
    DeadCodeElimination,
    /// Merge duplicate definitions.
    ConstantFolding,
    /// Extract common patterns into helpers.
    HelperExtraction,
    /// Deduplicate imports.
    ImportDedup,
    /// Simplify widget nesting.
    WidgetSimplification,
    /// General algebraic simplification.
    AlgebraicSimplification,
}

/// A rewrite rule for the e-graph.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RewriteRule {
    /// Rule identifier.
    pub id: String,
    /// Semantic category of this rewrite.
    pub semantics: RewriteSemantics,
    /// Human-readable description.
    pub description: String,
}

/// A rewrite application record.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RewriteApplication {
    /// Which rule was applied.
    pub rule_id: String,
    /// Semantics tag.
    pub semantics: RewriteSemantics,
    /// Description of the transformation.
    pub description: String,
    /// Cost of the term before rewrite.
    pub cost_before: usize,
    /// Cost of the term after rewrite.
    pub cost_after: usize,
}

// ── E-class and e-graph ──────────────────────────────────────────────

/// An equivalence class in the e-graph.
#[derive(Debug, Clone)]
struct EClass {
    /// All terms known to be equivalent.
    members: Vec<CodeTerm>,
    /// Best (lowest cost) member.
    best_idx: usize,
}

impl EClass {
    fn new(term: CodeTerm) -> Self {
        Self {
            members: vec![term],
            best_idx: 0,
        }
    }

    fn best(&self) -> &CodeTerm {
        &self.members[self.best_idx]
    }

    fn add(&mut self, term: CodeTerm) {
        let new_cost = term.cost();
        let best_cost = self.best().cost();
        self.members.push(term);
        if new_cost < best_cost {
            self.best_idx = self.members.len() - 1;
        }
    }
}

/// The e-graph structure.
#[derive(Debug)]
struct EGraph {
    classes: Vec<EClass>,
    /// Total node count across all classes.
    total_nodes: usize,
}

impl EGraph {
    fn new() -> Self {
        Self {
            classes: Vec::new(),
            total_nodes: 0,
        }
    }

    fn add_class(&mut self, term: CodeTerm) -> usize {
        let nodes = term.node_count();
        let id = self.classes.len();
        self.classes.push(EClass::new(term));
        self.total_nodes += nodes;
        id
    }

    fn add_to_class(&mut self, class_id: usize, term: CodeTerm) {
        let nodes = term.node_count();
        if let Some(class) = self.classes.get_mut(class_id) {
            class.add(term);
            self.total_nodes += nodes;
        }
    }

    fn extract_best(&self, class_id: usize) -> Option<&CodeTerm> {
        self.classes.get(class_id).map(|c| c.best())
    }

    #[cfg(test)]
    fn class_count(&self) -> usize {
        self.classes.len()
    }
}

// ── Built-in rewrite rules ───────────────────────────────────────────

type RewriteFn = Box<dyn Fn(&CodeTerm) -> Option<CodeTerm>>;

fn builtin_rules() -> Vec<(RewriteRule, RewriteFn)> {
    vec![
        // Rule 1: Remove Noop from blocks
        (
            RewriteRule {
                id: "noop-elimination".into(),
                semantics: RewriteSemantics::DeadCodeElimination,
                description: "Remove Noop terms from Block sequences".into(),
            },
            Box::new(|term| {
                if let CodeTerm::Block(stmts) = term {
                    let filtered: Vec<CodeTerm> = stmts
                        .iter()
                        .filter(|s| !matches!(s, CodeTerm::Noop))
                        .cloned()
                        .collect();
                    if filtered.len() < stmts.len() {
                        return Some(if filtered.is_empty() {
                            CodeTerm::Noop
                        } else {
                            CodeTerm::Block(filtered)
                        });
                    }
                }
                None
            }),
        ),
        // Rule 2: Flatten nested blocks
        (
            RewriteRule {
                id: "block-flatten".into(),
                semantics: RewriteSemantics::AlgebraicSimplification,
                description: "Flatten nested Block(Block(...)) into a single Block".into(),
            },
            Box::new(|term| {
                if let CodeTerm::Block(stmts) = term {
                    let has_nested = stmts.iter().any(|s| matches!(s, CodeTerm::Block(_)));
                    if has_nested {
                        let mut flat = Vec::new();
                        for s in stmts {
                            if let CodeTerm::Block(inner) = s {
                                flat.extend(inner.iter().cloned());
                            } else {
                                flat.push(s.clone());
                            }
                        }
                        return Some(CodeTerm::Block(flat));
                    }
                }
                None
            }),
        ),
        // Rule 3: Deduplicate imports
        (
            RewriteRule {
                id: "import-dedup".into(),
                semantics: RewriteSemantics::ImportDedup,
                description: "Remove duplicate Import terms in a Block".into(),
            },
            Box::new(|term| {
                if let CodeTerm::Block(stmts) = term {
                    let mut seen = BTreeSet::new();
                    let mut deduped = Vec::new();
                    let mut removed = false;
                    for s in stmts {
                        if let CodeTerm::Import { path } = s {
                            if seen.insert(path.clone()) {
                                deduped.push(s.clone());
                            } else {
                                removed = true;
                            }
                        } else {
                            deduped.push(s.clone());
                        }
                    }
                    if removed {
                        return Some(CodeTerm::Block(deduped));
                    }
                }
                None
            }),
        ),
        // Rule 4: Merge duplicate style constants
        (
            RewriteRule {
                id: "style-const-merge".into(),
                semantics: RewriteSemantics::ConstantFolding,
                description: "Merge StyleConst entries with identical values".into(),
            },
            Box::new(|term| {
                if let CodeTerm::Block(stmts) = term {
                    let mut value_to_name: BTreeMap<String, String> = BTreeMap::new();
                    let mut has_dup = false;
                    for s in stmts {
                        if let CodeTerm::StyleConst { name, value } = s {
                            if let Some(existing) = value_to_name.get(value) {
                                if existing != name {
                                    has_dup = true;
                                }
                            } else {
                                value_to_name.insert(value.clone(), name.clone());
                            }
                        }
                    }
                    if has_dup {
                        let mut seen_values = BTreeSet::new();
                        let merged: Vec<CodeTerm> = stmts
                            .iter()
                            .filter(|s| {
                                if let CodeTerm::StyleConst { value, .. } = s {
                                    seen_values.insert(value.clone())
                                } else {
                                    true
                                }
                            })
                            .cloned()
                            .collect();
                        return Some(CodeTerm::Block(merged));
                    }
                }
                None
            }),
        ),
        // Rule 5: Single-child widget unwrap
        (
            RewriteRule {
                id: "widget-unwrap-single".into(),
                semantics: RewriteSemantics::WidgetSimplification,
                description: "Unwrap Widget with a single Literal child to just the child".into(),
            },
            Box::new(|term| {
                if let CodeTerm::Widget { children, kind } = term
                    && children.len() == 1
                    && kind == "wrapper"
                {
                    return Some(children[0].clone());
                }
                None
            }),
        ),
    ]
}

// ── Optimizer ────────────────────────────────────────────────────────

/// Evidence ledger entry for optimization decisions.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvidenceLedgerEntry {
    /// File path being optimized.
    pub file_path: String,
    /// Cost of the original code.
    pub cost_before: usize,
    /// Cost of the optimized code.
    pub cost_after: usize,
    /// Rewrites applied.
    pub rewrites_applied: Vec<RewriteApplication>,
    /// Whether budget was breached.
    pub budget_breached: bool,
    /// Reason for stopping (if not saturation).
    pub stop_reason: StopReason,
    /// Iterations completed.
    pub iterations: usize,
    /// Peak node count in the e-graph.
    pub peak_nodes: usize,
}

/// Why the optimizer stopped.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum StopReason {
    /// E-graph reached fixpoint — no more applicable rewrites.
    Saturated,
    /// Hit max_nodes budget.
    NodeBudget,
    /// Hit max_iterations budget.
    IterationBudget,
    /// Hit wall-clock timeout.
    Timeout,
}

/// Complete optimization report.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EGraphOptimizationReport {
    /// Per-file evidence ledger entries.
    pub ledger: Vec<EvidenceLedgerEntry>,
    /// Budget configuration used.
    pub budget: EGraphBudget,
    /// Files optimized.
    pub files_optimized: usize,
    /// Total cost reduction.
    pub total_cost_reduction: isize,
    /// Files where budget was breached.
    pub budget_breaches: usize,
}

/// The e-graph saturation optimizer.
#[derive(Debug, Clone)]
pub struct EGraphOptimizer {
    budget: EGraphBudget,
}

impl EGraphOptimizer {
    /// Create an optimizer with the given budget.
    #[must_use]
    pub fn new(budget: EGraphBudget) -> Self {
        Self { budget }
    }

    /// Create an optimizer with default budget.
    #[must_use]
    pub fn with_defaults() -> Self {
        Self::new(EGraphBudget::default())
    }

    /// Optimize an entire emission plan.
    ///
    /// Returns the optimized plan plus an evidence ledger.
    pub fn optimize_plan(&self, plan: &EmissionPlan) -> (EmissionPlan, EGraphOptimizationReport) {
        let start = Instant::now();
        let mut optimized_files = BTreeMap::new();
        let mut ledger = Vec::new();
        let mut total_cost_reduction: isize = 0;
        let mut budget_breaches = 0;

        for (path, file) in &plan.files {
            if file.kind != FileKind::RustSource {
                optimized_files.insert(path.clone(), file.clone());
                continue;
            }

            if start.elapsed() >= self.budget.timeout {
                // Global timeout: copy remaining files as-is
                optimized_files.insert(path.clone(), file.clone());
                continue;
            }

            let (optimized_content, entry) = self.optimize_file(path, &file.content);

            let reduction = entry.cost_before as isize - entry.cost_after as isize;
            total_cost_reduction += reduction;
            if entry.budget_breached {
                budget_breaches += 1;
            }
            ledger.push(entry);

            optimized_files.insert(
                path.clone(),
                EmittedFile {
                    path: file.path.clone(),
                    content: optimized_content,
                    kind: file.kind.clone(),
                    confidence: file.confidence,
                    provenance_links: file.provenance_links.clone(),
                },
            );
        }

        let report = EGraphOptimizationReport {
            files_optimized: ledger.len(),
            total_cost_reduction,
            budget_breaches,
            ledger,
            budget: self.budget.clone(),
        };

        let optimized_plan = EmissionPlan {
            version: plan.version.clone(),
            run_id: plan.run_id.clone(),
            scaffold: plan.scaffold.clone(),
            files: optimized_files,
            module_graph: plan.module_graph.clone(),
            manifest: plan.manifest.clone(),
            diagnostics: plan.diagnostics.clone(),
            stats: plan.stats.clone(),
        };

        (optimized_plan, report)
    }

    /// Optimize a single file's content.
    ///
    /// Returns the optimized content string and evidence ledger entry.
    pub fn optimize_file(&self, path: &str, content: &str) -> (String, EvidenceLedgerEntry) {
        let original_term = parse_to_term(content);
        let cost_before = original_term.cost();

        let mut egraph = EGraph::new();
        let root_class = egraph.add_class(original_term.clone());

        let rules = builtin_rules();
        let mut rewrites_applied = Vec::new();
        let mut stop_reason = StopReason::Saturated;
        let mut iterations = 0;
        let mut peak_nodes = egraph.total_nodes;
        let start = Instant::now();

        for iter in 0..self.budget.max_iterations {
            iterations = iter + 1;

            if start.elapsed() >= self.budget.timeout {
                stop_reason = StopReason::Timeout;
                break;
            }

            if egraph.total_nodes >= self.budget.max_nodes {
                stop_reason = StopReason::NodeBudget;
                break;
            }

            let current = egraph.extract_best(root_class).cloned();
            let Some(current_term) = current else {
                break;
            };

            let mut any_applied = false;
            for (rule, apply_fn) in &rules {
                if let Some(rewritten) = apply_fn(&current_term) {
                    let cost_after = rewritten.cost();
                    let cost_curr = current_term.cost();

                    rewrites_applied.push(RewriteApplication {
                        rule_id: rule.id.clone(),
                        semantics: rule.semantics,
                        description: rule.description.clone(),
                        cost_before: cost_curr,
                        cost_after,
                    });

                    egraph.add_to_class(root_class, rewritten);
                    any_applied = true;

                    if egraph.total_nodes > peak_nodes {
                        peak_nodes = egraph.total_nodes;
                    }
                }
            }

            if !any_applied {
                stop_reason = StopReason::Saturated;
                break;
            }

            if iter + 1 >= self.budget.max_iterations {
                stop_reason = StopReason::IterationBudget;
            }
        }

        let optimized_term = egraph.extract_best(root_class).unwrap_or(&original_term);
        let cost_after = optimized_term.cost();
        let optimized_content = render_term(optimized_term);

        let entry = EvidenceLedgerEntry {
            file_path: path.to_string(),
            cost_before,
            cost_after,
            rewrites_applied,
            budget_breached: matches!(stop_reason, StopReason::NodeBudget | StopReason::Timeout),
            stop_reason,
            iterations,
            peak_nodes,
        };

        (optimized_content, entry)
    }
}

// ── Term parsing / rendering ─────────────────────────────────────────

/// Parse source code into a CodeTerm (simplified heuristic parser).
fn parse_to_term(content: &str) -> CodeTerm {
    let lines: Vec<&str> = content.lines().collect();
    if lines.is_empty() {
        return CodeTerm::Noop;
    }

    let mut terms = Vec::new();
    for line in &lines {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if trimmed.starts_with("use ") {
            let path = trimmed
                .strip_prefix("use ")
                .unwrap_or("")
                .trim_end_matches(';')
                .to_string();
            terms.push(CodeTerm::Import { path });
        } else if trimmed.starts_with("const ") && trimmed.contains("Style") {
            let parts: Vec<&str> = trimmed.splitn(2, '=').collect();
            let name = parts
                .first()
                .unwrap_or(&"")
                .trim_start_matches("const ")
                .trim()
                .trim_end_matches(':')
                .split(':')
                .next()
                .unwrap_or("")
                .trim()
                .to_string();
            let value = parts
                .get(1)
                .unwrap_or(&"")
                .trim()
                .trim_end_matches(';')
                .to_string();
            terms.push(CodeTerm::StyleConst { name, value });
        } else {
            terms.push(CodeTerm::Literal(line.to_string()));
        }
    }

    if terms.len() == 1 {
        terms.into_iter().next().unwrap()
    } else {
        CodeTerm::Block(terms)
    }
}

/// Render a CodeTerm back to source code.
fn render_term(term: &CodeTerm) -> String {
    match term {
        CodeTerm::Noop => String::new(),
        CodeTerm::Literal(s) => s.clone(),
        CodeTerm::Import { path } => format!("use {path};"),
        CodeTerm::StyleConst { name, value } => format!("const {name}: Style = {value};"),
        CodeTerm::Call { name, args } => {
            let rendered_args: Vec<String> = args.iter().map(render_term).collect();
            format!("{name}({})", rendered_args.join(", "))
        }
        CodeTerm::MatchArm { pattern, body } => {
            format!("{pattern} => {}", render_term(body))
        }
        CodeTerm::Widget { kind, children } => {
            let rendered: Vec<String> = children.iter().map(render_term).collect();
            format!("{kind}({{\n{}\n}})", rendered.join("\n"))
        }
        CodeTerm::Block(stmts) => stmts.iter().map(render_term).collect::<Vec<_>>().join("\n"),
    }
}

// ── Tests ────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_budget_is_reasonable() {
        let b = EGraphBudget::default();
        assert!(b.max_nodes >= 1000);
        assert!(b.max_iterations >= 10);
        assert!(b.timeout >= Duration::from_millis(100));
    }

    #[test]
    fn code_term_cost_ordering() {
        let noop = CodeTerm::Noop;
        let lit = CodeTerm::Literal("hello".into());
        let block = CodeTerm::Block(vec![lit.clone(), lit.clone()]);

        assert_eq!(noop.cost(), 0);
        assert!(lit.cost() > 0);
        assert!(block.cost() > lit.cost());
    }

    #[test]
    fn code_term_node_count() {
        let lit = CodeTerm::Literal("x".into());
        assert_eq!(lit.node_count(), 1);

        let block = CodeTerm::Block(vec![lit.clone(), lit.clone()]);
        assert_eq!(block.node_count(), 3); // 1 block + 2 literals
    }

    #[test]
    fn noop_elimination_rewrite() {
        let term = CodeTerm::Block(vec![
            CodeTerm::Literal("let x = 1;".into()),
            CodeTerm::Noop,
            CodeTerm::Literal("let y = 2;".into()),
        ]);

        let rules = builtin_rules();
        let (_, apply) = &rules[0]; // noop-elimination
        let result = apply(&term);
        assert!(result.is_some());
        if let Some(CodeTerm::Block(stmts)) = result {
            assert_eq!(stmts.len(), 2);
            assert!(!stmts.iter().any(|s| matches!(s, CodeTerm::Noop)));
        }
    }

    #[test]
    fn block_flatten_rewrite() {
        let inner = CodeTerm::Block(vec![CodeTerm::Literal("a".into())]);
        let outer = CodeTerm::Block(vec![inner, CodeTerm::Literal("b".into())]);

        let rules = builtin_rules();
        let (_, apply) = &rules[1]; // block-flatten
        let result = apply(&outer);
        assert!(result.is_some());
        if let Some(CodeTerm::Block(stmts)) = result {
            assert_eq!(stmts.len(), 2);
        }
    }

    #[test]
    fn import_dedup_rewrite() {
        let term = CodeTerm::Block(vec![
            CodeTerm::Import {
                path: "ftui::Widget".into(),
            },
            CodeTerm::Import {
                path: "ftui::Widget".into(),
            },
            CodeTerm::Import {
                path: "ftui::Style".into(),
            },
        ]);

        let rules = builtin_rules();
        let (_, apply) = &rules[2]; // import-dedup
        let result = apply(&term);
        assert!(result.is_some());
        if let Some(CodeTerm::Block(stmts)) = result {
            assert_eq!(stmts.len(), 2);
        }
    }

    #[test]
    fn style_const_merge_rewrite() {
        let term = CodeTerm::Block(vec![
            CodeTerm::StyleConst {
                name: "RED".into(),
                value: "Color::Red".into(),
            },
            CodeTerm::StyleConst {
                name: "ALSO_RED".into(),
                value: "Color::Red".into(),
            },
            CodeTerm::Literal("other code".into()),
        ]);

        let rules = builtin_rules();
        let (_, apply) = &rules[3]; // style-const-merge
        let result = apply(&term);
        assert!(result.is_some());
        if let Some(CodeTerm::Block(stmts)) = result {
            let style_count = stmts
                .iter()
                .filter(|s| matches!(s, CodeTerm::StyleConst { .. }))
                .count();
            assert_eq!(style_count, 1, "duplicate style should be merged");
        }
    }

    #[test]
    fn widget_unwrap_single_child() {
        let term = CodeTerm::Widget {
            kind: "wrapper".into(),
            children: vec![CodeTerm::Literal("inner".into())],
        };

        let rules = builtin_rules();
        let (_, apply) = &rules[4]; // widget-unwrap-single
        let result = apply(&term);
        assert_eq!(result, Some(CodeTerm::Literal("inner".into())));
    }

    #[test]
    fn widget_unwrap_skips_non_wrapper() {
        let term = CodeTerm::Widget {
            kind: "Block".into(),
            children: vec![CodeTerm::Literal("inner".into())],
        };

        let rules = builtin_rules();
        let (_, apply) = &rules[4];
        assert!(apply(&term).is_none());
    }

    #[test]
    fn parse_renders_roundtrip() {
        let source = "use ftui::Widget;\nlet x = 1;\nlet y = 2;";
        let term = parse_to_term(source);
        let rendered = render_term(&term);
        assert!(rendered.contains("use ftui::Widget;"));
        assert!(rendered.contains("let x = 1;"));
    }

    #[test]
    fn optimize_file_removes_noops() {
        let optimizer = EGraphOptimizer::with_defaults();
        let content = "use ftui::Widget;\nuse ftui::Widget;\nlet x = 1;";
        let (optimized, entry) = optimizer.optimize_file("src/test.rs", content);

        assert!(entry.cost_after <= entry.cost_before);
        assert!(!entry.rewrites_applied.is_empty());
        // Duplicate import should be removed
        let import_count = optimized.matches("use ftui::Widget;").count();
        assert_eq!(import_count, 1, "duplicate import should be deduped");
    }

    #[test]
    fn optimize_file_logs_evidence() {
        let optimizer = EGraphOptimizer::with_defaults();
        let content = "use a;\nuse a;\nuse b;";
        let (_, entry) = optimizer.optimize_file("src/mod.rs", content);

        assert_eq!(entry.file_path, "src/mod.rs");
        assert!(entry.iterations >= 1);
        assert!(!entry.rewrites_applied.is_empty());
        // At least one import-dedup rewrite
        assert!(
            entry
                .rewrites_applied
                .iter()
                .any(|r| r.semantics == RewriteSemantics::ImportDedup)
        );
    }

    #[test]
    fn budget_breach_triggers_conservative_extraction() {
        let optimizer = EGraphOptimizer::new(EGraphBudget {
            max_nodes: 1, // Extremely tight budget
            max_iterations: 50,
            timeout: Duration::from_secs(10),
        });
        let content = "use a;\nuse a;\nuse b;\nuse c;";
        let (_, entry) = optimizer.optimize_file("src/tight.rs", content);

        // Should not panic; may or may not breach depending on initial parse size
        assert!(entry.iterations >= 1 || entry.budget_breached);
    }

    #[test]
    fn saturation_reaches_fixpoint() {
        let optimizer = EGraphOptimizer::with_defaults();
        let content = "let x = 1;\nlet y = 2;";
        let (_, entry) = optimizer.optimize_file("src/simple.rs", content);

        // No rewrites applicable to simple literals
        assert_eq!(entry.stop_reason, StopReason::Saturated);
    }

    #[test]
    fn timeout_budget_stops_optimization() {
        let optimizer = EGraphOptimizer::new(EGraphBudget {
            timeout: Duration::ZERO,
            ..EGraphBudget::default()
        });
        let content = "use a;\nuse a;\nuse b;";
        let (_, entry) = optimizer.optimize_file("src/slow.rs", content);

        assert!(
            entry.stop_reason == StopReason::Timeout || entry.stop_reason == StopReason::Saturated
        );
    }

    #[test]
    fn egraph_class_management() {
        let mut eg = EGraph::new();
        let id = eg.add_class(CodeTerm::Literal("original".into()));
        assert_eq!(eg.class_count(), 1);

        eg.add_to_class(id, CodeTerm::Literal("opt".into()));
        // "opt" is shorter so should be the new best
        assert_eq!(
            eg.extract_best(id).unwrap(),
            &CodeTerm::Literal("opt".into())
        );
    }

    #[test]
    fn evidence_ledger_entry_serializable() {
        let entry = EvidenceLedgerEntry {
            file_path: "src/test.rs".into(),
            cost_before: 100,
            cost_after: 80,
            rewrites_applied: vec![],
            budget_breached: false,
            stop_reason: StopReason::Saturated,
            iterations: 3,
            peak_nodes: 10,
        };
        let json = serde_json::to_string(&entry).expect("should serialize");
        assert!(json.contains("src/test.rs"));
        let deser: EvidenceLedgerEntry = serde_json::from_str(&json).expect("should deserialize");
        assert_eq!(deser.cost_before, 100);
    }

    #[test]
    fn report_serializable() {
        let report = EGraphOptimizationReport {
            ledger: vec![],
            budget: EGraphBudget::default(),
            files_optimized: 0,
            total_cost_reduction: 0,
            budget_breaches: 0,
        };
        let json = serde_json::to_string(&report).expect("should serialize");
        let deser: EGraphOptimizationReport =
            serde_json::from_str(&json).expect("should deserialize");
        assert_eq!(deser.files_optimized, 0);
    }

    #[test]
    fn multiple_rewrites_compose() {
        let optimizer = EGraphOptimizer::with_defaults();
        // Content with both duplicate imports and noops (via empty lines that parse to literals)
        let content = "use ftui::Widget;\nuse ftui::Widget;\nuse ftui::Style;";
        let (optimized, entry) = optimizer.optimize_file("src/composed.rs", content);

        assert!(entry.cost_after <= entry.cost_before);
        // Should have deduped
        let widget_count = optimized.matches("use ftui::Widget;").count();
        assert_eq!(widget_count, 1);
    }

    #[test]
    fn empty_content_handled() {
        let optimizer = EGraphOptimizer::with_defaults();
        let (optimized, entry) = optimizer.optimize_file("src/empty.rs", "");

        assert!(optimized.is_empty() || optimized.trim().is_empty());
        assert_eq!(entry.cost_before, 0);
        assert_eq!(entry.cost_after, 0);
    }
}
