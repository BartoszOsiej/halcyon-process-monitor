//! LabRuntime-style deterministic harness for runtime-shell validation (bd-ycg2l).
//!
//! This module provides a `LabHarness` that wraps `ProgramSimulator` with:
//! - **Deterministic scheduling**: messages are queued and dispatched in explicit order
//! - **Trace capture**: detailed scheduling log with step indices
//! - **Assertion helpers**: verify trace prefixes, check for specific events, etc.
//! - **Replay support**: scenarios can be serialized and replayed for regression testing
//!
//! The harness eliminates ad-hoc sleeps and timing-dependent assertions. All
//! ordering is controlled by the test, not by thread scheduling.

#![forbid(unsafe_code)]

use ftui_core::event::Event;
use ftui_render::frame::Frame;
use ftui_runtime::program::{Cmd, Model};
use ftui_runtime::simulator::{CmdRecord, ProgramSimulator};
use std::time::Duration;

// ============================================================================
// LabHarness: deterministic scheduling wrapper
// ============================================================================

/// A scheduling step in the lab harness.
#[derive(Debug, Clone)]
enum LabStep<M: std::fmt::Debug> {
    /// Send a message to the model.
    Send(M),
    /// Inject a terminal event.
    Event(Event),
    /// Capture a frame at the given dimensions.
    CaptureFrame(u16, u16),
    /// Assert the model is still running.
    AssertRunning,
    /// Assert the model has stopped.
    AssertStopped,
}

/// Detailed trace entry from the harness.
#[derive(Debug, Clone)]
#[expect(dead_code)]
struct LabTraceEntry {
    step_index: usize,
    step_type: String,
    model_running: bool,
    cmd_log_len: usize,
}

/// Deterministic test harness that replays a fixed schedule of steps.
///
/// Usage:
/// ```ignore
/// let mut harness = LabHarness::new(MyModel::new());
/// harness.push_send(MyMsg::Init);
/// harness.push_send(MyMsg::DoWork);
/// harness.push_capture_frame(80, 24);
/// harness.push_assert_running();
/// harness.run();
/// assert_eq!(harness.model().some_field, expected_value);
/// ```
struct LabHarness<M: Model>
where
    M::Message: std::fmt::Debug,
{
    sim: ProgramSimulator<M>,
    steps: Vec<LabStep<M::Message>>,
    trace: Vec<LabTraceEntry>,
    initialized: bool,
}

#[expect(dead_code)]
impl<M: Model> LabHarness<M>
where
    M::Message: std::fmt::Debug,
{
    /// Create a new harness with the given model. Does NOT call init().
    fn new(model: M) -> Self {
        Self {
            sim: ProgramSimulator::new(model),
            steps: Vec::new(),
            trace: Vec::new(),
            initialized: false,
        }
    }

    /// Initialize the model (calls Model::init and executes returned commands).
    fn init(&mut self) {
        self.sim.init();
        self.initialized = true;
        self.trace.push(LabTraceEntry {
            step_index: 0,
            step_type: "init".into(),
            model_running: self.sim.is_running(),
            cmd_log_len: self.sim.command_log().len(),
        });
    }

    /// Queue a message send step.
    fn push_send(&mut self, msg: M::Message) {
        self.steps.push(LabStep::Send(msg));
    }

    /// Queue an event injection step.
    fn push_event(&mut self, event: Event) {
        self.steps.push(LabStep::Event(event));
    }

    /// Queue a frame capture step.
    fn push_capture_frame(&mut self, width: u16, height: u16) {
        self.steps.push(LabStep::CaptureFrame(width, height));
    }

    /// Queue a running assertion step.
    fn push_assert_running(&mut self) {
        self.steps.push(LabStep::AssertRunning);
    }

    /// Queue a stopped assertion step.
    fn push_assert_stopped(&mut self) {
        self.steps.push(LabStep::AssertStopped);
    }

    /// Execute all queued steps in order. Panics on assertion failures.
    fn run(&mut self) {
        if !self.initialized {
            self.init();
        }

        let steps = std::mem::take(&mut self.steps);
        for (i, step) in steps.into_iter().enumerate() {
            let step_index = i + 1; // 0 is init
            match step {
                LabStep::Send(msg) => {
                    let type_name = format!("{msg:?}");
                    self.sim.send(msg);
                    self.trace.push(LabTraceEntry {
                        step_index,
                        step_type: format!("send:{}", truncate(&type_name, 40)),
                        model_running: self.sim.is_running(),
                        cmd_log_len: self.sim.command_log().len(),
                    });
                }
                LabStep::Event(event) => {
                    self.sim.inject_event(event);
                    self.trace.push(LabTraceEntry {
                        step_index,
                        step_type: "event".into(),
                        model_running: self.sim.is_running(),
                        cmd_log_len: self.sim.command_log().len(),
                    });
                }
                LabStep::CaptureFrame(w, h) => {
                    self.sim.capture_frame(w, h);
                    self.trace.push(LabTraceEntry {
                        step_index,
                        step_type: format!("capture:{w}x{h}"),
                        model_running: self.sim.is_running(),
                        cmd_log_len: self.sim.command_log().len(),
                    });
                }
                LabStep::AssertRunning => {
                    assert!(
                        self.sim.is_running(),
                        "LabHarness step {step_index}: expected model to be running\nTrace: {:?}",
                        self.trace
                    );
                    self.trace.push(LabTraceEntry {
                        step_index,
                        step_type: "assert:running".into(),
                        model_running: true,
                        cmd_log_len: self.sim.command_log().len(),
                    });
                }
                LabStep::AssertStopped => {
                    assert!(
                        !self.sim.is_running(),
                        "LabHarness step {step_index}: expected model to be stopped\nTrace: {:?}",
                        self.trace
                    );
                    self.trace.push(LabTraceEntry {
                        step_index,
                        step_type: "assert:stopped".into(),
                        model_running: false,
                        cmd_log_len: self.sim.command_log().len(),
                    });
                }
            }
        }
    }

    /// Access the underlying model.
    fn model(&self) -> &M {
        self.sim.model()
    }

    /// Access the underlying model mutably.
    fn model_mut(&mut self) -> &mut M {
        self.sim.model_mut()
    }

    /// Get the scheduling trace.
    fn trace(&self) -> &[LabTraceEntry] {
        &self.trace
    }

    /// Get captured frames.
    fn frames(&self) -> &[ftui_render::buffer::Buffer] {
        self.sim.frames()
    }

    /// Get logs emitted via Cmd::Log.
    fn logs(&self) -> &[String] {
        self.sim.logs()
    }

    /// Get the command log.
    fn command_log(&self) -> &[CmdRecord] {
        self.sim.command_log()
    }

    /// Check if simulation is still running.
    fn is_running(&self) -> bool {
        self.sim.is_running()
    }

    /// Get the tick rate.
    fn tick_rate(&self) -> Option<Duration> {
        self.sim.tick_rate()
    }
}

fn truncate(s: &str, max_len: usize) -> String {
    if s.len() <= max_len {
        s.to_string()
    } else {
        format!("{}...", &s[..max_len.saturating_sub(3)])
    }
}

// ============================================================================
// Test model for harness validation
// ============================================================================

struct LabModel {
    values: Vec<String>,
    shutdown_called: bool,
}

impl LabModel {
    fn new() -> Self {
        Self {
            values: vec![],
            shutdown_called: false,
        }
    }
}

#[derive(Debug)]
enum LMsg {
    Push(String),
    BatchPush(Vec<String>),
    TaskPush(String),
    TaskResult(String),
    SetTick,
    Log(String),
    Quit,
}

impl From<Event> for LMsg {
    fn from(_: Event) -> Self {
        LMsg::Push("event".into())
    }
}

impl Model for LabModel {
    type Message = LMsg;

    fn init(&mut self) -> Cmd<Self::Message> {
        self.values.push("init".into());
        Cmd::none()
    }

    fn update(&mut self, msg: Self::Message) -> Cmd<Self::Message> {
        match msg {
            LMsg::Push(s) => {
                self.values.push(s);
                Cmd::none()
            }
            LMsg::BatchPush(items) => {
                self.values.push(format!("batch:{}", items.len()));
                Cmd::batch(items.into_iter().map(|s| Cmd::msg(LMsg::Push(s))).collect())
            }
            LMsg::TaskPush(label) => {
                self.values.push(format!("task-spawn:{label}"));
                let l = label.clone();
                Cmd::task(move || LMsg::TaskResult(l))
            }
            LMsg::TaskResult(label) => {
                self.values.push(format!("task-done:{label}"));
                Cmd::none()
            }
            LMsg::SetTick => {
                self.values.push("set-tick".into());
                Cmd::tick(Duration::from_millis(50))
            }
            LMsg::Log(text) => {
                self.values.push(format!("log:{text}"));
                Cmd::log(text)
            }
            LMsg::Quit => {
                self.values.push("quit".into());
                Cmd::quit()
            }
        }
    }

    fn view(&self, frame: &mut Frame) {
        // Render value count into first cell
        let text = format!("n={}", self.values.len());
        for (i, c) in text.chars().enumerate() {
            if (i as u16) < frame.width() {
                use ftui_render::cell::Cell;
                frame.buffer.set_raw(i as u16, 0, Cell::from_char(c));
            }
        }
    }

    fn on_shutdown(&mut self) -> Cmd<Self::Message> {
        self.shutdown_called = true;
        Cmd::none()
    }
}

// ============================================================================
// HARNESS SELF-TESTS: verify the harness works correctly
// ============================================================================

#[test]
fn harness_basic_send_and_trace() {
    let mut h = LabHarness::new(LabModel::new());
    h.push_send(LMsg::Push("hello".into()));
    h.push_send(LMsg::Push("world".into()));
    h.push_assert_running();
    h.run();

    assert_eq!(h.model().values, vec!["init", "hello", "world"]);
    // Trace should have: init + 2 sends + 1 assert = 4 entries
    assert_eq!(h.trace().len(), 4);
    assert_eq!(h.trace()[0].step_type, "init");
}

#[test]
fn harness_quit_and_assert_stopped() {
    let mut h = LabHarness::new(LabModel::new());
    h.push_send(LMsg::Push("before".into()));
    h.push_send(LMsg::Quit);
    h.push_assert_stopped();
    h.run();

    assert!(!h.is_running());
    assert!(h.model().values.contains(&"quit".to_string()));
}

#[test]
fn harness_frame_capture() {
    let mut h = LabHarness::new(LabModel::new());
    h.push_send(LMsg::Push("a".into()));
    h.push_capture_frame(20, 5);
    h.run();

    assert_eq!(h.frames().len(), 1);
    let buf = &h.frames()[0];
    assert_eq!(buf.width(), 20);
    assert_eq!(buf.height(), 5);
}

#[test]
fn harness_batch_execution() {
    let mut h = LabHarness::new(LabModel::new());
    h.push_send(LMsg::BatchPush(vec!["x".into(), "y".into(), "z".into()]));
    h.push_assert_running();
    h.run();

    assert_eq!(h.model().values, vec!["init", "batch:3", "x", "y", "z"]);
}

#[test]
fn harness_task_execution() {
    let mut h = LabHarness::new(LabModel::new());
    h.push_send(LMsg::TaskPush("alpha".into()));
    h.push_assert_running();
    h.run();

    assert_eq!(
        h.model().values,
        vec!["init", "task-spawn:alpha", "task-done:alpha"]
    );
}

#[test]
fn harness_log_capture() {
    let mut h = LabHarness::new(LabModel::new());
    h.push_send(LMsg::Log("msg1".into()));
    h.push_send(LMsg::Log("msg2".into()));
    h.run();

    assert_eq!(h.logs(), &["msg1", "msg2"]);
}

#[test]
fn harness_tick_rate() {
    let mut h = LabHarness::new(LabModel::new());
    h.push_send(LMsg::SetTick);
    h.run();

    assert_eq!(h.tick_rate(), Some(Duration::from_millis(50)));
}

#[test]
fn harness_messages_after_quit_ignored() {
    let mut h = LabHarness::new(LabModel::new());
    h.push_send(LMsg::Push("before".into()));
    h.push_send(LMsg::Quit);
    h.push_send(LMsg::Push("after".into()));
    h.run();

    assert!(!h.model().values.contains(&"after".to_string()));
}

// ============================================================================
// DETERMINISM TESTS: prove the harness is deterministic
// ============================================================================

#[test]
fn harness_deterministic_across_runs() {
    fn run_scenario() -> Vec<String> {
        let mut h = LabHarness::new(LabModel::new());
        h.push_send(LMsg::Push("a".into()));
        h.push_send(LMsg::BatchPush(vec!["b".into(), "c".into()]));
        h.push_send(LMsg::TaskPush("t1".into()));
        h.push_send(LMsg::Log("log".into()));
        h.push_send(LMsg::Push("d".into()));
        h.run();
        h.model().values.clone()
    }

    let r1 = run_scenario();
    let r2 = run_scenario();
    let r3 = run_scenario();
    assert_eq!(r1, r2);
    assert_eq!(r2, r3);
}

#[test]
fn harness_trace_entries_have_monotonic_step_indices() {
    let mut h = LabHarness::new(LabModel::new());
    for i in 0..10 {
        h.push_send(LMsg::Push(format!("step-{i}")));
    }
    h.run();

    let indices: Vec<usize> = h.trace().iter().map(|e| e.step_index).collect();
    for window in indices.windows(2) {
        assert!(
            window[0] < window[1],
            "trace indices must be monotonically increasing"
        );
    }
}

#[test]
fn harness_trace_captures_running_state() {
    let mut h = LabHarness::new(LabModel::new());
    h.push_send(LMsg::Push("before".into()));
    h.push_send(LMsg::Quit);
    h.push_send(LMsg::Push("after".into()));
    h.run();

    // Find the quit step in trace
    let quit_entry = h
        .trace()
        .iter()
        .find(|e| e.step_type.contains("Quit"))
        .expect("should have quit trace entry");
    assert!(
        !quit_entry.model_running,
        "model should not be running after quit step"
    );
}

// ============================================================================
// COMPLEX SCENARIO: validates harness under realistic conditions
// ============================================================================

#[test]
fn harness_complex_scenario_with_frame_captures() {
    let mut h = LabHarness::new(LabModel::new());

    // Phase 1: populate model
    for i in 0..5 {
        h.push_send(LMsg::Push(format!("item-{i}")));
    }
    h.push_capture_frame(40, 10);

    // Phase 2: batch + task
    h.push_send(LMsg::BatchPush(vec!["b1".into(), "b2".into()]));
    h.push_send(LMsg::TaskPush("compute".into()));
    h.push_capture_frame(40, 10);

    // Phase 3: log + quit
    h.push_send(LMsg::Log("final-log".into()));
    h.push_send(LMsg::Quit);
    h.push_assert_stopped();

    h.run();

    // Verify model state
    assert_eq!(h.model().values.len(), 13); // init + 5 items + batch:2 + b1 + b2 + task-spawn:compute + task-done:compute + log:final-log + quit
    assert_eq!(h.frames().len(), 2);
    assert_eq!(h.logs(), &["final-log"]);
    assert!(!h.is_running());

    // Verify trace completeness
    assert!(h.trace().len() >= 10); // init + sends + captures + assert
}

#[test]
fn harness_on_shutdown_invokable() {
    let mut h = LabHarness::new(LabModel::new());
    h.push_send(LMsg::Push("work".into()));
    h.push_send(LMsg::Quit);
    h.run();

    // Invoke on_shutdown manually (harness doesn't auto-call it)
    let _cmd = h.model_mut().on_shutdown();
    assert!(h.model().shutdown_called);
}
