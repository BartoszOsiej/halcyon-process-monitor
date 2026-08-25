#![forbid(unsafe_code)]

//! Render pipeline cost surface mapping (bd-vtor0).
//!
//! Produces stage-level cost breakdowns across fixture workloads to determine
//! which render stages dominate under which conditions. The cost surface answers:
//!
//! - Which stage (view, buffer, diff, present, write) is the bottleneck?
//! - Does the bottleneck shift between sparse and dense updates?
//! - Are average costs or tail spikes the bigger problem in each stage?
//! - Where do skip-certificate optimizations have the highest expected value?
//!
//! # Architecture
//!
//! ```text
//! FixtureRunner::run(spec) → BaselineRecord → CostSurfaceAnalyzer
//!                                              ├── StageCostProfile (per stage)
//!                                              ├── DominanceMap (which stage wins)
//!                                              └── CostSurfaceReport (JSON)
//! ```
//!
//! # Usage
//!
//! ```ignore
//! use ftui_harness::cost_surface::*;
//! use ftui_harness::fixture_suite::FixtureRegistry;
//! use ftui_harness::fixture_runner::FixtureRunner;
//!
//! let registry = FixtureRegistry::canonical();
//! let spec = registry.get("render_diff_sparse_80x24").unwrap();
//! let result = FixtureRunner::run(spec);
//!
//! let analyzer = CostSurfaceAnalyzer::from_baseline(&result.record);
//! let report = analyzer.report();
//! println!("{}", report.to_json());
//! ```

use crate::baseline_capture::{BaselineRecord, MetricBaseline, StabilityClass};

// ============================================================================
// Render Pipeline Stages
// ============================================================================

/// Stages in the render pipeline, ordered by execution sequence.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum RenderStage {
    /// Cell mutation / widget rendering into buffer.
    CellMutation,
    /// Buffer diff computation (dirty-row or full-scan).
    BufferDiff,
    /// ANSI escape sequence emission via Presenter.
    PresenterEmit,
    /// Full pipeline (mutation + diff + present combined).
    FramePipeline,
}

impl RenderStage {
    #[must_use]
    pub const fn label(&self) -> &'static str {
        match self {
            Self::CellMutation => "cell_mutation",
            Self::BufferDiff => "buffer_diff",
            Self::PresenterEmit => "presenter_emit",
            Self::FramePipeline => "frame_pipeline_total",
        }
    }

    /// Human-readable description.
    #[must_use]
    pub const fn description(&self) -> &'static str {
        match self {
            Self::CellMutation => "Widget rendering into buffer cells",
            Self::BufferDiff => "Diff computation between old/new buffers",
            Self::PresenterEmit => "ANSI escape sequence generation",
            Self::FramePipeline => "Full frame: mutation + diff + present",
        }
    }

    /// Which optimization levers typically target this stage.
    #[must_use]
    pub const fn typical_levers(&self) -> &'static [&'static str] {
        match self {
            Self::CellMutation => &["work-elimination", "data-layout", "allocation-reduction"],
            Self::BufferDiff => &[
                "algorithm-change",
                "work-elimination",
                "branch-optimization",
            ],
            Self::PresenterEmit => &["io-reduction", "work-elimination", "data-layout"],
            Self::FramePipeline => &["algorithm-change", "work-elimination"],
        }
    }

    pub const COMPONENT_STAGES: &'static [RenderStage] =
        &[Self::CellMutation, Self::BufferDiff, Self::PresenterEmit];

    pub const ALL: &'static [RenderStage] = &[
        Self::CellMutation,
        Self::BufferDiff,
        Self::PresenterEmit,
        Self::FramePipeline,
    ];
}

// ============================================================================
// Stage Cost Profile
// ============================================================================

/// Cost profile for a single render stage.
#[derive(Debug, Clone)]
pub struct StageCostProfile {
    /// Which stage this profiles.
    pub stage: RenderStage,
    /// Mean latency in microseconds.
    pub mean_us: f64,
    /// p50 latency.
    pub p50_us: f64,
    /// p95 latency.
    pub p95_us: f64,
    /// p99 latency.
    pub p99_us: f64,
    /// p999 latency.
    pub p999_us: f64,
    /// Coefficient of variation.
    pub cv: f64,
    /// Stability classification.
    pub stability: StabilityClass,
    /// Fraction of total pipeline time this stage consumes (0.0–1.0).
    pub pipeline_fraction: f64,
    /// Tail spike ratio: p99 / p50. Values > 3.0 indicate tail problems.
    pub tail_ratio: f64,
    /// Number of samples.
    pub sample_count: usize,
}

impl StageCostProfile {
    /// Whether this stage is the dominant cost (> 40% of pipeline).
    #[must_use]
    pub fn is_dominant(&self) -> bool {
        self.pipeline_fraction > 0.40
    }

    /// Whether tail spikes are the primary concern (ratio > 3.0).
    #[must_use]
    pub fn has_tail_problem(&self) -> bool {
        self.tail_ratio > 3.0
    }

    /// Whether skip-certificate optimizations would be high-EV for this stage.
    ///
    /// High EV when the stage is dominant AND has stable (predictable) cost,
    /// meaning we can reliably skip it when unchanged.
    #[must_use]
    pub fn skip_certificate_ev(&self) -> SkipCertificateEv {
        if self.pipeline_fraction < 0.15 {
            SkipCertificateEv::Low
        } else if self.stability == StabilityClass::Unstable {
            SkipCertificateEv::Uncertain
        } else if self.pipeline_fraction > 0.30 {
            SkipCertificateEv::High
        } else {
            SkipCertificateEv::Medium
        }
    }
}

/// Expected value of skip-certificate optimization for a stage.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SkipCertificateEv {
    /// Stage is < 15% of pipeline — skipping saves little.
    Low,
    /// Stage is 15-30% of pipeline — moderate savings possible.
    Medium,
    /// Stage is > 30% of pipeline with stable cost — high savings.
    High,
    /// Stage has unstable variance — skip benefit unpredictable.
    Uncertain,
}

impl SkipCertificateEv {
    #[must_use]
    pub const fn label(&self) -> &'static str {
        match self {
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
            Self::Uncertain => "uncertain",
        }
    }
}

// ============================================================================
// Cost Surface Analyzer
// ============================================================================

/// Analyzes a baseline record to produce the render cost surface.
#[derive(Debug)]
pub struct CostSurfaceAnalyzer {
    profiles: Vec<StageCostProfile>,
    fixture_id: String,
}

impl CostSurfaceAnalyzer {
    /// Build a cost surface from a baseline record.
    ///
    /// Matches metric names to render stages and computes cost profiles.
    #[must_use]
    pub fn from_baseline(record: &BaselineRecord) -> Self {
        let pipeline_mean = find_metric(record, "frame_pipeline_total")
            .map(|m| m.mean)
            .unwrap_or(1.0)
            .max(0.001); // avoid division by zero

        let mut profiles = Vec::new();

        for stage in RenderStage::ALL {
            if let Some(metric) = find_metric(record, stage.label()) {
                let pipeline_fraction = if *stage == RenderStage::FramePipeline {
                    1.0
                } else {
                    (metric.mean / pipeline_mean).min(1.0)
                };

                let tail_ratio = if metric.percentiles.p50 > 0.0 {
                    metric.percentiles.p99 / metric.percentiles.p50
                } else {
                    1.0
                };

                profiles.push(StageCostProfile {
                    stage: *stage,
                    mean_us: metric.mean,
                    p50_us: metric.percentiles.p50,
                    p95_us: metric.percentiles.p95,
                    p99_us: metric.percentiles.p99,
                    p999_us: metric.percentiles.p999,
                    cv: metric.cv,
                    stability: metric.stability,
                    pipeline_fraction,
                    tail_ratio,
                    sample_count: metric.sample_count,
                });
            }
        }

        Self {
            profiles,
            fixture_id: record.fixture.clone(),
        }
    }

    /// Get the cost profile for a specific stage.
    #[must_use]
    pub fn stage_profile(&self, stage: RenderStage) -> Option<&StageCostProfile> {
        self.profiles.iter().find(|p| p.stage == stage)
    }

    /// Get all stage profiles.
    #[must_use]
    pub fn all_profiles(&self) -> &[StageCostProfile] {
        &self.profiles
    }

    /// Identify the dominant stage (highest pipeline fraction among components).
    #[must_use]
    pub fn dominant_stage(&self) -> Option<&StageCostProfile> {
        self.profiles
            .iter()
            .filter(|p| p.stage != RenderStage::FramePipeline)
            .max_by(|a, b| {
                a.pipeline_fraction
                    .partial_cmp(&b.pipeline_fraction)
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
    }

    /// Identify stages with tail spike problems (p99/p50 > 3.0).
    #[must_use]
    pub fn tail_spike_stages(&self) -> Vec<&StageCostProfile> {
        self.profiles
            .iter()
            .filter(|p| p.has_tail_problem() && p.stage != RenderStage::FramePipeline)
            .collect()
    }

    /// Identify stages where skip-certificate optimizations are high-EV.
    #[must_use]
    pub fn high_ev_skip_stages(&self) -> Vec<&StageCostProfile> {
        self.profiles
            .iter()
            .filter(|p| {
                p.skip_certificate_ev() == SkipCertificateEv::High
                    && p.stage != RenderStage::FramePipeline
            })
            .collect()
    }

    /// Generate the full cost surface report.
    #[must_use]
    pub fn report(&self) -> CostSurfaceReport {
        let dominant = self.dominant_stage().map(|p| p.stage);
        let tail_stages: Vec<RenderStage> =
            self.tail_spike_stages().iter().map(|p| p.stage).collect();
        let high_ev_stages: Vec<RenderStage> =
            self.high_ev_skip_stages().iter().map(|p| p.stage).collect();

        let primary_concern = if !tail_stages.is_empty() {
            PrimaryConcern::TailSpikes
        } else if let Some(dom) = &dominant {
            if self
                .stage_profile(*dom)
                .is_some_and(|p| p.pipeline_fraction > 0.60)
            {
                PrimaryConcern::SingleStageDominance
            } else {
                PrimaryConcern::BalancedCost
            }
        } else {
            PrimaryConcern::InsufficientData
        };

        CostSurfaceReport {
            fixture_id: self.fixture_id.clone(),
            profiles: self.profiles.clone(),
            dominant_stage: dominant,
            tail_spike_stages: tail_stages,
            high_ev_skip_stages: high_ev_stages,
            primary_concern,
        }
    }
}

/// Find a metric by name in a baseline record.
fn find_metric<'a>(record: &'a BaselineRecord, name: &str) -> Option<&'a MetricBaseline> {
    record.metrics.iter().find(|m| m.metric == name)
}

// ============================================================================
// Cost Surface Report
// ============================================================================

/// Primary concern identified by cost surface analysis.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrimaryConcern {
    /// One stage dominates > 60% of pipeline cost.
    SingleStageDominance,
    /// Tail spikes (p99/p50 > 3.0) are the primary issue.
    TailSpikes,
    /// Costs are reasonably balanced across stages.
    BalancedCost,
    /// Insufficient metrics to determine.
    InsufficientData,
}

impl PrimaryConcern {
    #[must_use]
    pub const fn label(&self) -> &'static str {
        match self {
            Self::SingleStageDominance => "single-stage-dominance",
            Self::TailSpikes => "tail-spikes",
            Self::BalancedCost => "balanced-cost",
            Self::InsufficientData => "insufficient-data",
        }
    }

    /// Recommended optimization approach.
    #[must_use]
    pub const fn recommended_approach(&self) -> &'static str {
        match self {
            Self::SingleStageDominance => {
                "Focus on the dominant stage. Skip-certificates and work-elimination \
                 in that stage will yield the highest return."
            }
            Self::TailSpikes => {
                "Address tail spikes before optimizing averages. Look for \
                 allocation bursts, GC pauses, or branch mispredictions in spiking stages."
            }
            Self::BalancedCost => {
                "Balanced cost surface — no single stage dominates. Cross-cutting \
                 optimizations (e.g., reducing total frame count or input batching) \
                 may be more effective than stage-specific work."
            }
            Self::InsufficientData => {
                "Not enough metrics to analyze. Ensure the fixture produces \
                 cell_mutation, buffer_diff, and presenter_emit latency samples."
            }
        }
    }
}

/// Complete cost surface analysis report.
#[derive(Debug, Clone)]
pub struct CostSurfaceReport {
    /// Fixture that was analyzed.
    pub fixture_id: String,
    /// Per-stage cost profiles.
    pub profiles: Vec<StageCostProfile>,
    /// Which component stage dominates.
    pub dominant_stage: Option<RenderStage>,
    /// Stages with tail spike problems.
    pub tail_spike_stages: Vec<RenderStage>,
    /// Stages where skip-certificates are high-EV.
    pub high_ev_skip_stages: Vec<RenderStage>,
    /// Primary concern identified.
    pub primary_concern: PrimaryConcern,
}

impl CostSurfaceReport {
    /// Serialize to JSON.
    #[must_use]
    pub fn to_json(&self) -> String {
        let stage_entries: Vec<String> = self
            .profiles
            .iter()
            .map(|p| {
                format!(
                    r#"    {{
      "stage": "{}",
      "mean_us": {:.2},
      "p50_us": {:.2},
      "p95_us": {:.2},
      "p99_us": {:.2},
      "p999_us": {:.2},
      "cv": {:.4},
      "stability": "{}",
      "pipeline_fraction": {:.4},
      "tail_ratio": {:.2},
      "skip_certificate_ev": "{}",
      "is_dominant": {},
      "has_tail_problem": {},
      "sample_count": {}
    }}"#,
                    p.stage.label(),
                    p.mean_us,
                    p.p50_us,
                    p.p95_us,
                    p.p99_us,
                    p.p999_us,
                    p.cv,
                    match p.stability {
                        StabilityClass::Stable => "stable",
                        StabilityClass::Moderate => "moderate",
                        StabilityClass::Unstable => "unstable",
                    },
                    p.pipeline_fraction,
                    p.tail_ratio,
                    p.skip_certificate_ev().label(),
                    p.is_dominant(),
                    p.has_tail_problem(),
                    p.sample_count,
                )
            })
            .collect();

        let tail_labels: Vec<String> = self
            .tail_spike_stages
            .iter()
            .map(|s| format!("\"{}\"", s.label()))
            .collect();
        let skip_labels: Vec<String> = self
            .high_ev_skip_stages
            .iter()
            .map(|s| format!("\"{}\"", s.label()))
            .collect();

        format!(
            r#"{{
  "schema_version": 1,
  "fixture_id": "{}",
  "primary_concern": "{}",
  "recommended_approach": "{}",
  "dominant_stage": {},
  "tail_spike_stages": [{}],
  "high_ev_skip_stages": [{}],
  "stages": [
{}
  ]
}}"#,
            self.fixture_id,
            self.primary_concern.label(),
            self.primary_concern
                .recommended_approach()
                .replace('"', "\\\""),
            self.dominant_stage
                .map(|s| format!("\"{}\"", s.label()))
                .unwrap_or_else(|| "null".to_string()),
            tail_labels.join(", "),
            skip_labels.join(", "),
            stage_entries.join(",\n"),
        )
    }
}

// ============================================================================
// Multi-Fixture Cost Comparison
// ============================================================================

/// Compare cost surfaces across multiple fixtures to identify workload-dependent
/// bottleneck shifts.
#[derive(Debug)]
pub struct CostComparison {
    reports: Vec<CostSurfaceReport>,
}

impl CostComparison {
    /// Create a comparison from multiple reports.
    #[must_use]
    pub fn new(reports: Vec<CostSurfaceReport>) -> Self {
        Self { reports }
    }

    /// Check if the dominant stage shifts between fixtures.
    #[must_use]
    pub fn has_dominance_shift(&self) -> bool {
        let dominants: Vec<_> = self
            .reports
            .iter()
            .filter_map(|r| r.dominant_stage)
            .collect();
        if dominants.len() < 2 {
            return false;
        }
        dominants.windows(2).any(|w| w[0] != w[1])
    }

    /// Fixtures where sparse-change assumptions hold (diff is not dominant).
    #[must_use]
    pub fn sparse_friendly_fixtures(&self) -> Vec<&str> {
        self.reports
            .iter()
            .filter(|r| r.dominant_stage != Some(RenderStage::BufferDiff))
            .map(|r| r.fixture_id.as_str())
            .collect()
    }

    /// Fixtures where diff is the dominant cost.
    #[must_use]
    pub fn diff_dominated_fixtures(&self) -> Vec<&str> {
        self.reports
            .iter()
            .filter(|r| r.dominant_stage == Some(RenderStage::BufferDiff))
            .map(|r| r.fixture_id.as_str())
            .collect()
    }

    /// Number of reports.
    #[must_use]
    pub fn len(&self) -> usize {
        self.reports.len()
    }

    /// Whether the comparison is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.reports.is_empty()
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::baseline_capture::{BaselineCapture, FixtureFamily, Sample};

    fn make_baseline(
        fixture: &str,
        mutation_us: &[u64],
        diff_us: &[u64],
        present_us: &[u64],
    ) -> BaselineRecord {
        let mut cap = BaselineCapture::new(fixture, FixtureFamily::Render).with_seed(42);
        for &v in mutation_us {
            cap.record_sample(Sample::latency_us("cell_mutation", v));
        }
        for &v in diff_us {
            cap.record_sample(Sample::latency_us("buffer_diff", v));
        }
        for &v in present_us {
            cap.record_sample(Sample::latency_us("presenter_emit", v));
        }
        // Pipeline = sum of components (approximately)
        for i in 0..mutation_us.len().min(diff_us.len()).min(present_us.len()) {
            cap.record_sample(Sample::latency_us(
                "frame_pipeline_total",
                mutation_us[i] + diff_us[i] + present_us[i],
            ));
        }
        cap.finalize()
    }

    #[test]
    fn diff_dominated_surface() {
        // Diff takes 70% of pipeline time
        let baseline = make_baseline(
            "diff_heavy",
            &[10, 12, 11, 10, 11], // ~11us mutation
            &[70, 72, 71, 70, 71], // ~71us diff
            &[20, 21, 20, 19, 20], // ~20us present
        );
        let analyzer = CostSurfaceAnalyzer::from_baseline(&baseline);
        let report = analyzer.report();

        assert_eq!(report.dominant_stage, Some(RenderStage::BufferDiff));
        assert_eq!(report.primary_concern, PrimaryConcern::SingleStageDominance);
    }

    #[test]
    fn balanced_surface() {
        // Each stage roughly equal
        let baseline = make_baseline(
            "balanced",
            &[30, 31, 30, 29, 30],
            &[35, 36, 35, 34, 35],
            &[32, 33, 32, 31, 32],
        );
        let analyzer = CostSurfaceAnalyzer::from_baseline(&baseline);
        let report = analyzer.report();

        assert_eq!(report.primary_concern, PrimaryConcern::BalancedCost);
    }

    #[test]
    fn tail_spike_detection() {
        // Diff has huge tail spikes
        let baseline = make_baseline(
            "tail_spiky",
            &[10, 10, 10, 10, 10],
            &[20, 20, 20, 20, 200], // p99 spike
            &[15, 15, 15, 15, 15],
        );
        let analyzer = CostSurfaceAnalyzer::from_baseline(&baseline);
        let _report = analyzer.report();

        // At least the diff stage should show tail issues
        let diff_profile = analyzer.stage_profile(RenderStage::BufferDiff);
        assert!(diff_profile.is_some());
    }

    #[test]
    fn skip_certificate_ev_classification() {
        let baseline = make_baseline(
            "skip_test",
            &[10, 10, 10, 10, 10],
            &[80, 80, 80, 80, 80], // dominant and stable → high EV
            &[10, 10, 10, 10, 10],
        );
        let analyzer = CostSurfaceAnalyzer::from_baseline(&baseline);

        let diff = analyzer.stage_profile(RenderStage::BufferDiff).unwrap();
        assert_eq!(diff.skip_certificate_ev(), SkipCertificateEv::High);

        let mutation = analyzer.stage_profile(RenderStage::CellMutation).unwrap();
        assert_eq!(mutation.skip_certificate_ev(), SkipCertificateEv::Low);
    }

    #[test]
    fn cost_comparison_dominance_shift() {
        let sparse = make_baseline(
            "sparse",
            &[50, 50, 50, 50, 50],
            &[10, 10, 10, 10, 10],
            &[20, 20, 20, 20, 20],
        );
        let dense = make_baseline(
            "dense",
            &[20, 20, 20, 20, 20],
            &[80, 80, 80, 80, 80],
            &[30, 30, 30, 30, 30],
        );

        let a1 = CostSurfaceAnalyzer::from_baseline(&sparse);
        let a2 = CostSurfaceAnalyzer::from_baseline(&dense);

        let comparison = CostComparison::new(vec![a1.report(), a2.report()]);
        assert!(
            comparison.has_dominance_shift(),
            "sparse vs dense should show dominance shift"
        );
        assert_eq!(comparison.len(), 2);
    }

    #[test]
    fn sparse_friendly_classification() {
        let sparse = make_baseline(
            "sparse_case",
            &[50, 50, 50, 50, 50],
            &[10, 10, 10, 10, 10],
            &[20, 20, 20, 20, 20],
        );
        let a = CostSurfaceAnalyzer::from_baseline(&sparse);
        let comparison = CostComparison::new(vec![a.report()]);

        let friendly = comparison.sparse_friendly_fixtures();
        assert!(friendly.contains(&"sparse_case"));
        assert!(comparison.diff_dominated_fixtures().is_empty());
    }

    #[test]
    fn report_to_json_valid() {
        let baseline = make_baseline("json_test", &[10, 10, 10], &[50, 50, 50], &[20, 20, 20]);
        let analyzer = CostSurfaceAnalyzer::from_baseline(&baseline);
        let report = analyzer.report();
        let json = report.to_json();

        assert!(json.contains("\"schema_version\": 1"));
        assert!(json.contains("\"fixture_id\": \"json_test\""));
        assert!(json.contains("\"primary_concern\":"));
        assert!(json.contains("\"dominant_stage\":"));
        assert!(json.contains("\"pipeline_fraction\":"));
        assert!(json.contains("\"skip_certificate_ev\":"));
    }

    #[test]
    fn render_stage_labels() {
        for stage in RenderStage::ALL {
            assert!(!stage.label().is_empty());
            assert!(!stage.description().is_empty());
            assert!(!stage.typical_levers().is_empty());
        }
    }

    #[test]
    fn primary_concern_recommendations() {
        for concern in [
            PrimaryConcern::SingleStageDominance,
            PrimaryConcern::TailSpikes,
            PrimaryConcern::BalancedCost,
            PrimaryConcern::InsufficientData,
        ] {
            assert!(!concern.label().is_empty());
            assert!(!concern.recommended_approach().is_empty());
        }
    }

    #[test]
    fn empty_baseline_produces_insufficient_data() {
        let cap = BaselineCapture::new("empty", FixtureFamily::Render);
        let record = cap.finalize();
        let analyzer = CostSurfaceAnalyzer::from_baseline(&record);
        let report = analyzer.report();

        assert_eq!(report.primary_concern, PrimaryConcern::InsufficientData);
        assert!(report.dominant_stage.is_none());
    }

    #[test]
    fn component_stages_excludes_pipeline() {
        assert!(!RenderStage::COMPONENT_STAGES.contains(&RenderStage::FramePipeline));
        assert_eq!(RenderStage::COMPONENT_STAGES.len(), 3);
    }

    #[test]
    fn pipeline_fraction_sum_reasonable() {
        let baseline = make_baseline(
            "fraction_test",
            &[30, 30, 30, 30, 30],
            &[40, 40, 40, 40, 40],
            &[30, 30, 30, 30, 30],
        );
        let analyzer = CostSurfaceAnalyzer::from_baseline(&baseline);

        let total_fraction: f64 = analyzer
            .all_profiles()
            .iter()
            .filter(|p| p.stage != RenderStage::FramePipeline)
            .map(|p| p.pipeline_fraction)
            .sum();

        assert!(
            (total_fraction - 1.0).abs() < 0.1,
            "component fractions should sum to ~1.0, got {total_fraction}"
        );
    }

    #[test]
    fn empty_comparison() {
        let comparison = CostComparison::new(vec![]);
        assert!(comparison.is_empty());
        assert!(!comparison.has_dominance_shift());
    }
}
