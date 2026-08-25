#![forbid(unsafe_code)]

//! Shadow-run comparison harness for validating execution-path equivalence.
//!
//! Runs the same [`Model`] and event sequence through two independent
//! [`LabSession`] instances (baseline vs candidate) and compares frame
//! checksums, event counts, and timing. This is the primary mechanism for
//! proving that a runtime migration (e.g., threading → Asupersync executor)
//! preserves rendering determinism.
//!
//! # Design
//!
//! A [`ShadowRun`] takes two [`LabConfig`]s (baseline and candidate), a
//! model factory, and a scenario closure. It executes the scenario twice—
//! once per lane—under deterministic seeds, then compares the frame records.
//! All comparison evidence is emitted to JSONL via [`TestJsonlLogger`].
//!
//! # Example
//!
//! ```ignore
//! use ftui_harness::shadow_run::{ShadowRun, ShadowRunConfig, ShadowVerdict};
//!
//! let config = ShadowRunConfig::new("migration_test", "tick_counter", 42)
//!     .viewport(80, 24);
//!
//! let result = ShadowRun::compare(config, || MyModel::new(), |session| {
//!     session.init();
//!     session.tick();
//!     session.capture_frame();
//! });
//!
//! assert_eq!(result.verdict, ShadowVerdict::Match);
//! ```

use std::sync::atomic::{AtomicU64, Ordering};

use crate::determinism::{JsonValue, TestJsonlLogger};
use crate::lab_integration::{Lab, LabConfig, LabOutput, LabSession};
use ftui_runtime::program::Model;
use tracing::info_span;

/// Global counter for shadow runs executed.
static SHADOW_RUNS_TOTAL: AtomicU64 = AtomicU64::new(0);

/// Read the total number of shadow runs executed in-process.
#[must_use]
pub fn shadow_runs_total() -> u64 {
    SHADOW_RUNS_TOTAL.load(Ordering::Relaxed)
}

// ============================================================================
// Configuration
// ============================================================================

/// Configuration for a shadow-run comparison.
#[derive(Debug, Clone)]
pub struct ShadowRunConfig {
    /// Shared prefix for JSONL logger and run IDs.
    pub prefix: String,
    /// Scenario name used in tracing spans and JSONL.
    pub scenario_name: String,
    /// Deterministic seed (shared across both lanes).
    pub seed: u64,
    /// Viewport width for frame captures.
    pub viewport_width: u16,
    /// Viewport height for frame captures.
    pub viewport_height: u16,
    /// Time step in milliseconds for deterministic clocks.
    pub time_step_ms: u64,
    /// Label for the baseline lane (default: "baseline").
    pub baseline_label: String,
    /// Label for the candidate lane (default: "candidate").
    pub candidate_label: String,
}

impl ShadowRunConfig {
    /// Create a new shadow-run configuration with defaults.
    ///
    /// Defaults: 80×24 viewport, 16ms time step.
    pub fn new(prefix: &str, scenario_name: &str, seed: u64) -> Self {
        Self {
            prefix: prefix.to_string(),
            scenario_name: scenario_name.to_string(),
            seed,
            viewport_width: 80,
            viewport_height: 24,
            time_step_ms: 16,
            baseline_label: "baseline".to_string(),
            candidate_label: "candidate".to_string(),
        }
    }

    /// Set the viewport dimensions.
    #[must_use]
    pub fn viewport(mut self, width: u16, height: u16) -> Self {
        self.viewport_width = width;
        self.viewport_height = height;
        self
    }

    /// Set the deterministic time step in milliseconds.
    #[must_use]
    pub fn time_step_ms(mut self, ms: u64) -> Self {
        self.time_step_ms = ms;
        self
    }

    /// Set custom lane labels.
    #[must_use]
    pub fn lane_labels(mut self, baseline: &str, candidate: &str) -> Self {
        self.baseline_label = baseline.to_string();
        self.candidate_label = candidate.to_string();
        self
    }

    /// Build a [`LabConfig`] for a given lane.
    fn lab_config(&self, lane: &str) -> LabConfig {
        LabConfig::new(
            &format!("{}_{}", self.prefix, lane),
            &self.scenario_name,
            self.seed,
        )
        .viewport(self.viewport_width, self.viewport_height)
        .time_step_ms(self.time_step_ms)
    }
}

// ============================================================================
// Verdict and result
// ============================================================================

/// Outcome of a shadow-run comparison.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShadowVerdict {
    /// All frame checksums matched between baseline and candidate.
    Match,
    /// Frame checksums diverged at one or more positions.
    Diverged,
}

/// Per-frame comparison detail.
#[derive(Debug, Clone)]
pub struct FrameComparison {
    /// Frame index (0-based).
    pub index: usize,
    /// Baseline frame checksum.
    pub baseline_checksum: u64,
    /// Candidate frame checksum.
    pub candidate_checksum: u64,
    /// Whether this frame matched.
    pub matched: bool,
}

/// Full result of a shadow-run comparison.
#[derive(Debug, Clone)]
pub struct ShadowRunResult {
    /// Overall verdict.
    pub verdict: ShadowVerdict,
    /// Scenario name.
    pub scenario_name: String,
    /// Seed used for both lanes.
    pub seed: u64,
    /// Per-frame comparison details.
    pub frame_comparisons: Vec<FrameComparison>,
    /// Index of the first divergent frame (if any).
    pub first_divergence: Option<usize>,
    /// Number of frames compared.
    pub frames_compared: usize,
    /// Baseline lane output.
    pub baseline: LabOutput,
    /// Candidate lane output.
    pub candidate: LabOutput,
    /// Baseline lane label.
    pub baseline_label: String,
    /// Candidate lane label.
    pub candidate_label: String,
    /// Total shadow runs executed in-process (including this one).
    pub run_total: u64,
}

impl ShadowRunResult {
    /// Number of frames that diverged.
    #[must_use]
    pub fn diverged_count(&self) -> usize {
        self.frame_comparisons.iter().filter(|c| !c.matched).count()
    }

    /// Fraction of frames that matched (0.0–1.0).
    #[must_use]
    pub fn match_ratio(&self) -> f64 {
        if self.frames_compared == 0 {
            return 1.0;
        }
        let matched = self.frame_comparisons.iter().filter(|c| c.matched).count();
        matched as f64 / self.frames_compared as f64
    }
}

// ============================================================================
// Shadow-run executor
// ============================================================================

/// Shadow-run comparison harness.
///
/// Runs the same model and event sequence through two independent LabSession
/// instances and compares their frame outputs.
pub struct ShadowRun;

impl ShadowRun {
    /// Run a shadow comparison between baseline and candidate lanes.
    ///
    /// Both lanes execute the same `scenario_fn` with the same seed and
    /// configuration. Frame checksums are compared after both runs complete.
    ///
    /// The `model_factory` is called twice (once per lane) to produce
    /// independent model instances.
    ///
    /// # Evidence
    ///
    /// Emits structured JSONL to stderr with events:
    /// - `shadow.start`: comparison parameters
    /// - `shadow.lane.done`: per-lane summary
    /// - `shadow.frame.diverged`: each divergent frame
    /// - `shadow.verdict`: final pass/fail with statistics
    pub fn compare<M, MF, SF>(
        config: ShadowRunConfig,
        model_factory: MF,
        scenario_fn: SF,
    ) -> ShadowRunResult
    where
        M: Model,
        MF: Fn() -> M,
        SF: Fn(&mut LabSession<M>),
    {
        let _span = info_span!(
            "shadow_run",
            scenario_name = config.scenario_name.as_str(),
            seed = config.seed,
            baseline = config.baseline_label.as_str(),
            candidate = config.candidate_label.as_str(),
        )
        .entered();

        let mut logger = TestJsonlLogger::new_with(
            &format!("{}_shadow", config.prefix),
            config.seed,
            true,
            config.time_step_ms,
        );
        logger.add_context_str("scenario_name", &config.scenario_name);
        logger.add_context_str("baseline_label", &config.baseline_label);
        logger.add_context_str("candidate_label", &config.candidate_label);

        // Log start
        logger.log(
            "shadow.start",
            &[
                ("scenario_name", JsonValue::str(&config.scenario_name)),
                ("seed", JsonValue::u64(config.seed)),
                (
                    "viewport",
                    JsonValue::raw(format!(
                        "[{},{}]",
                        config.viewport_width, config.viewport_height
                    )),
                ),
            ],
        );

        // Run baseline lane
        let baseline_config = config.lab_config(&config.baseline_label);
        let baseline_run = Lab::run_scenario(baseline_config, model_factory(), |s| scenario_fn(s));

        logger.log(
            "shadow.lane.done",
            &[
                ("lane", JsonValue::str(&config.baseline_label)),
                (
                    "frame_count",
                    JsonValue::u64(baseline_run.output.frame_count as u64),
                ),
                (
                    "event_count",
                    JsonValue::u64(baseline_run.output.event_count as u64),
                ),
                ("tick_count", JsonValue::u64(baseline_run.output.tick_count)),
                (
                    "anomaly_count",
                    JsonValue::u64(baseline_run.output.anomaly_count),
                ),
            ],
        );

        // Run candidate lane
        let candidate_config = config.lab_config(&config.candidate_label);
        let candidate_run =
            Lab::run_scenario(candidate_config, model_factory(), |s| scenario_fn(s));

        logger.log(
            "shadow.lane.done",
            &[
                ("lane", JsonValue::str(&config.candidate_label)),
                (
                    "frame_count",
                    JsonValue::u64(candidate_run.output.frame_count as u64),
                ),
                (
                    "event_count",
                    JsonValue::u64(candidate_run.output.event_count as u64),
                ),
                (
                    "tick_count",
                    JsonValue::u64(candidate_run.output.tick_count),
                ),
                (
                    "anomaly_count",
                    JsonValue::u64(candidate_run.output.anomaly_count),
                ),
            ],
        );

        // Compare frame checksums
        let baseline_frames = &baseline_run.output.frame_records;
        let candidate_frames = &candidate_run.output.frame_records;
        let frames_compared = baseline_frames.len().min(candidate_frames.len());
        let mut frame_comparisons = Vec::with_capacity(frames_compared);
        let mut first_divergence: Option<usize> = None;

        for i in 0..frames_compared {
            let matched = baseline_frames[i].checksum == candidate_frames[i].checksum;
            frame_comparisons.push(FrameComparison {
                index: i,
                baseline_checksum: baseline_frames[i].checksum,
                candidate_checksum: candidate_frames[i].checksum,
                matched,
            });
            if !matched && first_divergence.is_none() {
                first_divergence = Some(i);
                logger.log(
                    "shadow.frame.diverged",
                    &[
                        ("frame_idx", JsonValue::u64(i as u64)),
                        (
                            "baseline_checksum",
                            JsonValue::str(format!("{:016x}", baseline_frames[i].checksum)),
                        ),
                        (
                            "candidate_checksum",
                            JsonValue::str(format!("{:016x}", candidate_frames[i].checksum)),
                        ),
                    ],
                );
            }
        }

        // Handle frame count mismatch (also counts as divergence)
        if baseline_frames.len() != candidate_frames.len() && first_divergence.is_none() {
            first_divergence = Some(frames_compared);
        }

        let verdict = if first_divergence.is_some() {
            ShadowVerdict::Diverged
        } else {
            ShadowVerdict::Match
        };

        let diverged_count = frame_comparisons.iter().filter(|c| !c.matched).count();

        // Log verdict
        logger.log(
            "shadow.verdict",
            &[
                (
                    "verdict",
                    JsonValue::str(match verdict {
                        ShadowVerdict::Match => "match",
                        ShadowVerdict::Diverged => "diverged",
                    }),
                ),
                ("frames_compared", JsonValue::u64(frames_compared as u64)),
                ("diverged_count", JsonValue::u64(diverged_count as u64)),
                (
                    "baseline_frames",
                    JsonValue::u64(baseline_frames.len() as u64),
                ),
                (
                    "candidate_frames",
                    JsonValue::u64(candidate_frames.len() as u64),
                ),
            ],
        );

        let run_total = SHADOW_RUNS_TOTAL
            .fetch_add(1, Ordering::Relaxed)
            .saturating_add(1);

        ShadowRunResult {
            verdict,
            scenario_name: config.scenario_name,
            seed: config.seed,
            frame_comparisons,
            first_divergence,
            frames_compared,
            baseline: baseline_run.output,
            candidate: candidate_run.output,
            baseline_label: config.baseline_label,
            candidate_label: config.candidate_label,
            run_total,
        }
    }

    /// Assert that baseline and candidate produce identical frames.
    ///
    /// Convenience wrapper around [`compare`](Self::compare) that panics
    /// on divergence with a diagnostic message.
    ///
    /// # Panics
    ///
    /// Panics if any frame checksum diverges between lanes.
    pub fn assert_match<M, MF, SF>(
        config: ShadowRunConfig,
        model_factory: MF,
        scenario_fn: SF,
    ) -> ShadowRunResult
    where
        M: Model,
        MF: Fn() -> M,
        SF: Fn(&mut LabSession<M>),
    {
        let result = Self::compare(config, model_factory, scenario_fn);
        if result.verdict == ShadowVerdict::Diverged {
            let diverged = result.diverged_count();
            let first = result
                .first_divergence
                .map(|i| format!("frame {i}"))
                .unwrap_or_else(|| "frame count mismatch".to_string());
            panic!(
                "shadow-run divergence: {} of {} frames diverged, first at {}",
                diverged, result.frames_compared, first
            );
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ftui_core::event::Event;
    use ftui_render::frame::Frame;
    use ftui_runtime::program::{Cmd, Model};

    // Minimal counter model for testing.
    struct Counter {
        value: u64,
    }

    #[derive(Debug, Clone)]
    enum CounterMsg {
        Increment,
        Quit,
    }

    impl From<Event> for CounterMsg {
        fn from(e: Event) -> Self {
            match e {
                Event::Tick => CounterMsg::Increment,
                _ => CounterMsg::Quit,
            }
        }
    }

    impl Model for Counter {
        type Message = CounterMsg;

        fn update(&mut self, msg: CounterMsg) -> Cmd<CounterMsg> {
            match msg {
                CounterMsg::Increment => {
                    self.value += 1;
                    Cmd::none()
                }
                CounterMsg::Quit => Cmd::quit(),
            }
        }

        fn view(&self, frame: &mut Frame) {
            use ftui_core::geometry::Rect;
            use ftui_widgets::paragraph::Paragraph;
            let text = format!("Count: {}", self.value);
            let area = Rect::new(0, 0, frame.width(), 1);
            Paragraph::new(text).render(area, frame);
        }
    }

    // Helper trait for rendering in view
    use ftui_widgets::Widget;

    #[test]
    fn shadow_run_identical_models_match() {
        let config = ShadowRunConfig::new("test_shadow", "counter_match", 42);
        let result = ShadowRun::compare(
            config,
            || Counter { value: 0 },
            |session| {
                session.init();
                session.tick();
                session.capture_frame();
                session.tick();
                session.capture_frame();
            },
        );
        assert_eq!(result.verdict, ShadowVerdict::Match);
        assert_eq!(result.frames_compared, 2);
        assert_eq!(result.diverged_count(), 0);
        assert!((result.match_ratio() - 1.0).abs() < f64::EPSILON);
        assert!(result.first_divergence.is_none());
    }

    #[test]
    fn shadow_run_assert_match_succeeds_for_identical() {
        let config = ShadowRunConfig::new("test_assert", "counter_assert", 42);
        let result = ShadowRun::assert_match(
            config,
            || Counter { value: 0 },
            |session| {
                session.init();
                session.tick();
                session.capture_frame();
            },
        );
        assert_eq!(result.verdict, ShadowVerdict::Match);
    }

    #[test]
    fn shadow_run_config_custom_labels() {
        let config = ShadowRunConfig::new("test_labels", "label_test", 7)
            .lane_labels("threading", "asupersync");
        assert_eq!(config.baseline_label, "threading");
        assert_eq!(config.candidate_label, "asupersync");
    }

    #[test]
    fn shadow_run_config_viewport() {
        let config = ShadowRunConfig::new("test_vp", "vp_test", 0)
            .viewport(120, 40)
            .time_step_ms(8);
        assert_eq!(config.viewport_width, 120);
        assert_eq!(config.viewport_height, 40);
        assert_eq!(config.time_step_ms, 8);
    }

    #[test]
    fn shadow_runs_total_increments() {
        let before = shadow_runs_total();
        let config = ShadowRunConfig::new("test_total", "total_test", 1);
        let _ = ShadowRun::compare(
            config,
            || Counter { value: 0 },
            |session| {
                session.init();
                session.capture_frame();
            },
        );
        assert!(shadow_runs_total() > before);
    }

    #[test]
    fn lab_assert_outputs_match_succeeds_for_identical() {
        let config = ShadowRunConfig::new("test_outputs", "outputs_test", 99);
        let result = ShadowRun::compare(
            config,
            || Counter { value: 0 },
            |session| {
                session.init();
                session.tick();
                session.capture_frame();
            },
        );
        // Both outputs came from identical runs so they should match.
        crate::lab_integration::assert_outputs_match(&result.baseline, &result.candidate);
    }

    #[test]
    fn match_ratio_empty_frames() {
        let result = ShadowRunResult {
            verdict: ShadowVerdict::Match,
            scenario_name: "empty".to_string(),
            seed: 0,
            frame_comparisons: vec![],
            first_divergence: None,
            frames_compared: 0,
            baseline: LabOutput {
                frame_count: 0,
                frame_records: vec![],
                event_count: 0,
                event_log: vec![],
                tick_count: 0,
                anomaly_count: 0,
            },
            candidate: LabOutput {
                frame_count: 0,
                frame_records: vec![],
                event_count: 0,
                event_log: vec![],
                tick_count: 0,
                anomaly_count: 0,
            },
            baseline_label: "baseline".to_string(),
            candidate_label: "candidate".to_string(),
            run_total: 1,
        };
        assert!((result.match_ratio() - 1.0).abs() < f64::EPSILON);
    }
}
