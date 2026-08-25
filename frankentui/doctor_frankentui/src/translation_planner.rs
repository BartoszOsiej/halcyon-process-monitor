// SPDX-License-Identifier: Apache-2.0
//! Translation planner with confidence-ranked strategies.
//!
//! Consumes a [`MigrationIr`], the `MappingAtlas`, and the
//! [`ConfidenceModel`] to emit an ordered list of translation strategy
//! decisions — one per IR segment — ranked by confidence and risk.
//!
//! Design invariants:
//! - **Determinism**: tie-breaking uses lexicographic ordering on segment
//!   id so that plans are reproducible across runs with the same input.
//! - **Decision records**: every segment emits a [`StrategyDecision`] that
//!   captures the chosen strategy, alternatives, rationale, and Bayesian
//!   posterior used for gating.
//! - **Capability-gap tickets**: when a mapping is `Unsupported` or
//!   `ExtendFtui`, the planner emits a [`CapabilityGapTicket`] for
//!   downstream triage.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::effect_canonical::CanonicalEffectModel;
use crate::intent_inference::IntentInferenceResult;
use crate::mapping_atlas::{
    MappingCategory, MappingEntry, RemediationStrategy, build_atlas, lookup,
};
use crate::migration_ir::{
    Capability, EffectKind, EventKind, IrNodeId, LayoutKind, MigrationIr, StateScope,
    TokenCategory, ViewNodeKind,
};
use crate::semantic_contract::{
    BayesianPosterior, ConfidenceModel, ExpectedLossResult, MigrationDecision,
    TransformationHandlingClass, TransformationRiskLevel,
};

// ── Constants ──────────────────────────────────────────────────────────

/// Current planner schema version.
pub const PLANNER_VERSION: &str = "translation-planner-v1";

/// Default seed for deterministic tie-breaking.
pub const DEFAULT_SEED: u64 = 0x_F7A4_D12B;

// ── Core Output Types ──────────────────────────────────────────────────

/// The complete translation plan produced by the planner.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TranslationPlan {
    /// Planner schema version.
    pub version: String,
    /// Unique run identifier (inherited from the IR).
    pub run_id: String,
    /// Deterministic seed used for tie-breaking.
    pub seed: u64,
    /// Per-segment strategy decisions, ordered by segment id.
    pub decisions: Vec<StrategyDecision>,
    /// Capability-gap tickets for segments that cannot be auto-translated.
    pub gap_tickets: Vec<CapabilityGapTicket>,
    /// Aggregate plan statistics.
    pub stats: PlanStats,
}

/// A strategy decision for a single IR segment.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StrategyDecision {
    /// The IR segment this decision applies to.
    pub segment: IrSegment,
    /// The chosen translation strategy.
    pub chosen: TranslationStrategy,
    /// Alternatives that were considered, ranked by score.
    pub alternatives: Vec<RankedAlternative>,
    /// The Bayesian posterior used to gate this decision.
    pub posterior: BayesianPosterior,
    /// The expected-loss result driving the decision.
    pub expected_loss: ExpectedLossResult,
    /// The gating decision from the confidence model.
    pub gate: MigrationDecision,
    /// Composite confidence score in [0, 1].
    pub confidence: f64,
    /// Human-readable rationale.
    pub rationale: String,
}

/// Identifies an IR segment that the planner reasons about.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct IrSegment {
    /// Node or construct id.
    pub id: IrNodeId,
    /// Human-readable name from the IR.
    pub name: String,
    /// Category that this segment belongs to.
    pub category: SegmentCategory,
    /// The mapping signature used for atlas lookup.
    pub mapping_signature: String,
}

/// Categories that IR segments can belong to, mirroring [`MappingCategory`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum SegmentCategory {
    View,
    State,
    Event,
    Effect,
    Layout,
    Style,
    Accessibility,
    Capability,
}

impl From<MappingCategory> for SegmentCategory {
    fn from(mc: MappingCategory) -> Self {
        match mc {
            MappingCategory::View => SegmentCategory::View,
            MappingCategory::State => SegmentCategory::State,
            MappingCategory::Event => SegmentCategory::Event,
            MappingCategory::Effect => SegmentCategory::Effect,
            MappingCategory::Layout => SegmentCategory::Layout,
            MappingCategory::Style => SegmentCategory::Style,
            MappingCategory::Accessibility => SegmentCategory::Accessibility,
            MappingCategory::Capability => SegmentCategory::Capability,
        }
    }
}

/// A concrete translation strategy.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TranslationStrategy {
    /// Strategy identifier (e.g. "direct-model-impl", "cmd-task-bridge").
    pub id: String,
    /// What this strategy does.
    pub description: String,
    /// The transformation policy class from the atlas.
    pub handling_class: TransformationHandlingClass,
    /// Risk level from the atlas.
    pub risk: TransformationRiskLevel,
    /// Target FrankenTUI construct.
    pub target_construct: String,
    /// Target crate.
    pub target_crate: String,
    /// Whether this strategy is fully automatable.
    pub automatable: bool,
    /// Remediation steps if the strategy fails.
    pub remediation: RemediationStrategy,
}

/// A ranked alternative strategy (not chosen).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RankedAlternative {
    /// The strategy.
    pub strategy: TranslationStrategy,
    /// Composite score in [0, 1].
    pub score: f64,
    /// Why this was not chosen.
    pub rejection_reason: String,
}

/// A capability-gap ticket emitted when no mapping exists.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CapabilityGapTicket {
    /// The segment that triggered this ticket.
    pub segment: IrSegment,
    /// Gap classification.
    pub gap_kind: GapKind,
    /// Human-readable description of the gap.
    pub description: String,
    /// Suggested remediation.
    pub suggested_remediation: String,
    /// Priority based on risk level.
    pub priority: GapPriority,
}

/// What kind of capability gap was detected.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum GapKind {
    /// No mapping exists at all (Unsupported).
    Unsupported,
    /// Mapping requires FrankenTUI extension (ExtendFtui).
    RequiresExtension,
    /// Mapping exists but confidence is too low.
    LowConfidence,
}

/// Priority for capability-gap tickets.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum GapPriority {
    Critical,
    High,
    Medium,
    Low,
}

/// Aggregate statistics for the translation plan.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlanStats {
    /// Total segments processed.
    pub total_segments: usize,
    /// Segments with auto-approvable strategies.
    pub auto_approve: usize,
    /// Segments requiring human review.
    pub human_review: usize,
    /// Segments rejected.
    pub rejected: usize,
    /// Capability-gap tickets emitted.
    pub gap_tickets: usize,
    /// Average confidence across all decisions.
    pub mean_confidence: f64,
    /// Counts by category.
    pub by_category: BTreeMap<String, usize>,
    /// Counts by handling class.
    pub by_handling_class: BTreeMap<String, usize>,
}

// ── Planner Configuration ──────────────────────────────────────────────

/// Configuration knobs for the translation planner.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlannerConfig {
    /// Deterministic seed for tie-breaking.
    pub seed: u64,
    /// Minimum confidence threshold below which a gap ticket is emitted.
    pub min_confidence_threshold: f64,
    /// Whether to include intent-inference signals in scoring.
    pub use_intent_signals: bool,
    /// Whether to include canonical-effect signals in scoring.
    pub use_effect_signals: bool,
}

impl Default for PlannerConfig {
    fn default() -> Self {
        Self {
            seed: DEFAULT_SEED,
            min_confidence_threshold: 0.3,
            use_intent_signals: true,
            use_effect_signals: true,
        }
    }
}

#[derive(Debug, Clone, Copy, Default)]
struct PlannerContext {
    process_spawn_available: bool,
}

// ── Public API ─────────────────────────────────────────────────────────

/// Build a translation plan from the IR and its enrichment inputs.
///
/// The planner:
/// 1. Enumerates all translatable segments from the IR.
/// 2. Looks up each segment in the `MappingAtlas`.
/// 3. Scores candidate strategies using the [`ConfidenceModel`] posterior.
/// 4. Emits [`CapabilityGapTicket`]s for unmapped or low-confidence segments.
/// 5. Orders decisions deterministically by segment id.
pub fn plan_translation(
    ir: &MigrationIr,
    confidence_model: &ConfidenceModel,
    intents: Option<&IntentInferenceResult>,
    effects: Option<&CanonicalEffectModel>,
    config: &PlannerConfig,
) -> TranslationPlan {
    let atlas = build_atlas();
    let context = planner_context(ir);

    // Step 1: enumerate segments.
    let segments = enumerate_segments(ir);

    // Step 2-4: plan each segment.
    let mut decisions = Vec::with_capacity(segments.len());
    let mut gap_tickets = Vec::new();

    for segment in &segments {
        let mapping = lookup(&atlas, &segment.mapping_signature);
        let decision = plan_segment(
            segment,
            mapping,
            confidence_model,
            intents,
            effects,
            config,
            &context,
            &mut gap_tickets,
        );
        decisions.push(decision);
    }

    // Step 5: stable sort by segment id (already iterated in BTreeMap order,
    // but sort to enforce the invariant).
    decisions.sort_by(|a, b| a.segment.id.cmp(&b.segment.id));
    gap_tickets.sort_by(|a, b| a.segment.id.cmp(&b.segment.id));

    let stats = compute_stats(&decisions, &gap_tickets);

    TranslationPlan {
        version: PLANNER_VERSION.to_string(),
        run_id: ir.run_id.clone(),
        seed: config.seed,
        decisions,
        gap_tickets,
        stats,
    }
}

fn planner_context(ir: &MigrationIr) -> PlannerContext {
    let process_spawn_available = ir.capabilities.required.contains(&Capability::ProcessSpawn)
        || ir.capabilities.optional.contains(&Capability::ProcessSpawn);
    PlannerContext {
        process_spawn_available,
    }
}

/// Convenience: plan with default config and no enrichment.
pub fn plan_translation_simple(
    ir: &MigrationIr,
    confidence_model: &ConfidenceModel,
) -> TranslationPlan {
    plan_translation(ir, confidence_model, None, None, &PlannerConfig::default())
}

// ── Segment Enumeration ────────────────────────────────────────────────

fn enumerate_segments(ir: &MigrationIr) -> Vec<IrSegment> {
    let mut segments = Vec::new();

    // View nodes.
    for (id, node) in &ir.view_tree.nodes {
        segments.push(IrSegment {
            id: id.clone(),
            name: node.name.clone(),
            category: SegmentCategory::View,
            mapping_signature: view_kind_signature(&node.kind),
        });
    }

    // State variables.
    for (id, var) in &ir.state_graph.variables {
        segments.push(IrSegment {
            id: id.clone(),
            name: var.name.clone(),
            category: SegmentCategory::State,
            mapping_signature: state_scope_signature(&var.scope),
        });
    }

    // Events.
    for (id, evt) in &ir.event_catalog.events {
        segments.push(IrSegment {
            id: id.clone(),
            name: evt.name.clone(),
            category: SegmentCategory::Event,
            mapping_signature: event_kind_signature(&evt.kind),
        });
    }

    // Effects.
    for (id, eff) in &ir.effect_registry.effects {
        segments.push(IrSegment {
            id: id.clone(),
            name: eff.name.clone(),
            category: SegmentCategory::Effect,
            mapping_signature: effect_kind_signature(&eff.kind),
        });
    }

    // Layout intents.
    for (node_id, layout) in &ir.style_intent.layouts {
        segments.push(IrSegment {
            id: node_id.clone(),
            name: format!("layout-{}", layout_kind_label(&layout.kind)),
            category: SegmentCategory::Layout,
            mapping_signature: layout_kind_signature(&layout.kind),
        });
    }

    // Style tokens.
    for (name, token) in &ir.style_intent.tokens {
        segments.push(IrSegment {
            id: IrNodeId(format!("ir-style-{name}")),
            name: token.name.clone(),
            category: SegmentCategory::Style,
            mapping_signature: token_category_signature(&token.category),
        });
    }

    // Theme declarations.
    for (i, theme) in ir.style_intent.themes.iter().enumerate() {
        segments.push(IrSegment {
            id: IrNodeId(format!("ir-theme-{i}")),
            name: theme.name.clone(),
            category: SegmentCategory::Style,
            mapping_signature: "ThemeDecl".to_string(),
        });
    }

    // Capabilities.
    let mut seen_caps = BTreeSet::new();
    for cap in ir
        .capabilities
        .required
        .iter()
        .chain(ir.capabilities.optional.iter())
    {
        let cap_name = format!("{cap:?}");
        if !seen_caps.insert(cap_name.clone()) {
            continue;
        }
        let sig = format!("Capability::{cap:?}");
        segments.push(IrSegment {
            id: IrNodeId(format!("ir-cap-{}", cap_name.to_lowercase())),
            name: cap_name,
            category: SegmentCategory::Capability,
            mapping_signature: sig,
        });
    }

    // Sort for determinism.
    segments.sort();
    segments
}

fn view_kind_signature(kind: &ViewNodeKind) -> String {
    match kind {
        ViewNodeKind::Component => "ViewNodeKind::Component".to_string(),
        ViewNodeKind::Element => "ViewNodeKind::Element".to_string(),
        ViewNodeKind::Fragment => "ViewNodeKind::Fragment".to_string(),
        ViewNodeKind::Portal => "ViewNodeKind::Portal".to_string(),
        ViewNodeKind::Provider => "ViewNodeKind::Provider".to_string(),
        ViewNodeKind::Consumer => "ViewNodeKind::Consumer".to_string(),
        ViewNodeKind::Route => "ViewNodeKind::Route".to_string(),
    }
}

fn state_scope_signature(scope: &StateScope) -> String {
    match scope {
        StateScope::Local => "StateScope::Local".to_string(),
        StateScope::Context => "StateScope::Context".to_string(),
        StateScope::Global => "StateScope::Global".to_string(),
        StateScope::Route => "StateScope::Route".to_string(),
        StateScope::Server => "StateScope::Server".to_string(),
    }
}

fn event_kind_signature(kind: &EventKind) -> String {
    match kind {
        EventKind::UserInput => "EventKind::UserInput".to_string(),
        EventKind::Lifecycle => "EventKind::Lifecycle".to_string(),
        EventKind::Timer => "EventKind::Timer".to_string(),
        EventKind::Network => "EventKind::Network".to_string(),
        EventKind::Custom => "EventKind::Custom".to_string(),
    }
}

fn effect_kind_signature(kind: &EffectKind) -> String {
    match kind {
        EffectKind::Dom => "EffectKind::Dom".to_string(),
        EffectKind::Network => "EffectKind::Network".to_string(),
        EffectKind::Timer => "EffectKind::Timer".to_string(),
        EffectKind::Storage => "EffectKind::Storage".to_string(),
        EffectKind::Subscription => "EffectKind::Subscription".to_string(),
        EffectKind::Process => "EffectKind::Process".to_string(),
        EffectKind::Telemetry => "EffectKind::Telemetry".to_string(),
        EffectKind::Other => "EffectKind::Other".to_string(),
    }
}

fn layout_kind_signature(kind: &LayoutKind) -> String {
    match kind {
        LayoutKind::Flex => "LayoutKind::Flex".to_string(),
        LayoutKind::Grid => "LayoutKind::Grid".to_string(),
        LayoutKind::Absolute => "LayoutKind::Absolute".to_string(),
        LayoutKind::Stack => "LayoutKind::Stack".to_string(),
        LayoutKind::Flow => "LayoutKind::Flow".to_string(),
    }
}

fn layout_kind_label(kind: &LayoutKind) -> &'static str {
    match kind {
        LayoutKind::Flex => "flex",
        LayoutKind::Grid => "grid",
        LayoutKind::Absolute => "absolute",
        LayoutKind::Stack => "stack",
        LayoutKind::Flow => "flow",
    }
}

fn token_category_signature(category: &TokenCategory) -> String {
    match category {
        TokenCategory::Color => "StyleToken::Color".to_string(),
        TokenCategory::Spacing => "StyleToken::Spacing".to_string(),
        TokenCategory::Typography => "StyleToken::Typography".to_string(),
        TokenCategory::Border => "StyleToken::Border".to_string(),
        TokenCategory::Shadow => "StyleToken::Shadow".to_string(),
        TokenCategory::Animation => "StyleToken::Animation".to_string(),
        TokenCategory::Breakpoint => "StyleToken::Breakpoint".to_string(),
        TokenCategory::ZIndex => "StyleToken::ZIndex".to_string(),
    }
}

// ── Per-Segment Planning ───────────────────────────────────────────────

#[allow(clippy::too_many_arguments)]
fn plan_segment(
    segment: &IrSegment,
    mapping: Option<&MappingEntry>,
    confidence_model: &ConfidenceModel,
    intents: Option<&IntentInferenceResult>,
    effects: Option<&CanonicalEffectModel>,
    config: &PlannerConfig,
    context: &PlannerContext,
    gap_tickets: &mut Vec<CapabilityGapTicket>,
) -> StrategyDecision {
    let use_process_fallback = should_use_process_fallback(segment, context);

    // Compute base evidence counts from the mapping quality.
    let (successes, failures) = mapping_evidence(mapping, use_process_fallback);

    // Apply enrichment signals.
    let (intent_boost, effect_boost) = compute_enrichment_boosts(segment, intents, effects, config);

    let total_successes = successes + intent_boost;
    let total_failures = failures;

    // Compute posterior and decision.
    let posterior = confidence_model.compute_posterior(total_successes, total_failures);
    let expected_loss = confidence_model.expected_loss_decision(&posterior, None, None);
    let gate = expected_loss.decision;

    // Build candidate strategies.
    let (chosen, alternatives) =
        select_strategy(segment, mapping, &posterior, config, use_process_fallback);

    // Composite confidence = posterior mean weighted by strategy quality.
    let confidence = compute_composite_confidence(&chosen, &posterior, effect_boost);

    // Emit gap tickets for unsupported/low-confidence segments.
    if let Some(entry) = mapping {
        match entry.policy {
            TransformationHandlingClass::Unsupported => {
                gap_tickets.push(CapabilityGapTicket {
                    segment: segment.clone(),
                    gap_kind: GapKind::Unsupported,
                    description: format!(
                        "No translation available for {} ({})",
                        segment.name, segment.mapping_signature
                    ),
                    suggested_remediation: entry.remediation.approach.clone(),
                    priority: risk_to_gap_priority(&entry.risk),
                });
            }
            TransformationHandlingClass::ExtendFtui => {
                gap_tickets.push(CapabilityGapTicket {
                    segment: segment.clone(),
                    gap_kind: GapKind::RequiresExtension,
                    description: format!(
                        "Translation requires FrankenTUI extension for {} ({})",
                        segment.name, segment.mapping_signature
                    ),
                    suggested_remediation: entry.remediation.approach.clone(),
                    priority: risk_to_gap_priority(&entry.risk),
                });
            }
            _ => {}
        }
    } else {
        gap_tickets.push(CapabilityGapTicket {
            segment: segment.clone(),
            gap_kind: GapKind::Unsupported,
            description: format!(
                "No atlas mapping found for {} ({})",
                segment.name, segment.mapping_signature
            ),
            suggested_remediation: "Manual implementation required".to_string(),
            priority: GapPriority::High,
        });
    }

    if confidence < config.min_confidence_threshold
        && let Some(entry) = mapping
        && entry.policy != TransformationHandlingClass::Unsupported
        && entry.policy != TransformationHandlingClass::ExtendFtui
    {
        gap_tickets.push(CapabilityGapTicket {
            segment: segment.clone(),
            gap_kind: GapKind::LowConfidence,
            description: format!(
                "Confidence {confidence:.2} below threshold {} for {}",
                config.min_confidence_threshold, segment.name
            ),
            suggested_remediation: "Increase test evidence or review manually".to_string(),
            priority: GapPriority::Medium,
        });
    }

    let rationale = build_rationale(segment, mapping, &posterior, gate, use_process_fallback);

    StrategyDecision {
        segment: segment.clone(),
        chosen,
        alternatives,
        posterior,
        expected_loss,
        gate,
        confidence,
        rationale,
    }
}

/// Convert atlas evidence into Bayesian success/failure counts.
fn mapping_evidence(mapping: Option<&MappingEntry>, use_process_fallback: bool) -> (u32, u32) {
    if use_process_fallback {
        // Degraded path: process effects can still translate via Cmd::task fallback,
        // but with less certainty than native process-subscription support.
        return (10, 3);
    }

    match mapping {
        Some(entry) => {
            let base_successes: u32 = match entry.policy {
                TransformationHandlingClass::Exact => 20,
                TransformationHandlingClass::Approximate => 10,
                TransformationHandlingClass::ExtendFtui => 3,
                TransformationHandlingClass::Unsupported => 0,
            };
            let base_failures: u32 = match entry.risk {
                TransformationRiskLevel::Low => 1,
                TransformationRiskLevel::Medium => 3,
                TransformationRiskLevel::High => 8,
                TransformationRiskLevel::Critical => 15,
            };
            (base_successes, base_failures)
        }
        None => (0, 5),
    }
}

/// Compute enrichment boosts from intent-inference and canonical-effects.
fn compute_enrichment_boosts(
    segment: &IrSegment,
    intents: Option<&IntentInferenceResult>,
    effects: Option<&CanonicalEffectModel>,
    config: &PlannerConfig,
) -> (u32, f64) {
    let mut intent_boost: u32 = 0;
    let mut effect_score: f64 = 0.0;

    if config.use_intent_signals
        && let Some(intent_result) = intents
    {
        let intent_conf = intent_result.overall_confidence.score;
        intent_boost = (intent_conf * 5.0) as u32;
    }

    if config.use_effect_signals
        && let Some(effect_model) = effects
        && segment.category == SegmentCategory::Effect
        && let Some(canonical) = effect_model.effects.get(&segment.id)
    {
        effect_score = canonical.confidence.score;
    }

    (intent_boost, effect_score)
}

/// Select the best strategy and rank alternatives.
fn select_strategy(
    segment: &IrSegment,
    mapping: Option<&MappingEntry>,
    posterior: &BayesianPosterior,
    _config: &PlannerConfig,
    use_process_fallback: bool,
) -> (TranslationStrategy, Vec<RankedAlternative>) {
    if use_process_fallback {
        let fallback = TranslationStrategy {
            id: format!("{}-in-process-fallback", segment.mapping_signature),
            description: format!(
                "Translate {} using bounded in-process task fallback",
                segment.name
            ),
            handling_class: TransformationHandlingClass::Approximate,
            risk: TransformationRiskLevel::Medium,
            target_construct: "Cmd::task(...) bounded in-process worker".to_string(),
            target_crate: "ftui-runtime".to_string(),
            automatable: true,
            remediation: RemediationStrategy {
                approach: "Process spawn capability missing; emit Cmd::task fallback and avoid external process dependency".to_string(),
                automatable: true,
                effort: crate::mapping_atlas::EffortLevel::Medium,
            },
        };

        let mut alternatives = Vec::new();
        if let Some(entry) = mapping {
            alternatives.push(RankedAlternative {
                strategy: strategy_from_mapping(entry, &segment.mapping_signature),
                score: posterior.mean * 0.6,
                rejection_reason:
                    "Requires process spawn capability which is not declared by the IR".to_string(),
            });
        }
        return (fallback, alternatives);
    }

    match mapping {
        Some(entry) => {
            let primary = strategy_from_mapping(entry, &segment.mapping_signature);
            // For Approximate mappings, also offer a manual alternative.
            let mut alternatives = Vec::new();
            if entry.policy == TransformationHandlingClass::Approximate {
                let manual = TranslationStrategy {
                    id: format!("{}-manual-review", segment.mapping_signature),
                    description: format!(
                        "Manual translation of {} with human review",
                        segment.name
                    ),
                    handling_class: TransformationHandlingClass::Exact,
                    risk: TransformationRiskLevel::Low,
                    target_construct: entry.target.construct.clone(),
                    target_crate: entry.target.crate_name.clone(),
                    automatable: false,
                    remediation: entry.remediation.clone(),
                };
                alternatives.push(RankedAlternative {
                    strategy: manual,
                    score: posterior.mean * 0.8,
                    rejection_reason: "Requires manual effort; automated strategy preferred"
                        .to_string(),
                });
            }
            (primary, alternatives)
        }
        None => {
            let fallback = TranslationStrategy {
                id: format!("{}-unmapped-stub", segment.mapping_signature),
                description: format!("Stub placeholder for unmapped construct {}", segment.name),
                handling_class: TransformationHandlingClass::Unsupported,
                risk: TransformationRiskLevel::High,
                target_construct: "todo!() stub".to_string(),
                target_crate: "unknown".to_string(),
                automatable: false,
                remediation: RemediationStrategy {
                    approach: "Manual implementation required".to_string(),
                    automatable: false,
                    effort: crate::mapping_atlas::EffortLevel::High,
                },
            };
            (fallback, Vec::new())
        }
    }
}

fn should_use_process_fallback(segment: &IrSegment, context: &PlannerContext) -> bool {
    segment.mapping_signature == "EffectKind::Process" && !context.process_spawn_available
}

fn strategy_from_mapping(entry: &MappingEntry, signature: &str) -> TranslationStrategy {
    TranslationStrategy {
        id: format!("{}-auto", signature),
        description: format!(
            "Translate {} → {} ({})",
            entry.source_signature, entry.target.construct, entry.target.crate_name
        ),
        handling_class: entry.policy,
        risk: entry.risk,
        target_construct: entry.target.construct.clone(),
        target_crate: entry.target.crate_name.clone(),
        automatable: entry.remediation.automatable,
        remediation: entry.remediation.clone(),
    }
}

fn compute_composite_confidence(
    strategy: &TranslationStrategy,
    posterior: &BayesianPosterior,
    effect_boost: f64,
) -> f64 {
    let policy_weight: f64 = match strategy.handling_class {
        TransformationHandlingClass::Exact => 1.0,
        TransformationHandlingClass::Approximate => 0.7,
        TransformationHandlingClass::ExtendFtui => 0.4,
        TransformationHandlingClass::Unsupported => 0.1,
    };
    let base: f64 = posterior.mean * policy_weight;
    let boosted: f64 = base + effect_boost * 0.1;
    boosted.clamp(0.0, 1.0)
}

fn risk_to_gap_priority(risk: &TransformationRiskLevel) -> GapPriority {
    match risk {
        TransformationRiskLevel::Critical => GapPriority::Critical,
        TransformationRiskLevel::High => GapPriority::High,
        TransformationRiskLevel::Medium => GapPriority::Medium,
        TransformationRiskLevel::Low => GapPriority::Low,
    }
}

fn build_rationale(
    segment: &IrSegment,
    mapping: Option<&MappingEntry>,
    posterior: &BayesianPosterior,
    gate: MigrationDecision,
    use_process_fallback: bool,
) -> String {
    let mapping_desc = match mapping {
        Some(entry) => format!(
            "Atlas mapping: {} → {} ({:?}, {:?} risk)",
            entry.source_signature, entry.target.construct, entry.policy, entry.risk
        ),
        None => "No atlas mapping found".to_string(),
    };
    let precondition_note = if use_process_fallback {
        " Process precondition not met; selected Cmd::task fallback."
    } else {
        ""
    };

    format!(
        "Segment {} ({}): {}. Posterior mean={:.3} [CI: {:.3}–{:.3}]. Gate: {:?}.",
        segment.id.0,
        segment.name,
        mapping_desc,
        posterior.mean,
        posterior.credible_lower,
        posterior.credible_upper,
        gate
    ) + precondition_note
}

// ── Statistics ──────────────────────────────────────────────────────────

fn compute_stats(decisions: &[StrategyDecision], gap_tickets: &[CapabilityGapTicket]) -> PlanStats {
    let total = decisions.len();
    let auto_approve = decisions
        .iter()
        .filter(|d| d.gate == MigrationDecision::AutoApprove)
        .count();
    let human_review = decisions
        .iter()
        .filter(|d| d.gate == MigrationDecision::HumanReview)
        .count();
    let rejected = decisions
        .iter()
        .filter(|d| {
            matches!(
                d.gate,
                MigrationDecision::Reject | MigrationDecision::HardReject
            )
        })
        .count();

    let mean_confidence = if total > 0 {
        decisions.iter().map(|d| d.confidence).sum::<f64>() / total as f64
    } else {
        0.0
    };

    let mut by_category: BTreeMap<String, usize> = BTreeMap::new();
    let mut by_handling_class: BTreeMap<String, usize> = BTreeMap::new();

    for d in decisions {
        *by_category
            .entry(format!("{:?}", d.segment.category))
            .or_insert(0) += 1;
        *by_handling_class
            .entry(format!("{:?}", d.chosen.handling_class))
            .or_insert(0) += 1;
    }

    PlanStats {
        total_segments: total,
        auto_approve,
        human_review,
        rejected,
        gap_tickets: gap_tickets.len(),
        mean_confidence,
        by_category,
        by_handling_class,
    }
}

// ── Tests ──────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::*;
    use crate::migration_ir::{
        EffectDecl, EventDecl, IrBuilder, Provenance, StateVariable, ViewNode,
    };
    use crate::semantic_contract::load_builtin_confidence_model;

    fn test_model() -> ConfidenceModel {
        load_builtin_confidence_model().expect("builtin confidence model")
    }

    fn test_provenance() -> Provenance {
        Provenance {
            file: "test.tsx".to_string(),
            line: 1,
            column: None,
            source_name: None,
            policy_category: None,
        }
    }

    fn minimal_ir() -> MigrationIr {
        let mut builder = IrBuilder::new("test-planner".to_string(), "planner-tests".to_string());
        let node_id = IrNodeId("ir-node-app".to_string());
        builder.add_root(node_id.clone());
        builder.add_view_node(ViewNode {
            id: node_id,
            kind: ViewNodeKind::Component,
            name: "App".to_string(),
            children: Vec::new(),
            props: Vec::new(),
            slots: Vec::new(),
            conditions: Vec::new(),
            provenance: test_provenance(),
        });
        builder.add_state_variable(StateVariable {
            id: IrNodeId("ir-state-count".to_string()),
            name: "count".to_string(),
            scope: StateScope::Local,
            type_annotation: Some("number".to_string()),
            initial_value: Some("0".to_string()),
            readers: BTreeSet::new(),
            writers: BTreeSet::new(),
            provenance: test_provenance(),
        });
        builder.add_event(EventDecl {
            id: IrNodeId("ir-evt-click".to_string()),
            name: "onClick".to_string(),
            kind: EventKind::UserInput,
            source_node: None,
            payload_type: None,
            provenance: test_provenance(),
        });
        builder.build()
    }

    fn ir_with_unsupported() -> MigrationIr {
        let mut builder = IrBuilder::new(
            "test-planner-unsupported".to_string(),
            "planner-tests".to_string(),
        );
        let node_id = IrNodeId("ir-node-app2".to_string());
        builder.add_root(node_id.clone());
        builder.add_view_node(ViewNode {
            id: node_id,
            kind: ViewNodeKind::Component,
            name: "App".to_string(),
            children: Vec::new(),
            props: Vec::new(),
            slots: Vec::new(),
            conditions: Vec::new(),
            provenance: test_provenance(),
        });
        builder.add_effect(EffectDecl {
            id: IrNodeId("ir-eff-dom".to_string()),
            name: "domMutation".to_string(),
            kind: EffectKind::Dom,
            dependencies: BTreeSet::new(),
            has_cleanup: false,
            reads: BTreeSet::new(),
            writes: BTreeSet::new(),
            provenance: test_provenance(),
        });
        builder.build()
    }

    fn ir_with_process_effect(process_spawn_available: bool) -> MigrationIr {
        let mut builder = IrBuilder::new(
            "test-planner-process".to_string(),
            "planner-process-tests".to_string(),
        );
        builder.add_effect(EffectDecl {
            id: IrNodeId("ir-eff-process".to_string()),
            name: "workerLoop".to_string(),
            kind: EffectKind::Process,
            dependencies: BTreeSet::new(),
            has_cleanup: true,
            reads: BTreeSet::new(),
            writes: BTreeSet::new(),
            provenance: test_provenance(),
        });
        if process_spawn_available {
            builder.require_capability(Capability::ProcessSpawn);
        }
        builder.build()
    }

    #[test]
    fn plan_produces_deterministic_output() {
        let ir = minimal_ir();
        let model = test_model();
        let plan1 = plan_translation_simple(&ir, &model);
        let plan2 = plan_translation_simple(&ir, &model);

        assert_eq!(plan1.decisions.len(), plan2.decisions.len());
        for (d1, d2) in plan1.decisions.iter().zip(&plan2.decisions) {
            assert_eq!(d1.segment.id, d2.segment.id);
            assert_eq!(d1.confidence, d2.confidence);
            assert_eq!(d1.chosen.id, d2.chosen.id);
        }
    }

    #[test]
    fn plan_version_is_set() {
        let ir = minimal_ir();
        let model = test_model();
        let plan = plan_translation_simple(&ir, &model);
        assert_eq!(plan.version, PLANNER_VERSION);
    }

    #[test]
    fn plan_run_id_matches_ir() {
        let ir = minimal_ir();
        let model = test_model();
        let plan = plan_translation_simple(&ir, &model);
        assert_eq!(plan.run_id, ir.run_id);
    }

    #[test]
    fn plan_has_decisions_for_all_segments() {
        let ir = minimal_ir();
        let model = test_model();
        let plan = plan_translation_simple(&ir, &model);
        // minimal_ir creates: 1 component + 1 state + 1 event = 3 segments
        assert!(plan.decisions.len() >= 3);
    }

    #[test]
    fn decisions_sorted_by_segment_id() {
        let ir = minimal_ir();
        let model = test_model();
        let plan = plan_translation_simple(&ir, &model);
        for window in plan.decisions.windows(2) {
            assert!(window[0].segment.id <= window[1].segment.id);
        }
    }

    #[test]
    fn exact_mapping_has_high_confidence() {
        let ir = minimal_ir();
        let model = test_model();
        let plan = plan_translation_simple(&ir, &model);
        // Component → Model is Exact, should have high confidence.
        let component_decision = plan
            .decisions
            .iter()
            .find(|d| d.segment.mapping_signature == "ViewNodeKind::Component");
        if let Some(d) = component_decision {
            assert!(
                d.confidence > 0.5,
                "Exact mapping confidence {:.2} should be > 0.5",
                d.confidence
            );
        }
    }

    #[test]
    fn unsupported_mapping_emits_gap_ticket() {
        let ir = ir_with_unsupported();
        let model = test_model();
        let plan = plan_translation_simple(&ir, &model);
        let dom_gaps: Vec<_> = plan
            .gap_tickets
            .iter()
            .filter(|t| t.segment.mapping_signature == "EffectKind::Dom")
            .collect();
        assert!(
            !dom_gaps.is_empty(),
            "Dom effect should emit a capability-gap ticket"
        );
        assert_eq!(dom_gaps[0].gap_kind, GapKind::Unsupported);
    }

    #[test]
    fn stats_sum_correctly() {
        let ir = minimal_ir();
        let model = test_model();
        let plan = plan_translation_simple(&ir, &model);
        assert_eq!(plan.stats.total_segments, plan.decisions.len());
        let cat_sum: usize = plan.stats.by_category.values().sum();
        assert_eq!(cat_sum, plan.stats.total_segments);
    }

    #[test]
    fn stats_handling_class_sum() {
        let ir = minimal_ir();
        let model = test_model();
        let plan = plan_translation_simple(&ir, &model);
        let hc_sum: usize = plan.stats.by_handling_class.values().sum();
        assert_eq!(hc_sum, plan.stats.total_segments);
    }

    #[test]
    fn plan_with_custom_config() {
        let ir = minimal_ir();
        let model = test_model();
        let config = PlannerConfig {
            seed: 42,
            min_confidence_threshold: 0.9,
            use_intent_signals: false,
            use_effect_signals: false,
        };
        let plan = plan_translation(&ir, &model, None, None, &config);
        assert_eq!(plan.seed, 42);
        // With threshold 0.9, more segments should emit low-confidence gaps.
        let low_conf_gaps: Vec<_> = plan
            .gap_tickets
            .iter()
            .filter(|t| t.gap_kind == GapKind::LowConfidence)
            .collect();
        // At least some should be below 0.9.
        assert!(
            !low_conf_gaps.is_empty() || plan.decisions.iter().all(|d| d.confidence >= 0.9),
            "High threshold should flag low-confidence segments"
        );
    }

    #[test]
    fn gap_tickets_sorted_by_segment_id() {
        let ir = ir_with_unsupported();
        let model = test_model();
        let plan = plan_translation_simple(&ir, &model);
        for window in plan.gap_tickets.windows(2) {
            assert!(window[0].segment.id <= window[1].segment.id);
        }
    }

    #[test]
    fn strategy_from_exact_mapping_is_automatable() {
        let atlas = build_atlas();
        let entry =
            lookup(&atlas, "ViewNodeKind::Component").expect("Component mapping should exist");
        let strategy = strategy_from_mapping(entry, "ViewNodeKind::Component");
        assert!(strategy.automatable);
        assert_eq!(strategy.handling_class, TransformationHandlingClass::Exact);
    }

    #[test]
    fn strategy_from_approximate_mapping_has_alternatives() {
        let mut builder = IrBuilder::new("test-approx".to_string(), "planner-tests".to_string());
        let node_id = IrNodeId("ir-node-app3".to_string());
        builder.add_root(node_id.clone());
        builder.add_view_node(ViewNode {
            id: node_id,
            kind: ViewNodeKind::Component,
            name: "App".to_string(),
            children: Vec::new(),
            props: Vec::new(),
            slots: Vec::new(),
            conditions: Vec::new(),
            provenance: test_provenance(),
        });
        builder.add_effect(EffectDecl {
            id: IrNodeId("ir-eff-net".to_string()),
            name: "network_call".to_string(),
            kind: EffectKind::Network,
            dependencies: BTreeSet::new(),
            has_cleanup: false,
            reads: BTreeSet::new(),
            writes: BTreeSet::new(),
            provenance: test_provenance(),
        });
        let ir = builder.build();
        let model = test_model();
        let plan = plan_translation_simple(&ir, &model);
        // Network effects are Approximate — should have alternatives.
        let net_decision = plan
            .decisions
            .iter()
            .find(|d| d.segment.mapping_signature == "EffectKind::Network");
        if let Some(d) = net_decision {
            assert_eq!(
                d.chosen.handling_class,
                TransformationHandlingClass::Approximate
            );
            assert!(
                !d.alternatives.is_empty(),
                "Approximate mapping should offer alternatives"
            );
        }
    }

    #[test]
    fn confidence_bounded_0_1() {
        let ir = minimal_ir();
        let model = test_model();
        let plan = plan_translation_simple(&ir, &model);
        for d in &plan.decisions {
            assert!(
                (0.0..=1.0).contains(&d.confidence),
                "Confidence {:.2} out of bounds",
                d.confidence
            );
        }
    }

    #[test]
    fn rationale_contains_segment_info() {
        let ir = minimal_ir();
        let model = test_model();
        let plan = plan_translation_simple(&ir, &model);
        for d in &plan.decisions {
            assert!(
                d.rationale.contains(&d.segment.name) || d.rationale.contains(&d.segment.id.0),
                "Rationale should reference segment"
            );
        }
    }

    #[test]
    fn empty_ir_produces_empty_plan() {
        let ir = IrBuilder::new("test-empty".to_string(), "planner-tests".to_string()).build();
        let model = test_model();
        let plan = plan_translation_simple(&ir, &model);
        assert_eq!(plan.decisions.len(), 0);
        assert_eq!(plan.gap_tickets.len(), 0);
        assert_eq!(plan.stats.total_segments, 0);
    }

    #[test]
    fn seed_is_preserved_in_plan() {
        let ir = minimal_ir();
        let model = test_model();
        let config = PlannerConfig {
            seed: 0xDEADBEEF,
            ..PlannerConfig::default()
        };
        let plan = plan_translation(&ir, &model, None, None, &config);
        assert_eq!(plan.seed, 0xDEADBEEF);
    }

    #[test]
    fn segment_categories_match_ir_source() {
        let ir = minimal_ir();
        let model = test_model();
        let plan = plan_translation_simple(&ir, &model);
        for d in &plan.decisions {
            match d.segment.category {
                SegmentCategory::View => {
                    assert!(d.segment.mapping_signature.starts_with("ViewNodeKind::"));
                }
                SegmentCategory::State => {
                    assert!(d.segment.mapping_signature.starts_with("StateScope::"));
                }
                SegmentCategory::Event => {
                    assert!(d.segment.mapping_signature.starts_with("EventKind::"));
                }
                SegmentCategory::Effect => {
                    assert!(d.segment.mapping_signature.starts_with("EffectKind::"));
                }
                SegmentCategory::Layout => {
                    assert!(d.segment.mapping_signature.starts_with("LayoutKind::"));
                }
                SegmentCategory::Capability => {
                    assert!(d.segment.mapping_signature.starts_with("Capability::"));
                }
                _ => {}
            }
        }
    }

    #[test]
    fn mapping_evidence_exact_vs_unsupported() {
        let atlas = build_atlas();
        let exact = lookup(&atlas, "ViewNodeKind::Component");
        let unsupported = lookup(&atlas, "EffectKind::Dom");

        let (ex_s, ex_f) = mapping_evidence(exact, false);
        let (un_s, un_f) = mapping_evidence(unsupported, false);

        assert!(
            ex_s > un_s,
            "Exact mapping should have more successes than unsupported"
        );
        assert!(
            ex_f < un_f,
            "Exact mapping should have fewer failures than unsupported"
        );
    }

    #[test]
    fn risk_to_priority_ordering() {
        assert!(risk_to_gap_priority(&TransformationRiskLevel::Critical) < GapPriority::High);
        assert!(risk_to_gap_priority(&TransformationRiskLevel::Low) == GapPriority::Low);
    }

    #[test]
    fn composite_confidence_respects_policy_weight() {
        let posterior = BayesianPosterior {
            alpha: 20.0,
            beta: 2.0,
            mean: 0.9,
            variance: 0.004,
            credible_lower: 0.8,
            credible_upper: 0.95,
        };
        let exact_strategy = TranslationStrategy {
            id: "test-exact".to_string(),
            description: "test".to_string(),
            handling_class: TransformationHandlingClass::Exact,
            risk: TransformationRiskLevel::Low,
            target_construct: "Model".to_string(),
            target_crate: "ftui-runtime".to_string(),
            automatable: true,
            remediation: RemediationStrategy {
                approach: "none".to_string(),
                automatable: true,
                effort: crate::mapping_atlas::EffortLevel::Trivial,
            },
        };
        let unsupported_strategy = TranslationStrategy {
            handling_class: TransformationHandlingClass::Unsupported,
            ..exact_strategy.clone()
        };

        let conf_exact = compute_composite_confidence(&exact_strategy, &posterior, 0.0);
        let conf_unsupported = compute_composite_confidence(&unsupported_strategy, &posterior, 0.0);

        assert!(
            conf_exact > conf_unsupported,
            "Exact {conf_exact:.2} should score higher than Unsupported {conf_unsupported:.2}"
        );
    }

    // ── Style/Theme Integration Tests (v2) ──

    fn ir_with_style_tokens() -> MigrationIr {
        use crate::migration_ir::{StyleToken, ThemeDecl, TokenCategory};
        let mut builder = IrBuilder::new(
            "test-planner-styles".to_string(),
            "planner-style-tests".to_string(),
        );
        let node_id = IrNodeId("ir-node-styled".to_string());
        builder.add_root(node_id.clone());
        builder.add_view_node(ViewNode {
            id: node_id,
            kind: ViewNodeKind::Component,
            name: "StyledApp".to_string(),
            children: Vec::new(),
            props: Vec::new(),
            slots: Vec::new(),
            conditions: Vec::new(),
            provenance: test_provenance(),
        });
        builder.add_style_token(StyleToken {
            name: "primary-color".to_string(),
            category: TokenCategory::Color,
            value: "#ff0000".to_string(),
            provenance: Some(test_provenance()),
        });
        builder.add_style_token(StyleToken {
            name: "body-font".to_string(),
            category: TokenCategory::Typography,
            value: "bold 16px sans-serif".to_string(),
            provenance: Some(test_provenance()),
        });
        builder.add_style_token(StyleToken {
            name: "shadow-main".to_string(),
            category: TokenCategory::Shadow,
            value: "0 2px 4px rgba(0,0,0,0.2)".to_string(),
            provenance: Some(test_provenance()),
        });
        builder.add_theme(ThemeDecl {
            name: "dark-mode".to_string(),
            tokens: {
                let mut t = BTreeMap::new();
                t.insert("bg".to_string(), "#000".to_string());
                t
            },
            is_default: false,
        });
        builder.build()
    }

    #[test]
    fn plan_enumerates_style_tokens() {
        let ir = ir_with_style_tokens();
        let model = test_model();
        let plan = plan_translation_simple(&ir, &model);

        let style_decisions: Vec<_> = plan
            .decisions
            .iter()
            .filter(|d| d.segment.category == SegmentCategory::Style)
            .collect();

        // 3 tokens + 1 theme = 4 style segments
        assert_eq!(
            style_decisions.len(),
            4,
            "Expected 4 style segments, got {}",
            style_decisions.len()
        );
    }

    #[test]
    fn style_token_signatures_match_atlas() {
        let ir = ir_with_style_tokens();
        let model = test_model();
        let plan = plan_translation_simple(&ir, &model);

        let sigs: Vec<_> = plan
            .decisions
            .iter()
            .filter(|d| d.segment.category == SegmentCategory::Style)
            .map(|d| d.segment.mapping_signature.as_str())
            .collect();

        assert!(sigs.contains(&"StyleToken::Color"), "Missing Color token");
        assert!(
            sigs.contains(&"StyleToken::Typography"),
            "Missing Typography token"
        );
        assert!(sigs.contains(&"StyleToken::Shadow"), "Missing Shadow token");
        assert!(sigs.contains(&"ThemeDecl"), "Missing ThemeDecl");
    }

    #[test]
    fn theme_segment_has_style_category() {
        let ir = ir_with_style_tokens();
        let model = test_model();
        let plan = plan_translation_simple(&ir, &model);

        let theme = plan
            .decisions
            .iter()
            .find(|d| d.segment.mapping_signature == "ThemeDecl")
            .expect("Should have a ThemeDecl decision");

        assert_eq!(theme.segment.category, SegmentCategory::Style);
    }

    #[test]
    fn shadow_token_gets_approximate_strategy() {
        let ir = ir_with_style_tokens();
        let model = test_model();
        let plan = plan_translation_simple(&ir, &model);

        let shadow = plan
            .decisions
            .iter()
            .find(|d| d.segment.mapping_signature == "StyleToken::Shadow")
            .expect("Should have Shadow decision");

        assert_eq!(
            shadow.chosen.handling_class,
            TransformationHandlingClass::Approximate
        );
    }

    #[test]
    fn token_category_signature_covers_all_categories() {
        let categories = [
            (TokenCategory::Color, "StyleToken::Color"),
            (TokenCategory::Spacing, "StyleToken::Spacing"),
            (TokenCategory::Typography, "StyleToken::Typography"),
            (TokenCategory::Border, "StyleToken::Border"),
            (TokenCategory::Shadow, "StyleToken::Shadow"),
            (TokenCategory::Animation, "StyleToken::Animation"),
            (TokenCategory::Breakpoint, "StyleToken::Breakpoint"),
            (TokenCategory::ZIndex, "StyleToken::ZIndex"),
        ];
        for (cat, expected) in categories {
            assert_eq!(token_category_signature(&cat), expected);
        }
    }

    #[test]
    fn plan_with_styles_includes_all_segment_types() {
        let ir = ir_with_style_tokens();
        let model = test_model();
        let plan = plan_translation_simple(&ir, &model);

        let cats: BTreeSet<_> = plan
            .decisions
            .iter()
            .map(|d| format!("{:?}", d.segment.category))
            .collect();

        assert!(cats.contains("View"), "Missing View segments");
        assert!(cats.contains("Style"), "Missing Style segments");
    }

    #[test]
    fn optional_capability_segments_are_enumerated() {
        let mut builder =
            IrBuilder::new("test-optional-cap".to_string(), "planner-tests".to_string());
        builder.optional_capability(Capability::ProcessSpawn);
        let ir = builder.build();
        let model = test_model();
        let plan = plan_translation_simple(&ir, &model);

        assert!(
            plan.decisions
                .iter()
                .any(|d| d.segment.mapping_signature == "Capability::ProcessSpawn"),
            "Optional capabilities should be planned as capability segments"
        );
    }

    #[test]
    fn process_effect_uses_subscription_path_when_capability_present() {
        let ir = ir_with_process_effect(true);
        let model = test_model();
        let plan = plan_translation_simple(&ir, &model);

        let decision = plan
            .decisions
            .iter()
            .find(|d| d.segment.mapping_signature == "EffectKind::Process")
            .expect("Process effect decision must exist");

        assert!(
            decision.chosen.id.ends_with("-auto"),
            "Expected direct atlas path when process capability is present"
        );
        assert!(
            !decision.rationale.contains("Process precondition not met"),
            "Precondition note should not appear when capability is present"
        );
    }

    #[test]
    fn process_effect_falls_back_when_process_capability_missing() {
        let ir = ir_with_process_effect(false);
        let model = test_model();
        let plan = plan_translation_simple(&ir, &model);

        let decision = plan
            .decisions
            .iter()
            .find(|d| d.segment.mapping_signature == "EffectKind::Process")
            .expect("Process effect decision must exist");

        assert!(
            decision.chosen.id.ends_with("-in-process-fallback"),
            "Expected in-process fallback strategy without ProcessSpawn capability"
        );
        assert_eq!(
            decision.chosen.handling_class,
            TransformationHandlingClass::Approximate
        );
        assert!(
            decision.rationale.contains("Process precondition not met"),
            "Fallback rationale should document precondition mismatch"
        );
    }
}
