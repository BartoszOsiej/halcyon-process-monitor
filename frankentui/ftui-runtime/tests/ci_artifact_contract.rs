//! CI artifact contracts, replay helpers, and failure-triage reports (bd-cn65n).
//!
//! This module defines the artifact contract for the Asupersync migration CI.
//! Each test suite must emit consistent, replay-friendly artifacts. Missing or
//! malformed evidence is a hard failure. Operators can navigate from a failed
//! gate to replay instructions using the `ReplayBundle` type.
//!
//! # Artifact Contract
//!
//! Every validation suite must produce:
//! 1. A trace (ordered list of lifecycle events)
//! 2. A verdict (pass/fail with reason)
//! 3. A replay bundle (inputs + expected outputs for reproduction)
//!
//! These are validated by the tests in this file.

#![forbid(unsafe_code)]

use ftui_core::event::Event;
use ftui_render::frame::Frame;
use ftui_runtime::program::{Cmd, Model, RuntimeLane};
use ftui_runtime::simulator::ProgramSimulator;

// ============================================================================
// Artifact contract types
// ============================================================================

/// A replay bundle captures everything needed to reproduce a test result.
#[derive(Debug, Clone)]
struct ReplayBundle {
    /// Human-readable name for the scenario.
    scenario: String,
    /// Which runtime lane was tested.
    lane: RuntimeLane,
    /// Input messages (serialized as debug strings for replay).
    inputs: Vec<String>,
    /// Expected trace output.
    expected_trace: Vec<String>,
    /// Actual trace output.
    actual_trace: Vec<String>,
    /// Whether the test passed.
    passed: bool,
    /// Failure reason if any.
    failure_reason: Option<String>,
}

impl ReplayBundle {
    fn new(scenario: &str, lane: RuntimeLane) -> Self {
        Self {
            scenario: scenario.into(),
            lane,
            inputs: vec![],
            expected_trace: vec![],
            actual_trace: vec![],
            passed: true,
            failure_reason: None,
        }
    }

    fn add_input(&mut self, input: &str) {
        self.inputs.push(input.into());
    }

    fn set_expected(&mut self, trace: Vec<String>) {
        self.expected_trace = trace;
    }

    fn set_actual(&mut self, trace: Vec<String>) {
        self.actual_trace = trace;
    }

    fn verify(&mut self) {
        if self.actual_trace != self.expected_trace {
            self.passed = false;
            self.failure_reason = Some(format!(
                "Trace mismatch:\n  expected: {:?}\n  actual:   {:?}",
                self.expected_trace, self.actual_trace
            ));
        }
    }

    /// Generate triage report for operators.
    fn triage_report(&self) -> String {
        let mut report = String::new();
        report.push_str(&format!("=== Triage Report: {} ===\n", self.scenario));
        report.push_str(&format!("Lane: {}\n", self.lane));
        report.push_str(&format!(
            "Verdict: {}\n",
            if self.passed { "PASS" } else { "FAIL" }
        ));

        if let Some(ref reason) = self.failure_reason {
            report.push_str(&format!("Failure: {reason}\n"));
        }

        report.push_str("\n--- Replay Instructions ---\n");
        report.push_str("1. Create model with ArtifactModel { trace: vec![] }\n");
        report.push_str("2. Call sim.init()\n");
        for (i, input) in self.inputs.iter().enumerate() {
            report.push_str(&format!("3.{i}. sim.send({input})\n"));
        }
        report.push_str("4. Compare trace against expected\n");

        report.push_str("\n--- Expected Trace ---\n");
        for (i, entry) in self.expected_trace.iter().enumerate() {
            report.push_str(&format!("  [{i:3}] {entry}\n"));
        }

        if !self.passed {
            report.push_str("\n--- Actual Trace ---\n");
            for (i, entry) in self.actual_trace.iter().enumerate() {
                let marker = if i < self.expected_trace.len() && entry != &self.expected_trace[i] {
                    " <-- DIVERGES"
                } else if i >= self.expected_trace.len() {
                    " <-- EXTRA"
                } else {
                    ""
                };
                report.push_str(&format!("  [{i:3}] {entry}{marker}\n"));
            }
        }

        report
    }

    fn assert_passed(&self) {
        assert!(
            self.passed,
            "Artifact contract violation:\n{}",
            self.triage_report()
        );
    }
}

// ============================================================================
// Artifact model
// ============================================================================

struct ArtifactModel {
    trace: Vec<String>,
}

#[derive(Debug)]
enum AMsg {
    Step(String),
    Batch(Vec<String>),
    Task(String),
    TaskDone(String),
    Quit,
}

impl From<Event> for AMsg {
    fn from(_: Event) -> Self {
        AMsg::Step("event".into())
    }
}

impl Model for ArtifactModel {
    type Message = AMsg;

    fn init(&mut self) -> Cmd<Self::Message> {
        self.trace.push("init".into());
        Cmd::none()
    }

    fn update(&mut self, msg: Self::Message) -> Cmd<Self::Message> {
        match msg {
            AMsg::Step(s) => {
                self.trace.push(format!("step:{s}"));
                Cmd::none()
            }
            AMsg::Batch(items) => {
                self.trace.push(format!("batch:{}", items.len()));
                Cmd::batch(items.into_iter().map(|s| Cmd::msg(AMsg::Step(s))).collect())
            }
            AMsg::Task(l) => {
                self.trace.push(format!("task:{l}"));
                let lc = l.clone();
                Cmd::task(move || AMsg::TaskDone(lc))
            }
            AMsg::TaskDone(l) => {
                self.trace.push(format!("done:{l}"));
                Cmd::none()
            }
            AMsg::Quit => {
                self.trace.push("quit".into());
                Cmd::quit()
            }
        }
    }

    fn view(&self, _frame: &mut Frame) {}
}

fn new_sim() -> ProgramSimulator<ArtifactModel> {
    let mut sim = ProgramSimulator::new(ArtifactModel { trace: vec![] });
    sim.init();
    sim
}

// ============================================================================
// CONTRACT: ReplayBundle captures complete evidence
// ============================================================================

#[test]
fn artifact_replay_bundle_captures_pass() {
    let mut bundle = ReplayBundle::new("basic_steps", RuntimeLane::Structured);
    bundle.add_input("Step(a)");
    bundle.add_input("Step(b)");

    let mut sim = new_sim();
    sim.send(AMsg::Step("a".into()));
    sim.send(AMsg::Step("b".into()));

    let expected = vec![
        "init".to_string(),
        "step:a".to_string(),
        "step:b".to_string(),
    ];
    bundle.set_expected(expected);
    bundle.set_actual(sim.model().trace.clone());
    bundle.verify();

    assert!(bundle.passed);
    assert!(bundle.failure_reason.is_none());
    bundle.assert_passed();
}

#[test]
fn artifact_replay_bundle_detects_mismatch() {
    let mut bundle = ReplayBundle::new("intentional_mismatch", RuntimeLane::Structured);
    bundle.set_expected(vec!["init".to_string(), "step:a".to_string()]);
    bundle.set_actual(vec!["init".to_string(), "step:WRONG".to_string()]);
    bundle.verify();

    assert!(!bundle.passed);
    assert!(bundle.failure_reason.is_some());
    let reason = bundle.failure_reason.as_ref().unwrap();
    assert!(
        reason.contains("Trace mismatch"),
        "reason should mention mismatch"
    );
}

#[test]
fn artifact_triage_report_contains_replay_instructions() {
    let mut bundle = ReplayBundle::new("triage_test", RuntimeLane::Structured);
    bundle.add_input("Step(x)");
    bundle.add_input("Batch([y, z])");
    bundle.set_expected(vec!["init".into(), "step:x".into()]);
    bundle.set_actual(vec!["init".into(), "step:x".into()]);
    bundle.verify();

    let report = bundle.triage_report();
    assert!(report.contains("Triage Report: triage_test"));
    assert!(report.contains("Lane: structured"));
    assert!(report.contains("Verdict: PASS"));
    assert!(report.contains("Replay Instructions"));
    assert!(report.contains("Step(x)"));
    assert!(report.contains("Batch([y, z])"));
    assert!(report.contains("Expected Trace"));
}

#[test]
fn artifact_triage_report_marks_divergence() {
    let mut bundle = ReplayBundle::new("divergence", RuntimeLane::Legacy);
    bundle.set_expected(vec!["init".into(), "step:a".into()]);
    bundle.set_actual(vec!["init".into(), "step:WRONG".into()]);
    bundle.verify();

    let report = bundle.triage_report();
    assert!(report.contains("FAIL"));
    assert!(report.contains("DIVERGES"));
}

// ============================================================================
// CONTRACT: Validation suites produce complete artifacts
// ============================================================================

#[test]
fn artifact_happy_path_complete() {
    let mut bundle = ReplayBundle::new("happy_path", RuntimeLane::Structured);
    bundle.add_input("Step(a)");
    bundle.add_input("Batch([b, c])");
    bundle.add_input("Task(t)");

    let mut sim = new_sim();
    sim.send(AMsg::Step("a".into()));
    sim.send(AMsg::Batch(vec!["b".into(), "c".into()]));
    sim.send(AMsg::Task("t".into()));

    let expected = vec![
        "init", "step:a", "batch:2", "step:b", "step:c", "task:t", "done:t",
    ]
    .into_iter()
    .map(String::from)
    .collect();

    bundle.set_expected(expected);
    bundle.set_actual(sim.model().trace.clone());
    bundle.verify();
    bundle.assert_passed();

    // Artifact completeness checks
    assert!(!bundle.inputs.is_empty(), "inputs must be recorded");
    assert!(
        !bundle.expected_trace.is_empty(),
        "expected trace must be set"
    );
    assert!(!bundle.actual_trace.is_empty(), "actual trace must be set");
    assert_eq!(bundle.scenario, "happy_path");
    assert_eq!(bundle.lane, RuntimeLane::Structured);
}

#[test]
fn artifact_failure_path_complete() {
    let mut bundle = ReplayBundle::new("quit_path", RuntimeLane::Structured);
    bundle.add_input("Step(before)");
    bundle.add_input("Quit");
    bundle.add_input("Step(after)");

    let mut sim = new_sim();
    sim.send(AMsg::Step("before".into()));
    sim.send(AMsg::Quit);
    sim.send(AMsg::Step("after".into()));

    let expected = vec!["init", "step:before", "quit"]
        .into_iter()
        .map(String::from)
        .collect();

    bundle.set_expected(expected);
    bundle.set_actual(sim.model().trace.clone());
    bundle.verify();
    bundle.assert_passed();
    assert!(!sim.is_running());
}

#[test]
fn artifact_stress_path_complete() {
    let mut bundle = ReplayBundle::new("stress_100_batch", RuntimeLane::Structured);
    let items: Vec<String> = (0..100).map(|i| format!("{i}")).collect();
    bundle.add_input("Batch([0..99])");

    let mut sim = new_sim();
    sim.send(AMsg::Batch(items));

    let trace = sim.model().trace.clone();
    // Build expected: init, batch:100, step:0, step:1, ..., step:99
    let mut expected = vec!["init".to_string(), "batch:100".to_string()];
    for i in 0..100 {
        expected.push(format!("step:{i}"));
    }

    bundle.set_expected(expected);
    bundle.set_actual(trace);
    bundle.verify();
    bundle.assert_passed();
}

// ============================================================================
// CONTRACT: Missing evidence is a hard failure
// ============================================================================

#[test]
fn artifact_missing_expected_is_failure() {
    let mut bundle = ReplayBundle::new("missing_expected", RuntimeLane::Structured);
    // Don't set expected trace
    bundle.set_actual(vec!["init".into()]);
    bundle.verify();

    // Empty expected != non-empty actual = mismatch
    assert!(!bundle.passed, "missing expected trace must be a failure");
}

#[test]
fn artifact_missing_actual_is_failure() {
    let mut bundle = ReplayBundle::new("missing_actual", RuntimeLane::Structured);
    bundle.set_expected(vec!["init".into()]);
    // Don't set actual trace
    bundle.verify();

    // Non-empty expected != empty actual = mismatch
    assert!(!bundle.passed, "missing actual trace must be a failure");
}

// ============================================================================
// CONTRACT: Replay bundles are deterministic
// ============================================================================

#[test]
fn artifact_replay_deterministic() {
    fn run_and_bundle() -> ReplayBundle {
        let mut bundle = ReplayBundle::new("determinism", RuntimeLane::Structured);
        bundle.add_input("Step(x)");
        bundle.add_input("Batch([y, z])");
        bundle.add_input("Task(t)");

        let mut sim = new_sim();
        sim.send(AMsg::Step("x".into()));
        sim.send(AMsg::Batch(vec!["y".into(), "z".into()]));
        sim.send(AMsg::Task("t".into()));

        bundle.set_actual(sim.model().trace.clone());
        bundle.set_expected(sim.model().trace.clone()); // self-consistent
        bundle.verify();
        bundle
    }

    let b1 = run_and_bundle();
    let b2 = run_and_bundle();
    let b3 = run_and_bundle();

    assert_eq!(b1.actual_trace, b2.actual_trace);
    assert_eq!(b2.actual_trace, b3.actual_trace);
    assert!(b1.passed && b2.passed && b3.passed);
}
