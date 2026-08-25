// SPDX-License-Identifier: Apache-2.0
//! Coverage-guided prioritization for backlog and parity investment.
//!
//! Uses corpus coverage data, gap triage results, and failure telemetry
//! to rank recommendations by expected user impact. Each recommendation
//! includes expected coverage gain, confidence lift, and rationale.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::fixture_taxonomy::{BlindSpot, BlindSpotImpact, CoverageReport};
use crate::gap_triage::{TriageBucket, TriageItem, TriageReport};

// ── Configuration ────────────────────────────────────────────────────────

/// Configuration for the prioritizer.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PrioritizerConfig {
    /// Weight for coverage gap signal (0.0–1.0).
    pub coverage_weight: f64,
    /// Weight for triage severity signal (0.0–1.0).
    pub triage_weight: f64,
    /// Weight for failure frequency signal (0.0–1.0).
    pub failure_weight: f64,
    /// Minimum score for a recommendation to be emitted.
    pub min_recommendation_score: f64,
    /// Maximum number of recommendations to emit.
    pub max_recommendations: usize,
}

impl Default for PrioritizerConfig {
    fn default() -> Self {
        Self {
            coverage_weight: 0.35,
            triage_weight: 0.40,
            failure_weight: 0.25,
            min_recommendation_score: 0.1,
            max_recommendations: 50,
        }
    }
}

// ── Failure Telemetry ────────────────────────────────────────────────────

/// Failure telemetry from migration runs.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct FailureTelemetry {
    /// Per-segment failure counts.
    pub segment_failures: BTreeMap<String, usize>,
    /// Per-category failure counts.
    pub category_failures: BTreeMap<String, usize>,
    /// Total migration runs observed.
    pub total_runs: usize,
    /// Per-dimension failure counts (pattern names).
    pub dimension_failures: BTreeMap<String, usize>,
}

// ── Recommendation Types ─────────────────────────────────────────────────

/// A single prioritized recommendation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Recommendation {
    /// Unique recommendation id.
    pub id: String,
    /// What kind of investment this recommends.
    pub kind: RecommendationKind,
    /// Composite priority score (0.0–1.0).
    pub score: f64,
    /// Expected coverage gain (percentage points, 0.0–100.0).
    pub expected_coverage_gain: f64,
    /// Expected confidence lift (0.0–1.0).
    pub expected_confidence_lift: f64,
    /// Human-readable rationale.
    pub rationale: String,
    /// Category this recommendation belongs to.
    pub category: String,
    /// Dimension or segment being addressed.
    pub target: String,
    /// Signals that contributed to the score.
    pub signals: RecommendationSignals,
}

/// What kind of work is recommended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RecommendationKind {
    /// Add corpus fixture covering a blind spot.
    AddFixture,
    /// Improve translator mapping for a gap.
    ImproveTranslator,
    /// Add test coverage for a failing pattern.
    AddTest,
    /// Implement missing feature/capability.
    ImplementFeature,
}

/// Signals that contributed to a recommendation's score.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecommendationSignals {
    /// Coverage gap signal (0.0–1.0).
    pub coverage_gap: f64,
    /// Triage severity signal (0.0–1.0).
    pub triage_severity: f64,
    /// Failure frequency signal (0.0–1.0).
    pub failure_frequency: f64,
}

/// Full prioritization report.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PrioritizationReport {
    /// Configuration used.
    pub config: PrioritizerConfig,
    /// Ranked recommendations (highest score first).
    pub recommendations: Vec<Recommendation>,
    /// Summary statistics.
    pub stats: PrioritizationStats,
}

/// Summary statistics for the prioritization.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PrioritizationStats {
    /// Total candidate signals evaluated.
    pub candidates_evaluated: usize,
    /// Recommendations emitted (after filtering).
    pub recommendations_emitted: usize,
    /// Breakdown by kind.
    pub by_kind: BTreeMap<String, usize>,
    /// Total expected coverage gain (sum across recommendations).
    pub total_expected_coverage_gain: f64,
    /// Mean recommendation score.
    pub mean_score: f64,
}

// ── Public API ───────────────────────────────────────────────────────────

/// Run coverage-guided prioritization.
///
/// Combines signals from three sources:
/// 1. **Coverage blind spots** → `AddFixture` recommendations
/// 2. **Triage items** → `ImproveTranslator` / `ImplementFeature`
/// 3. **Failure telemetry** → `AddTest` recommendations
pub fn prioritize(
    coverage: &CoverageReport,
    triage: &TriageReport,
    failures: &FailureTelemetry,
    config: &PrioritizerConfig,
) -> PrioritizationReport {
    let mut candidates = Vec::new();

    // Source 1: blind spots from coverage analysis.
    for (i, blind_spot) in coverage.blind_spots.iter().enumerate() {
        candidates.push(blind_spot_to_candidate(blind_spot, i, coverage));
    }

    // Source 2: triage items.
    for item in &triage.items {
        candidates.push(triage_item_to_candidate(item));
    }

    // Source 3: failure telemetry per dimension.
    for (dim, &count) in &failures.dimension_failures {
        if count > 0 {
            candidates.push(failure_to_candidate(dim, count, failures));
        }
    }

    let total_candidates = candidates.len();

    // Score and rank.
    let mut recommendations: Vec<Recommendation> = candidates
        .into_iter()
        .map(|c| score_candidate(c, config))
        .filter(|r| r.score >= config.min_recommendation_score)
        .collect();

    // Stable sort: score descending, then id ascending.
    recommendations.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.id.cmp(&b.id))
    });

    recommendations.truncate(config.max_recommendations);

    let stats = compute_stats(&recommendations, total_candidates);

    PrioritizationReport {
        config: config.clone(),
        recommendations,
        stats,
    }
}

/// Convenience: prioritize with default configuration.
pub fn prioritize_default(
    coverage: &CoverageReport,
    triage: &TriageReport,
    failures: &FailureTelemetry,
) -> PrioritizationReport {
    prioritize(coverage, triage, failures, &PrioritizerConfig::default())
}

// ── Candidate Construction ───────────────────────────────────────────────

struct Candidate {
    id: String,
    kind: RecommendationKind,
    category: String,
    target: String,
    coverage_gap: f64,
    triage_severity: f64,
    failure_frequency: f64,
    expected_coverage_gain: f64,
    expected_confidence_lift: f64,
    rationale: String,
}

fn blind_spot_to_candidate(spot: &BlindSpot, index: usize, coverage: &CoverageReport) -> Candidate {
    let impact_score = match spot.impact {
        BlindSpotImpact::High => 1.0,
        BlindSpotImpact::Medium => 0.6,
        BlindSpotImpact::Low => 0.3,
    };

    let total_dims = coverage.stats.total_dimensions_possible.max(1) as f64;
    let coverage_gain = (1.0 / total_dims) * 100.0;

    Candidate {
        id: format!("cov-blind-{:04}", index),
        kind: RecommendationKind::AddFixture,
        category: spot.category.clone(),
        target: spot.dimension.clone(),
        coverage_gap: impact_score,
        triage_severity: 0.0,
        failure_frequency: 0.0,
        expected_coverage_gain: coverage_gain,
        expected_confidence_lift: impact_score * 0.05,
        rationale: format!(
            "Coverage blind spot: {} / {} (impact: {:?}). Adding a fixture would gain {:.1}% coverage.",
            spot.category, spot.dimension, spot.impact, coverage_gain
        ),
    }
}

fn triage_item_to_candidate(item: &TriageItem) -> Candidate {
    let severity = item.score;
    let kind = match item.bucket {
        TriageBucket::Immediate => RecommendationKind::ImplementFeature,
        TriageBucket::NearTerm => RecommendationKind::ImproveTranslator,
        TriageBucket::Deferred => RecommendationKind::ImproveTranslator,
    };

    let confidence_lift = match item.bucket {
        TriageBucket::Immediate => severity * 0.15,
        TriageBucket::NearTerm => severity * 0.10,
        TriageBucket::Deferred => severity * 0.05,
    };

    Candidate {
        id: format!("tri-{}", item.gap_id),
        kind,
        category: item.category.clone(),
        target: item.segment_name.clone(),
        coverage_gap: 0.0,
        triage_severity: severity,
        failure_frequency: 0.0,
        expected_coverage_gain: 0.0,
        expected_confidence_lift: confidence_lift,
        rationale: format!(
            "Gap triage: {} (bucket: {:?}, score: {:.2}). {}",
            item.segment_name, item.bucket, item.score, item.decision_rationale
        ),
    }
}

fn failure_to_candidate(dimension: &str, count: usize, telemetry: &FailureTelemetry) -> Candidate {
    let total = telemetry.total_runs.max(1) as f64;
    let failure_rate = (count as f64) / total;

    Candidate {
        id: format!("fail-{}", dimension.replace(' ', "_").to_lowercase()),
        kind: RecommendationKind::AddTest,
        category: "failure_telemetry".into(),
        target: dimension.into(),
        coverage_gap: 0.0,
        triage_severity: 0.0,
        failure_frequency: failure_rate.min(1.0),
        expected_coverage_gain: 0.0,
        expected_confidence_lift: failure_rate * 0.10,
        rationale: format!(
            "Failure telemetry: {} failed {}/{} runs ({:.0}%). Adding tests reduces regression risk.",
            dimension,
            count,
            telemetry.total_runs,
            failure_rate * 100.0
        ),
    }
}

// ── Scoring ──────────────────────────────────────────────────────────────

fn score_candidate(c: Candidate, config: &PrioritizerConfig) -> Recommendation {
    let score = config.coverage_weight * c.coverage_gap
        + config.triage_weight * c.triage_severity
        + config.failure_weight * c.failure_frequency;

    Recommendation {
        id: c.id,
        kind: c.kind,
        score: score.clamp(0.0, 1.0),
        expected_coverage_gain: c.expected_coverage_gain,
        expected_confidence_lift: c.expected_confidence_lift,
        rationale: c.rationale,
        category: c.category,
        target: c.target,
        signals: RecommendationSignals {
            coverage_gap: c.coverage_gap,
            triage_severity: c.triage_severity,
            failure_frequency: c.failure_frequency,
        },
    }
}

// ── Stats ────────────────────────────────────────────────────────────────

fn compute_stats(
    recommendations: &[Recommendation],
    candidates_evaluated: usize,
) -> PrioritizationStats {
    let mut by_kind: BTreeMap<String, usize> = BTreeMap::new();
    let mut total_gain = 0.0;
    let mut score_sum = 0.0;

    for r in recommendations {
        let kind_name = match r.kind {
            RecommendationKind::AddFixture => "add_fixture",
            RecommendationKind::ImproveTranslator => "improve_translator",
            RecommendationKind::AddTest => "add_test",
            RecommendationKind::ImplementFeature => "implement_feature",
        };
        *by_kind.entry(kind_name.into()).or_insert(0) += 1;
        total_gain += r.expected_coverage_gain;
        score_sum += r.score;
    }

    let mean_score = if recommendations.is_empty() {
        0.0
    } else {
        score_sum / recommendations.len() as f64
    };

    PrioritizationStats {
        candidates_evaluated,
        recommendations_emitted: recommendations.len(),
        by_kind,
        total_expected_coverage_gain: total_gain,
        mean_score,
    }
}

// ── Tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::capability_gap::{BacklogAction, GapRemediation, GapSeverity};
    use crate::fixture_taxonomy::CoverageStats;
    use crate::gap_triage::{TriageBuckets, TriageConfig, TriageSignals, TriageStats};

    fn sample_coverage() -> CoverageReport {
        CoverageReport {
            ui_coverage: BTreeMap::from([("StaticContent".into(), 5)]),
            state_coverage: BTreeMap::new(),
            effect_coverage: BTreeMap::new(),
            style_coverage: BTreeMap::new(),
            accessibility_coverage: BTreeMap::new(),
            terminal_coverage: BTreeMap::new(),
            data_coverage: BTreeMap::new(),
            blind_spots: vec![
                BlindSpot {
                    category: "ui".into(),
                    dimension: "RecursiveTree".into(),
                    impact: BlindSpotImpact::High,
                },
                BlindSpot {
                    category: "effect".into(),
                    dimension: "WebSocketConnection".into(),
                    impact: BlindSpotImpact::Medium,
                },
                BlindSpot {
                    category: "style".into(),
                    dimension: "CssVariables".into(),
                    impact: BlindSpotImpact::Low,
                },
            ],
            overrepresented: vec![],
            stats: CoverageStats {
                total_fixtures: 10,
                total_dimensions_possible: 50,
                total_dimensions_covered: 20,
                coverage_percentage: 40.0,
                average_dimensions_per_fixture: 4.0,
                tier_distribution: BTreeMap::new(),
            },
        }
    }

    fn sample_triage() -> TriageReport {
        TriageReport {
            version: "gap-triage-v1".into(),
            run_id: "test-run".into(),
            config: TriageConfig::default(),
            items: vec![
                TriageItem {
                    gap_id: "gap-001".into(),
                    segment_id: "seg-001".into(),
                    segment_name: "WebSocketEffect".into(),
                    category: "effect".into(),
                    severity: GapSeverity::Critical,
                    bucket: TriageBucket::Immediate,
                    score: 0.85,
                    signals: TriageSignals {
                        impact: 0.9,
                        frequency: 0.7,
                        blocking: 0.8,
                        risk: 0.6,
                    },
                    remediation: GapRemediation {
                        approach: "Implement WebSocket adapter".into(),
                        automatable: false,
                        effort: "medium".into(),
                        backlog_action: BacklogAction::CreateFeatureRequest,
                    },
                    decision_rationale: "Critical gap, high impact".into(),
                },
                TriageItem {
                    gap_id: "gap-002".into(),
                    segment_id: "seg-002".into(),
                    segment_name: "ThemeContext".into(),
                    category: "style".into(),
                    severity: GapSeverity::Major,
                    bucket: TriageBucket::NearTerm,
                    score: 0.55,
                    signals: TriageSignals {
                        impact: 0.5,
                        frequency: 0.6,
                        blocking: 0.4,
                        risk: 0.3,
                    },
                    remediation: GapRemediation {
                        approach: "Improve theme mapping fidelity".into(),
                        automatable: true,
                        effort: "low".into(),
                        backlog_action: BacklogAction::CreateMigrationTask,
                    },
                    decision_rationale: "Medium priority, moderate impact".into(),
                },
            ],
            buckets: TriageBuckets {
                immediate: vec!["gap-001".into()],
                near_term: vec!["gap-002".into()],
                deferred: vec![],
            },
            stats: TriageStats {
                total_triaged: 2,
                immediate_count: 1,
                near_term_count: 1,
                deferred_count: 0,
                mean_score: 0.7,
                median_score: 0.7,
                by_category: BTreeMap::new(),
                by_bucket: BTreeMap::new(),
                blocking_gap_count: 1,
                automatable_count: 1,
            },
        }
    }

    fn sample_failures() -> FailureTelemetry {
        FailureTelemetry {
            segment_failures: BTreeMap::from([("seg-001".into(), 5)]),
            category_failures: BTreeMap::from([("effect".into(), 5)]),
            total_runs: 20,
            dimension_failures: BTreeMap::from([
                ("WebSocketConnection".into(), 8),
                ("TimerInterval".into(), 3),
            ]),
        }
    }

    #[test]
    fn prioritize_produces_recommendations() {
        let report = prioritize_default(&sample_coverage(), &sample_triage(), &sample_failures());
        assert!(!report.recommendations.is_empty());
    }

    #[test]
    fn recommendations_sorted_by_score_descending() {
        let report = prioritize_default(&sample_coverage(), &sample_triage(), &sample_failures());
        for pair in report.recommendations.windows(2) {
            assert!(
                pair[0].score >= pair[1].score,
                "not sorted: {} >= {} failed",
                pair[0].score,
                pair[1].score
            );
        }
    }

    #[test]
    fn high_impact_blind_spot_scores_higher_than_low() {
        let report = prioritize_default(&sample_coverage(), &sample_triage(), &sample_failures());
        let high = report
            .recommendations
            .iter()
            .find(|r| r.target == "RecursiveTree");
        let low = report
            .recommendations
            .iter()
            .find(|r| r.target == "CssVariables");

        assert!(high.is_some());
        assert!(low.is_some());
        assert!(high.unwrap().score > low.unwrap().score);
    }

    #[test]
    fn triage_immediate_becomes_implement_feature() {
        let report = prioritize_default(&sample_coverage(), &sample_triage(), &sample_failures());
        let ws = report
            .recommendations
            .iter()
            .find(|r| r.id == "tri-gap-001")
            .expect("should have triage recommendation for gap-001");
        assert_eq!(ws.kind, RecommendationKind::ImplementFeature);
    }

    #[test]
    fn triage_near_term_becomes_improve_translator() {
        let report = prioritize_default(&sample_coverage(), &sample_triage(), &sample_failures());
        let theme = report
            .recommendations
            .iter()
            .find(|r| r.id == "tri-gap-002")
            .expect("should have triage recommendation for gap-002");
        assert_eq!(theme.kind, RecommendationKind::ImproveTranslator);
    }

    #[test]
    fn failure_telemetry_becomes_add_test() {
        let report = prioritize_default(&sample_coverage(), &sample_triage(), &sample_failures());
        let ws_fail = report
            .recommendations
            .iter()
            .find(|r| r.id == "fail-websocketconnection")
            .expect("should have failure recommendation");
        assert_eq!(ws_fail.kind, RecommendationKind::AddTest);
        assert!(ws_fail.signals.failure_frequency > 0.0);
    }

    #[test]
    fn coverage_gain_positive_for_blind_spots() {
        let report = prioritize_default(&sample_coverage(), &sample_triage(), &sample_failures());
        for r in &report.recommendations {
            if r.kind == RecommendationKind::AddFixture {
                assert!(
                    r.expected_coverage_gain > 0.0,
                    "AddFixture should have positive coverage gain"
                );
            }
        }
    }

    #[test]
    fn confidence_lift_positive_for_triage_items() {
        let report = prioritize_default(&sample_coverage(), &sample_triage(), &sample_failures());
        for r in &report.recommendations {
            if r.kind == RecommendationKind::ImplementFeature
                || r.kind == RecommendationKind::ImproveTranslator
            {
                assert!(
                    r.expected_confidence_lift > 0.0,
                    "triage-based recs should have positive confidence lift"
                );
            }
        }
    }

    #[test]
    fn stats_candidate_count_matches() {
        let coverage = sample_coverage();
        let triage = sample_triage();
        let failures = sample_failures();
        let report = prioritize_default(&coverage, &triage, &failures);

        let expected_candidates =
            coverage.blind_spots.len() + triage.items.len() + failures.dimension_failures.len();
        assert_eq!(report.stats.candidates_evaluated, expected_candidates);
    }

    #[test]
    fn stats_by_kind_sums_to_total() {
        let report = prioritize_default(&sample_coverage(), &sample_triage(), &sample_failures());
        let sum: usize = report.stats.by_kind.values().sum();
        assert_eq!(sum, report.stats.recommendations_emitted);
    }

    #[test]
    fn max_recommendations_respected() {
        let config = PrioritizerConfig {
            max_recommendations: 2,
            min_recommendation_score: 0.0,
            ..Default::default()
        };
        let report = prioritize(
            &sample_coverage(),
            &sample_triage(),
            &sample_failures(),
            &config,
        );
        assert!(report.recommendations.len() <= 2);
    }

    #[test]
    fn min_score_filter_works() {
        let config = PrioritizerConfig {
            min_recommendation_score: 0.99,
            ..Default::default()
        };
        let report = prioritize(
            &sample_coverage(),
            &sample_triage(),
            &sample_failures(),
            &config,
        );
        for r in &report.recommendations {
            assert!(r.score >= 0.99);
        }
    }

    #[test]
    fn empty_inputs_produce_empty_report() {
        let coverage = CoverageReport {
            ui_coverage: BTreeMap::new(),
            state_coverage: BTreeMap::new(),
            effect_coverage: BTreeMap::new(),
            style_coverage: BTreeMap::new(),
            accessibility_coverage: BTreeMap::new(),
            terminal_coverage: BTreeMap::new(),
            data_coverage: BTreeMap::new(),
            blind_spots: vec![],
            overrepresented: vec![],
            stats: CoverageStats {
                total_fixtures: 0,
                total_dimensions_possible: 0,
                total_dimensions_covered: 0,
                coverage_percentage: 0.0,
                average_dimensions_per_fixture: 0.0,
                tier_distribution: BTreeMap::new(),
            },
        };
        let triage = TriageReport {
            version: "v1".into(),
            run_id: "empty".into(),
            config: TriageConfig::default(),
            items: vec![],
            buckets: TriageBuckets {
                immediate: vec![],
                near_term: vec![],
                deferred: vec![],
            },
            stats: TriageStats {
                total_triaged: 0,
                immediate_count: 0,
                near_term_count: 0,
                deferred_count: 0,
                mean_score: 0.0,
                median_score: 0.0,
                by_category: BTreeMap::new(),
                by_bucket: BTreeMap::new(),
                blocking_gap_count: 0,
                automatable_count: 0,
            },
        };
        let failures = FailureTelemetry::default();

        let report = prioritize_default(&coverage, &triage, &failures);
        assert!(report.recommendations.is_empty());
        assert_eq!(report.stats.candidates_evaluated, 0);
    }

    #[test]
    fn json_roundtrip() {
        let report = prioritize_default(&sample_coverage(), &sample_triage(), &sample_failures());
        let json = serde_json::to_value(&report).expect("serialize");
        let decoded: PrioritizationReport = serde_json::from_value(json).expect("deserialize");
        assert_eq!(decoded.recommendations.len(), report.recommendations.len());
        assert_eq!(
            decoded.stats.candidates_evaluated,
            report.stats.candidates_evaluated
        );
    }

    #[test]
    fn custom_weights_change_ranking() {
        let coverage = sample_coverage();
        let triage = sample_triage();
        let failures = sample_failures();

        // Coverage-heavy config.
        let cov_config = PrioritizerConfig {
            coverage_weight: 0.9,
            triage_weight: 0.05,
            failure_weight: 0.05,
            min_recommendation_score: 0.0,
            max_recommendations: 50,
        };
        let cov_report = prioritize(&coverage, &triage, &failures, &cov_config);

        // Triage-heavy config.
        let tri_config = PrioritizerConfig {
            coverage_weight: 0.05,
            triage_weight: 0.9,
            failure_weight: 0.05,
            min_recommendation_score: 0.0,
            max_recommendations: 50,
        };
        let tri_report = prioritize(&coverage, &triage, &failures, &tri_config);

        // Top recommendation should differ.
        assert_ne!(
            cov_report.recommendations[0].kind, tri_report.recommendations[0].kind,
            "different weights should produce different rankings"
        );
    }

    #[test]
    fn score_clamped_to_unit_interval() {
        let report = prioritize_default(&sample_coverage(), &sample_triage(), &sample_failures());
        for r in &report.recommendations {
            assert!(
                (0.0..=1.0).contains(&r.score),
                "score {} out of [0,1]",
                r.score
            );
        }
    }

    #[test]
    fn mean_score_correct() {
        let report = prioritize_default(&sample_coverage(), &sample_triage(), &sample_failures());
        if !report.recommendations.is_empty() {
            let sum: f64 = report.recommendations.iter().map(|r| r.score).sum();
            let expected = sum / report.recommendations.len() as f64;
            assert!(
                (report.stats.mean_score - expected).abs() < 1e-10,
                "mean_score mismatch: {} vs {}",
                report.stats.mean_score,
                expected
            );
        }
    }
}
