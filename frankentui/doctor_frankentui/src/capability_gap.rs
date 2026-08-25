//! Automated capability gap detector.
//!
//! Consumes [`TranslationPlan`] decisions and [`MappingAtlas`] entries to detect
//! when required source semantics lack sufficient FrankenTUI mapping. Produces
//! normalized [`GapReport`] with severity, provenance, and machine-actionable
//! backlog items.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::mapping_atlas::{MappingAtlas, MappingEntry, build_atlas, lookup};
use crate::migration_ir::{Capability, CapabilityProfile, MigrationIr, Provenance};
use crate::semantic_contract::{
    MigrationDecision, TransformationHandlingClass, TransformationRiskLevel,
};
use crate::translation_planner::{
    CapabilityGapTicket, GapKind, GapPriority, IrSegment, StrategyDecision, TranslationPlan,
};

/// Current version of the gap report schema.
pub const GAP_REPORT_VERSION: &str = "gap-report-v1";

// ── Primary output ──────────────────────────────────────────────────────────

/// Full gap analysis report for a migration run.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GapReport {
    pub version: String,
    pub run_id: String,
    pub records: Vec<GapRecord>,
    pub capability_gaps: Vec<CapabilityGapRecord>,
    pub summary: GapSummary,
}

/// A single detected gap record tied to an IR segment.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GapRecord {
    pub id: String,
    pub segment: IrSegment,
    pub severity: GapSeverity,
    pub category: GapCategory,
    pub kind: GapKind,
    pub description: String,
    pub user_impact: UserImpact,
    pub provenance: Option<Provenance>,
    pub remediation: GapRemediation,
    pub decision_context: DecisionContext,
}

/// A capability-level gap record (required capability without FrankenTUI support).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CapabilityGapRecord {
    pub capability: String,
    pub required: bool,
    pub severity: GapSeverity,
    pub description: String,
    pub workaround: Option<String>,
}

// ── Severity and classification ─────────────────────────────────────────────

/// Gap severity classification aligned with risk levels.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum GapSeverity {
    /// Blocks migration entirely.
    Blocker,
    /// Requires significant manual work or extension.
    Critical,
    /// Needs attention but has workarounds.
    Major,
    /// Minor degradation, can proceed.
    Minor,
    /// Informational only.
    Info,
}

/// Semantic category of the gap.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum GapCategory {
    /// No FrankenTUI mapping exists.
    Unsupported,
    /// Mapping exists but requires FrankenTUI extension.
    RequiresExtension,
    /// Mapping exists but confidence is too low.
    LowConfidence,
    /// Decision gated by human review.
    HumanReviewRequired,
    /// Capability not available in FrankenTUI.
    MissingCapability,
    /// Platform assumption may not hold.
    PlatformRisk,
}

/// User-facing impact assessment.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum UserImpact {
    /// Feature completely unavailable.
    FeatureLoss,
    /// Feature degraded but functional.
    Degraded,
    /// Visual/cosmetic difference only.
    Cosmetic,
    /// No visible impact to end user.
    None,
}

// ── Remediation ─────────────────────────────────────────────────────────────

/// How the gap can be remediated.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GapRemediation {
    pub approach: String,
    pub automatable: bool,
    pub effort: String,
    pub backlog_action: BacklogAction,
}

/// Machine-actionable backlog item derived from the gap.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum BacklogAction {
    /// Create a feature request for FrankenTUI.
    CreateFeatureRequest,
    /// Create a manual migration task.
    CreateMigrationTask,
    /// Flag for human review before proceeding.
    FlagForReview,
    /// No action needed, informational.
    NoAction,
}

/// Contextual info about the decision that produced this gap.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DecisionContext {
    pub migration_decision: String,
    pub handling_class: String,
    pub risk_level: String,
    pub confidence: f64,
    pub rationale: String,
}

// ── Summary statistics ──────────────────────────────────────────────────────

/// Aggregate gap statistics for triage.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GapSummary {
    pub total_gaps: usize,
    pub blockers: usize,
    pub critical: usize,
    pub major: usize,
    pub minor: usize,
    pub info: usize,
    pub capability_gaps: usize,
    pub by_category: BTreeMap<String, usize>,
    pub by_segment_category: BTreeMap<String, usize>,
    pub migration_feasibility: MigrationFeasibility,
}

/// Overall migration feasibility assessment.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MigrationFeasibility {
    /// All segments have viable mappings.
    Clear,
    /// Some gaps exist but have workarounds.
    Feasible,
    /// Significant gaps require extension work.
    Challenging,
    /// Blockers prevent migration without major effort.
    Blocked,
}

// ── Core API ────────────────────────────────────────────────────────────────

/// Detect capability gaps from a translation plan and the source IR.
///
/// This is the primary entry point. It:
/// 1. Scans plan decisions for unsupported/low-confidence segments.
/// 2. Promotes existing gap tickets from the planner.
/// 3. Cross-checks IR capabilities against the mapping atlas.
/// 4. Produces a deterministic, machine-actionable report.
pub fn detect_gaps(plan: &TranslationPlan, ir: &MigrationIr) -> GapReport {
    detect_gaps_with_atlas(plan, ir, &build_atlas())
}

/// Detect gaps with an explicit atlas (for testing).
pub fn detect_gaps_with_atlas(
    plan: &TranslationPlan,
    ir: &MigrationIr,
    atlas: &MappingAtlas,
) -> GapReport {
    let mut records = Vec::new();
    let mut capability_gaps = Vec::new();
    let mut gap_counter = 0;

    // Phase 1: Scan strategy decisions for gaps.
    for decision in &plan.decisions {
        if let Some(record) = decision_to_gap(decision, atlas, &mut gap_counter) {
            records.push(record);
        }
    }

    // Phase 2: Promote planner gap tickets that weren't already captured.
    for ticket in &plan.gap_tickets {
        if !records.iter().any(|r| r.segment.id == ticket.segment.id) {
            records.push(ticket_to_gap(ticket, &mut gap_counter));
        }
    }

    // Phase 3: Check required capabilities against atlas.
    detect_capability_gaps(&ir.capabilities, atlas, &mut capability_gaps);

    // Phase 4: Check platform assumptions.
    for assumption in &ir.capabilities.platform_assumptions {
        capability_gaps.push(CapabilityGapRecord {
            capability: format!("platform:{}", assumption.assumption),
            required: true,
            severity: GapSeverity::Major,
            description: format!(
                "Platform assumption may not hold: {}. Impact: {}",
                assumption.assumption, assumption.impact_if_wrong
            ),
            workaround: Some(format!("Evidence: {}", assumption.evidence)),
        });
    }

    // Sort records deterministically by severity (desc) then segment id.
    records.sort_by(|a, b| {
        a.severity
            .cmp(&b.severity)
            .then_with(|| a.segment.id.cmp(&b.segment.id))
    });
    capability_gaps.sort_by(|a, b| {
        a.severity
            .cmp(&b.severity)
            .then_with(|| a.capability.cmp(&b.capability))
    });

    let summary = compute_summary(&records, &capability_gaps);

    GapReport {
        version: GAP_REPORT_VERSION.to_string(),
        run_id: plan.run_id.clone(),
        records,
        capability_gaps,
        summary,
    }
}

// ── Decision analysis ───────────────────────────────────────────────────────

/// Convert a strategy decision into a gap record if it represents a gap.
fn decision_to_gap(
    decision: &StrategyDecision,
    atlas: &MappingAtlas,
    counter: &mut usize,
) -> Option<GapRecord> {
    let handling = &decision.chosen.handling_class;
    let gate = &decision.gate;

    // Determine if this decision represents a gap.
    let (category, severity, impact) = match (handling, gate) {
        // Unsupported → always a gap
        (TransformationHandlingClass::Unsupported, _) => (
            GapCategory::Unsupported,
            risk_to_severity(decision.chosen.risk),
            UserImpact::FeatureLoss,
        ),
        // Requires FrankenTUI extension → gap
        (TransformationHandlingClass::ExtendFtui, _) => (
            GapCategory::RequiresExtension,
            risk_to_severity_lowered(decision.chosen.risk),
            UserImpact::Degraded,
        ),
        // Rejected by decision gate → gap
        (_, MigrationDecision::Reject | MigrationDecision::HardReject) => (
            GapCategory::LowConfidence,
            GapSeverity::Critical,
            UserImpact::FeatureLoss,
        ),
        // Human review required → potential gap
        (_, MigrationDecision::HumanReview) if decision.confidence < 0.5 => (
            GapCategory::HumanReviewRequired,
            GapSeverity::Major,
            UserImpact::Degraded,
        ),
        // Low confidence even if auto-approved → informational
        (_, _) if decision.confidence < 0.3 => (
            GapCategory::LowConfidence,
            GapSeverity::Minor,
            UserImpact::Cosmetic,
        ),
        // No gap detected
        _ => return None,
    };

    *counter += 1;
    let id = format!("gap-{:04}", counter);

    // Look up provenance from atlas for context.
    let mapping = lookup(atlas, &decision.segment.mapping_signature);

    let remediation = if let Some(entry) = mapping {
        remediation_from_mapping(entry, &category)
    } else {
        GapRemediation {
            approach: "No mapping exists; manual implementation required".to_string(),
            automatable: false,
            effort: "High".to_string(),
            backlog_action: BacklogAction::CreateFeatureRequest,
        }
    };

    Some(GapRecord {
        id,
        segment: decision.segment.clone(),
        severity,
        category,
        kind: category_to_gap_kind(&category),
        description: format!(
            "{}: {} (handling: {:?}, risk: {:?})",
            decision.segment.name, decision.rationale, handling, decision.chosen.risk,
        ),
        user_impact: impact,
        provenance: None, // IR provenance not carried in StrategyDecision
        remediation,
        decision_context: DecisionContext {
            migration_decision: format!("{:?}", decision.gate),
            handling_class: format!("{:?}", handling),
            risk_level: format!("{:?}", decision.chosen.risk),
            confidence: decision.confidence,
            rationale: decision.rationale.clone(),
        },
    })
}

/// Promote a planner gap ticket to a full gap record.
fn ticket_to_gap(ticket: &CapabilityGapTicket, counter: &mut usize) -> GapRecord {
    *counter += 1;
    let id = format!("gap-{:04}", counter);

    GapRecord {
        id,
        segment: ticket.segment.clone(),
        severity: priority_to_severity(ticket.priority),
        category: gap_kind_to_category(ticket.gap_kind),
        kind: ticket.gap_kind,
        description: ticket.description.clone(),
        user_impact: gap_kind_to_impact(ticket.gap_kind),
        provenance: None,
        remediation: GapRemediation {
            approach: ticket.suggested_remediation.clone(),
            automatable: false,
            effort: priority_to_effort(ticket.priority).to_string(),
            backlog_action: gap_kind_to_action(ticket.gap_kind),
        },
        decision_context: DecisionContext {
            migration_decision: "N/A".to_string(),
            handling_class: format!("{:?}", ticket.gap_kind),
            risk_level: format!("{:?}", ticket.priority),
            confidence: 0.0,
            rationale: ticket.description.clone(),
        },
    }
}

// ── Capability analysis ─────────────────────────────────────────────────────

/// Check required/optional capabilities against what the atlas can map.
fn detect_capability_gaps(
    profile: &CapabilityProfile,
    atlas: &MappingAtlas,
    out: &mut Vec<CapabilityGapRecord>,
) {
    for cap in &profile.required {
        let sig = capability_signature(cap);
        let mapping = lookup(atlas, &sig);
        match mapping {
            None => {
                out.push(CapabilityGapRecord {
                    capability: sig,
                    required: true,
                    severity: GapSeverity::Critical,
                    description: format!("Required capability {:?} has no FrankenTUI mapping", cap),
                    workaround: None,
                });
            }
            Some(entry) if entry.policy == TransformationHandlingClass::Unsupported => {
                out.push(CapabilityGapRecord {
                    capability: sig,
                    required: true,
                    severity: GapSeverity::Blocker,
                    description: format!("Required capability {:?} is explicitly unsupported", cap),
                    workaround: Some(entry.remediation.approach.clone()),
                });
            }
            Some(entry) if entry.policy == TransformationHandlingClass::ExtendFtui => {
                out.push(CapabilityGapRecord {
                    capability: sig,
                    required: true,
                    severity: GapSeverity::Major,
                    description: format!(
                        "Required capability {:?} needs FrankenTUI extension",
                        cap
                    ),
                    workaround: Some(entry.remediation.approach.clone()),
                });
            }
            _ => {} // Exact or Approximate — no gap
        }
    }

    for cap in &profile.optional {
        let sig = capability_signature(cap);
        let mapping = lookup(atlas, &sig);
        if mapping.is_none() {
            out.push(CapabilityGapRecord {
                capability: sig,
                required: false,
                severity: GapSeverity::Info,
                description: format!(
                    "Optional capability {:?} has no FrankenTUI mapping (graceful degradation expected)",
                    cap
                ),
                workaround: Some("Feature will be disabled in migrated application".to_string()),
            });
        }
    }
}

/// Map a Capability enum variant to its atlas signature.
fn capability_signature(cap: &Capability) -> String {
    match cap {
        Capability::MouseInput => "Capability::MouseInput".to_string(),
        Capability::KeyboardInput => "Capability::KeyboardInput".to_string(),
        Capability::TouchInput => "Capability::TouchInput".to_string(),
        Capability::NetworkAccess => "Capability::NetworkAccess".to_string(),
        Capability::FileSystem => "Capability::FileSystem".to_string(),
        Capability::Clipboard => "Capability::Clipboard".to_string(),
        Capability::Timers => "Capability::Timers".to_string(),
        Capability::AlternateScreen => "Capability::AlternateScreen".to_string(),
        Capability::TrueColor => "Capability::TrueColor".to_string(),
        Capability::Unicode => "Capability::Unicode".to_string(),
        Capability::InlineMode => "Capability::InlineMode".to_string(),
        Capability::ProcessSpawn => "Capability::ProcessSpawn".to_string(),
        Capability::Custom(name) => format!("Capability::Custom({})", name),
    }
}

// ── Summary computation ─────────────────────────────────────────────────────

fn compute_summary(records: &[GapRecord], cap_gaps: &[CapabilityGapRecord]) -> GapSummary {
    let mut blockers = 0usize;
    let mut critical = 0usize;
    let mut major = 0usize;
    let mut minor = 0usize;
    let mut info = 0usize;
    let mut by_category: BTreeMap<String, usize> = BTreeMap::new();
    let mut by_segment_category: BTreeMap<String, usize> = BTreeMap::new();

    for r in records {
        match r.severity {
            GapSeverity::Blocker => blockers += 1,
            GapSeverity::Critical => critical += 1,
            GapSeverity::Major => major += 1,
            GapSeverity::Minor => minor += 1,
            GapSeverity::Info => info += 1,
        }
        *by_category.entry(format!("{:?}", r.category)).or_default() += 1;
        *by_segment_category
            .entry(format!("{:?}", r.segment.category))
            .or_default() += 1;
    }

    // Count capability gap severities too.
    for cg in cap_gaps {
        match cg.severity {
            GapSeverity::Blocker => blockers += 1,
            GapSeverity::Critical => critical += 1,
            GapSeverity::Major => major += 1,
            GapSeverity::Minor => minor += 1,
            GapSeverity::Info => info += 1,
        }
    }

    let total = records.len() + cap_gaps.len();

    let feasibility = if blockers > 0 {
        MigrationFeasibility::Blocked
    } else if critical > 0 {
        MigrationFeasibility::Challenging
    } else if major > 0 {
        MigrationFeasibility::Feasible
    } else {
        MigrationFeasibility::Clear
    };

    GapSummary {
        total_gaps: total,
        blockers,
        critical,
        major,
        minor,
        info,
        capability_gaps: cap_gaps.len(),
        by_category,
        by_segment_category,
        migration_feasibility: feasibility,
    }
}

// ── Conversion helpers ──────────────────────────────────────────────────────

fn risk_to_severity(risk: TransformationRiskLevel) -> GapSeverity {
    match risk {
        TransformationRiskLevel::Critical => GapSeverity::Blocker,
        TransformationRiskLevel::High => GapSeverity::Critical,
        TransformationRiskLevel::Medium => GapSeverity::Major,
        TransformationRiskLevel::Low => GapSeverity::Minor,
    }
}

/// One step lower than risk_to_severity (for ExtendFtui which is fixable).
fn risk_to_severity_lowered(risk: TransformationRiskLevel) -> GapSeverity {
    match risk {
        TransformationRiskLevel::Critical => GapSeverity::Critical,
        TransformationRiskLevel::High => GapSeverity::Major,
        TransformationRiskLevel::Medium => GapSeverity::Minor,
        TransformationRiskLevel::Low => GapSeverity::Info,
    }
}

fn priority_to_severity(priority: GapPriority) -> GapSeverity {
    match priority {
        GapPriority::Critical => GapSeverity::Blocker,
        GapPriority::High => GapSeverity::Critical,
        GapPriority::Medium => GapSeverity::Major,
        GapPriority::Low => GapSeverity::Minor,
    }
}

fn priority_to_effort(priority: GapPriority) -> &'static str {
    match priority {
        GapPriority::Critical => "High",
        GapPriority::High => "High",
        GapPriority::Medium => "Medium",
        GapPriority::Low => "Low",
    }
}

fn gap_kind_to_category(kind: GapKind) -> GapCategory {
    match kind {
        GapKind::Unsupported => GapCategory::Unsupported,
        GapKind::RequiresExtension => GapCategory::RequiresExtension,
        GapKind::LowConfidence => GapCategory::LowConfidence,
    }
}

fn gap_kind_to_impact(kind: GapKind) -> UserImpact {
    match kind {
        GapKind::Unsupported => UserImpact::FeatureLoss,
        GapKind::RequiresExtension => UserImpact::Degraded,
        GapKind::LowConfidence => UserImpact::Cosmetic,
    }
}

fn gap_kind_to_action(kind: GapKind) -> BacklogAction {
    match kind {
        GapKind::Unsupported => BacklogAction::CreateFeatureRequest,
        GapKind::RequiresExtension => BacklogAction::CreateMigrationTask,
        GapKind::LowConfidence => BacklogAction::FlagForReview,
    }
}

fn category_to_gap_kind(category: &GapCategory) -> GapKind {
    match category {
        GapCategory::Unsupported => GapKind::Unsupported,
        GapCategory::RequiresExtension => GapKind::RequiresExtension,
        GapCategory::LowConfidence => GapKind::LowConfidence,
        GapCategory::HumanReviewRequired => GapKind::LowConfidence,
        GapCategory::MissingCapability => GapKind::Unsupported,
        GapCategory::PlatformRisk => GapKind::RequiresExtension,
    }
}

fn remediation_from_mapping(entry: &MappingEntry, gap_category: &GapCategory) -> GapRemediation {
    let action = match gap_category {
        GapCategory::Unsupported => BacklogAction::CreateFeatureRequest,
        GapCategory::RequiresExtension => BacklogAction::CreateMigrationTask,
        _ => BacklogAction::FlagForReview,
    };

    GapRemediation {
        approach: entry.remediation.approach.clone(),
        automatable: entry.remediation.automatable,
        effort: format!("{:?}", entry.remediation.effort),
        backlog_action: action,
    }
}

// ── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mapping_atlas::RemediationStrategy;
    use crate::migration_ir::{IrBuilder, IrNodeId, PlatformAssumption};
    use crate::semantic_contract::BayesianPosterior;
    use crate::translation_planner::{PlanStats, SegmentCategory};

    fn minimal_plan() -> TranslationPlan {
        TranslationPlan {
            version: "test".to_string(),
            run_id: "test-run".to_string(),
            seed: 42,
            decisions: Vec::new(),
            gap_tickets: Vec::new(),
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
        }
    }

    fn minimal_ir() -> MigrationIr {
        IrBuilder::new("test-run".to_string(), "test-project".to_string()).build()
    }

    fn make_posterior() -> BayesianPosterior {
        BayesianPosterior {
            alpha: 2.0,
            beta: 1.0,
            mean: 0.667,
            variance: 0.056,
            credible_lower: 0.3,
            credible_upper: 0.95,
        }
    }

    fn make_loss(decision: MigrationDecision) -> crate::semantic_contract::ExpectedLossResult {
        crate::semantic_contract::ExpectedLossResult {
            decision,
            posterior: make_posterior(),
            expected_loss_accept: 0.1,
            expected_loss_reject: 0.2,
            expected_loss_hold: 0.15,
            rationale: "test".to_string(),
            claim_id: None,
            policy_id: None,
        }
    }

    fn make_segment(name: &str, cat: SegmentCategory) -> IrSegment {
        IrSegment {
            id: IrNodeId(format!("seg-{}", name)),
            name: name.to_string(),
            category: cat,
            mapping_signature: format!("test::{}", name),
        }
    }

    fn make_decision(
        name: &str,
        cat: SegmentCategory,
        handling: TransformationHandlingClass,
        risk: TransformationRiskLevel,
        gate: MigrationDecision,
        confidence: f64,
    ) -> StrategyDecision {
        StrategyDecision {
            segment: make_segment(name, cat),
            chosen: crate::translation_planner::TranslationStrategy {
                id: format!("strat-{}", name),
                description: "test strategy".to_string(),
                handling_class: handling,
                risk,
                target_construct: "Widget".to_string(),
                target_crate: "ftui-widgets".to_string(),
                automatable: true,
                remediation: RemediationStrategy {
                    approach: "test approach".to_string(),
                    automatable: true,
                    effort: crate::mapping_atlas::EffortLevel::Low,
                },
            },
            alternatives: Vec::new(),
            posterior: make_posterior(),
            expected_loss: make_loss(gate),
            gate,
            confidence,
            rationale: "test rationale".to_string(),
        }
    }

    #[test]
    fn empty_plan_produces_empty_report() {
        let plan = minimal_plan();
        let ir = minimal_ir();
        let report = detect_gaps(&plan, &ir);

        assert_eq!(report.version, GAP_REPORT_VERSION);
        assert_eq!(report.run_id, "test-run");
        assert!(report.records.is_empty());
        assert_eq!(report.summary.total_gaps, 0);
        assert_eq!(
            report.summary.migration_feasibility,
            MigrationFeasibility::Clear
        );
    }

    #[test]
    fn unsupported_decision_creates_gap() {
        let mut plan = minimal_plan();
        plan.decisions.push(make_decision(
            "broken",
            SegmentCategory::View,
            TransformationHandlingClass::Unsupported,
            TransformationRiskLevel::High,
            MigrationDecision::Reject,
            0.1,
        ));

        let ir = minimal_ir();
        let report = detect_gaps(&plan, &ir);

        assert_eq!(report.records.len(), 1);
        assert_eq!(report.records[0].category, GapCategory::Unsupported);
        assert_eq!(report.records[0].severity, GapSeverity::Critical);
        assert_eq!(report.records[0].user_impact, UserImpact::FeatureLoss);
    }

    #[test]
    fn extend_ftui_creates_gap() {
        let mut plan = minimal_plan();
        plan.decisions.push(make_decision(
            "needs-ext",
            SegmentCategory::Effect,
            TransformationHandlingClass::ExtendFtui,
            TransformationRiskLevel::Medium,
            MigrationDecision::HumanReview,
            0.5,
        ));

        let ir = minimal_ir();
        let report = detect_gaps(&plan, &ir);

        assert_eq!(report.records.len(), 1);
        assert_eq!(report.records[0].category, GapCategory::RequiresExtension);
        assert_eq!(report.records[0].severity, GapSeverity::Minor);
        assert_eq!(report.records[0].user_impact, UserImpact::Degraded);
    }

    #[test]
    fn rejected_decision_creates_gap() {
        let mut plan = minimal_plan();
        plan.decisions.push(make_decision(
            "rejected",
            SegmentCategory::State,
            TransformationHandlingClass::Approximate,
            TransformationRiskLevel::Low,
            MigrationDecision::HardReject,
            0.8,
        ));

        let ir = minimal_ir();
        let report = detect_gaps(&plan, &ir);

        assert_eq!(report.records.len(), 1);
        assert_eq!(report.records[0].category, GapCategory::LowConfidence);
        assert_eq!(report.records[0].severity, GapSeverity::Critical);
    }

    #[test]
    fn human_review_low_confidence_creates_gap() {
        let mut plan = minimal_plan();
        plan.decisions.push(make_decision(
            "uncertain",
            SegmentCategory::Event,
            TransformationHandlingClass::Exact,
            TransformationRiskLevel::Low,
            MigrationDecision::HumanReview,
            0.3,
        ));

        let ir = minimal_ir();
        let report = detect_gaps(&plan, &ir);

        assert_eq!(report.records.len(), 1);
        assert_eq!(report.records[0].category, GapCategory::HumanReviewRequired);
        assert_eq!(report.records[0].severity, GapSeverity::Major);
    }

    #[test]
    fn auto_approve_high_confidence_no_gap() {
        let mut plan = minimal_plan();
        plan.decisions.push(make_decision(
            "fine",
            SegmentCategory::View,
            TransformationHandlingClass::Exact,
            TransformationRiskLevel::Low,
            MigrationDecision::AutoApprove,
            0.95,
        ));

        let ir = minimal_ir();
        let report = detect_gaps(&plan, &ir);

        assert!(report.records.is_empty());
        assert_eq!(
            report.summary.migration_feasibility,
            MigrationFeasibility::Clear
        );
    }

    #[test]
    fn very_low_confidence_even_if_approved() {
        let mut plan = minimal_plan();
        plan.decisions.push(make_decision(
            "shaky",
            SegmentCategory::Layout,
            TransformationHandlingClass::Approximate,
            TransformationRiskLevel::Low,
            MigrationDecision::AutoApprove,
            0.2,
        ));

        let ir = minimal_ir();
        let report = detect_gaps(&plan, &ir);

        assert_eq!(report.records.len(), 1);
        assert_eq!(report.records[0].category, GapCategory::LowConfidence);
        assert_eq!(report.records[0].severity, GapSeverity::Minor);
    }

    #[test]
    fn gap_tickets_promoted_to_records() {
        let mut plan = minimal_plan();
        plan.gap_tickets.push(CapabilityGapTicket {
            segment: make_segment("ticket-seg", SegmentCategory::Capability),
            gap_kind: GapKind::Unsupported,
            description: "No mapping for custom capability".to_string(),
            suggested_remediation: "Implement custom adapter".to_string(),
            priority: GapPriority::High,
        });

        let ir = minimal_ir();
        let report = detect_gaps(&plan, &ir);

        assert_eq!(report.records.len(), 1);
        assert_eq!(report.records[0].kind, GapKind::Unsupported);
        assert!(report.records[0].description.contains("custom capability"));
    }

    #[test]
    fn duplicate_ticket_not_promoted() {
        let seg = make_segment("dup", SegmentCategory::View);
        let mut plan = minimal_plan();
        // Decision already covers this segment.
        plan.decisions.push(make_decision(
            "dup",
            SegmentCategory::View,
            TransformationHandlingClass::Unsupported,
            TransformationRiskLevel::Low,
            MigrationDecision::Reject,
            0.1,
        ));
        // Ticket for same segment.
        plan.gap_tickets.push(CapabilityGapTicket {
            segment: seg,
            gap_kind: GapKind::Unsupported,
            description: "duplicate".to_string(),
            suggested_remediation: "n/a".to_string(),
            priority: GapPriority::Low,
        });

        let ir = minimal_ir();
        let report = detect_gaps(&plan, &ir);

        // Should only have 1 record, not 2.
        assert_eq!(report.records.len(), 1);
    }

    #[test]
    fn platform_assumptions_create_capability_gaps() {
        let ir = {
            let mut ir = minimal_ir();
            ir.capabilities
                .platform_assumptions
                .push(PlatformAssumption {
                    assumption: "Assumes 256-color terminal".to_string(),
                    evidence: "Uses ANSI 256 color codes".to_string(),
                    impact_if_wrong: "Colors fallback to 16-color palette".to_string(),
                });
            ir
        };
        let plan = minimal_plan();
        let report = detect_gaps(&plan, &ir);

        assert_eq!(report.capability_gaps.len(), 1);
        assert!(
            report.capability_gaps[0]
                .capability
                .starts_with("platform:")
        );
        assert_eq!(report.capability_gaps[0].severity, GapSeverity::Major);
    }

    #[test]
    fn summary_counts_are_consistent() {
        let mut plan = minimal_plan();
        plan.decisions.push(make_decision(
            "blocker",
            SegmentCategory::View,
            TransformationHandlingClass::Unsupported,
            TransformationRiskLevel::Critical,
            MigrationDecision::HardReject,
            0.0,
        ));
        plan.decisions.push(make_decision(
            "major",
            SegmentCategory::State,
            TransformationHandlingClass::ExtendFtui,
            TransformationRiskLevel::High,
            MigrationDecision::HumanReview,
            0.4,
        ));
        plan.decisions.push(make_decision(
            "minor",
            SegmentCategory::Layout,
            TransformationHandlingClass::Approximate,
            TransformationRiskLevel::Low,
            MigrationDecision::AutoApprove,
            0.2,
        ));

        let ir = minimal_ir();
        let report = detect_gaps(&plan, &ir);

        let s = &report.summary;
        assert_eq!(
            s.total_gaps,
            report.records.len() + report.capability_gaps.len()
        );
        assert_eq!(
            s.blockers + s.critical + s.major + s.minor + s.info,
            s.total_gaps
        );
    }

    #[test]
    fn feasibility_blocked_when_blockers() {
        let mut plan = minimal_plan();
        plan.decisions.push(make_decision(
            "critical-unsupported",
            SegmentCategory::View,
            TransformationHandlingClass::Unsupported,
            TransformationRiskLevel::Critical,
            MigrationDecision::HardReject,
            0.0,
        ));

        let ir = minimal_ir();
        let report = detect_gaps(&plan, &ir);

        assert_eq!(
            report.summary.migration_feasibility,
            MigrationFeasibility::Blocked
        );
    }

    #[test]
    fn feasibility_challenging_when_critical_no_blockers() {
        let mut plan = minimal_plan();
        plan.decisions.push(make_decision(
            "high-risk",
            SegmentCategory::Effect,
            TransformationHandlingClass::Unsupported,
            TransformationRiskLevel::High,
            MigrationDecision::Reject,
            0.1,
        ));

        let ir = minimal_ir();
        let report = detect_gaps(&plan, &ir);

        assert_eq!(
            report.summary.migration_feasibility,
            MigrationFeasibility::Challenging
        );
    }

    #[test]
    fn feasibility_feasible_when_only_major() {
        let mut plan = minimal_plan();
        plan.decisions.push(make_decision(
            "medium-ext",
            SegmentCategory::Layout,
            TransformationHandlingClass::ExtendFtui,
            TransformationRiskLevel::High,
            MigrationDecision::HumanReview,
            0.6,
        ));

        let ir = minimal_ir();
        let report = detect_gaps(&plan, &ir);

        assert_eq!(
            report.summary.migration_feasibility,
            MigrationFeasibility::Feasible
        );
    }

    #[test]
    fn records_sorted_by_severity_then_id() {
        let mut plan = minimal_plan();
        plan.decisions.push(make_decision(
            "z-minor",
            SegmentCategory::View,
            TransformationHandlingClass::Approximate,
            TransformationRiskLevel::Low,
            MigrationDecision::AutoApprove,
            0.2,
        ));
        plan.decisions.push(make_decision(
            "a-blocker",
            SegmentCategory::State,
            TransformationHandlingClass::Unsupported,
            TransformationRiskLevel::Critical,
            MigrationDecision::HardReject,
            0.0,
        ));

        let ir = minimal_ir();
        let report = detect_gaps(&plan, &ir);

        assert_eq!(report.records.len(), 2);
        assert!(report.records[0].severity <= report.records[1].severity);
    }

    #[test]
    fn deterministic_output() {
        let mut plan = minimal_plan();
        plan.decisions.push(make_decision(
            "seg-a",
            SegmentCategory::View,
            TransformationHandlingClass::Unsupported,
            TransformationRiskLevel::Medium,
            MigrationDecision::Reject,
            0.3,
        ));
        plan.decisions.push(make_decision(
            "seg-b",
            SegmentCategory::Event,
            TransformationHandlingClass::ExtendFtui,
            TransformationRiskLevel::Low,
            MigrationDecision::HumanReview,
            0.5,
        ));

        let ir = minimal_ir();
        let r1 = detect_gaps(&plan, &ir);
        let r2 = detect_gaps(&plan, &ir);

        let j1 = serde_json::to_string(&r1).unwrap();
        let j2 = serde_json::to_string(&r2).unwrap();
        assert_eq!(j1, j2);
    }

    #[test]
    fn gap_record_has_backlog_action() {
        let mut plan = minimal_plan();
        plan.decisions.push(make_decision(
            "feat-req",
            SegmentCategory::View,
            TransformationHandlingClass::Unsupported,
            TransformationRiskLevel::High,
            MigrationDecision::Reject,
            0.1,
        ));

        let ir = minimal_ir();
        let report = detect_gaps(&plan, &ir);

        // All unsupported gaps should suggest creating a feature request.
        for r in &report.records {
            if r.category == GapCategory::Unsupported {
                assert_eq!(
                    r.remediation.backlog_action,
                    BacklogAction::CreateFeatureRequest
                );
            }
        }
    }

    #[test]
    fn by_category_counts_match() {
        let mut plan = minimal_plan();
        plan.decisions.push(make_decision(
            "view-gap",
            SegmentCategory::View,
            TransformationHandlingClass::Unsupported,
            TransformationRiskLevel::Low,
            MigrationDecision::Reject,
            0.1,
        ));
        plan.decisions.push(make_decision(
            "state-gap",
            SegmentCategory::State,
            TransformationHandlingClass::Unsupported,
            TransformationRiskLevel::Low,
            MigrationDecision::Reject,
            0.1,
        ));

        let ir = minimal_ir();
        let report = detect_gaps(&plan, &ir);

        let total_by_seg: usize = report.summary.by_segment_category.values().sum();
        assert_eq!(total_by_seg, report.records.len());
    }

    #[test]
    fn risk_severity_mapping() {
        assert_eq!(
            risk_to_severity(TransformationRiskLevel::Critical),
            GapSeverity::Blocker
        );
        assert_eq!(
            risk_to_severity(TransformationRiskLevel::High),
            GapSeverity::Critical
        );
        assert_eq!(
            risk_to_severity(TransformationRiskLevel::Medium),
            GapSeverity::Major
        );
        assert_eq!(
            risk_to_severity(TransformationRiskLevel::Low),
            GapSeverity::Minor
        );
    }

    #[test]
    fn lowered_risk_severity_mapping() {
        assert_eq!(
            risk_to_severity_lowered(TransformationRiskLevel::Critical),
            GapSeverity::Critical
        );
        assert_eq!(
            risk_to_severity_lowered(TransformationRiskLevel::High),
            GapSeverity::Major
        );
        assert_eq!(
            risk_to_severity_lowered(TransformationRiskLevel::Medium),
            GapSeverity::Minor
        );
        assert_eq!(
            risk_to_severity_lowered(TransformationRiskLevel::Low),
            GapSeverity::Info
        );
    }
}
