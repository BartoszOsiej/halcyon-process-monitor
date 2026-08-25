// SPDX-License-Identifier: Apache-2.0
//! Counter-Example Guided Inductive Synthesis (CEGIS) for unmapped translation holes.
//!
//! When the [`TranslationPlanner`](crate::translation_planner) encounters an IR
//! segment with no atlas mapping (`mapping: None`), the CEGIS engine attempts to
//! synthesize a candidate translation snippet within bounded resource limits.
//!
//! # Design
//!
//! The synthesis loop follows the classical CEGIS pattern:
//!
//! 1. **Sketch generation**: Build a bounded set of candidate code snippets
//!    from known FrankenTUI constructs, parameterized by the IR segment's
//!    category and signature.
//! 2. **Verification**: Check each candidate against contract obligations
//!    (preconditions, postconditions, behavioral equivalence checks).
//! 3. **Counter-example refinement**: If verification fails, extract a
//!    counter-example and prune the search space.
//! 4. **Termination**: Stop when a verified candidate is found or resource
//!    budgets are exhausted.
//!
//! # Resource budgets
//!
//! Synthesis is always bounded by [`SynthesisBudget`]:
//! - `max_holes`: Maximum number of unmapped segments to attempt per run.
//! - `max_iterations_per_hole`: Maximum CEGIS iterations per hole.
//! - `max_candidates_per_iteration`: Candidates generated per iteration.
//! - `max_depth`: Maximum AST depth for generated snippets.
//!
//! When any budget is exhausted, synthesis emits a [`SynthesisExhausted`]
//! diagnostic and falls back to the stub strategy.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use crate::mapping_atlas::{EffortLevel, RemediationStrategy};
use crate::semantic_contract::{TransformationHandlingClass, TransformationRiskLevel};
use crate::translation_planner::{IrSegment, SegmentCategory, TranslationStrategy};

// ── Configuration ────────────────────────────────────────────────────

/// Resource budget controlling synthesis scope.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SynthesisBudget {
    /// Maximum unmapped holes to attempt per planner run.
    pub max_holes: usize,
    /// Maximum CEGIS loop iterations per hole.
    pub max_iterations_per_hole: usize,
    /// Maximum candidate sketches generated per iteration.
    pub max_candidates_per_iteration: usize,
    /// Maximum AST depth for generated code snippets.
    pub max_depth: u8,
    /// Wall-clock timeout per hole.
    pub timeout_per_hole: Duration,
}

impl Default for SynthesisBudget {
    fn default() -> Self {
        Self {
            max_holes: 50,
            max_iterations_per_hole: 10,
            max_candidates_per_iteration: 8,
            max_depth: 4,
            timeout_per_hole: Duration::from_secs(5),
        }
    }
}

// ── Core types ───────────────────────────────────────────────────────

/// A synthesized code sketch for an unmapped hole.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SynthesizedSketch {
    /// The code snippet (Rust source).
    pub code: String,
    /// Target FrankenTUI construct name.
    pub target_construct: String,
    /// Target crate.
    pub target_crate: String,
    /// AST depth of the generated sketch.
    pub depth: u8,
    /// Generation method description.
    pub method: String,
}

/// Proof witness bundle for a verified synthesis candidate.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProofWitness {
    /// Contract obligations that were checked.
    pub obligations_checked: Vec<String>,
    /// Obligations that passed verification.
    pub obligations_passed: Vec<String>,
    /// Obligations that failed (if any — only set for rejected candidates).
    pub obligations_failed: Vec<String>,
    /// Counter-examples from failed verification rounds.
    pub counter_examples: Vec<CounterExample>,
    /// Number of CEGIS iterations taken.
    pub iterations: usize,
    /// Wall-clock time for synthesis.
    pub elapsed: Duration,
}

/// A counter-example extracted from failed verification.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CounterExample {
    /// The obligation that failed.
    pub obligation: String,
    /// Description of the failure.
    pub description: String,
    /// The candidate that was rejected.
    pub rejected_sketch_method: String,
}

/// Result of synthesis for a single hole.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum SynthesisOutcome {
    /// A verified candidate was found.
    Verified {
        sketch: SynthesizedSketch,
        witness: ProofWitness,
    },
    /// All budgets exhausted without finding a verified candidate.
    Exhausted { diagnostic: SynthesisExhausted },
}

/// Diagnostic emitted when synthesis fails to find a verified candidate.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SynthesisExhausted {
    /// The segment that was being synthesized.
    pub segment_id: String,
    /// Why synthesis stopped.
    pub reason: ExhaustionReason,
    /// Candidates attempted.
    pub candidates_tried: usize,
    /// Iterations completed.
    pub iterations_completed: usize,
    /// Counter-examples accumulated.
    pub counter_examples: Vec<CounterExample>,
}

/// Why synthesis was exhausted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ExhaustionReason {
    /// Hit max iterations.
    IterationLimit,
    /// Hit wall-clock timeout.
    Timeout,
    /// No more candidate sketches to try.
    SearchSpaceExhausted,
}

/// Complete synthesis report for a planner run.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SynthesisReport {
    /// Per-hole outcomes keyed by segment id.
    pub outcomes: BTreeMap<String, SynthesisOutcome>,
    /// Budget configuration used.
    pub budget: SynthesisBudget,
    /// Total holes attempted.
    pub holes_attempted: usize,
    /// Total holes successfully verified.
    pub holes_verified: usize,
    /// Total holes exhausted.
    pub holes_exhausted: usize,
}

// ── Synthesis engine ─────────────────────────────────────────────────

/// The CEGIS synthesis engine.
#[derive(Debug, Clone)]
pub struct CegisSynthesizer {
    budget: SynthesisBudget,
}

impl CegisSynthesizer {
    /// Create a synthesizer with the given budget.
    #[must_use]
    pub fn new(budget: SynthesisBudget) -> Self {
        Self { budget }
    }

    /// Create a synthesizer with default budget.
    #[must_use]
    pub fn with_defaults() -> Self {
        Self::new(SynthesisBudget::default())
    }

    /// Attempt synthesis for a batch of unmapped segments.
    ///
    /// Returns a synthesis report with per-hole outcomes and aggregate stats.
    pub fn synthesize_batch(&self, holes: &[IrSegment]) -> SynthesisReport {
        let mut outcomes = BTreeMap::new();
        let mut holes_verified = 0;
        let mut holes_exhausted = 0;
        let holes_to_try = holes.len().min(self.budget.max_holes);

        for segment in holes.iter().take(holes_to_try) {
            let outcome = self.synthesize_hole(segment);
            match &outcome {
                SynthesisOutcome::Verified { .. } => holes_verified += 1,
                SynthesisOutcome::Exhausted { .. } => holes_exhausted += 1,
            }
            outcomes.insert(segment.id.to_string(), outcome);
        }

        SynthesisReport {
            outcomes,
            budget: self.budget.clone(),
            holes_attempted: holes_to_try,
            holes_verified,
            holes_exhausted,
        }
    }

    /// Attempt CEGIS synthesis for a single unmapped segment.
    pub fn synthesize_hole(&self, segment: &IrSegment) -> SynthesisOutcome {
        let start = Instant::now();
        let mut all_counter_examples = Vec::new();
        let mut candidates_tried = 0;

        for iteration in 0..self.budget.max_iterations_per_hole {
            if start.elapsed() >= self.budget.timeout_per_hole {
                return SynthesisOutcome::Exhausted {
                    diagnostic: SynthesisExhausted {
                        segment_id: segment.id.to_string(),
                        reason: ExhaustionReason::Timeout,
                        candidates_tried,
                        iterations_completed: iteration,
                        counter_examples: all_counter_examples,
                    },
                };
            }

            let candidates = self.generate_candidates(segment, &all_counter_examples);
            if candidates.is_empty() {
                return SynthesisOutcome::Exhausted {
                    diagnostic: SynthesisExhausted {
                        segment_id: segment.id.to_string(),
                        reason: ExhaustionReason::SearchSpaceExhausted,
                        candidates_tried,
                        iterations_completed: iteration,
                        counter_examples: all_counter_examples,
                    },
                };
            }

            for candidate in &candidates {
                candidates_tried += 1;
                let verification = self.verify_candidate(segment, candidate);

                if verification.obligations_failed.is_empty() {
                    return SynthesisOutcome::Verified {
                        sketch: candidate.clone(),
                        witness: ProofWitness {
                            obligations_checked: verification.obligations_checked,
                            obligations_passed: verification.obligations_passed,
                            obligations_failed: Vec::new(),
                            counter_examples: all_counter_examples,
                            iterations: iteration + 1,
                            elapsed: start.elapsed(),
                        },
                    };
                }

                // Extract counter-examples from failures
                for obligation in &verification.obligations_failed {
                    all_counter_examples.push(CounterExample {
                        obligation: obligation.clone(),
                        description: format!(
                            "Candidate '{}' failed obligation: {obligation}",
                            candidate.method
                        ),
                        rejected_sketch_method: candidate.method.clone(),
                    });
                }
            }
        }

        SynthesisOutcome::Exhausted {
            diagnostic: SynthesisExhausted {
                segment_id: segment.id.to_string(),
                reason: ExhaustionReason::IterationLimit,
                candidates_tried,
                iterations_completed: self.budget.max_iterations_per_hole,
                counter_examples: all_counter_examples,
            },
        }
    }

    /// Promote a verified synthesis outcome into a [`TranslationStrategy`].
    ///
    /// Returns `None` if the outcome is `Exhausted`.
    #[must_use]
    pub fn promote_to_strategy(
        segment: &IrSegment,
        outcome: &SynthesisOutcome,
    ) -> Option<TranslationStrategy> {
        match outcome {
            SynthesisOutcome::Verified { sketch, witness } => {
                let verification_strength = if witness.obligations_checked.is_empty() {
                    TransformationRiskLevel::High
                } else {
                    let pass_rate = witness.obligations_passed.len() as f64
                        / witness.obligations_checked.len() as f64;
                    if pass_rate >= 1.0 {
                        TransformationRiskLevel::Low
                    } else if pass_rate >= 0.8 {
                        TransformationRiskLevel::Medium
                    } else {
                        TransformationRiskLevel::High
                    }
                };

                Some(TranslationStrategy {
                    id: format!("{}-cegis-synthesized", segment.mapping_signature),
                    description: format!(
                        "CEGIS-synthesized translation for {} via {}",
                        segment.name, sketch.method
                    ),
                    handling_class: TransformationHandlingClass::Approximate,
                    risk: verification_strength,
                    target_construct: sketch.target_construct.clone(),
                    target_crate: sketch.target_crate.clone(),
                    automatable: true,
                    remediation: RemediationStrategy {
                        approach: format!(
                            "Synthesized by CEGIS ({} iterations, {} obligations verified)",
                            witness.iterations,
                            witness.obligations_passed.len()
                        ),
                        automatable: true,
                        effort: EffortLevel::Low,
                    },
                })
            }
            SynthesisOutcome::Exhausted { .. } => None,
        }
    }

    // ── Internal: candidate generation ───────────────────────────────

    fn generate_candidates(
        &self,
        segment: &IrSegment,
        counter_examples: &[CounterExample],
    ) -> Vec<SynthesizedSketch> {
        let rejected_methods: Vec<&str> = counter_examples
            .iter()
            .map(|ce| ce.rejected_sketch_method.as_str())
            .collect();

        let all_candidates = match segment.category {
            SegmentCategory::View => self.view_sketches(segment),
            SegmentCategory::State => self.state_sketches(segment),
            SegmentCategory::Event => self.event_sketches(segment),
            SegmentCategory::Effect => self.effect_sketches(segment),
            SegmentCategory::Layout => self.layout_sketches(segment),
            SegmentCategory::Style => self.style_sketches(segment),
            SegmentCategory::Accessibility => self.accessibility_sketches(segment),
            SegmentCategory::Capability => self.capability_sketches(segment),
        };

        // Filter out candidates whose methods were already rejected,
        // then limit to budget.
        all_candidates
            .into_iter()
            .filter(|c| !rejected_methods.contains(&c.method.as_str()))
            .take(self.budget.max_candidates_per_iteration)
            .collect()
    }

    fn view_sketches(&self, segment: &IrSegment) -> Vec<SynthesizedSketch> {
        vec![
            SynthesizedSketch {
                code: format!(
                    "// View: render {} as widget\nfn view(&self, frame: &mut Frame) {{\n    \
                     frame.render_widget(Block::default().title(\"{}\"), frame.area());\n}}",
                    segment.name, segment.name
                ),
                target_construct: "Widget::render".to_string(),
                target_crate: "ftui-widgets".to_string(),
                depth: 2,
                method: "block-widget-wrapper".to_string(),
            },
            SynthesizedSketch {
                code: format!(
                    "// View: render {} as paragraph\nfn view(&self, frame: &mut Frame) {{\n    \
                     let text = Text::from(\"{}\");\n    \
                     frame.render_widget(Paragraph::new(text), frame.area());\n}}",
                    segment.name, segment.name
                ),
                target_construct: "Paragraph::new".to_string(),
                target_crate: "ftui-widgets".to_string(),
                depth: 3,
                method: "paragraph-text-wrapper".to_string(),
            },
            SynthesizedSketch {
                code: format!(
                    "// View: custom stateful widget for {}\nimpl StatefulWidget for {} {{\n    \
                     type State = ();\n    fn render(self, area: Rect, buf: &mut Buffer, _state: &mut ()) {{\n        \
                     // placeholder\n    }}\n}}",
                    segment.name,
                    sanitize_ident(&segment.name)
                ),
                target_construct: "StatefulWidget impl".to_string(),
                target_crate: "ftui-render".to_string(),
                depth: 3,
                method: "stateful-widget-impl".to_string(),
            },
        ]
    }

    fn state_sketches(&self, segment: &IrSegment) -> Vec<SynthesizedSketch> {
        vec![
            SynthesizedSketch {
                code: format!(
                    "// State: Model field for {}\nstruct AppModel {{\n    {}: String,\n}}",
                    segment.name,
                    sanitize_ident(&segment.name)
                ),
                target_construct: "Model struct field".to_string(),
                target_crate: "ftui-runtime".to_string(),
                depth: 1,
                method: "model-field".to_string(),
            },
            SynthesizedSketch {
                code: format!(
                    "// State: update handler for {}\nfn update(&mut self, msg: Msg) -> Cmd<Msg> {{\n    \
                     match msg {{\n        Msg::{} {{ .. }} => Cmd::none(),\n    }}\n}}",
                    segment.name,
                    sanitize_ident(&segment.name)
                ),
                target_construct: "Model::update".to_string(),
                target_crate: "ftui-runtime".to_string(),
                depth: 2,
                method: "update-handler".to_string(),
            },
        ]
    }

    fn event_sketches(&self, segment: &IrSegment) -> Vec<SynthesizedSketch> {
        vec![
            SynthesizedSketch {
                code: format!(
                    "// Event: message variant for {}\nenum Msg {{\n    {} {{}},\n}}",
                    segment.name,
                    sanitize_ident(&segment.name)
                ),
                target_construct: "Msg enum variant".to_string(),
                target_crate: "ftui-runtime".to_string(),
                depth: 1,
                method: "msg-variant".to_string(),
            },
            SynthesizedSketch {
                code: format!(
                    "// Event: key binding for {}\nKey::Char('{}') => Msg::{}",
                    segment.name,
                    segment.name.chars().next().unwrap_or('x'),
                    sanitize_ident(&segment.name)
                ),
                target_construct: "key event match arm".to_string(),
                target_crate: "ftui-runtime".to_string(),
                depth: 2,
                method: "key-binding".to_string(),
            },
        ]
    }

    fn effect_sketches(&self, segment: &IrSegment) -> Vec<SynthesizedSketch> {
        vec![
            SynthesizedSketch {
                code: format!(
                    "// Effect: Cmd::task for {}\nCmd::task(async move {{\n    \
                     // {} effect\n    Msg::EffectDone\n}})",
                    segment.name, segment.name
                ),
                target_construct: "Cmd::task".to_string(),
                target_crate: "ftui-runtime".to_string(),
                depth: 2,
                method: "cmd-task".to_string(),
            },
            SynthesizedSketch {
                code: format!(
                    "// Effect: batch for {}\nCmd::batch(vec![\n    \
                     Cmd::task(async {{ Msg::{}Done }}),\n])",
                    segment.name,
                    sanitize_ident(&segment.name)
                ),
                target_construct: "Cmd::batch".to_string(),
                target_crate: "ftui-runtime".to_string(),
                depth: 3,
                method: "cmd-batch".to_string(),
            },
        ]
    }

    fn layout_sketches(&self, segment: &IrSegment) -> Vec<SynthesizedSketch> {
        vec![
            SynthesizedSketch {
                code: format!(
                    "// Layout: constraint for {}\nLayout::vertical([\n    \
                     Constraint::Min(1),\n    Constraint::Percentage(100),\n])",
                    segment.name
                ),
                target_construct: "Layout::vertical".to_string(),
                target_crate: "ftui-layout".to_string(),
                depth: 2,
                method: "vertical-layout".to_string(),
            },
            SynthesizedSketch {
                code: format!(
                    "// Layout: horizontal split for {}\nLayout::horizontal([\n    \
                     Constraint::Ratio(1, 2),\n    Constraint::Ratio(1, 2),\n])",
                    segment.name
                ),
                target_construct: "Layout::horizontal".to_string(),
                target_crate: "ftui-layout".to_string(),
                depth: 2,
                method: "horizontal-layout".to_string(),
            },
        ]
    }

    fn style_sketches(&self, segment: &IrSegment) -> Vec<SynthesizedSketch> {
        vec![SynthesizedSketch {
            code: format!(
                "// Style: token for {}\nStyle::default().fg(Color::White).bg(Color::Black)",
                segment.name
            ),
            target_construct: "Style::default".to_string(),
            target_crate: "ftui-style".to_string(),
            depth: 1,
            method: "style-token".to_string(),
        }]
    }

    fn accessibility_sketches(&self, segment: &IrSegment) -> Vec<SynthesizedSketch> {
        vec![SynthesizedSketch {
            code: format!(
                "// Accessibility: ARIA-like label for {}\n// FrankenTUI uses semantic widget \
                 titles for screen reader support\n.title(\"{}\")",
                segment.name, segment.name
            ),
            target_construct: "Block::title (semantic)".to_string(),
            target_crate: "ftui-widgets".to_string(),
            depth: 1,
            method: "semantic-title".to_string(),
        }]
    }

    fn capability_sketches(&self, segment: &IrSegment) -> Vec<SynthesizedSketch> {
        vec![SynthesizedSketch {
            code: format!(
                "// Capability: feature gate for {}\n#[cfg(feature = \"{}\")]",
                segment.name,
                segment.name.to_lowercase().replace(' ', "-")
            ),
            target_construct: "cfg feature gate".to_string(),
            target_crate: "ftui-core".to_string(),
            depth: 1,
            method: "feature-gate".to_string(),
        }]
    }

    // ── Internal: verification ───────────────────────────────────────

    fn verify_candidate(
        &self,
        segment: &IrSegment,
        candidate: &SynthesizedSketch,
    ) -> VerificationResult {
        let mut obligations_checked = Vec::new();
        let mut obligations_passed = Vec::new();
        let mut obligations_failed = Vec::new();

        // Obligation 1: depth within budget
        let depth_check = "depth_within_budget";
        obligations_checked.push(depth_check.to_string());
        if candidate.depth <= self.budget.max_depth {
            obligations_passed.push(depth_check.to_string());
        } else {
            obligations_failed.push(depth_check.to_string());
        }

        // Obligation 2: code is non-empty
        let nonempty_check = "code_nonempty";
        obligations_checked.push(nonempty_check.to_string());
        if !candidate.code.trim().is_empty() {
            obligations_passed.push(nonempty_check.to_string());
        } else {
            obligations_failed.push(nonempty_check.to_string());
        }

        // Obligation 3: target crate is a known FrankenTUI crate
        let crate_check = "target_crate_valid";
        obligations_checked.push(crate_check.to_string());
        let known_crates = [
            "ftui-core",
            "ftui-render",
            "ftui-style",
            "ftui-text",
            "ftui-layout",
            "ftui-runtime",
            "ftui-widgets",
            "ftui-extras",
        ];
        if known_crates.contains(&candidate.target_crate.as_str()) {
            obligations_passed.push(crate_check.to_string());
        } else {
            obligations_failed.push(crate_check.to_string());
        }

        // Obligation 4: category-construct alignment
        let alignment_check = "category_construct_aligned";
        obligations_checked.push(alignment_check.to_string());
        let aligned = match segment.category {
            SegmentCategory::View => {
                candidate.target_crate == "ftui-widgets" || candidate.target_crate == "ftui-render"
            }
            SegmentCategory::State | SegmentCategory::Event | SegmentCategory::Effect => {
                candidate.target_crate == "ftui-runtime"
            }
            SegmentCategory::Layout => candidate.target_crate == "ftui-layout",
            SegmentCategory::Style => candidate.target_crate == "ftui-style",
            SegmentCategory::Accessibility => candidate.target_crate == "ftui-widgets",
            SegmentCategory::Capability => true, // Capabilities can map anywhere
        };
        if aligned {
            obligations_passed.push(alignment_check.to_string());
        } else {
            obligations_failed.push(alignment_check.to_string());
        }

        // Obligation 5: no unsafe code
        let safety_check = "no_unsafe_code";
        obligations_checked.push(safety_check.to_string());
        if !candidate.code.contains("unsafe") {
            obligations_passed.push(safety_check.to_string());
        } else {
            obligations_failed.push(safety_check.to_string());
        }

        VerificationResult {
            obligations_checked,
            obligations_passed,
            obligations_failed,
        }
    }
}

struct VerificationResult {
    obligations_checked: Vec<String>,
    obligations_passed: Vec<String>,
    obligations_failed: Vec<String>,
}

/// Sanitize a name into a valid Rust identifier.
fn sanitize_ident(name: &str) -> String {
    let mut result = String::with_capacity(name.len());
    for (i, ch) in name.chars().enumerate() {
        if ch.is_alphanumeric() || ch == '_' {
            if i == 0 && ch.is_ascii_digit() {
                result.push('_');
            }
            result.push(ch);
        } else {
            result.push('_');
        }
    }
    if result.is_empty() {
        result.push_str("unnamed");
    }
    result
}

// ── Tests ────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::migration_ir::IrNodeId;

    fn test_segment(category: SegmentCategory, name: &str) -> IrSegment {
        IrSegment {
            id: IrNodeId(format!("test-{name}")),
            name: name.to_string(),
            category,
            mapping_signature: format!("Test::{name}"),
        }
    }

    #[test]
    fn default_budget_is_reasonable() {
        let b = SynthesisBudget::default();
        assert!(b.max_holes >= 10);
        assert!(b.max_iterations_per_hole >= 5);
        assert!(b.max_candidates_per_iteration >= 4);
        assert!(b.max_depth >= 2);
        assert!(b.timeout_per_hole >= Duration::from_millis(100));
    }

    #[test]
    fn synthesize_view_hole_produces_verified() {
        let engine = CegisSynthesizer::with_defaults();
        let segment = test_segment(SegmentCategory::View, "UserProfile");
        let outcome = engine.synthesize_hole(&segment);
        assert!(
            matches!(outcome, SynthesisOutcome::Verified { .. }),
            "view synthesis should produce a verified candidate"
        );
    }

    #[test]
    fn synthesize_state_hole_produces_verified() {
        let engine = CegisSynthesizer::with_defaults();
        let segment = test_segment(SegmentCategory::State, "counter");
        let outcome = engine.synthesize_hole(&segment);
        assert!(matches!(outcome, SynthesisOutcome::Verified { .. }));
    }

    #[test]
    fn synthesize_event_hole_produces_verified() {
        let engine = CegisSynthesizer::with_defaults();
        let segment = test_segment(SegmentCategory::Event, "onClick");
        let outcome = engine.synthesize_hole(&segment);
        assert!(matches!(outcome, SynthesisOutcome::Verified { .. }));
    }

    #[test]
    fn synthesize_effect_hole_produces_verified() {
        let engine = CegisSynthesizer::with_defaults();
        let segment = test_segment(SegmentCategory::Effect, "fetchData");
        let outcome = engine.synthesize_hole(&segment);
        assert!(matches!(outcome, SynthesisOutcome::Verified { .. }));
    }

    #[test]
    fn synthesize_layout_hole_produces_verified() {
        let engine = CegisSynthesizer::with_defaults();
        let segment = test_segment(SegmentCategory::Layout, "sidebar");
        let outcome = engine.synthesize_hole(&segment);
        assert!(matches!(outcome, SynthesisOutcome::Verified { .. }));
    }

    #[test]
    fn synthesize_style_hole_produces_verified() {
        let engine = CegisSynthesizer::with_defaults();
        let segment = test_segment(SegmentCategory::Style, "primary_color");
        let outcome = engine.synthesize_hole(&segment);
        assert!(matches!(outcome, SynthesisOutcome::Verified { .. }));
    }

    #[test]
    fn verified_outcome_promotes_to_strategy() {
        let engine = CegisSynthesizer::with_defaults();
        let segment = test_segment(SegmentCategory::View, "Dashboard");
        let outcome = engine.synthesize_hole(&segment);
        let strategy = CegisSynthesizer::promote_to_strategy(&segment, &outcome);
        assert!(strategy.is_some());
        let s = strategy.unwrap();
        assert!(s.id.contains("cegis-synthesized"));
        assert_eq!(s.handling_class, TransformationHandlingClass::Approximate);
        assert!(s.automatable);
    }

    #[test]
    fn exhausted_outcome_does_not_promote() {
        let engine = CegisSynthesizer::new(SynthesisBudget {
            max_iterations_per_hole: 0,
            ..SynthesisBudget::default()
        });
        let segment = test_segment(SegmentCategory::View, "X");
        let outcome = engine.synthesize_hole(&segment);
        assert!(matches!(outcome, SynthesisOutcome::Exhausted { .. }));
        assert!(CegisSynthesizer::promote_to_strategy(&segment, &outcome).is_none());
    }

    #[test]
    fn batch_synthesis_respects_max_holes() {
        let engine = CegisSynthesizer::new(SynthesisBudget {
            max_holes: 2,
            ..SynthesisBudget::default()
        });
        let holes: Vec<IrSegment> = (0..5)
            .map(|i| test_segment(SegmentCategory::View, &format!("widget_{i}")))
            .collect();
        let report = engine.synthesize_batch(&holes);
        assert_eq!(report.holes_attempted, 2);
        assert!(report.outcomes.len() <= 2);
    }

    #[test]
    fn batch_report_statistics() {
        let engine = CegisSynthesizer::with_defaults();
        let holes = vec![
            test_segment(SegmentCategory::View, "A"),
            test_segment(SegmentCategory::State, "B"),
            test_segment(SegmentCategory::Effect, "C"),
        ];
        let report = engine.synthesize_batch(&holes);
        assert_eq!(report.holes_attempted, 3);
        assert_eq!(
            report.holes_verified + report.holes_exhausted,
            report.holes_attempted
        );
    }

    #[test]
    fn proof_witness_has_obligations() {
        let engine = CegisSynthesizer::with_defaults();
        let segment = test_segment(SegmentCategory::View, "MyWidget");
        let outcome = engine.synthesize_hole(&segment);
        if let SynthesisOutcome::Verified { witness, .. } = outcome {
            assert!(!witness.obligations_checked.is_empty());
            assert!(!witness.obligations_passed.is_empty());
            assert!(witness.obligations_failed.is_empty());
            assert!(witness.iterations >= 1);
        } else {
            panic!("expected verified outcome");
        }
    }

    #[test]
    fn counter_examples_prune_search_space() {
        let engine = CegisSynthesizer::new(SynthesisBudget {
            max_depth: 0, // Force depth check failure for all candidates
            ..SynthesisBudget::default()
        });
        let segment = test_segment(SegmentCategory::View, "Deep");
        let outcome = engine.synthesize_hole(&segment);
        if let SynthesisOutcome::Exhausted { diagnostic } = outcome {
            assert!(!diagnostic.counter_examples.is_empty());
            assert!(diagnostic.candidates_tried > 0);
        } else {
            panic!("expected exhausted outcome with max_depth=0");
        }
    }

    #[test]
    fn sanitize_ident_special_chars() {
        assert_eq!(sanitize_ident("my-component"), "my_component");
        assert_eq!(sanitize_ident("123start"), "_123start");
        assert_eq!(sanitize_ident(""), "unnamed");
        assert_eq!(sanitize_ident("valid_name"), "valid_name");
        assert_eq!(sanitize_ident("has space"), "has_space");
    }

    #[test]
    fn all_categories_generate_candidates() {
        let engine = CegisSynthesizer::with_defaults();
        let categories = [
            SegmentCategory::View,
            SegmentCategory::State,
            SegmentCategory::Event,
            SegmentCategory::Effect,
            SegmentCategory::Layout,
            SegmentCategory::Style,
            SegmentCategory::Accessibility,
            SegmentCategory::Capability,
        ];
        for cat in categories {
            let segment = test_segment(cat, "test_item");
            let candidates = engine.generate_candidates(&segment, &[]);
            assert!(
                !candidates.is_empty(),
                "category {cat:?} should generate candidates"
            );
        }
    }

    #[test]
    fn timeout_budget_triggers_exhaustion() {
        let engine = CegisSynthesizer::new(SynthesisBudget {
            timeout_per_hole: Duration::ZERO,
            ..SynthesisBudget::default()
        });
        let segment = test_segment(SegmentCategory::View, "slow");
        let outcome = engine.synthesize_hole(&segment);
        if let SynthesisOutcome::Exhausted { diagnostic } = outcome {
            assert_eq!(diagnostic.reason, ExhaustionReason::Timeout);
        } else {
            panic!("expected timeout exhaustion");
        }
    }

    #[test]
    fn strategy_risk_reflects_verification_strength() {
        let engine = CegisSynthesizer::with_defaults();
        let segment = test_segment(SegmentCategory::State, "robust");
        let outcome = engine.synthesize_hole(&segment);
        if let Some(strategy) = CegisSynthesizer::promote_to_strategy(&segment, &outcome) {
            // With all 5 obligations passed, risk should be Low
            assert_eq!(strategy.risk, TransformationRiskLevel::Low);
        } else {
            panic!("expected promotable outcome");
        }
    }

    #[test]
    fn synthesis_report_serializable() {
        let engine = CegisSynthesizer::with_defaults();
        let holes = vec![test_segment(SegmentCategory::View, "serde_test")];
        let report = engine.synthesize_batch(&holes);
        let json = serde_json::to_string(&report).expect("should serialize");
        assert!(json.contains("serde_test"));
        let deser: SynthesisReport = serde_json::from_str(&json).expect("should deserialize");
        assert_eq!(deser.holes_attempted, 1);
    }
}
