// SPDX-License-Identifier: Apache-2.0
//! Explain mode for strategy decisions and risk tradeoffs.
//!
//! Produces transparent, mathematically grounded decision explanations
//! for mappings, approximations, capability-gap handling, and rollout
//! verdicts. Supports concise and verbose disclosure levels, including
//! galaxy-brain cards (equation, substitutions, intuition).
//!
//! Explanations are stable, diffable, and reconstructable from the
//! underlying [`TranslationPlan`] and [`ConfidenceModel`] artifacts.

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::semantic_contract::{
    BayesianPosterior, ConfidenceModel, ExpectedLossResult, MigrationDecision,
    TransformationHandlingClass, TransformationRiskLevel,
};
use crate::translation_planner::{
    CapabilityGapTicket, GapKind, GapPriority, PlanStats, StrategyDecision, TranslationPlan,
};

// ── Verbosity ────────────────────────────────────────────────────────────

/// How much detail to include in explanations.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[derive(Default)]
pub enum Verbosity {
    /// One-line summary per decision.
    #[default]
    Concise,
    /// Full rationale including posterior, loss, and policy references.
    Verbose,
}

impl fmt::Display for Verbosity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Concise => f.write_str("concise"),
            Self::Verbose => f.write_str("verbose"),
        }
    }
}

// ── Galaxy-Brain Card ────────────────────────────────────────────────────

/// A "galaxy-brain" transparency card: equation, substitutions, intuition.
///
/// Emitted in verbose mode to show the mathematical reasoning behind
/// a gating decision in human-readable form.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct GalaxyBrainCard {
    /// LaTeX-ish equation string (e.g. "E[L(accept)] = p·0 + (1-p)·L_miss").
    pub equation: String,
    /// Concrete substitutions (e.g. ["p = 0.87", "L_miss = 8.0"]).
    pub substitutions: Vec<String>,
    /// Plain-English intuition.
    pub intuition: String,
}

// ── Explanation Types ────────────────────────────────────────────────────

/// Explanation for a single strategy decision.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DecisionExplanation {
    /// Segment id being explained.
    pub segment_id: String,
    /// Segment name.
    pub segment_name: String,
    /// Chosen strategy id.
    pub strategy_id: String,
    /// Handling class of the chosen strategy.
    pub handling_class: TransformationHandlingClass,
    /// Risk level.
    pub risk: TransformationRiskLevel,
    /// Gating decision.
    pub gate: MigrationDecision,
    /// Composite confidence score.
    pub confidence: f64,
    /// Concise one-line summary.
    pub summary: String,
    /// Verbose rationale (empty in concise mode).
    pub rationale: String,
    /// Policy clause references.
    pub policy_refs: Vec<String>,
    /// Posterior summary line.
    pub posterior_summary: String,
    /// Expected-loss summary line.
    pub loss_summary: String,
    /// Guarantee status line.
    pub guarantee_status: String,
    /// Galaxy-brain card (only in verbose mode).
    pub galaxy_brain: Option<GalaxyBrainCard>,
    /// Number of alternatives considered.
    pub alternatives_count: usize,
}

/// Explanation for a capability-gap ticket.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GapExplanation {
    /// Segment id.
    pub segment_id: String,
    /// Segment name.
    pub segment_name: String,
    /// Gap classification.
    pub gap_kind: GapKind,
    /// Priority.
    pub priority: GapPriority,
    /// Concise summary.
    pub summary: String,
    /// Suggested remediation.
    pub remediation: String,
}

/// Full plan explanation bundle.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlanExplanation {
    /// Planner version.
    pub version: String,
    /// Run id.
    pub run_id: String,
    /// Verbosity level used.
    pub verbosity: Verbosity,
    /// Per-decision explanations.
    pub decisions: Vec<DecisionExplanation>,
    /// Per-gap explanations.
    pub gaps: Vec<GapExplanation>,
    /// Aggregate stats summary.
    pub stats_summary: StatsSummary,
}

/// Human-readable aggregate statistics.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StatsSummary {
    pub total_segments: usize,
    pub auto_approved: usize,
    pub human_review: usize,
    pub rejected: usize,
    pub gap_tickets: usize,
    pub mean_confidence: f64,
    pub text: String,
}

// ── Public API ───────────────────────────────────────────────────────────

/// Explain a translation plan at the given verbosity level.
///
/// The optional [`ConfidenceModel`] enriches verbose explanations with
/// loss-matrix and decision-boundary references.
pub fn explain_plan(
    plan: &TranslationPlan,
    verbosity: Verbosity,
    model: Option<&ConfidenceModel>,
) -> PlanExplanation {
    let decisions: Vec<DecisionExplanation> = plan
        .decisions
        .iter()
        .map(|d| explain_decision(d, verbosity, model))
        .collect();

    let gaps: Vec<GapExplanation> = plan
        .gap_tickets
        .iter()
        .map(|g| explain_gap(g, verbosity))
        .collect();

    let stats_summary = explain_stats(&plan.stats);

    PlanExplanation {
        version: plan.version.clone(),
        run_id: plan.run_id.clone(),
        verbosity,
        decisions,
        gaps,
        stats_summary,
    }
}

/// Explain a single strategy decision.
pub fn explain_decision(
    decision: &StrategyDecision,
    verbosity: Verbosity,
    model: Option<&ConfidenceModel>,
) -> DecisionExplanation {
    let summary = format_decision_summary(decision);
    let posterior_summary = format_posterior(&decision.posterior);
    let loss_summary = format_loss(&decision.expected_loss);
    let guarantee_status = format_guarantee(decision);
    let policy_refs = extract_policy_refs(decision);

    let (rationale, galaxy_brain) = match verbosity {
        Verbosity::Concise => (String::new(), None),
        Verbosity::Verbose => {
            let r = format_verbose_rationale(decision, model);
            let gb = build_galaxy_brain_card(decision, model);
            (r, Some(gb))
        }
    };

    DecisionExplanation {
        segment_id: decision.segment.id.to_string(),
        segment_name: decision.segment.name.clone(),
        strategy_id: decision.chosen.id.clone(),
        handling_class: decision.chosen.handling_class,
        risk: decision.chosen.risk,
        gate: decision.gate,
        confidence: decision.confidence,
        summary,
        rationale,
        policy_refs,
        posterior_summary,
        loss_summary,
        guarantee_status,
        galaxy_brain,
        alternatives_count: decision.alternatives.len(),
    }
}

/// Explain a capability-gap ticket.
pub fn explain_gap(gap: &CapabilityGapTicket, _verbosity: Verbosity) -> GapExplanation {
    let summary = format!(
        "[{}] {} — {} (priority: {})",
        format_gap_kind(gap.gap_kind),
        gap.segment.name,
        gap.description,
        format_gap_priority(gap.priority),
    );

    GapExplanation {
        segment_id: gap.segment.id.to_string(),
        segment_name: gap.segment.name.clone(),
        gap_kind: gap.gap_kind,
        priority: gap.priority,
        summary,
        remediation: gap.suggested_remediation.clone(),
    }
}

/// Render a plan explanation as stable, diffable text.
pub fn render_text(explanation: &PlanExplanation) -> String {
    let mut out = String::new();

    out.push_str(&format!(
        "=== Migration Plan Explanation ({}) ===\n",
        explanation.verbosity
    ));
    out.push_str(&format!(
        "Version: {}  Run: {}\n\n",
        explanation.version, explanation.run_id
    ));

    // Stats.
    out.push_str(&format!("{}\n\n", explanation.stats_summary.text));

    // Decisions.
    out.push_str(&format!(
        "── Decisions ({}) ──\n",
        explanation.decisions.len()
    ));
    for (i, d) in explanation.decisions.iter().enumerate() {
        out.push_str(&format!("  {}. {}\n", i + 1, d.summary));
        if explanation.verbosity == Verbosity::Verbose {
            out.push_str(&format!("     Posterior: {}\n", d.posterior_summary));
            out.push_str(&format!("     Loss:      {}\n", d.loss_summary));
            out.push_str(&format!("     Guarantee: {}\n", d.guarantee_status));
            if !d.policy_refs.is_empty() {
                out.push_str(&format!("     Policy:    {}\n", d.policy_refs.join(", ")));
            }
            if !d.rationale.is_empty() {
                out.push_str(&format!("     Rationale: {}\n", d.rationale));
            }
            if let Some(gb) = &d.galaxy_brain {
                out.push_str("     Galaxy-Brain Card:\n");
                out.push_str(&format!("       Eq:         {}\n", gb.equation));
                for sub in &gb.substitutions {
                    out.push_str(&format!("       Substitute: {}\n", sub));
                }
                out.push_str(&format!("       Intuition:  {}\n", gb.intuition));
            }
        }
    }
    out.push('\n');

    // Gaps.
    if !explanation.gaps.is_empty() {
        out.push_str(&format!("── Gaps ({}) ──\n", explanation.gaps.len()));
        for (i, g) in explanation.gaps.iter().enumerate() {
            out.push_str(&format!("  {}. {}\n", i + 1, g.summary));
            if explanation.verbosity == Verbosity::Verbose {
                out.push_str(&format!("     Remediation: {}\n", g.remediation));
            }
        }
    }

    out
}

/// Render a plan explanation as JSON value.
pub fn render_json(explanation: &PlanExplanation) -> serde_json::Value {
    serde_json::to_value(explanation).unwrap_or(serde_json::Value::Null)
}

// ── Formatting Helpers ───────────────────────────────────────────────────

fn format_decision_summary(d: &StrategyDecision) -> String {
    format!(
        "{} → {} [{}] confidence={:.2} gate={} risk={}",
        d.segment.name,
        d.chosen.id,
        format_handling_class(d.chosen.handling_class),
        d.confidence,
        format_gate(d.gate),
        format_risk(d.chosen.risk),
    )
}

fn format_posterior(p: &BayesianPosterior) -> String {
    format!(
        "Beta({:.2},{:.2}) mean={:.4} CI=[{:.4},{:.4}]",
        p.alpha, p.beta, p.mean, p.credible_lower, p.credible_upper
    )
}

fn format_loss(el: &ExpectedLossResult) -> String {
    format!(
        "E[L]: accept={:.4} hold={:.4} reject={:.4} → {}",
        el.expected_loss_accept,
        el.expected_loss_hold,
        el.expected_loss_reject,
        format_gate(el.decision),
    )
}

fn format_guarantee(d: &StrategyDecision) -> String {
    let ci_width = d.posterior.credible_upper - d.posterior.credible_lower;
    let coverage = if ci_width < 0.15 {
        "tight"
    } else if ci_width < 0.35 {
        "moderate"
    } else {
        "wide"
    };

    let automatable = if d.chosen.automatable {
        "automatable"
    } else {
        "manual"
    };

    format!(
        "CI coverage={} ({:.0}%) {}",
        coverage,
        ci_width * 100.0,
        automatable,
    )
}

fn format_verbose_rationale(d: &StrategyDecision, model: Option<&ConfidenceModel>) -> String {
    let mut parts = Vec::new();

    parts.push(d.rationale.clone());

    if !d.alternatives.is_empty() {
        let alt_summary: Vec<String> = d
            .alternatives
            .iter()
            .map(|a| {
                format!(
                    "{}(score={:.2},rejected={})",
                    a.strategy.id, a.score, a.rejection_reason
                )
            })
            .collect();
        parts.push(format!("Alternatives: {}", alt_summary.join("; ")));
    }

    if let Some(m) = model {
        parts.push(format!(
            "Boundaries: auto_approve>{:.2} human_review=[{:.2},{:.2}] reject<{:.2}",
            m.decision_boundaries.auto_approve_threshold,
            m.decision_boundaries.human_review_lower,
            m.decision_boundaries.human_review_upper,
            m.decision_boundaries.reject_threshold,
        ));
    }

    parts.join(" | ")
}

fn build_galaxy_brain_card(
    d: &StrategyDecision,
    model: Option<&ConfidenceModel>,
) -> GalaxyBrainCard {
    let p = &d.posterior;
    let el = &d.expected_loss;

    let (l_miss, l_hold) = if let Some(m) = model {
        (m.loss_matrix.accept_incorrect, m.loss_matrix.hold_correct)
    } else {
        (el.expected_loss_accept, el.expected_loss_hold)
    };

    GalaxyBrainCard {
        equation: "E[L(a)] = p_correct * L(a,correct) + (1 - p_correct) * L(a,incorrect)".into(),
        substitutions: vec![
            format!("p_correct = posterior.mean = {:.4}", p.mean),
            format!("L(accept,incorrect) = {:.2}", l_miss),
            format!("L(hold,correct) = {:.2}", l_hold),
            format!("E[L(accept)] = {:.4}", el.expected_loss_accept),
            format!("E[L(hold)]   = {:.4}", el.expected_loss_hold),
            format!("E[L(reject)] = {:.4}", el.expected_loss_reject),
        ],
        intuition: format!(
            "With posterior mean {:.2}%, {} is optimal because its expected loss ({:.4}) \
             is lowest among the three actions.",
            p.mean * 100.0,
            format_gate(el.decision),
            match el.decision {
                MigrationDecision::AutoApprove => el.expected_loss_accept,
                MigrationDecision::Reject | MigrationDecision::HardReject => {
                    el.expected_loss_reject
                }
                _ => el.expected_loss_hold,
            },
        ),
    }
}

fn extract_policy_refs(d: &StrategyDecision) -> Vec<String> {
    let mut refs = Vec::new();
    if let Some(claim_id) = &d.expected_loss.claim_id {
        refs.push(format!("claim:{}", claim_id));
    }
    if let Some(policy_id) = &d.expected_loss.policy_id {
        refs.push(format!("policy:{}", policy_id));
    }
    refs
}

fn explain_stats(stats: &PlanStats) -> StatsSummary {
    let text = format!(
        "Segments: {} total | {} auto-approve | {} human-review | {} rejected | {} gaps | mean confidence {:.2}",
        stats.total_segments,
        stats.auto_approve,
        stats.human_review,
        stats.rejected,
        stats.gap_tickets,
        stats.mean_confidence,
    );

    StatsSummary {
        total_segments: stats.total_segments,
        auto_approved: stats.auto_approve,
        human_review: stats.human_review,
        rejected: stats.rejected,
        gap_tickets: stats.gap_tickets,
        mean_confidence: stats.mean_confidence,
        text,
    }
}

fn format_handling_class(hc: TransformationHandlingClass) -> &'static str {
    match hc {
        TransformationHandlingClass::Exact => "exact",
        TransformationHandlingClass::Approximate => "approximate",
        TransformationHandlingClass::ExtendFtui => "extend-ftui",
        TransformationHandlingClass::Unsupported => "unsupported",
    }
}

fn format_risk(r: TransformationRiskLevel) -> &'static str {
    match r {
        TransformationRiskLevel::Low => "low",
        TransformationRiskLevel::Medium => "medium",
        TransformationRiskLevel::High => "high",
        TransformationRiskLevel::Critical => "critical",
    }
}

fn format_gate(g: MigrationDecision) -> &'static str {
    match g {
        MigrationDecision::AutoApprove => "auto-approve",
        MigrationDecision::HumanReview => "human-review",
        MigrationDecision::Reject => "reject",
        MigrationDecision::HardReject => "hard-reject",
        MigrationDecision::Rollback => "rollback",
        MigrationDecision::ConservativeFallback => "conservative-fallback",
    }
}

fn format_gap_kind(k: GapKind) -> &'static str {
    match k {
        GapKind::Unsupported => "UNSUPPORTED",
        GapKind::RequiresExtension => "EXTENSION",
        GapKind::LowConfidence => "LOW-CONFIDENCE",
    }
}

fn format_gap_priority(p: GapPriority) -> &'static str {
    match p {
        GapPriority::Critical => "critical",
        GapPriority::High => "high",
        GapPriority::Medium => "medium",
        GapPriority::Low => "low",
    }
}

// ── Tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mapping_atlas::RemediationStrategy;
    use crate::migration_ir::IrNodeId;
    use crate::translation_planner::{
        IrSegment, RankedAlternative, SegmentCategory, TranslationStrategy,
    };
    use std::collections::BTreeMap;

    fn sample_posterior() -> BayesianPosterior {
        BayesianPosterior {
            alpha: 10.0,
            beta: 2.0,
            mean: 0.833,
            variance: 0.011,
            credible_lower: 0.65,
            credible_upper: 0.95,
        }
    }

    fn sample_loss() -> ExpectedLossResult {
        ExpectedLossResult {
            decision: MigrationDecision::AutoApprove,
            posterior: sample_posterior(),
            expected_loss_accept: 1.334,
            expected_loss_reject: 6.664,
            expected_loss_hold: 3.0,
            rationale: "accept has lowest expected loss".into(),
            claim_id: Some("claim-state-001".into()),
            policy_id: Some("policy-exact-state".into()),
        }
    }

    fn sample_strategy() -> TranslationStrategy {
        TranslationStrategy {
            id: "direct-model-impl".into(),
            description: "Direct Model trait implementation".into(),
            handling_class: TransformationHandlingClass::Exact,
            risk: TransformationRiskLevel::Low,
            target_construct: "Model".into(),
            target_crate: "ftui".into(),
            automatable: true,
            remediation: RemediationStrategy {
                approach: "Direct translation".into(),
                automatable: true,
                effort: crate::mapping_atlas::EffortLevel::Trivial,
            },
        }
    }

    fn sample_decision() -> StrategyDecision {
        StrategyDecision {
            segment: IrSegment {
                id: IrNodeId("view-001".into()),
                name: "MainView".into(),
                category: SegmentCategory::View,
                mapping_signature: "view::MainView".into(),
            },
            chosen: sample_strategy(),
            alternatives: vec![RankedAlternative {
                strategy: TranslationStrategy {
                    id: "widget-wrapper".into(),
                    description: "Widget wrapper pattern".into(),
                    handling_class: TransformationHandlingClass::Approximate,
                    risk: TransformationRiskLevel::Medium,
                    target_construct: "Widget".into(),
                    target_crate: "ftui-widgets".into(),
                    automatable: false,
                    remediation: RemediationStrategy {
                        approach: "Manual widget adaptation".into(),
                        automatable: false,
                        effort: crate::mapping_atlas::EffortLevel::Medium,
                    },
                },
                score: 0.65,
                rejection_reason: "lower confidence than direct-model-impl".into(),
            }],
            posterior: sample_posterior(),
            expected_loss: sample_loss(),
            gate: MigrationDecision::AutoApprove,
            confidence: 0.87,
            rationale: "Exact mapping with high posterior mean".into(),
        }
    }

    fn sample_gap() -> CapabilityGapTicket {
        CapabilityGapTicket {
            segment: IrSegment {
                id: IrNodeId("effect-007".into()),
                name: "WebSocketEffect".into(),
                category: SegmentCategory::Effect,
                mapping_signature: "effect::WebSocket".into(),
            },
            gap_kind: GapKind::Unsupported,
            description: "No WebSocket effect mapping exists".into(),
            suggested_remediation: "Implement custom Cmd adapter".into(),
            priority: GapPriority::High,
        }
    }

    fn sample_plan() -> TranslationPlan {
        TranslationPlan {
            version: "translation-planner-v1".into(),
            run_id: "test-run-001".into(),
            seed: 0xDEADBEEF,
            decisions: vec![sample_decision()],
            gap_tickets: vec![sample_gap()],
            stats: PlanStats {
                total_segments: 10,
                auto_approve: 7,
                human_review: 2,
                rejected: 1,
                gap_tickets: 1,
                mean_confidence: 0.78,
                by_category: BTreeMap::new(),
                by_handling_class: BTreeMap::new(),
            },
        }
    }

    #[test]
    fn explain_concise_produces_summary() {
        let plan = sample_plan();
        let explanation = explain_plan(&plan, Verbosity::Concise, None);

        assert_eq!(explanation.verbosity, Verbosity::Concise);
        assert_eq!(explanation.decisions.len(), 1);
        assert_eq!(explanation.gaps.len(), 1);

        let d = &explanation.decisions[0];
        assert!(d.summary.contains("MainView"));
        assert!(d.summary.contains("direct-model-impl"));
        assert!(d.rationale.is_empty());
        assert!(d.galaxy_brain.is_none());
    }

    #[test]
    fn explain_verbose_includes_rationale_and_galaxy_brain() {
        let plan = sample_plan();
        let explanation = explain_plan(&plan, Verbosity::Verbose, None);

        let d = &explanation.decisions[0];
        assert!(!d.rationale.is_empty());
        assert!(d.galaxy_brain.is_some());

        let gb = d.galaxy_brain.as_ref().unwrap();
        assert!(gb.equation.contains("E[L(a)]"));
        assert!(!gb.substitutions.is_empty());
        assert!(gb.intuition.contains("auto-approve"));
    }

    #[test]
    fn explain_verbose_with_model_references_boundaries() {
        let model_json = include_str!("../contracts/opentui_confidence_model_v1.json");
        let model = ConfidenceModel::parse_and_validate(model_json).unwrap();

        let plan = sample_plan();
        let explanation = explain_plan(&plan, Verbosity::Verbose, Some(&model));

        let d = &explanation.decisions[0];
        assert!(d.rationale.contains("Boundaries:"));
        assert!(d.rationale.contains("auto_approve>"));
    }

    #[test]
    fn explain_gap_includes_kind_and_priority() {
        let gap = sample_gap();
        let g = explain_gap(&gap, Verbosity::Concise);

        assert!(g.summary.contains("UNSUPPORTED"));
        assert!(g.summary.contains("WebSocketEffect"));
        assert!(g.summary.contains("high"));
        assert_eq!(g.gap_kind, GapKind::Unsupported);
        assert_eq!(g.priority, GapPriority::High);
    }

    #[test]
    fn stats_summary_text_contains_counts() {
        let plan = sample_plan();
        let explanation = explain_plan(&plan, Verbosity::Concise, None);

        let s = &explanation.stats_summary;
        assert_eq!(s.total_segments, 10);
        assert_eq!(s.auto_approved, 7);
        assert_eq!(s.human_review, 2);
        assert_eq!(s.rejected, 1);
        assert!(s.text.contains("10 total"));
        assert!(s.text.contains("7 auto-approve"));
    }

    #[test]
    fn render_text_concise_is_compact() {
        let plan = sample_plan();
        let explanation = explain_plan(&plan, Verbosity::Concise, None);
        let text = render_text(&explanation);

        assert!(text.contains("=== Migration Plan Explanation (concise) ==="));
        assert!(text.contains("MainView"));
        // Concise mode should not contain galaxy-brain header.
        assert!(!text.contains("Galaxy-Brain Card:"));
    }

    #[test]
    fn render_text_verbose_includes_galaxy_brain() {
        let plan = sample_plan();
        let explanation = explain_plan(&plan, Verbosity::Verbose, None);
        let text = render_text(&explanation);

        assert!(text.contains("Galaxy-Brain Card:"));
        assert!(text.contains("Eq:"));
        assert!(text.contains("Intuition:"));
        assert!(text.contains("Posterior:"));
        assert!(text.contains("Loss:"));
    }

    #[test]
    fn render_json_roundtrip() {
        let plan = sample_plan();
        let explanation = explain_plan(&plan, Verbosity::Verbose, None);
        let json = render_json(&explanation);

        // Verify it can be deserialized back.
        let decoded: PlanExplanation =
            serde_json::from_value(json.clone()).expect("roundtrip failed");
        assert_eq!(decoded.run_id, "test-run-001");
        assert_eq!(decoded.decisions.len(), 1);
        assert_eq!(decoded.gaps.len(), 1);
    }

    #[test]
    fn render_text_is_stable_across_calls() {
        let plan = sample_plan();
        let e1 = explain_plan(&plan, Verbosity::Verbose, None);
        let e2 = explain_plan(&plan, Verbosity::Verbose, None);

        let t1 = render_text(&e1);
        let t2 = render_text(&e2);
        assert_eq!(t1, t2, "text output must be stable/deterministic");
    }

    #[test]
    fn policy_refs_extracted_from_loss() {
        let d = sample_decision();
        let e = explain_decision(&d, Verbosity::Concise, None);

        assert_eq!(e.policy_refs.len(), 2);
        assert!(e.policy_refs.contains(&"claim:claim-state-001".to_string()));
        assert!(
            e.policy_refs
                .contains(&"policy:policy-exact-state".to_string())
        );
    }

    #[test]
    fn posterior_summary_format() {
        let p = sample_posterior();
        let s = format_posterior(&p);
        assert!(s.contains("Beta(10.00,2.00)"));
        assert!(s.contains("mean=0.8330"));
        assert!(s.contains("CI=[0.6500,0.9500]"));
    }

    #[test]
    fn loss_summary_format() {
        let el = sample_loss();
        let s = format_loss(&el);
        assert!(s.contains("accept=1.3340"));
        assert!(s.contains("hold=3.0000"));
        assert!(s.contains("reject=6.6640"));
        assert!(s.contains("auto-approve"));
    }

    #[test]
    fn guarantee_status_for_tight_ci() {
        let mut d = sample_decision();
        d.posterior.credible_lower = 0.80;
        d.posterior.credible_upper = 0.90;
        let s = format_guarantee(&d);
        assert!(s.contains("tight"));
        assert!(s.contains("automatable"));
    }

    #[test]
    fn guarantee_status_for_wide_ci() {
        let mut d = sample_decision();
        d.posterior.credible_lower = 0.20;
        d.posterior.credible_upper = 0.90;
        let s = format_guarantee(&d);
        assert!(s.contains("wide"));
    }

    #[test]
    fn galaxy_brain_card_substitutions_reference_posterior() {
        let d = sample_decision();
        let gb = build_galaxy_brain_card(&d, None);
        assert!(gb.substitutions[0].contains("0.8330"));
    }

    #[test]
    fn verbosity_display() {
        assert_eq!(format!("{}", Verbosity::Concise), "concise");
        assert_eq!(format!("{}", Verbosity::Verbose), "verbose");
    }

    #[test]
    fn empty_plan_produces_empty_explanation() {
        let plan = TranslationPlan {
            version: "v1".into(),
            run_id: "empty-run".into(),
            seed: 0,
            decisions: vec![],
            gap_tickets: vec![],
            stats: PlanStats {
                total_segments: 0,
                auto_approve: 0,
                human_review: 0,
                rejected: 0,
                gap_tickets: 0,
                mean_confidence: 0.0,
                by_category: BTreeMap::new(),
                by_handling_class: BTreeMap::new(),
            },
        };

        let explanation = explain_plan(&plan, Verbosity::Verbose, None);
        assert!(explanation.decisions.is_empty());
        assert!(explanation.gaps.is_empty());
        assert_eq!(explanation.stats_summary.total_segments, 0);
    }

    #[test]
    fn concise_gap_includes_remediation_field() {
        let gap = sample_gap();
        let g = explain_gap(&gap, Verbosity::Concise);
        assert_eq!(g.remediation, "Implement custom Cmd adapter");
    }

    #[test]
    fn render_text_gaps_section_present() {
        let plan = sample_plan();
        let explanation = explain_plan(&plan, Verbosity::Verbose, None);
        let text = render_text(&explanation);
        assert!(text.contains("── Gaps (1) ──"));
        assert!(text.contains("Remediation:"));
    }
}
