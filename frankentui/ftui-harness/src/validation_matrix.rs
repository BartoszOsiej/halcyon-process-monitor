#![forbid(unsafe_code)]

//! Comprehensive validation matrix and logging contract for performance work (bd-pou7j).
//!
//! Maps every performance lane to its minimum required validation coverage
//! and logging obligations. This module is the single source of truth for
//! what proof is needed before any performance change graduates from diagnosis
//! to implementation and from implementation to production.
//!
//! # Design principles
//!
//! 1. **Exhaustive**: Every lane has explicit obligations for unit, property,
//!    integration, E2E, soak, replay, and shadow-comparison coverage.
//! 2. **Gating vs informative**: Each obligation is classified as either
//!    gating (must pass for promotion) or informative (reported but not blocking).
//! 3. **Negative assertions**: Explicitly covers what must NOT change, including
//!    no-op behavior, bounded degradation, and challenge-fixture fallback.
//! 4. **Failure forensics**: Defines minimum diagnostic output for failure triage.
//!
//! # Usage
//!
//! ```ignore
//! use ftui_harness::validation_matrix::*;
//!
//! let matrix = ValidationMatrix::canonical();
//! for obligation in matrix.obligations_for(PerfLane::Render) {
//!     println!("{}: {} ({})", obligation.id, obligation.description,
//!         if obligation.gating { "GATING" } else { "informative" });
//! }
//! ```

use std::collections::BTreeMap;

use crate::baseline_capture::FixtureFamily;

// ============================================================================
// Performance Lanes
// ============================================================================

/// Performance lanes aligned with fixture families.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum PerfLane {
    /// Render pipeline: buffer, diff, presenter, frame.
    Render,
    /// Runtime: event loop, subscriptions, effects, shutdown.
    Runtime,
    /// Doctor: capture, seed, suite, report workflows.
    Doctor,
    /// Cross-lane: validation that spans multiple subsystems.
    CrossLane,
}

impl PerfLane {
    #[must_use]
    pub const fn label(&self) -> &'static str {
        match self {
            Self::Render => "render",
            Self::Runtime => "runtime",
            Self::Doctor => "doctor",
            Self::CrossLane => "cross-lane",
        }
    }

    /// Map from `FixtureFamily` for cross-referencing with fixture suites.
    #[must_use]
    pub const fn from_fixture_family(family: FixtureFamily) -> Self {
        match family {
            FixtureFamily::Render => Self::Render,
            FixtureFamily::Runtime => Self::Runtime,
            FixtureFamily::Doctor => Self::Doctor,
            FixtureFamily::Challenge => Self::CrossLane,
        }
    }

    pub const ALL: &'static [PerfLane] =
        &[Self::Render, Self::Runtime, Self::Doctor, Self::CrossLane];
}

// ============================================================================
// Validation Level
// ============================================================================

/// Types of validation coverage required.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum ValidationLevel {
    /// Inline `#[cfg(test)]` unit tests covering invariants and edge cases.
    Unit,
    /// Property-based / fuzzing tests (proptest) for invariant exploration.
    Property,
    /// Cross-module integration tests.
    Integration,
    /// End-to-end scripts exercising realistic workflows.
    EndToEnd,
    /// Long-running soak tests for stability and resource leak detection.
    Soak,
    /// Deterministic replay with checksum verification.
    Replay,
    /// Shadow-run comparison between old and new implementations.
    ShadowComparison,
}

impl ValidationLevel {
    #[must_use]
    pub const fn label(&self) -> &'static str {
        match self {
            Self::Unit => "unit",
            Self::Property => "property",
            Self::Integration => "integration",
            Self::EndToEnd => "e2e",
            Self::Soak => "soak",
            Self::Replay => "replay",
            Self::ShadowComparison => "shadow-comparison",
        }
    }

    /// Typical execution time category.
    #[must_use]
    pub const fn time_class(&self) -> TimeClass {
        match self {
            Self::Unit => TimeClass::Fast,
            Self::Property => TimeClass::Medium,
            Self::Integration => TimeClass::Medium,
            Self::EndToEnd => TimeClass::Slow,
            Self::Soak => TimeClass::VeryLong,
            Self::Replay => TimeClass::Medium,
            Self::ShadowComparison => TimeClass::Slow,
        }
    }

    pub const ALL: &'static [ValidationLevel] = &[
        Self::Unit,
        Self::Property,
        Self::Integration,
        Self::EndToEnd,
        Self::Soak,
        Self::Replay,
        Self::ShadowComparison,
    ];
}

/// Execution time classification for CI scheduling.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TimeClass {
    /// < 10 seconds. Run on every commit.
    Fast,
    /// 10s - 2 minutes. Run in CI.
    Medium,
    /// 2 - 10 minutes. Run in nightly or pre-merge.
    Slow,
    /// > 10 minutes. Run in scheduled soak jobs.
    VeryLong,
}

impl TimeClass {
    #[must_use]
    pub const fn label(&self) -> &'static str {
        match self {
            Self::Fast => "fast",
            Self::Medium => "medium",
            Self::Slow => "slow",
            Self::VeryLong => "very-long",
        }
    }
}

// ============================================================================
// Assertion Category
// ============================================================================

/// What an obligation asserts about the system.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AssertionCategory {
    /// Something MUST improve (latency, throughput).
    Improvement,
    /// Something MUST NOT regress beyond threshold.
    NoRegression,
    /// Something MUST remain exactly unchanged (negative control).
    NoChange,
    /// Degradation is acceptable within declared bounds.
    BoundedDegradation,
    /// Graceful fallback behavior under stress/failure.
    GracefulFallback,
    /// Failure diagnosis artifacts must be present and actionable.
    FailureForensics,
}

impl AssertionCategory {
    #[must_use]
    pub const fn label(&self) -> &'static str {
        match self {
            Self::Improvement => "improvement",
            Self::NoRegression => "no-regression",
            Self::NoChange => "no-change",
            Self::BoundedDegradation => "bounded-degradation",
            Self::GracefulFallback => "graceful-fallback",
            Self::FailureForensics => "failure-forensics",
        }
    }

    /// Whether this is a negative assertion (proving absence of harm).
    #[must_use]
    pub const fn is_negative(&self) -> bool {
        matches!(
            self,
            Self::NoRegression | Self::NoChange | Self::BoundedDegradation
        )
    }
}

// ============================================================================
// Validation Obligation
// ============================================================================

/// A single validation obligation in the matrix.
#[derive(Debug, Clone)]
pub struct ValidationObligation {
    /// Stable identifier (e.g., "render.unit.diff-invariants").
    pub id: String,
    /// Performance lane this obligation belongs to.
    pub lane: PerfLane,
    /// Required validation level.
    pub level: ValidationLevel,
    /// What this obligation asserts.
    pub assertion: AssertionCategory,
    /// Whether this obligation gates promotion (true) or is informative (false).
    pub gating: bool,
    /// Human-readable description of what must be proven.
    pub description: String,
    /// Expected artifact outputs (e.g., "baseline.json", "replay.jsonl").
    pub expected_artifacts: Vec<String>,
    /// Failure-forensics: what diagnostic output must be present on failure.
    pub failure_diagnostics: Vec<String>,
    /// Related fixture IDs from the fixture suite.
    pub fixture_ids: Vec<String>,
    /// Tags for filtering.
    pub tags: Vec<String>,
}

impl ValidationObligation {
    /// Builder constructor.
    #[must_use]
    pub fn new(id: &str, lane: PerfLane, level: ValidationLevel) -> Self {
        Self {
            id: id.to_string(),
            lane,
            level,
            assertion: AssertionCategory::NoRegression,
            gating: true,
            description: String::new(),
            expected_artifacts: Vec::new(),
            failure_diagnostics: Vec::new(),
            fixture_ids: Vec::new(),
            tags: Vec::new(),
        }
    }

    #[must_use]
    pub fn assertion(mut self, a: AssertionCategory) -> Self {
        self.assertion = a;
        self
    }

    #[must_use]
    pub fn gating(mut self, g: bool) -> Self {
        self.gating = g;
        self
    }

    #[must_use]
    pub fn description(mut self, d: &str) -> Self {
        self.description = d.to_string();
        self
    }

    #[must_use]
    pub fn artifacts(mut self, a: Vec<&str>) -> Self {
        self.expected_artifacts = a.into_iter().map(String::from).collect();
        self
    }

    #[must_use]
    pub fn diagnostics(mut self, d: Vec<&str>) -> Self {
        self.failure_diagnostics = d.into_iter().map(String::from).collect();
        self
    }

    #[must_use]
    pub fn fixtures(mut self, f: Vec<&str>) -> Self {
        self.fixture_ids = f.into_iter().map(String::from).collect();
        self
    }

    #[must_use]
    pub fn tags(mut self, t: Vec<&str>) -> Self {
        self.tags = t.into_iter().map(String::from).collect();
        self
    }

    /// Serialize to JSON.
    #[must_use]
    pub fn to_json(&self) -> String {
        let artifacts: Vec<String> = self
            .expected_artifacts
            .iter()
            .map(|a| format!("\"{a}\""))
            .collect();
        let diagnostics: Vec<String> = self
            .failure_diagnostics
            .iter()
            .map(|d| format!("\"{d}\""))
            .collect();
        let fixtures: Vec<String> = self
            .fixture_ids
            .iter()
            .map(|f| format!("\"{f}\""))
            .collect();
        let tags: Vec<String> = self.tags.iter().map(|t| format!("\"{t}\"")).collect();

        format!(
            r#"{{
    "id": "{}",
    "lane": "{}",
    "level": "{}",
    "assertion": "{}",
    "gating": {},
    "description": "{}",
    "expected_artifacts": [{}],
    "failure_diagnostics": [{}],
    "fixture_ids": [{}],
    "tags": [{}]
  }}"#,
            self.id,
            self.lane.label(),
            self.level.label(),
            self.assertion.label(),
            self.gating,
            self.description.replace('"', "\\\""),
            artifacts.join(", "),
            diagnostics.join(", "),
            fixtures.join(", "),
            tags.join(", "),
        )
    }
}

// ============================================================================
// Logging Schema Field
// ============================================================================

/// A field in the performance logging schema.
#[derive(Debug, Clone)]
pub struct LogField {
    /// Field name (e.g., "run_id", "stage", "latency_us").
    pub name: String,
    /// Data type (string, u64, f64, bool, timestamp).
    pub field_type: String,
    /// Whether this field is required in every log event.
    pub required: bool,
    /// Description of what this field contains.
    pub description: String,
}

impl LogField {
    #[must_use]
    pub fn new(name: &str, field_type: &str, required: bool, description: &str) -> Self {
        Self {
            name: name.to_string(),
            field_type: field_type.to_string(),
            required,
            description: description.to_string(),
        }
    }
}

/// Logging contract for a performance lane.
#[derive(Debug, Clone)]
pub struct LoggingContract {
    /// Lane this contract applies to.
    pub lane: PerfLane,
    /// Required fields in every log event for this lane.
    pub fields: Vec<LogField>,
    /// Stable event names that must be emitted.
    pub required_events: Vec<String>,
    /// Reason codes that the lane must define for failure classification.
    pub reason_codes: Vec<String>,
}

impl LoggingContract {
    #[must_use]
    pub fn new(lane: PerfLane) -> Self {
        Self {
            lane,
            fields: Vec::new(),
            required_events: Vec::new(),
            reason_codes: Vec::new(),
        }
    }

    #[must_use]
    pub fn field(mut self, f: LogField) -> Self {
        self.fields.push(f);
        self
    }

    #[must_use]
    pub fn event(mut self, name: &str) -> Self {
        self.required_events.push(name.to_string());
        self
    }

    #[must_use]
    pub fn reason_code(mut self, code: &str) -> Self {
        self.reason_codes.push(code.to_string());
        self
    }
}

// ============================================================================
// Validation Matrix
// ============================================================================

/// The complete validation matrix for performance work.
#[derive(Debug, Clone)]
pub struct ValidationMatrix {
    obligations: Vec<ValidationObligation>,
    logging_contracts: BTreeMap<PerfLane, LoggingContract>,
}

impl ValidationMatrix {
    /// Create an empty matrix.
    #[must_use]
    pub fn new() -> Self {
        Self {
            obligations: Vec::new(),
            logging_contracts: BTreeMap::new(),
        }
    }

    /// Add a validation obligation.
    pub fn add_obligation(&mut self, obligation: ValidationObligation) {
        self.obligations.push(obligation);
    }

    /// Set the logging contract for a lane.
    pub fn set_logging_contract(&mut self, contract: LoggingContract) {
        self.logging_contracts.insert(contract.lane, contract);
    }

    /// All obligations in the matrix.
    #[must_use]
    pub fn all_obligations(&self) -> &[ValidationObligation] {
        &self.obligations
    }

    /// Obligations filtered by lane.
    #[must_use]
    pub fn obligations_for(&self, lane: PerfLane) -> Vec<&ValidationObligation> {
        self.obligations.iter().filter(|o| o.lane == lane).collect()
    }

    /// Obligations filtered by validation level.
    #[must_use]
    pub fn obligations_at_level(&self, level: ValidationLevel) -> Vec<&ValidationObligation> {
        self.obligations
            .iter()
            .filter(|o| o.level == level)
            .collect()
    }

    /// Only gating obligations (must pass for promotion).
    #[must_use]
    pub fn gating_obligations(&self) -> Vec<&ValidationObligation> {
        self.obligations.iter().filter(|o| o.gating).collect()
    }

    /// Only negative assertions (proving absence of harm).
    #[must_use]
    pub fn negative_assertions(&self) -> Vec<&ValidationObligation> {
        self.obligations
            .iter()
            .filter(|o| o.assertion.is_negative())
            .collect()
    }

    /// Logging contract for a lane.
    #[must_use]
    pub fn logging_contract_for(&self, lane: PerfLane) -> Option<&LoggingContract> {
        self.logging_contracts.get(&lane)
    }

    /// Total obligation count.
    #[must_use]
    pub fn obligation_count(&self) -> usize {
        self.obligations.len()
    }

    /// Serialize the full matrix to JSON.
    #[must_use]
    pub fn to_json(&self) -> String {
        let gating_count = self.gating_obligations().len();
        let negative_count = self.negative_assertions().len();
        let entries: Vec<String> = self.obligations.iter().map(|o| o.to_json()).collect();

        format!(
            r#"{{
  "schema_version": 1,
  "total_obligations": {},
  "gating_count": {},
  "informative_count": {},
  "negative_assertion_count": {},
  "obligations": [
{}
  ]
}}"#,
            self.obligations.len(),
            gating_count,
            self.obligations.len() - gating_count,
            negative_count,
            entries
                .iter()
                .map(|e| format!("  {e}"))
                .collect::<Vec<_>>()
                .join(",\n"),
        )
    }

    /// Build the canonical validation matrix for all performance lanes.
    #[must_use]
    pub fn canonical() -> Self {
        let mut matrix = Self::new();

        // ====================================================================
        // RENDER LANE
        // ====================================================================

        matrix.add_obligation(
            ValidationObligation::new("render.unit.diff-invariants", PerfLane::Render, ValidationLevel::Unit)
                .assertion(AssertionCategory::NoRegression)
                .description("Diff engine must produce identical output for identical input pairs across all strategies")
                .artifacts(vec!["test_results.json"])
                .diagnostics(vec!["buffer_hex_dump", "cell_mismatch_report"])
                .fixtures(vec!["render_diff_sparse_80x24", "render_diff_dense_80x24"])
                .tags(vec!["diff", "determinism"]),
        );

        matrix.add_obligation(
            ValidationObligation::new(
                "render.unit.presenter-state-tracking",
                PerfLane::Render,
                ValidationLevel::Unit,
            )
            .assertion(AssertionCategory::Improvement)
            .description("Presenter must eliminate redundant SGR sequences via state tracking")
            .artifacts(vec!["ansi_bytes_per_frame.json"])
            .diagnostics(vec!["sgr_sequence_log", "state_transition_trace"])
            .fixtures(vec!["render_presenter_emit_120x40"])
            .tags(vec!["presenter", "ansi", "output-cost"]),
        );

        matrix.add_obligation(
            ValidationObligation::new(
                "render.integration.pipeline-budget",
                PerfLane::Render,
                ValidationLevel::Integration,
            )
            .assertion(AssertionCategory::NoRegression)
            .description(
                "Full render pipeline p99 must remain within frame budget at target viewports",
            )
            .artifacts(vec!["baseline.json", "pipeline_latency.jsonl"])
            .diagnostics(vec![
                "frame_budget_violation_report",
                "stage_timing_breakdown",
            ])
            .fixtures(vec!["render_pipeline_full_200x60"])
            .tags(vec!["pipeline", "budget", "latency"]),
        );

        matrix.add_obligation(
            ValidationObligation::new(
                "render.e2e.resize-stability",
                PerfLane::Render,
                ValidationLevel::EndToEnd,
            )
            .assertion(AssertionCategory::GracefulFallback)
            .description("Resize storms must not produce garbled output, panics, or resource leaks")
            .artifacts(vec!["resize_storm_report.json", "frame_checksums.jsonl"])
            .diagnostics(vec![
                "panic_backtrace",
                "buffer_corruption_snapshot",
                "resize_event_log",
            ])
            .fixtures(vec!["challenge_resize_storm"])
            .tags(vec!["resize", "stress", "fallback"]),
        );

        matrix.add_obligation(
            ValidationObligation::new(
                "render.replay.frame-determinism",
                PerfLane::Render,
                ValidationLevel::Replay,
            )
            .assertion(AssertionCategory::NoChange)
            .description(
                "Replay of deterministic fixtures must produce byte-identical frame checksums",
            )
            .artifacts(vec!["replay.jsonl", "checksum_comparison.json"])
            .diagnostics(vec!["frame_mismatch_diff", "replay_divergence_point"])
            .fixtures(vec!["render_diff_sparse_80x24", "control_static_screen"])
            .tags(vec!["replay", "determinism", "negative-control"]),
        );

        matrix.add_obligation(
            ValidationObligation::new(
                "render.shadow.diff-strategy-parity",
                PerfLane::Render,
                ValidationLevel::ShadowComparison,
            )
            .assertion(AssertionCategory::NoChange)
            .description("Shadow-run old vs new diff strategy must produce identical ANSI output")
            .artifacts(vec!["shadow_run_result.json"])
            .diagnostics(vec!["shadow_mismatch_diff", "strategy_decision_log"])
            .fixtures(vec!["render_diff_sparse_80x24", "render_diff_dense_80x24"])
            .tags(vec!["shadow-run", "diff", "parity"]),
        );

        matrix.add_obligation(
            ValidationObligation::new(
                "render.negative.static-zero-diff",
                PerfLane::Render,
                ValidationLevel::Unit,
            )
            .assertion(AssertionCategory::NoChange)
            .description("Static screens must produce exactly zero diff output after initial frame")
            .artifacts(vec!["zero_diff_proof.json"])
            .diagnostics(vec!["unexpected_diff_cells", "dirty_row_report"])
            .fixtures(vec!["control_static_screen"])
            .tags(vec!["negative-control", "zero-diff"]),
        );

        // ====================================================================
        // RUNTIME LANE
        // ====================================================================

        matrix.add_obligation(
            ValidationObligation::new("runtime.unit.event-loop-overhead", PerfLane::Runtime, ValidationLevel::Unit)
                .assertion(AssertionCategory::NoRegression)
                .description("Event loop cycle overhead must remain below 1ms at standard viewport with sparse updates")
                .artifacts(vec!["cycle_timing.json"])
                .diagnostics(vec!["cycle_breakdown_trace", "hot_path_profile"])
                .fixtures(vec!["runtime_event_loop_steady"])
                .tags(vec!["event-loop", "overhead", "latency"]),
        );

        matrix.add_obligation(
            ValidationObligation::new(
                "runtime.unit.subscription-lifecycle",
                PerfLane::Runtime,
                ValidationLevel::Unit,
            )
            .assertion(AssertionCategory::NoRegression)
            .description(
                "Subscription start/stop must not leak handles, memory, or background tasks",
            )
            .artifacts(vec!["lifecycle_counters.json"])
            .diagnostics(vec!["leaked_handle_report", "active_subscription_dump"])
            .fixtures(vec!["runtime_subscription_churn"])
            .tags(vec!["subscriptions", "lifecycle", "leak-detection"]),
        );

        matrix.add_obligation(
            ValidationObligation::new(
                "runtime.integration.cancellation-prompt",
                PerfLane::Runtime,
                ValidationLevel::Integration,
            )
            .assertion(AssertionCategory::NoRegression)
            .description(
                "Command cancellation must complete within 5ms without accumulated side effects",
            )
            .artifacts(vec!["cancellation_latency.json"])
            .diagnostics(vec!["pending_effect_dump", "cancellation_trace"])
            .fixtures(vec!["runtime_cancellation_rapid"])
            .tags(vec!["cancellation", "effects", "latency"]),
        );

        matrix.add_obligation(
            ValidationObligation::new(
                "runtime.e2e.shutdown-determinism",
                PerfLane::Runtime,
                ValidationLevel::EndToEnd,
            )
            .assertion(AssertionCategory::NoChange)
            .description(
                "Shutdown sequence frame hashes must be identical across runs with same seed",
            )
            .artifacts(vec!["shutdown_checksums.json", "terminal_state_proof.json"])
            .diagnostics(vec!["shutdown_sequence_log", "terminal_restore_diff"])
            .fixtures(vec!["runtime_shutdown_determinism"])
            .tags(vec!["shutdown", "determinism", "terminal-state"]),
        );

        matrix.add_obligation(
            ValidationObligation::new("runtime.soak.input-backpressure", PerfLane::Runtime, ValidationLevel::Soak)
                .assertion(AssertionCategory::BoundedDegradation)
                .gating(false) // informative for now — bounds TBD
                .description("Input floods must not cause unbounded queue growth; batching may increase frame time within 2x steady-state")
                .artifacts(vec!["input_queue_depth.jsonl", "frame_latency_under_load.json"])
                .diagnostics(vec!["queue_growth_trace", "dropped_event_report"])
                .fixtures(vec!["challenge_input_flood"])
                .tags(vec!["input", "backpressure", "soak", "bounded-degradation"]),
        );

        matrix.add_obligation(
            ValidationObligation::new(
                "runtime.e2e.mode-contract",
                PerfLane::Runtime,
                ValidationLevel::EndToEnd,
            )
            .assertion(AssertionCategory::BoundedDegradation)
            .description(
                "Healthy, stressed, degraded, and recovered modes must preserve strict behaviors while emitting explicit mode and recovery signals",
            )
            .artifacts(vec![
                "runtime_mode_trace.jsonl",
                "degradation_contract_report.json",
                "recovery_summary.json",
            ])
            .diagnostics(vec![
                "mode_transition_diff",
                "strict_guarantee_violation_report",
                "signal_gap_report",
            ])
            .fixtures(vec!["challenge_input_flood", "challenge_mixed_workload"])
            .tags(vec!["runtime-mode", "degradation", "recovery", "ux-contract"]),
        );

        matrix.add_obligation(
            ValidationObligation::new(
                "runtime.replay.mode-transition-determinism",
                PerfLane::Runtime,
                ValidationLevel::Replay,
            )
            .assertion(AssertionCategory::NoChange)
            .description(
                "Fixed pressure schedules must reproduce the same mode transitions, fallback reasons, and recovery completion markers",
            )
            .artifacts(vec!["runtime_mode_trace.jsonl", "mode_transition_checksum.json"])
            .diagnostics(vec!["mode_transition_diff", "recovery_reason_drift"])
            .fixtures(vec!["challenge_input_flood"])
            .tags(vec!["runtime-mode", "replay", "determinism", "recovery"]),
        );

        matrix.add_obligation(
            ValidationObligation::new(
                "runtime.negative.idle-overhead",
                PerfLane::Runtime,
                ValidationLevel::Unit,
            )
            .assertion(AssertionCategory::NoChange)
            .description("Idle runtime with no events must consume near-zero CPU")
            .artifacts(vec!["idle_cpu_measurement.json"])
            .diagnostics(vec!["idle_activity_trace", "spurious_wakeup_report"])
            .fixtures(vec!["control_idle_runtime"])
            .tags(vec!["negative-control", "idle", "overhead"]),
        );

        // ====================================================================
        // DOCTOR LANE
        // ====================================================================

        matrix.add_obligation(
            ValidationObligation::new("doctor.integration.workflow-completion", PerfLane::Doctor, ValidationLevel::Integration)
                .assertion(AssertionCategory::NoRegression)
                .description("Doctor capture-suite-report workflow must complete within timeout and produce valid artifact manifest")
                .artifacts(vec!["summary.json", "artifact_manifest.json"])
                .diagnostics(vec!["command_manifest.txt", "stderr_logs", "stage_timing_breakdown"])
                .fixtures(vec!["doctor_capture_workflow"])
                .tags(vec!["doctor", "workflow", "artifacts"]),
        );

        matrix.add_obligation(
            ValidationObligation::new("doctor.integration.seed-lifecycle", PerfLane::Doctor, ValidationLevel::Integration)
                .assertion(AssertionCategory::NoRegression)
                .description("Seed orchestration must emit structured lifecycle events and respect explicit deadlines")
                .artifacts(vec!["seed_log.jsonl", "stage_lifecycle.json"])
                .diagnostics(vec!["retry_exhaustion_report", "deadline_violation_log"])
                .fixtures(vec!["doctor_seed_orchestration"])
                .tags(vec!["doctor", "seed", "lifecycle"]),
        );

        matrix.add_obligation(
            ValidationObligation::new(
                "doctor.integration.runtime-mode-reporting",
                PerfLane::Doctor,
                ValidationLevel::Integration,
            )
            .assertion(AssertionCategory::GracefulFallback)
            .description(
                "Doctor summaries and manifests must surface runtime degraded/fallback intervals so operators can classify runs without raw log inspection",
            )
            .artifacts(vec![
                "summary.json",
                "artifact_manifest.json",
                "runtime_mode_report.json",
            ])
            .diagnostics(vec![
                "runtime_mode_signal_gap",
                "summary_signal_diff",
                "artifact_linkage_report",
            ])
            .fixtures(vec![
                "doctor_capture_workflow",
                "challenge_doctor_degraded_network",
            ])
            .tags(vec!["doctor", "runtime-mode", "artifacts", "graceful-fallback"]),
        );

        matrix.add_obligation(
            ValidationObligation::new("doctor.e2e.failure-diagnostics", PerfLane::Doctor, ValidationLevel::EndToEnd)
                .assertion(AssertionCategory::FailureForensics)
                .description("Doctor failure paths must produce actionable failure signatures and remediation hints")
                .artifacts(vec!["failure_signatures.json", "case_results.json", "replay_triage_report.json"])
                .diagnostics(vec!["failure_signature_dump", "missing_artifact_report"])
                .fixtures(vec!["challenge_doctor_degraded_network"])
                .tags(vec!["doctor", "failure", "forensics"]),
        );

        matrix.add_obligation(
            ValidationObligation::new(
                "doctor.soak.determinism-stability",
                PerfLane::Doctor,
                ValidationLevel::Soak,
            )
            .assertion(AssertionCategory::NoChange)
            .description(
                "Determinism soak must show zero non-volatile divergence across N repeated runs",
            )
            .artifacts(vec!["determinism_report.json"])
            .diagnostics(vec!["divergence_point_report", "volatile_field_list"])
            .fixtures(vec!["doctor_capture_workflow"])
            .tags(vec!["doctor", "determinism", "soak"]),
        );

        // ====================================================================
        // CROSS-LANE
        // ====================================================================

        matrix.add_obligation(
            ValidationObligation::new("cross.shadow.migration-parity", PerfLane::CrossLane, ValidationLevel::ShadowComparison)
                .assertion(AssertionCategory::NoChange)
                .description("Shadow-run comparison must prove semantic parity between old and new implementations")
                .artifacts(vec!["shadow_run_result.json", "evidence_bundle.json"])
                .diagnostics(vec!["parity_violation_diff", "evidence_gap_report"])
                .tags(vec!["shadow-run", "parity", "migration"]),
        );

        matrix.add_obligation(
            ValidationObligation::new("cross.e2e.mixed-workload-stability", PerfLane::CrossLane, ValidationLevel::EndToEnd)
                .assertion(AssertionCategory::BoundedDegradation)
                .description("Mixed workload must not cause subsystem starvation; frame p95 must remain below 2x steady-state")
                .artifacts(vec!["mixed_workload_report.json"])
                .diagnostics(vec!["subsystem_timing_breakdown", "starvation_evidence"])
                .fixtures(vec!["challenge_mixed_workload"])
                .tags(vec!["mixed", "concurrency", "bounded-degradation"]),
        );

        matrix.add_obligation(
            ValidationObligation::new("cross.property.invariant-preservation", PerfLane::CrossLane, ValidationLevel::Property)
                .assertion(AssertionCategory::NoRegression)
                .gating(false) // informative until property tests reach coverage targets
                .description("Property tests must verify that optimization changes preserve documented invariants")
                .artifacts(vec!["proptest_results.json"])
                .diagnostics(vec!["counterexample_report", "shrunk_input_dump"])
                .tags(vec!["property", "invariants", "proptest"]),
        );

        // ====================================================================
        // LOGGING CONTRACTS
        // ====================================================================

        matrix.set_logging_contract(Self::build_common_logging_contract(PerfLane::Render));
        matrix.set_logging_contract(Self::build_common_logging_contract(PerfLane::Runtime));
        matrix.set_logging_contract(Self::build_common_logging_contract(PerfLane::Doctor));
        matrix.set_logging_contract(Self::build_common_logging_contract(PerfLane::CrossLane));

        matrix
    }

    /// Build a logging contract with common fields plus lane-specific events.
    fn build_common_logging_contract(lane: PerfLane) -> LoggingContract {
        let mut contract = LoggingContract::new(lane)
            .field(LogField::new(
                "run_id",
                "string",
                true,
                "Unique identifier for this benchmark/test run",
            ))
            .field(LogField::new(
                "event",
                "string",
                true,
                "Stable event name from the lane's event vocabulary",
            ))
            .field(LogField::new(
                "event_idx",
                "u64",
                true,
                "Monotonic event index within this run",
            ))
            .field(LogField::new(
                "timestamp_us",
                "u64",
                true,
                "Microsecond timestamp (deterministic or wall-clock)",
            ))
            .field(LogField::new(
                "fixture_id",
                "string",
                true,
                "Fixture identifier from the fixture suite",
            ))
            .field(LogField::new(
                "seed",
                "u64",
                true,
                "RNG seed used for this run",
            ))
            .field(LogField::new(
                "stage",
                "string",
                false,
                "Pipeline stage or phase name",
            ))
            .field(LogField::new(
                "latency_us",
                "u64",
                false,
                "Operation latency in microseconds",
            ))
            .field(LogField::new(
                "reason",
                "string",
                false,
                "Reason code for decisions, failures, or fallbacks",
            ))
            .field(LogField::new(
                "mismatch_category",
                "string",
                false,
                "Category of detected mismatch (semantic, observability, benchmark-overfit, expected-fallback)",
            ))
            .field(LogField::new(
                "replay_pointer",
                "string",
                false,
                "Path to replay asset for reproducing this event",
            ));

        // Lane-specific required events
        match lane {
            PerfLane::Render => {
                contract = contract
                    .event("diff_decision")
                    .event("frame_budget_check")
                    .event("presenter_emit")
                    .event("strategy_switch")
                    .reason_code("budget_exceeded")
                    .reason_code("strategy_fallback")
                    .reason_code("dirty_row_threshold");
            }
            PerfLane::Runtime => {
                contract = contract
                    .field(LogField::new(
                        "runtime_mode",
                        "string",
                        false,
                        "Current user-visible runtime mode (healthy, stressed, degraded, recovered)",
                    ))
                    .field(LogField::new(
                        "mode_before",
                        "string",
                        false,
                        "Previous runtime mode when recording a transition",
                    ))
                    .field(LogField::new(
                        "mode_after",
                        "string",
                        false,
                        "Next runtime mode when recording a transition",
                    ))
                    .field(LogField::new(
                        "pressure_class",
                        "string",
                        false,
                        "Pressure class driving the transition or fallback decision",
                    ))
                    .field(LogField::new(
                        "recovery_latency_us",
                        "u64",
                        false,
                        "Time spent recovering from degraded mode before healthy service resumed",
                    ))
                    .field(LogField::new(
                        "strict_guarantees",
                        "string",
                        false,
                        "Machine-readable list of guarantees preserved while degraded",
                    ))
                    .field(LogField::new(
                        "work_disposition",
                        "string",
                        false,
                        "How pending work was preserved, deferred, coalesced, or dropped",
                    ))
                    .event("cycle_start")
                    .event("cycle_complete")
                    .event("subscription_lifecycle")
                    .event("effect_dispatch")
                    .event("mode_transition")
                    .event("fallback_activation")
                    .event("recovery_complete")
                    .event("shutdown_sequence")
                    .reason_code("cancellation_requested")
                    .reason_code("timeout_exceeded")
                    .reason_code("subscription_panic")
                    .reason_code("input_backpressure")
                    .reason_code("mixed_workload_pressure")
                    .reason_code("recovery_hysteresis")
                    .reason_code("strict_guarantee_violation");
            }
            PerfLane::Doctor => {
                contract = contract
                    .field(LogField::new(
                        "runtime_mode",
                        "string",
                        false,
                        "Most severe runtime mode observed during the recorded workflow",
                    ))
                    .field(LogField::new(
                        "recovery_outcome",
                        "string",
                        false,
                        "How the runtime returned to healthy service, or why it did not",
                    ))
                    .event("stage_started")
                    .event("stage_completed")
                    .event("stage_failed")
                    .event("rpc_retry_scheduled")
                    .event("rpc_retry_exhausted")
                    .event("runtime_mode_summary")
                    .event("seed_complete")
                    .reason_code("handshake_failure")
                    .reason_code("retry_exhausted")
                    .reason_code("deadline_exceeded")
                    .reason_code("artifact_missing")
                    .reason_code("runtime_mode_signal_missing");
            }
            PerfLane::CrossLane => {
                contract = contract
                    .event("shadow_comparison_start")
                    .event("shadow_comparison_result")
                    .event("parity_violation")
                    .reason_code("semantic_mismatch")
                    .reason_code("observability_gap")
                    .reason_code("benchmark_overfit");
            }
        }

        contract
    }
}

impl Default for ValidationMatrix {
    fn default() -> Self {
        Self::new()
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonical_matrix_is_populated() {
        let matrix = ValidationMatrix::canonical();
        assert!(
            matrix.obligation_count() >= 16,
            "expected at least 16 obligations, got {}",
            matrix.obligation_count()
        );
    }

    #[test]
    fn every_lane_has_obligations() {
        let matrix = ValidationMatrix::canonical();
        for lane in PerfLane::ALL {
            assert!(
                !matrix.obligations_for(*lane).is_empty(),
                "lane {} has no obligations",
                lane.label()
            );
        }
    }

    #[test]
    fn every_obligation_has_description() {
        let matrix = ValidationMatrix::canonical();
        for ob in matrix.all_obligations() {
            assert!(
                !ob.description.is_empty(),
                "obligation {} missing description",
                ob.id
            );
        }
    }

    #[test]
    fn every_obligation_has_artifacts() {
        let matrix = ValidationMatrix::canonical();
        for ob in matrix.all_obligations() {
            assert!(
                !ob.expected_artifacts.is_empty(),
                "obligation {} missing expected artifacts",
                ob.id
            );
        }
    }

    #[test]
    fn every_obligation_has_diagnostics() {
        let matrix = ValidationMatrix::canonical();
        for ob in matrix.all_obligations() {
            assert!(
                !ob.failure_diagnostics.is_empty(),
                "obligation {} missing failure diagnostics",
                ob.id
            );
        }
    }

    #[test]
    fn obligation_ids_are_unique() {
        let matrix = ValidationMatrix::canonical();
        let mut seen = std::collections::HashSet::new();
        for ob in matrix.all_obligations() {
            assert!(seen.insert(&ob.id), "duplicate obligation id: {}", ob.id);
        }
    }

    #[test]
    fn has_gating_and_informative_obligations() {
        let matrix = ValidationMatrix::canonical();
        let gating = matrix.gating_obligations();
        let total = matrix.obligation_count();
        assert!(!gating.is_empty(), "no gating obligations");
        assert!(
            gating.len() < total,
            "all obligations are gating — expected some informative"
        );
    }

    #[test]
    fn has_negative_assertions() {
        let matrix = ValidationMatrix::canonical();
        let negative = matrix.negative_assertions();
        assert!(
            negative.len() >= 4,
            "expected at least 4 negative assertions, got {}",
            negative.len()
        );
    }

    #[test]
    fn has_failure_forensics_obligation() {
        let matrix = ValidationMatrix::canonical();
        let forensics: Vec<_> = matrix
            .all_obligations()
            .iter()
            .filter(|o| o.assertion == AssertionCategory::FailureForensics)
            .collect();
        assert!(!forensics.is_empty(), "no failure forensics obligations");
    }

    #[test]
    fn every_lane_has_logging_contract() {
        let matrix = ValidationMatrix::canonical();
        for lane in PerfLane::ALL {
            let contract = matrix.logging_contract_for(*lane);
            assert!(
                contract.is_some(),
                "lane {} has no logging contract",
                lane.label()
            );
            let contract = contract.unwrap();
            assert!(
                !contract.fields.is_empty(),
                "lane {} logging contract has no fields",
                lane.label()
            );
            assert!(
                !contract.required_events.is_empty(),
                "lane {} logging contract has no required events",
                lane.label()
            );
            assert!(
                !contract.reason_codes.is_empty(),
                "lane {} logging contract has no reason codes",
                lane.label()
            );
        }
    }

    #[test]
    fn logging_contracts_have_common_fields() {
        let matrix = ValidationMatrix::canonical();
        for lane in PerfLane::ALL {
            let contract = matrix.logging_contract_for(*lane).unwrap();
            let field_names: Vec<&str> = contract.fields.iter().map(|f| f.name.as_str()).collect();
            for required in [
                "run_id",
                "event",
                "event_idx",
                "timestamp_us",
                "fixture_id",
                "seed",
            ] {
                assert!(
                    field_names.contains(&required),
                    "lane {} missing common field: {}",
                    lane.label(),
                    required
                );
            }
        }
    }

    #[test]
    fn perf_lane_from_fixture_family() {
        assert_eq!(
            PerfLane::from_fixture_family(FixtureFamily::Render),
            PerfLane::Render
        );
        assert_eq!(
            PerfLane::from_fixture_family(FixtureFamily::Runtime),
            PerfLane::Runtime
        );
        assert_eq!(
            PerfLane::from_fixture_family(FixtureFamily::Doctor),
            PerfLane::Doctor
        );
        assert_eq!(
            PerfLane::from_fixture_family(FixtureFamily::Challenge),
            PerfLane::CrossLane
        );
    }

    #[test]
    fn validation_level_labels_and_time_classes() {
        for level in ValidationLevel::ALL {
            assert!(!level.label().is_empty());
            assert!(!level.time_class().label().is_empty());
        }
    }

    #[test]
    fn assertion_category_negative_classification() {
        assert!(!AssertionCategory::Improvement.is_negative());
        assert!(AssertionCategory::NoRegression.is_negative());
        assert!(AssertionCategory::NoChange.is_negative());
        assert!(AssertionCategory::BoundedDegradation.is_negative());
        assert!(!AssertionCategory::GracefulFallback.is_negative());
        assert!(!AssertionCategory::FailureForensics.is_negative());
    }

    #[test]
    fn obligation_to_json_valid() {
        let ob =
            ValidationObligation::new("test.unit.foo", PerfLane::Render, ValidationLevel::Unit)
                .description("test description")
                .artifacts(vec!["result.json"])
                .diagnostics(vec!["error_log"])
                .tags(vec!["test"]);
        let json = ob.to_json();
        assert!(json.contains("\"id\": \"test.unit.foo\""));
        assert!(json.contains("\"lane\": \"render\""));
        assert!(json.contains("\"level\": \"unit\""));
        assert!(json.contains("\"gating\": true"));
    }

    #[test]
    fn matrix_to_json_has_counts() {
        let matrix = ValidationMatrix::canonical();
        let json = matrix.to_json();
        assert!(json.contains("\"schema_version\": 1"));
        assert!(json.contains("\"total_obligations\":"));
        assert!(json.contains("\"gating_count\":"));
        assert!(json.contains("\"informative_count\":"));
        assert!(json.contains("\"negative_assertion_count\":"));
    }

    #[test]
    fn obligations_at_level_filters_correctly() {
        let matrix = ValidationMatrix::canonical();
        let unit = matrix.obligations_at_level(ValidationLevel::Unit);
        assert!(!unit.is_empty());
        for ob in &unit {
            assert_eq!(ob.level, ValidationLevel::Unit);
        }
    }

    #[test]
    fn empty_matrix() {
        let matrix = ValidationMatrix::new();
        assert_eq!(matrix.obligation_count(), 0);
        assert!(matrix.all_obligations().is_empty());
        assert!(matrix.gating_obligations().is_empty());
    }
}
