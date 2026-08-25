#![forbid(unsafe_code)]

//! Deterministic macrobenchmark and replay fixture suites (bd-muv6p).
//!
//! Defines canonical fixture specifications for render, runtime, and doctor
//! lanes, partitioned into regression suites and adversarial challenge suites.
//! Each fixture carries rich metadata: reproducibility rules, artifact naming,
//! provenance, and rationale.
//!
//! # Design principles
//!
//! 1. **Representative workloads**: Fixtures exercise realistic screen sizes,
//!    state transitions, and interaction patterns — not synthetic micro-ops.
//! 2. **Dual partition**: Canonical regression fixtures prove stability;
//!    adversarial challenge fixtures prevent overfitting to narrow benchmarks.
//! 3. **Traceable**: Every fixture links to an optimization question and
//!    documents which user/operator pain it approximates.
//! 4. **Reproducible**: Seeds, timing controls, host fingerprints, and
//!    fixture versioning ensure cross-run and cross-machine comparability.
//!
//! # Usage
//!
//! ```ignore
//! use ftui_harness::fixture_suite::*;
//!
//! let registry = FixtureRegistry::canonical();
//! for spec in registry.by_family(FixtureFamily::Render) {
//!     let rules = spec.reproducibility();
//!     let name = spec.artifact_path("baseline", "json");
//!     println!("{}: {} (seed={})", spec.id, spec.name, rules.seed);
//! }
//! ```

use std::collections::BTreeMap;

use crate::baseline_capture::FixtureFamily;

// ============================================================================
// Suite Partition
// ============================================================================

/// Whether a fixture is part of the canonical regression suite or the
/// adversarial challenge suite.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SuitePartition {
    /// Canonical regression: deterministic, stable workloads used for
    /// go/no-go regression gating. Optimizations must not regress these.
    Canonical,
    /// Adversarial challenge: stress tests, edge cases, and pathological
    /// inputs that guard against overfitting to the canonical set.
    Challenge,
    /// Negative control: workloads expected to show no change, used to
    /// verify that optimizations don't have unintended side effects.
    NegativeControl,
}

impl SuitePartition {
    /// Human-readable label.
    #[must_use]
    pub const fn label(&self) -> &'static str {
        match self {
            Self::Canonical => "canonical",
            Self::Challenge => "challenge",
            Self::NegativeControl => "negative-control",
        }
    }
}

// ============================================================================
// Viewport Specification
// ============================================================================

/// A viewport size for fixture execution.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ViewportSpec {
    pub width: u16,
    pub height: u16,
}

impl ViewportSpec {
    /// Standard terminal: 80x24.
    pub const STANDARD: Self = Self {
        width: 80,
        height: 24,
    };
    /// Medium terminal: 120x40.
    pub const MEDIUM: Self = Self {
        width: 120,
        height: 40,
    };
    /// Large terminal: 200x60.
    pub const LARGE: Self = Self {
        width: 200,
        height: 60,
    };
    /// Tiny terminal: 40x10 (stress test for cramped layouts).
    pub const TINY: Self = Self {
        width: 40,
        height: 10,
    };
    /// Ultra-wide: 320x24 (common in tiled WM setups).
    pub const ULTRAWIDE: Self = Self {
        width: 320,
        height: 24,
    };

    #[must_use]
    pub const fn new(width: u16, height: u16) -> Self {
        Self { width, height }
    }

    /// Total cell count.
    #[must_use]
    pub const fn cell_count(&self) -> u32 {
        self.width as u32 * self.height as u32
    }

    /// Label for artifact naming (e.g., "80x24").
    #[must_use]
    pub fn label(&self) -> String {
        format!("{}x{}", self.width, self.height)
    }
}

// ============================================================================
// State Transition Pattern
// ============================================================================

/// Categories of state transitions a fixture exercises.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TransitionPattern {
    /// < 5% of cells change between frames.
    SparseUpdate,
    /// 5-25% of cells change between frames.
    ModerateUpdate,
    /// > 50% of cells change (full repaint territory).
    LargeInvalidation,
    /// Viewport dimensions change mid-run.
    ResizeChurn,
    /// Subscriptions start/stop rapidly.
    SubscriptionChurn,
    /// Commands cancelled before completion.
    Cancellation,
    /// External I/O with artificial latency/failure.
    DegradedIo,
    /// Timeout-driven abort paths.
    Timeout,
    /// Heavy artifact/evidence file writes.
    ArtifactHeavy,
    /// Input events arrive faster than frame budget.
    InputStorm,
    /// Mixed workload combining several patterns.
    Mixed,
}

impl TransitionPattern {
    #[must_use]
    pub const fn label(&self) -> &'static str {
        match self {
            Self::SparseUpdate => "sparse-update",
            Self::ModerateUpdate => "moderate-update",
            Self::LargeInvalidation => "large-invalidation",
            Self::ResizeChurn => "resize-churn",
            Self::SubscriptionChurn => "subscription-churn",
            Self::Cancellation => "cancellation",
            Self::DegradedIo => "degraded-io",
            Self::Timeout => "timeout",
            Self::ArtifactHeavy => "artifact-heavy",
            Self::InputStorm => "input-storm",
            Self::Mixed => "mixed",
        }
    }
}

// ============================================================================
// Reproducibility Rules
// ============================================================================

/// Reproducibility configuration for a fixture.
#[derive(Debug, Clone)]
pub struct ReproducibilityRules {
    /// Deterministic seed for RNG.
    pub seed: u64,
    /// Fixed time step in milliseconds (0 = wall-clock).
    pub time_step_ms: u64,
    /// Whether to use deterministic timestamps (T000000 format).
    pub deterministic_time: bool,
    /// Fixture schema version (bumped when fixture semantics change).
    pub fixture_version: u32,
    /// Whether host fingerprint must match for baseline comparison.
    pub require_host_match: bool,
    /// Minimum sample count for statistical validity.
    pub min_samples: u32,
    /// Maximum wall-clock seconds allowed per fixture run.
    pub timeout_secs: u32,
}

impl ReproducibilityRules {
    /// Default rules: deterministic, seed 42, 16ms steps, 30 samples.
    #[must_use]
    pub const fn default_deterministic() -> Self {
        Self {
            seed: 42,
            time_step_ms: 16,
            deterministic_time: true,
            fixture_version: 1,
            require_host_match: false,
            min_samples: 30,
            timeout_secs: 60,
        }
    }

    /// Rules for challenge fixtures: wall-clock, higher timeout.
    #[must_use]
    pub const fn challenge() -> Self {
        Self {
            seed: 42,
            time_step_ms: 0, // wall-clock for realistic stress
            deterministic_time: false,
            fixture_version: 1,
            require_host_match: true, // challenges are host-sensitive
            min_samples: 10,
            timeout_secs: 120,
        }
    }
}

// ============================================================================
// Fixture Specification
// ============================================================================

/// A complete macrobenchmark fixture specification.
#[derive(Debug, Clone)]
pub struct FixtureSpec {
    /// Unique fixture identifier (e.g., "render_diff_sparse_80x24").
    pub id: String,
    /// Human-readable name.
    pub name: String,
    /// Which performance lane this fixture measures.
    pub family: FixtureFamily,
    /// Canonical regression vs adversarial challenge.
    pub partition: SuitePartition,
    /// Primary viewport for this fixture.
    pub viewport: ViewportSpec,
    /// Additional viewports to test (for multi-size coverage).
    pub extra_viewports: Vec<ViewportSpec>,
    /// State transition patterns exercised.
    pub transitions: Vec<TransitionPattern>,
    /// Reproducibility configuration.
    pub rules: ReproducibilityRules,
    /// Number of frames to run.
    pub frame_count: u32,
    /// Why this fixture exists — which user/operator pain it approximates.
    pub rationale: String,
    /// Which optimization question or invariant this fixture tests.
    pub tests_hypothesis: String,
    /// Whether results are expected to show improvement (true) or stability
    /// (false, for negative controls).
    pub expects_improvement: bool,
    /// Tags for filtering and grouping.
    pub tags: Vec<String>,
}

impl FixtureSpec {
    /// Builder constructor.
    #[must_use]
    pub fn new(id: &str, name: &str, family: FixtureFamily) -> Self {
        Self {
            id: id.to_string(),
            name: name.to_string(),
            family,
            partition: SuitePartition::Canonical,
            viewport: ViewportSpec::STANDARD,
            extra_viewports: Vec::new(),
            transitions: Vec::new(),
            rules: ReproducibilityRules::default_deterministic(),
            frame_count: 100,
            rationale: String::new(),
            tests_hypothesis: String::new(),
            expects_improvement: true,
            tags: Vec::new(),
        }
    }

    #[must_use]
    pub fn partition(mut self, p: SuitePartition) -> Self {
        self.partition = p;
        if p == SuitePartition::Challenge {
            self.rules = ReproducibilityRules::challenge();
        }
        self
    }

    #[must_use]
    pub fn viewport(mut self, v: ViewportSpec) -> Self {
        self.viewport = v;
        self
    }

    #[must_use]
    pub fn extra_viewports(mut self, vs: Vec<ViewportSpec>) -> Self {
        self.extra_viewports = vs;
        self
    }

    #[must_use]
    pub fn transitions(mut self, ts: Vec<TransitionPattern>) -> Self {
        self.transitions = ts;
        self
    }

    #[must_use]
    pub fn rules(mut self, r: ReproducibilityRules) -> Self {
        self.rules = r;
        self
    }

    #[must_use]
    pub fn frame_count(mut self, n: u32) -> Self {
        self.frame_count = n;
        self
    }

    #[must_use]
    pub fn rationale(mut self, r: &str) -> Self {
        self.rationale = r.to_string();
        self
    }

    #[must_use]
    pub fn tests_hypothesis(mut self, h: &str) -> Self {
        self.tests_hypothesis = h.to_string();
        self
    }

    #[must_use]
    pub fn expects_improvement(mut self, b: bool) -> Self {
        self.expects_improvement = b;
        self
    }

    #[must_use]
    pub fn tags(mut self, tags: Vec<&str>) -> Self {
        self.tags = tags.into_iter().map(String::from).collect();
        self
    }

    /// Get the reproducibility rules for this fixture.
    #[must_use]
    pub fn reproducibility(&self) -> &ReproducibilityRules {
        &self.rules
    }

    /// Generate the artifact path for a given artifact type and extension.
    ///
    /// Convention: `{family}/{partition}/{id}/{artifact_type}.{ext}`
    ///
    /// Examples:
    /// - `render/canonical/render_diff_sparse_80x24/baseline.json`
    /// - `challenge/challenge/resize_storm_rapid/profile.jsonl`
    #[must_use]
    pub fn artifact_path(&self, artifact_type: &str, ext: &str) -> String {
        format!(
            "{}/{}/{}/{}.{}",
            self.family.label(),
            self.partition.label(),
            self.id,
            artifact_type,
            ext,
        )
    }

    /// Generate the replay asset path.
    #[must_use]
    pub fn replay_path(&self) -> String {
        self.artifact_path("replay", "jsonl")
    }

    /// Generate the baseline path.
    #[must_use]
    pub fn baseline_path(&self) -> String {
        self.artifact_path("baseline", "json")
    }

    /// Serialize metadata to JSON for the fixture manifest.
    #[must_use]
    pub fn to_json(&self) -> String {
        let transitions: Vec<String> = self
            .transitions
            .iter()
            .map(|t| format!("\"{}\"", t.label()))
            .collect();
        let extra_vps: Vec<String> = self
            .extra_viewports
            .iter()
            .map(|v| format!("\"{}\"", v.label()))
            .collect();
        let tags: Vec<String> = self.tags.iter().map(|t| format!("\"{t}\"")).collect();

        format!(
            r#"{{
  "id": "{}",
  "name": "{}",
  "family": "{}",
  "partition": "{}",
  "viewport": "{}",
  "extra_viewports": [{}],
  "transitions": [{}],
  "frame_count": {},
  "seed": {},
  "time_step_ms": {},
  "deterministic_time": {},
  "fixture_version": {},
  "expects_improvement": {},
  "rationale": "{}",
  "tests_hypothesis": "{}",
  "tags": [{}],
  "baseline_path": "{}",
  "replay_path": "{}"
}}"#,
            self.id,
            self.name,
            self.family.label(),
            self.partition.label(),
            self.viewport.label(),
            extra_vps.join(", "),
            transitions.join(", "),
            self.frame_count,
            self.rules.seed,
            self.rules.time_step_ms,
            self.rules.deterministic_time,
            self.rules.fixture_version,
            self.expects_improvement,
            self.rationale.replace('"', "\\\""),
            self.tests_hypothesis.replace('"', "\\\""),
            tags.join(", "),
            self.baseline_path(),
            self.replay_path(),
        )
    }
}

// ============================================================================
// Fixture Registry
// ============================================================================

/// Registry of all macrobenchmark fixture specifications.
#[derive(Debug, Clone)]
pub struct FixtureRegistry {
    fixtures: BTreeMap<String, FixtureSpec>,
}

impl FixtureRegistry {
    /// Create an empty registry.
    #[must_use]
    pub fn new() -> Self {
        Self {
            fixtures: BTreeMap::new(),
        }
    }

    /// Register a fixture specification.
    pub fn register(&mut self, spec: FixtureSpec) {
        self.fixtures.insert(spec.id.clone(), spec);
    }

    /// Look up a fixture by ID.
    #[must_use]
    pub fn get(&self, id: &str) -> Option<&FixtureSpec> {
        self.fixtures.get(id)
    }

    /// All fixtures in the registry.
    #[must_use]
    pub fn all(&self) -> Vec<&FixtureSpec> {
        self.fixtures.values().collect()
    }

    /// Fixtures filtered by family.
    #[must_use]
    pub fn by_family(&self, family: FixtureFamily) -> Vec<&FixtureSpec> {
        self.fixtures
            .values()
            .filter(|f| f.family == family)
            .collect()
    }

    /// Fixtures filtered by suite partition.
    #[must_use]
    pub fn by_partition(&self, partition: SuitePartition) -> Vec<&FixtureSpec> {
        self.fixtures
            .values()
            .filter(|f| f.partition == partition)
            .collect()
    }

    /// Fixtures filtered by tag.
    #[must_use]
    pub fn by_tag(&self, tag: &str) -> Vec<&FixtureSpec> {
        self.fixtures
            .values()
            .filter(|f| f.tags.iter().any(|t| t == tag))
            .collect()
    }

    /// Fixtures filtered by transition pattern.
    #[must_use]
    pub fn by_transition(&self, pattern: TransitionPattern) -> Vec<&FixtureSpec> {
        self.fixtures
            .values()
            .filter(|f| f.transitions.contains(&pattern))
            .collect()
    }

    /// Total fixture count.
    #[must_use]
    pub fn len(&self) -> usize {
        self.fixtures.len()
    }

    /// Whether the registry is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.fixtures.is_empty()
    }

    /// Serialize the full manifest to JSON.
    #[must_use]
    pub fn manifest_json(&self) -> String {
        let canonical_count = self.by_partition(SuitePartition::Canonical).len();
        let challenge_count = self.by_partition(SuitePartition::Challenge).len();
        let negative_count = self.by_partition(SuitePartition::NegativeControl).len();

        let entries: Vec<String> = self.fixtures.values().map(|f| f.to_json()).collect();

        format!(
            r#"{{
  "schema_version": 1,
  "total_fixtures": {},
  "canonical_count": {},
  "challenge_count": {},
  "negative_control_count": {},
  "fixtures": [
{}
  ]
}}"#,
            self.fixtures.len(),
            canonical_count,
            challenge_count,
            negative_count,
            entries
                .iter()
                .map(|e| format!("    {e}"))
                .collect::<Vec<_>>()
                .join(",\n"),
        )
    }

    /// Build the canonical fixture registry with all built-in fixtures.
    ///
    /// This is the single source of truth for macrobenchmark workloads.
    #[must_use]
    pub fn canonical() -> Self {
        let mut reg = Self::new();

        // ====================================================================
        // RENDER LANE — Canonical
        // ====================================================================

        reg.register(
            FixtureSpec::new(
                "render_diff_sparse_80x24",
                "Sparse diff at standard viewport",
                FixtureFamily::Render,
            )
            .viewport(ViewportSpec::STANDARD)
            .extra_viewports(vec![ViewportSpec::MEDIUM, ViewportSpec::LARGE])
            .transitions(vec![TransitionPattern::SparseUpdate])
            .frame_count(200)
            .rationale(
                "Most real TUI frames change < 5% of cells. This tests the \
                 diff engine's fast path where dirty-row tracking should dominate.",
            )
            .tests_hypothesis("Dirty-row diff strategy should beat full-scan for sparse updates.")
            .tags(vec!["diff", "fast-path", "regression"]),
        );

        reg.register(
            FixtureSpec::new(
                "render_diff_dense_80x24",
                "Dense diff at standard viewport",
                FixtureFamily::Render,
            )
            .viewport(ViewportSpec::STANDARD)
            .extra_viewports(vec![ViewportSpec::LARGE])
            .transitions(vec![TransitionPattern::LargeInvalidation])
            .frame_count(100)
            .rationale(
                "Dashboard refreshes and visual effect screens repaint > 50% \
                 of cells per frame. This exercises the full-scan diff path.",
            )
            .tests_hypothesis(
                "Full-scan diff should complete within frame budget even at \
                 high change rates.",
            )
            .tags(vec!["diff", "full-scan", "regression"]),
        );

        reg.register(
            FixtureSpec::new(
                "render_presenter_emit_120x40",
                "Presenter ANSI emission at medium viewport",
                FixtureFamily::Render,
            )
            .viewport(ViewportSpec::MEDIUM)
            .transitions(vec![TransitionPattern::ModerateUpdate])
            .frame_count(150)
            .rationale(
                "Presenter state tracking should avoid redundant SGR sequences. \
                 This measures ANSI bytes emitted per frame at moderate churn.",
            )
            .tests_hypothesis("State-tracked presenter should emit < 40% of naive ANSI output.")
            .tags(vec!["presenter", "ansi", "output-cost", "regression"]),
        );

        reg.register(
            FixtureSpec::new(
                "render_pipeline_full_200x60",
                "Full render pipeline at large viewport",
                FixtureFamily::Render,
            )
            .viewport(ViewportSpec::LARGE)
            .transitions(vec![TransitionPattern::ModerateUpdate])
            .frame_count(100)
            .rationale(
                "End-to-end pipeline: buffer creation, widget render, diff, \
                 present. Tests whether large viewports stay within 16ms budget.",
            )
            .tests_hypothesis("Full pipeline p99 should remain under 16ms at 200x60.")
            .tags(vec!["pipeline", "budget", "e2e", "regression"]),
        );

        // ====================================================================
        // RUNTIME LANE — Canonical
        // ====================================================================

        reg.register(
            FixtureSpec::new(
                "runtime_event_loop_steady",
                "Steady-state event loop throughput",
                FixtureFamily::Runtime,
            )
            .viewport(ViewportSpec::STANDARD)
            .transitions(vec![TransitionPattern::SparseUpdate])
            .frame_count(500)
            .rationale(
                "The core event loop processes events, runs update(), calls \
                 view(), and presents. This measures steady-state overhead.",
            )
            .tests_hypothesis(
                "Event loop overhead should be < 1ms per cycle at 80x24 with \
                 sparse updates.",
            )
            .tags(vec!["event-loop", "throughput", "regression"]),
        );

        reg.register(
            FixtureSpec::new(
                "runtime_subscription_churn",
                "Subscription start/stop cycles",
                FixtureFamily::Runtime,
            )
            .viewport(ViewportSpec::STANDARD)
            .transitions(vec![TransitionPattern::SubscriptionChurn])
            .frame_count(200)
            .rationale(
                "Apps that toggle timers, file watchers, and network listeners \
                 exercise subscription lifecycle. Churn should not leak resources.",
            )
            .tests_hypothesis(
                "Subscription churn should not accumulate memory or handles \
                 beyond O(active) at any point.",
            )
            .tags(vec!["subscriptions", "lifecycle", "regression"]),
        );

        reg.register(
            FixtureSpec::new(
                "runtime_cancellation_rapid",
                "Rapid command cancellation",
                FixtureFamily::Runtime,
            )
            .viewport(ViewportSpec::STANDARD)
            .transitions(vec![TransitionPattern::Cancellation])
            .frame_count(100)
            .rationale(
                "Users may trigger commands and immediately cancel (Ctrl-C, \
                 mode switch). Cancellation should be prompt without resource leaks.",
            )
            .tests_hypothesis(
                "Command cancellation latency should be < 5ms with no \
                 accumulated side effects.",
            )
            .tags(vec!["cancellation", "effects", "regression"]),
        );

        reg.register(
            FixtureSpec::new(
                "runtime_shutdown_determinism",
                "Deterministic shutdown sequence",
                FixtureFamily::Runtime,
            )
            .viewport(ViewportSpec::STANDARD)
            .transitions(vec![
                TransitionPattern::SubscriptionChurn,
                TransitionPattern::Cancellation,
            ])
            .frame_count(50)
            .rationale(
                "Shutdown must restore terminal state, drain subscriptions, \
                 and complete pending effects in deterministic order.",
            )
            .tests_hypothesis(
                "Shutdown frame hashes should be identical across runs with \
                 the same seed.",
            )
            .tags(vec!["shutdown", "determinism", "regression"]),
        );

        // ====================================================================
        // DOCTOR LANE — Canonical
        // ====================================================================

        reg.register(
            FixtureSpec::new(
                "doctor_capture_workflow",
                "Doctor capture-suite-report chain",
                FixtureFamily::Doctor,
            )
            .viewport(ViewportSpec::STANDARD)
            .transitions(vec![TransitionPattern::ArtifactHeavy])
            .frame_count(50)
            .rules(ReproducibilityRules {
                timeout_secs: 120,
                ..ReproducibilityRules::default_deterministic()
            })
            .rationale(
                "The doctor_frankentui capture → suite → report workflow \
                 produces evidence artifacts. This measures wall-clock overhead \
                 and artifact correctness.",
            )
            .tests_hypothesis(
                "Doctor workflow should complete within 60s and produce a \
                 valid artifact manifest.",
            )
            .tags(vec!["doctor", "workflow", "artifacts", "regression"]),
        );

        reg.register(
            FixtureSpec::new(
                "doctor_seed_orchestration",
                "Seed-demo RPC orchestration",
                FixtureFamily::Doctor,
            )
            .viewport(ViewportSpec::STANDARD)
            .transitions(vec![
                TransitionPattern::DegradedIo,
                TransitionPattern::Timeout,
            ])
            .frame_count(30)
            .rules(ReproducibilityRules {
                timeout_secs: 90,
                ..ReproducibilityRules::default_deterministic()
            })
            .rationale(
                "Seed orchestration involves network probes, retries, and \
                 RPC calls. This ensures structured retry/deadline behavior.",
            )
            .tests_hypothesis(
                "Seed should emit structured lifecycle events and respect \
                 explicit deadlines without mystery waits.",
            )
            .tags(vec!["doctor", "seed", "networking", "regression"]),
        );

        // ====================================================================
        // CHALLENGE SUITE — Adversarial
        // ====================================================================

        reg.register(
            FixtureSpec::new(
                "challenge_resize_storm",
                "Rapid resize storm",
                FixtureFamily::Render,
            )
            .partition(SuitePartition::Challenge)
            .viewport(ViewportSpec::STANDARD)
            .extra_viewports(vec![
                ViewportSpec::TINY,
                ViewportSpec::LARGE,
                ViewportSpec::ULTRAWIDE,
            ])
            .transitions(vec![TransitionPattern::ResizeChurn])
            .frame_count(300)
            .rationale(
                "Terminal multiplexers and tiling WMs send rapid resize events. \
                 The renderer must coalesce and handle without corruption or panic.",
            )
            .tests_hypothesis(
                "No frame should produce garbled output or panic during resize \
                 storms across 4 viewport sizes.",
            )
            .tags(vec!["resize", "stress", "challenge"]),
        );

        reg.register(
            FixtureSpec::new(
                "challenge_input_flood",
                "Input event flood beyond frame budget",
                FixtureFamily::Runtime,
            )
            .partition(SuitePartition::Challenge)
            .viewport(ViewportSpec::STANDARD)
            .transitions(vec![TransitionPattern::InputStorm])
            .frame_count(200)
            .rationale(
                "Paste operations and held-down keys can generate events faster \
                 than the frame rate. The runtime must not drop events or \
                 accumulate unbounded backlogs.",
            )
            .tests_hypothesis(
                "Input processing should batch events per frame without \
                 unbounded queue growth.",
            )
            .tags(vec!["input", "flood", "stress", "challenge"]),
        );

        reg.register(
            FixtureSpec::new(
                "challenge_mixed_workload",
                "Combined render + runtime + IO stress",
                FixtureFamily::Runtime,
            )
            .partition(SuitePartition::Challenge)
            .viewport(ViewportSpec::MEDIUM)
            .transitions(vec![TransitionPattern::Mixed])
            .frame_count(250)
            .rationale(
                "Real applications combine widget rendering, subscription \
                 events, effect execution, and occasional resizes simultaneously. \
                 This exercises all paths concurrently.",
            )
            .tests_hypothesis(
                "No single subsystem should starve others under concurrent \
                 load. Frame p95 should remain below 2x steady-state.",
            )
            .tags(vec!["mixed", "concurrency", "stress", "challenge"]),
        );

        reg.register(
            FixtureSpec::new(
                "challenge_doctor_degraded_network",
                "Doctor seed under degraded network",
                FixtureFamily::Doctor,
            )
            .partition(SuitePartition::Challenge)
            .viewport(ViewportSpec::STANDARD)
            .transitions(vec![
                TransitionPattern::DegradedIo,
                TransitionPattern::Timeout,
            ])
            .frame_count(20)
            .rationale(
                "Doctor seed must handle high-latency and intermittent network \
                 failures gracefully, with structured failure signatures.",
            )
            .tests_hypothesis(
                "Seed should exhaust retries, emit stage_failed events, and \
                 produce actionable failure diagnostics.",
            )
            .tags(vec!["doctor", "network", "failure", "challenge"]),
        );

        // ====================================================================
        // NEGATIVE CONTROLS
        // ====================================================================

        reg.register(
            FixtureSpec::new(
                "control_static_screen",
                "Static screen with no updates",
                FixtureFamily::Render,
            )
            .partition(SuitePartition::NegativeControl)
            .viewport(ViewportSpec::STANDARD)
            .transitions(vec![])
            .frame_count(100)
            .expects_improvement(false)
            .rationale(
                "A fully static screen should produce zero diff output after \
                 the initial frame. Any optimization that changes this is wrong.",
            )
            .tests_hypothesis("Diff output should be exactly zero bytes for frames 2-N.")
            .tags(vec!["static", "control", "zero-diff"]),
        );

        reg.register(
            FixtureSpec::new(
                "control_idle_runtime",
                "Idle runtime with no events",
                FixtureFamily::Runtime,
            )
            .partition(SuitePartition::NegativeControl)
            .viewport(ViewportSpec::STANDARD)
            .transitions(vec![])
            .frame_count(50)
            .expects_improvement(false)
            .rationale(
                "An idle runtime with no events and no subscriptions should \
                 consume near-zero CPU. Any optimization that increases idle \
                 overhead is a regression.",
            )
            .tests_hypothesis("Idle CPU usage should remain below 1% with no scheduled work.")
            .tags(vec!["idle", "control", "overhead"]),
        );

        reg
    }
}

impl Default for FixtureRegistry {
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
    fn canonical_registry_is_populated() {
        let reg = FixtureRegistry::canonical();
        assert!(
            reg.len() >= 14,
            "expected at least 14 built-in fixtures, got {}",
            reg.len()
        );
    }

    #[test]
    fn registry_has_all_families() {
        let reg = FixtureRegistry::canonical();
        assert!(
            !reg.by_family(FixtureFamily::Render).is_empty(),
            "render fixtures missing"
        );
        assert!(
            !reg.by_family(FixtureFamily::Runtime).is_empty(),
            "runtime fixtures missing"
        );
        assert!(
            !reg.by_family(FixtureFamily::Doctor).is_empty(),
            "doctor fixtures missing"
        );
    }

    #[test]
    fn registry_has_all_partitions() {
        let reg = FixtureRegistry::canonical();
        assert!(
            !reg.by_partition(SuitePartition::Canonical).is_empty(),
            "canonical partition missing"
        );
        assert!(
            !reg.by_partition(SuitePartition::Challenge).is_empty(),
            "challenge partition missing"
        );
        assert!(
            !reg.by_partition(SuitePartition::NegativeControl).is_empty(),
            "negative control partition missing"
        );
    }

    #[test]
    fn every_fixture_has_rationale() {
        let reg = FixtureRegistry::canonical();
        for spec in reg.all() {
            assert!(
                !spec.rationale.is_empty(),
                "fixture {} missing rationale",
                spec.id
            );
        }
    }

    #[test]
    fn every_fixture_has_hypothesis() {
        let reg = FixtureRegistry::canonical();
        for spec in reg.all() {
            assert!(
                !spec.tests_hypothesis.is_empty(),
                "fixture {} missing hypothesis",
                spec.id
            );
        }
    }

    #[test]
    fn every_fixture_has_tags() {
        let reg = FixtureRegistry::canonical();
        for spec in reg.all() {
            assert!(!spec.tags.is_empty(), "fixture {} missing tags", spec.id);
        }
    }

    #[test]
    fn fixture_ids_are_unique() {
        let reg = FixtureRegistry::canonical();
        let ids: Vec<&str> = reg.all().iter().map(|f| f.id.as_str()).collect();
        let mut seen = std::collections::HashSet::new();
        for id in &ids {
            assert!(seen.insert(*id), "duplicate fixture id: {id}");
        }
    }

    #[test]
    fn artifact_path_convention() {
        let spec = FixtureSpec::new("test_fix", "Test", FixtureFamily::Render);
        assert_eq!(
            spec.baseline_path(),
            "render/canonical/test_fix/baseline.json"
        );
        assert_eq!(spec.replay_path(), "render/canonical/test_fix/replay.jsonl");
        assert_eq!(
            spec.artifact_path("profile", "jsonl"),
            "render/canonical/test_fix/profile.jsonl"
        );
    }

    #[test]
    fn challenge_partition_sets_challenge_rules() {
        let spec = FixtureSpec::new("ch", "Challenge", FixtureFamily::Render)
            .partition(SuitePartition::Challenge);
        assert_eq!(spec.rules.time_step_ms, 0, "challenges use wall-clock");
        assert!(spec.rules.require_host_match);
    }

    #[test]
    fn viewport_spec_cell_count() {
        assert_eq!(ViewportSpec::STANDARD.cell_count(), 80 * 24);
        assert_eq!(ViewportSpec::LARGE.cell_count(), 200 * 60);
        assert_eq!(ViewportSpec::TINY.cell_count(), 40 * 10);
    }

    #[test]
    fn viewport_spec_label() {
        assert_eq!(ViewportSpec::STANDARD.label(), "80x24");
        assert_eq!(ViewportSpec::ULTRAWIDE.label(), "320x24");
    }

    #[test]
    fn suite_partition_labels() {
        assert_eq!(SuitePartition::Canonical.label(), "canonical");
        assert_eq!(SuitePartition::Challenge.label(), "challenge");
        assert_eq!(SuitePartition::NegativeControl.label(), "negative-control");
    }

    #[test]
    fn transition_pattern_labels() {
        assert_eq!(TransitionPattern::SparseUpdate.label(), "sparse-update");
        assert_eq!(TransitionPattern::ResizeChurn.label(), "resize-churn");
        assert_eq!(TransitionPattern::Mixed.label(), "mixed");
    }

    #[test]
    fn registry_by_tag() {
        let reg = FixtureRegistry::canonical();
        let diff_fixtures = reg.by_tag("diff");
        assert!(
            diff_fixtures.len() >= 2,
            "expected at least 2 diff-tagged fixtures"
        );
    }

    #[test]
    fn registry_by_transition() {
        let reg = FixtureRegistry::canonical();
        let resize = reg.by_transition(TransitionPattern::ResizeChurn);
        assert!(
            !resize.is_empty(),
            "expected at least one resize-churn fixture"
        );
    }

    #[test]
    fn registry_lookup_by_id() {
        let reg = FixtureRegistry::canonical();
        let spec = reg.get("render_diff_sparse_80x24");
        assert!(spec.is_some());
        let spec = spec.unwrap();
        assert_eq!(spec.family, FixtureFamily::Render);
        assert_eq!(spec.partition, SuitePartition::Canonical);
    }

    #[test]
    fn fixture_to_json_valid() {
        let spec = FixtureSpec::new("json_test", "JSON Test", FixtureFamily::Render)
            .rationale("test rationale")
            .tests_hypothesis("test hypothesis")
            .tags(vec!["test"]);
        let json = spec.to_json();
        assert!(json.contains("\"id\": \"json_test\""));
        assert!(json.contains("\"family\": \"render\""));
        assert!(json.contains("\"partition\": \"canonical\""));
        assert!(json.contains("\"rationale\": \"test rationale\""));
        assert!(json.contains("\"baseline_path\":"));
        assert!(json.contains("\"replay_path\":"));
    }

    #[test]
    fn manifest_json_has_counts() {
        let reg = FixtureRegistry::canonical();
        let manifest = reg.manifest_json();
        assert!(manifest.contains("\"schema_version\": 1"));
        assert!(manifest.contains("\"total_fixtures\":"));
        assert!(manifest.contains("\"canonical_count\":"));
        assert!(manifest.contains("\"challenge_count\":"));
    }

    #[test]
    fn negative_controls_do_not_expect_improvement() {
        let reg = FixtureRegistry::canonical();
        for spec in reg.by_partition(SuitePartition::NegativeControl) {
            assert!(
                !spec.expects_improvement,
                "negative control {} should not expect improvement",
                spec.id
            );
        }
    }

    #[test]
    fn reproducibility_defaults() {
        let rules = ReproducibilityRules::default_deterministic();
        assert_eq!(rules.seed, 42);
        assert_eq!(rules.time_step_ms, 16);
        assert!(rules.deterministic_time);
        assert!(!rules.require_host_match);
        assert_eq!(rules.min_samples, 30);
    }

    #[test]
    fn challenge_reproducibility_uses_wall_clock() {
        let rules = ReproducibilityRules::challenge();
        assert_eq!(rules.time_step_ms, 0);
        assert!(!rules.deterministic_time);
        assert!(rules.require_host_match);
    }

    #[test]
    fn empty_registry() {
        let reg = FixtureRegistry::new();
        assert!(reg.is_empty());
        assert_eq!(reg.len(), 0);
        assert!(reg.all().is_empty());
    }

    #[test]
    fn register_and_retrieve() {
        let mut reg = FixtureRegistry::new();
        reg.register(
            FixtureSpec::new("custom", "Custom Fixture", FixtureFamily::Runtime)
                .rationale("custom test")
                .tests_hypothesis("custom hypothesis")
                .tags(vec!["custom"]),
        );
        assert_eq!(reg.len(), 1);
        assert!(reg.get("custom").is_some());
        assert!(reg.get("nonexistent").is_none());
    }
}
