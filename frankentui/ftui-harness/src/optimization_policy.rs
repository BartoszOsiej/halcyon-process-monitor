#![forbid(unsafe_code)]

//! Optimization admission policy and opportunity scoring (bd-dyr5a).
//!
//! Provides a self-contained decision framework for whether a proposed
//! optimization is worth implementing. Every candidate must pass through
//! a scoring rubric and policy gate before code changes are allowed.
//!
//! # Core principles
//!
//! 1. **One lever per bead**: Each implementation bead targets one primary
//!    optimization lever unless a documented exception is approved.
//! 2. **Evidence before and after**: Candidates must attach baseline profiles
//!    and post-implementation re-profiles as proof artifacts.
//! 3. **Scored admission**: Impact × Confidence / Effort must exceed the
//!    policy threshold for a candidate to graduate from diagnosis.
//! 4. **Strategic exceptions**: Low-scoring but strategically necessary work
//!    can proceed via explicit exception with documented justification.
//!
//! # Usage
//!
//! ```ignore
//! use ftui_harness::optimization_policy::*;
//!
//! let candidate = OptimizationCandidate::new("dirty-row-tracking", PrimaryLever::AlgorithmChange)
//!     .impact(ImpactScore::High)
//!     .confidence(ConfidenceLevel::Measured)
//!     .effort(EffortEstimate::Medium)
//!     .rationale("Dirty-row tracking avoids full-scan diff for sparse updates")
//!     .baseline_id("baseline-render-diff-sparse-80x24")
//!     .hotspot_id("ftui_render::diff::compute");
//!
//! let policy = AdmissionPolicy::default_strict();
//! let verdict = policy.evaluate(&candidate);
//! assert!(verdict.admitted);
//! ```

use crate::baseline_capture::FixtureFamily;

// ============================================================================
// Primary Lever
// ============================================================================

/// The single primary optimization lever for a candidate.
/// Each implementation bead targets exactly one lever.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PrimaryLever {
    /// Change the algorithm (e.g., O(n²) → O(n log n)).
    AlgorithmChange,
    /// Reduce data structure size or improve layout (cache lines, alignment).
    DataLayout,
    /// Eliminate redundant work (caching, memoization, deduplication).
    WorkElimination,
    /// Reduce I/O volume or frequency (batching, coalescing).
    IoReduction,
    /// Improve concurrency (parallelism, lock reduction, pipeline overlap).
    Concurrency,
    /// Reduce allocation count or churn (pooling, arena, stack allocation).
    AllocationReduction,
    /// Improve branch prediction or eliminate branches.
    BranchOptimization,
    /// Leverage SIMD or hardware-specific acceleration.
    HardwareAcceleration,
}

impl PrimaryLever {
    #[must_use]
    pub const fn label(&self) -> &'static str {
        match self {
            Self::AlgorithmChange => "algorithm-change",
            Self::DataLayout => "data-layout",
            Self::WorkElimination => "work-elimination",
            Self::IoReduction => "io-reduction",
            Self::Concurrency => "concurrency",
            Self::AllocationReduction => "allocation-reduction",
            Self::BranchOptimization => "branch-optimization",
            Self::HardwareAcceleration => "hardware-acceleration",
        }
    }

    /// Typical risk level for this lever type.
    #[must_use]
    pub const fn typical_risk(&self) -> RiskLevel {
        match self {
            Self::AlgorithmChange => RiskLevel::High,
            Self::DataLayout => RiskLevel::Medium,
            Self::WorkElimination => RiskLevel::Low,
            Self::IoReduction => RiskLevel::Low,
            Self::Concurrency => RiskLevel::High,
            Self::AllocationReduction => RiskLevel::Medium,
            Self::BranchOptimization => RiskLevel::Medium,
            Self::HardwareAcceleration => RiskLevel::High,
        }
    }

    pub const ALL: &'static [PrimaryLever] = &[
        Self::AlgorithmChange,
        Self::DataLayout,
        Self::WorkElimination,
        Self::IoReduction,
        Self::Concurrency,
        Self::AllocationReduction,
        Self::BranchOptimization,
        Self::HardwareAcceleration,
    ];
}

// ============================================================================
// Scoring Dimensions
// ============================================================================

/// Expected performance impact magnitude.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum ImpactScore {
    /// < 5% improvement. Rarely worth implementing alone.
    Negligible,
    /// 5-20% improvement in the targeted metric.
    Low,
    /// 20-50% improvement.
    Medium,
    /// > 50% improvement or moves a metric from failing to passing a gate.
    High,
    /// Qualitative change: enables something previously impossible.
    Transformative,
}

impl ImpactScore {
    #[must_use]
    pub const fn numeric(&self) -> f64 {
        match self {
            Self::Negligible => 1.0,
            Self::Low => 2.0,
            Self::Medium => 4.0,
            Self::High => 7.0,
            Self::Transformative => 10.0,
        }
    }

    #[must_use]
    pub const fn label(&self) -> &'static str {
        match self {
            Self::Negligible => "negligible",
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
            Self::Transformative => "transformative",
        }
    }
}

/// Confidence in the predicted impact.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum ConfidenceLevel {
    /// Speculation without profiling data.
    Speculative,
    /// Based on profiling data from a different context or rough estimates.
    Estimated,
    /// Based on profiling data from the actual target fixture/workload.
    Measured,
    /// Confirmed by a prototype or proof-of-concept implementation.
    Proven,
}

impl ConfidenceLevel {
    #[must_use]
    pub const fn multiplier(&self) -> f64 {
        match self {
            Self::Speculative => 0.3,
            Self::Estimated => 0.6,
            Self::Measured => 0.85,
            Self::Proven => 1.0,
        }
    }

    #[must_use]
    pub const fn label(&self) -> &'static str {
        match self {
            Self::Speculative => "speculative",
            Self::Estimated => "estimated",
            Self::Measured => "measured",
            Self::Proven => "proven",
        }
    }
}

/// Implementation effort estimate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum EffortEstimate {
    /// < 1 hour: minor code change, clear path.
    Trivial,
    /// 1-4 hours: localized change with testing.
    Low,
    /// 4-16 hours: multiple files, moderate testing.
    Medium,
    /// 16-40 hours: significant refactor, extensive testing.
    High,
    /// > 40 hours: architectural change, cross-cutting.
    VeryHigh,
}

impl EffortEstimate {
    /// Effort divisor — higher effort reduces the score.
    #[must_use]
    pub const fn divisor(&self) -> f64 {
        match self {
            Self::Trivial => 1.0,
            Self::Low => 1.5,
            Self::Medium => 3.0,
            Self::High => 5.0,
            Self::VeryHigh => 8.0,
        }
    }

    #[must_use]
    pub const fn label(&self) -> &'static str {
        match self {
            Self::Trivial => "trivial",
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
            Self::VeryHigh => "very-high",
        }
    }
}

/// Risk level for an optimization change.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum RiskLevel {
    /// Low risk: localized, well-tested path.
    Low,
    /// Medium risk: moderate blast radius or complexity.
    Medium,
    /// High risk: cross-cutting, concurrency, or correctness-sensitive.
    High,
}

impl RiskLevel {
    /// Risk penalty applied to the score.
    #[must_use]
    pub const fn penalty(&self) -> f64 {
        match self {
            Self::Low => 0.0,
            Self::Medium => 0.5,
            Self::High => 1.5,
        }
    }

    #[must_use]
    pub const fn label(&self) -> &'static str {
        match self {
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
        }
    }
}

// ============================================================================
// Optimization Candidate
// ============================================================================

/// A proposed optimization candidate for admission evaluation.
#[derive(Debug, Clone)]
pub struct OptimizationCandidate {
    /// Short identifier for the candidate.
    pub id: String,
    /// Primary optimization lever.
    pub lever: PrimaryLever,
    /// Expected impact.
    pub impact: ImpactScore,
    /// Confidence in the prediction.
    pub confidence: ConfidenceLevel,
    /// Implementation effort.
    pub effort: EffortEstimate,
    /// Risk level (defaults to lever's typical risk).
    pub risk: RiskLevel,
    /// Which performance lane this targets.
    pub lane_family: FixtureFamily,
    /// Why this optimization matters.
    pub rationale: String,
    /// Baseline ID from baseline_capture for pre-implementation evidence.
    pub baseline_id: String,
    /// Hotspot ID from hotspot_extraction that motivated this candidate.
    pub hotspot_id: String,
    /// Related fixture IDs from fixture_suite.
    pub fixture_ids: Vec<String>,
    /// Whether this is a strategic exception (low score but necessary).
    pub strategic_exception: bool,
    /// Justification for strategic exception (required if exception is true).
    pub exception_justification: String,
}

impl OptimizationCandidate {
    /// Create a new candidate with minimal required fields.
    #[must_use]
    pub fn new(id: &str, lever: PrimaryLever) -> Self {
        Self {
            id: id.to_string(),
            lever,
            impact: ImpactScore::Medium,
            confidence: ConfidenceLevel::Estimated,
            effort: EffortEstimate::Medium,
            risk: lever.typical_risk(),
            lane_family: FixtureFamily::Render,
            rationale: String::new(),
            baseline_id: String::new(),
            hotspot_id: String::new(),
            fixture_ids: Vec::new(),
            strategic_exception: false,
            exception_justification: String::new(),
        }
    }

    #[must_use]
    pub fn impact(mut self, i: ImpactScore) -> Self {
        self.impact = i;
        self
    }

    #[must_use]
    pub fn confidence(mut self, c: ConfidenceLevel) -> Self {
        self.confidence = c;
        self
    }

    #[must_use]
    pub fn effort(mut self, e: EffortEstimate) -> Self {
        self.effort = e;
        self
    }

    #[must_use]
    pub fn risk(mut self, r: RiskLevel) -> Self {
        self.risk = r;
        self
    }

    #[must_use]
    pub fn lane(mut self, f: FixtureFamily) -> Self {
        self.lane_family = f;
        self
    }

    #[must_use]
    pub fn rationale(mut self, r: &str) -> Self {
        self.rationale = r.to_string();
        self
    }

    #[must_use]
    pub fn baseline_id(mut self, b: &str) -> Self {
        self.baseline_id = b.to_string();
        self
    }

    #[must_use]
    pub fn hotspot_id(mut self, h: &str) -> Self {
        self.hotspot_id = h.to_string();
        self
    }

    #[must_use]
    pub fn fixtures(mut self, f: Vec<&str>) -> Self {
        self.fixture_ids = f.into_iter().map(String::from).collect();
        self
    }

    #[must_use]
    pub fn strategic_exception(mut self, justification: &str) -> Self {
        self.strategic_exception = true;
        self.exception_justification = justification.to_string();
        self
    }

    /// Compute the raw opportunity score: Impact × Confidence / Effort - Risk.
    #[must_use]
    pub fn score(&self) -> f64 {
        let raw = self.impact.numeric() * self.confidence.multiplier() / self.effort.divisor();
        (raw - self.risk.penalty()).max(0.0)
    }

    /// Serialize to JSON.
    #[must_use]
    pub fn to_json(&self) -> String {
        let fixtures: Vec<String> = self
            .fixture_ids
            .iter()
            .map(|f| format!("\"{f}\""))
            .collect();
        format!(
            r#"{{
  "id": "{}",
  "lever": "{}",
  "impact": "{}",
  "confidence": "{}",
  "effort": "{}",
  "risk": "{}",
  "lane": "{}",
  "score": {:.3},
  "rationale": "{}",
  "baseline_id": "{}",
  "hotspot_id": "{}",
  "fixture_ids": [{}],
  "strategic_exception": {},
  "exception_justification": "{}"
}}"#,
            self.id,
            self.lever.label(),
            self.impact.label(),
            self.confidence.label(),
            self.effort.label(),
            self.risk.label(),
            self.lane_family.label(),
            self.score(),
            self.rationale.replace('"', "\\\""),
            self.baseline_id,
            self.hotspot_id,
            fixtures.join(", "),
            self.strategic_exception,
            self.exception_justification.replace('"', "\\\""),
        )
    }
}

// ============================================================================
// Required Artifacts
// ============================================================================

/// Artifacts required before and after optimization implementation.
#[derive(Debug, Clone)]
pub struct RequiredArtifacts {
    /// Pre-implementation: baseline profile against target fixtures.
    pub pre_baseline: bool,
    /// Pre-implementation: hotspot table from profiling.
    pub pre_hotspot_table: bool,
    /// Pre-implementation: rollback plan documenting how to revert.
    pub pre_rollback_plan: bool,
    /// Post-implementation: re-profile against same fixtures.
    pub post_reprofile: bool,
    /// Post-implementation: shadow-run comparison (old vs new).
    pub post_shadow_run: bool,
    /// Post-implementation: replay determinism proof.
    pub post_replay_proof: bool,
    /// Post-implementation: negative-control verification.
    pub post_negative_control: bool,
}

impl RequiredArtifacts {
    /// Standard requirements for all candidates.
    #[must_use]
    pub const fn standard() -> Self {
        Self {
            pre_baseline: true,
            pre_hotspot_table: true,
            pre_rollback_plan: true,
            post_reprofile: true,
            post_shadow_run: true,
            post_replay_proof: true,
            post_negative_control: true,
        }
    }

    /// Relaxed requirements for strategic exceptions.
    #[must_use]
    pub const fn exception() -> Self {
        Self {
            pre_baseline: true,
            pre_hotspot_table: false,
            pre_rollback_plan: true,
            post_reprofile: true,
            post_shadow_run: true,
            post_replay_proof: false,
            post_negative_control: true,
        }
    }

    /// Count of required artifacts.
    #[must_use]
    pub const fn count(&self) -> u32 {
        let mut n = 0;
        if self.pre_baseline {
            n += 1;
        }
        if self.pre_hotspot_table {
            n += 1;
        }
        if self.pre_rollback_plan {
            n += 1;
        }
        if self.post_reprofile {
            n += 1;
        }
        if self.post_shadow_run {
            n += 1;
        }
        if self.post_replay_proof {
            n += 1;
        }
        if self.post_negative_control {
            n += 1;
        }
        n
    }
}

// ============================================================================
// Admission Verdict
// ============================================================================

/// Result of evaluating a candidate against the admission policy.
#[derive(Debug, Clone)]
pub struct AdmissionVerdict {
    /// Whether the candidate is admitted.
    pub admitted: bool,
    /// Computed opportunity score.
    pub score: f64,
    /// Policy threshold that was applied.
    pub threshold: f64,
    /// Reason for the verdict.
    pub reason: String,
    /// Required artifacts for this candidate.
    pub required_artifacts: RequiredArtifacts,
    /// Warnings (non-blocking issues).
    pub warnings: Vec<String>,
}

impl AdmissionVerdict {
    /// Serialize to JSON.
    #[must_use]
    pub fn to_json(&self) -> String {
        let warnings: Vec<String> = self.warnings.iter().map(|w| format!("\"{w}\"")).collect();
        format!(
            r#"{{
  "admitted": {},
  "score": {:.3},
  "threshold": {:.3},
  "reason": "{}",
  "required_artifact_count": {},
  "warnings": [{}]
}}"#,
            self.admitted,
            self.score,
            self.threshold,
            self.reason.replace('"', "\\\""),
            self.required_artifacts.count(),
            warnings.join(", "),
        )
    }
}

// ============================================================================
// Admission Policy
// ============================================================================

/// The optimization admission policy.
#[derive(Debug, Clone)]
pub struct AdmissionPolicy {
    /// Minimum score for standard admission.
    pub score_threshold: f64,
    /// Whether strategic exceptions are allowed.
    pub allow_exceptions: bool,
    /// Whether speculative-confidence candidates are allowed.
    pub allow_speculative: bool,
    /// Maximum number of concurrent in-flight optimizations per lane.
    pub max_concurrent_per_lane: u32,
}

impl AdmissionPolicy {
    /// Default strict policy: score >= 1.0, exceptions allowed, no speculative.
    #[must_use]
    pub const fn default_strict() -> Self {
        Self {
            score_threshold: 1.0,
            allow_exceptions: true,
            allow_speculative: false,
            max_concurrent_per_lane: 2,
        }
    }

    /// Relaxed policy for exploration phases.
    #[must_use]
    pub const fn exploration() -> Self {
        Self {
            score_threshold: 0.5,
            allow_exceptions: true,
            allow_speculative: true,
            max_concurrent_per_lane: 4,
        }
    }

    /// Evaluate a candidate against this policy.
    #[must_use]
    pub fn evaluate(&self, candidate: &OptimizationCandidate) -> AdmissionVerdict {
        let score = candidate.score();
        let mut warnings = Vec::new();

        // Check speculative confidence
        if candidate.confidence == ConfidenceLevel::Speculative && !self.allow_speculative {
            return AdmissionVerdict {
                admitted: false,
                score,
                threshold: self.score_threshold,
                reason: "Speculative-confidence candidates are not allowed by this policy. \
                         Profile the target workload to upgrade confidence."
                    .to_string(),
                required_artifacts: RequiredArtifacts::standard(),
                warnings,
            };
        }

        // Check missing evidence
        if candidate.baseline_id.is_empty() {
            warnings.push(
                "No baseline_id specified — pre-implementation evidence will be weak".to_string(),
            );
        }
        if candidate.hotspot_id.is_empty() {
            warnings.push(
                "No hotspot_id specified — optimization target is not profiling-driven".to_string(),
            );
        }
        if candidate.rationale.is_empty() {
            warnings.push(
                "No rationale provided — justification will be unclear to reviewers".to_string(),
            );
        }

        // Strategic exception path
        if candidate.strategic_exception {
            if !self.allow_exceptions {
                return AdmissionVerdict {
                    admitted: false,
                    score,
                    threshold: self.score_threshold,
                    reason: "Strategic exceptions are disabled by this policy.".to_string(),
                    required_artifacts: RequiredArtifacts::standard(),
                    warnings,
                };
            }
            if candidate.exception_justification.is_empty() {
                return AdmissionVerdict {
                    admitted: false,
                    score,
                    threshold: self.score_threshold,
                    reason: "Strategic exception requested but no justification provided."
                        .to_string(),
                    required_artifacts: RequiredArtifacts::exception(),
                    warnings,
                };
            }
            warnings.push(format!(
                "Admitted as strategic exception (score {score:.3} < threshold {:.3})",
                self.score_threshold
            ));
            return AdmissionVerdict {
                admitted: true,
                score,
                threshold: self.score_threshold,
                reason: format!("Strategic exception: {}", candidate.exception_justification),
                required_artifacts: RequiredArtifacts::exception(),
                warnings,
            };
        }

        // Standard score gate
        if score >= self.score_threshold {
            AdmissionVerdict {
                admitted: true,
                score,
                threshold: self.score_threshold,
                reason: format!(
                    "Score {score:.3} >= threshold {:.3}. Lever: {}, Impact: {}, Confidence: {}",
                    self.score_threshold,
                    candidate.lever.label(),
                    candidate.impact.label(),
                    candidate.confidence.label(),
                ),
                required_artifacts: RequiredArtifacts::standard(),
                warnings,
            }
        } else {
            AdmissionVerdict {
                admitted: false,
                score,
                threshold: self.score_threshold,
                reason: format!(
                    "Score {score:.3} < threshold {:.3}. Consider: higher-impact lever, \
                     better profiling data, or strategic exception with justification.",
                    self.score_threshold,
                ),
                required_artifacts: RequiredArtifacts::standard(),
                warnings,
            }
        }
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn high_impact_measured_low_effort_admits() {
        let candidate = OptimizationCandidate::new("test-opt", PrimaryLever::WorkElimination)
            .impact(ImpactScore::High)
            .confidence(ConfidenceLevel::Measured)
            .effort(EffortEstimate::Low)
            .rationale("Eliminate redundant diff computation");

        let policy = AdmissionPolicy::default_strict();
        let verdict = policy.evaluate(&candidate);
        assert!(
            verdict.admitted,
            "high-impact measured candidate should be admitted"
        );
        assert!(verdict.score > policy.score_threshold);
    }

    #[test]
    fn negligible_speculative_high_effort_rejects() {
        let candidate = OptimizationCandidate::new("bad-opt", PrimaryLever::HardwareAcceleration)
            .impact(ImpactScore::Negligible)
            .confidence(ConfidenceLevel::Speculative)
            .effort(EffortEstimate::VeryHigh);

        let policy = AdmissionPolicy::default_strict();
        let verdict = policy.evaluate(&candidate);
        assert!(
            !verdict.admitted,
            "speculative candidates rejected by strict policy"
        );
    }

    #[test]
    fn strategic_exception_admits_low_score() {
        let candidate = OptimizationCandidate::new("strategic", PrimaryLever::DataLayout)
            .impact(ImpactScore::Low)
            .confidence(ConfidenceLevel::Estimated)
            .effort(EffortEstimate::High)
            .strategic_exception("Required for cache-line alignment before SIMD work");

        let policy = AdmissionPolicy::default_strict();
        let verdict = policy.evaluate(&candidate);
        assert!(verdict.admitted, "strategic exception should be admitted");
        assert!(
            verdict.reason.contains("Strategic exception"),
            "reason should mention exception"
        );
    }

    #[test]
    fn strategic_exception_without_justification_rejects() {
        let mut candidate = OptimizationCandidate::new("no-reason", PrimaryLever::DataLayout)
            .impact(ImpactScore::Low)
            .effort(EffortEstimate::High);
        candidate.strategic_exception = true;
        // No justification set

        let policy = AdmissionPolicy::default_strict();
        let verdict = policy.evaluate(&candidate);
        assert!(
            !verdict.admitted,
            "exception without justification should be rejected"
        );
    }

    #[test]
    fn exploration_policy_allows_speculative() {
        let candidate = OptimizationCandidate::new("explore", PrimaryLever::AlgorithmChange)
            .impact(ImpactScore::Medium)
            .confidence(ConfidenceLevel::Speculative)
            .effort(EffortEstimate::Medium);

        let policy = AdmissionPolicy::exploration();
        let verdict = policy.evaluate(&candidate);
        // Score: 4.0 * 0.3 / 3.0 - 1.5 = -1.1 → 0.0 (clamped)
        // Still below 0.5 threshold, so rejected on score
        assert!(!verdict.admitted);
    }

    #[test]
    fn score_formula_correct() {
        let candidate = OptimizationCandidate::new("formula", PrimaryLever::WorkElimination)
            .impact(ImpactScore::High) // 7.0
            .confidence(ConfidenceLevel::Measured) // 0.85
            .effort(EffortEstimate::Low) // 1.5
            .risk(RiskLevel::Low); // 0.0

        // 7.0 * 0.85 / 1.5 - 0.0 = 3.967
        let score = candidate.score();
        assert!((score - 3.967).abs() < 0.01, "expected ~3.967, got {score}");
    }

    #[test]
    fn score_clamped_to_zero() {
        let candidate = OptimizationCandidate::new("negative", PrimaryLever::Concurrency)
            .impact(ImpactScore::Negligible) // 1.0
            .confidence(ConfidenceLevel::Speculative) // 0.3
            .effort(EffortEstimate::VeryHigh) // 8.0
            .risk(RiskLevel::High); // 1.5

        // 1.0 * 0.3 / 8.0 - 1.5 = -1.4625 → clamped to 0.0
        assert!(
            candidate.score() < 0.01,
            "negative score should be clamped to 0"
        );
    }

    #[test]
    fn missing_evidence_generates_warnings() {
        let candidate = OptimizationCandidate::new("no-evidence", PrimaryLever::WorkElimination)
            .impact(ImpactScore::High)
            .confidence(ConfidenceLevel::Measured)
            .effort(EffortEstimate::Trivial);
        // No baseline_id, hotspot_id, or rationale

        let policy = AdmissionPolicy::default_strict();
        let verdict = policy.evaluate(&candidate);
        assert!(verdict.admitted, "should still admit on score");
        assert!(
            verdict.warnings.len() >= 3,
            "expected at least 3 warnings for missing evidence, got {}",
            verdict.warnings.len()
        );
    }

    #[test]
    fn primary_lever_labels_and_risk() {
        for lever in PrimaryLever::ALL {
            assert!(!lever.label().is_empty());
            assert!(!lever.typical_risk().label().is_empty());
        }
    }

    #[test]
    fn all_impact_scores_ordered() {
        assert!(ImpactScore::Negligible < ImpactScore::Low);
        assert!(ImpactScore::Low < ImpactScore::Medium);
        assert!(ImpactScore::Medium < ImpactScore::High);
        assert!(ImpactScore::High < ImpactScore::Transformative);
    }

    #[test]
    fn all_confidence_levels_ordered() {
        assert!(ConfidenceLevel::Speculative < ConfidenceLevel::Estimated);
        assert!(ConfidenceLevel::Estimated < ConfidenceLevel::Measured);
        assert!(ConfidenceLevel::Measured < ConfidenceLevel::Proven);
    }

    #[test]
    fn required_artifacts_counts() {
        assert_eq!(RequiredArtifacts::standard().count(), 7);
        assert!(RequiredArtifacts::exception().count() < RequiredArtifacts::standard().count());
    }

    #[test]
    fn candidate_to_json_valid() {
        let candidate = OptimizationCandidate::new("json-test", PrimaryLever::IoReduction)
            .impact(ImpactScore::Medium)
            .confidence(ConfidenceLevel::Measured)
            .effort(EffortEstimate::Low)
            .rationale("test rationale")
            .baseline_id("baseline-001")
            .hotspot_id("mod::func");
        let json = candidate.to_json();
        assert!(json.contains("\"id\": \"json-test\""));
        assert!(json.contains("\"lever\": \"io-reduction\""));
        assert!(json.contains("\"score\":"));
        assert!(json.contains("\"baseline_id\": \"baseline-001\""));
    }

    #[test]
    fn verdict_to_json_valid() {
        let verdict = AdmissionVerdict {
            admitted: true,
            score: 3.5,
            threshold: 1.0,
            reason: "test reason".to_string(),
            required_artifacts: RequiredArtifacts::standard(),
            warnings: vec!["test warning".to_string()],
        };
        let json = verdict.to_json();
        assert!(json.contains("\"admitted\": true"));
        assert!(json.contains("\"score\": 3.500"));
        assert!(json.contains("\"required_artifact_count\": 7"));
    }

    #[test]
    fn default_strict_policy_values() {
        let policy = AdmissionPolicy::default_strict();
        assert!((policy.score_threshold - 1.0).abs() < 0.01);
        assert!(policy.allow_exceptions);
        assert!(!policy.allow_speculative);
        assert_eq!(policy.max_concurrent_per_lane, 2);
    }

    #[test]
    fn exploration_policy_values() {
        let policy = AdmissionPolicy::exploration();
        assert!((policy.score_threshold - 0.5).abs() < 0.01);
        assert!(policy.allow_speculative);
        assert_eq!(policy.max_concurrent_per_lane, 4);
    }

    #[test]
    fn one_lever_discipline() {
        // Verify that a candidate has exactly one lever
        let candidate = OptimizationCandidate::new("single-lever", PrimaryLever::WorkElimination);
        // The type system enforces one lever per candidate — this test documents the invariant
        assert_eq!(candidate.lever, PrimaryLever::WorkElimination);
    }
}
