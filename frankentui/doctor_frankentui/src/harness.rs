//! OpenTUI reference execution harness for baseline behavior capture.
//!
//! Executes source OpenTUI fixtures in a controlled harness to capture canonical
//! baseline behavior artifacts (state traces, interaction responses, render
//! outputs).
//!
//! # Design Principles
//!
//! 1. **Deterministic**: same fixture + events always produce identical artifacts.
//! 2. **Scriptable**: event injection with precise timing control.
//! 3. **Consumable**: artifacts use structured formats for downstream comparison.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

// ── Run Configuration ────────────────────────────────────────────────────

/// Configuration for a baseline capture run.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HarnessConfig {
    /// Unique run identifier for traceability.
    pub run_id: String,
    /// Path to the fixture project directory.
    pub fixture_path: PathBuf,
    /// Viewport width in columns.
    pub viewport_width: u16,
    /// Viewport height in rows.
    pub viewport_height: u16,
    /// Maximum run duration before timeout.
    pub timeout: Duration,
    /// Whether to capture render output at each step.
    pub capture_renders: bool,
    /// Whether to capture state traces at each step.
    pub capture_state_traces: bool,
    /// Environment variables to set for the fixture.
    pub env_vars: BTreeMap<String, String>,
    /// Scripted events to inject during the run.
    pub events: Vec<ScriptedEvent>,
}

impl HarnessConfig {
    /// Create a new config with defaults.
    pub fn new(run_id: impl Into<String>, fixture_path: impl Into<PathBuf>) -> Self {
        Self {
            run_id: run_id.into(),
            fixture_path: fixture_path.into(),
            viewport_width: 80,
            viewport_height: 24,
            timeout: Duration::from_secs(30),
            capture_renders: true,
            capture_state_traces: true,
            env_vars: BTreeMap::new(),
            events: Vec::new(),
        }
    }

    /// Set viewport dimensions.
    pub fn with_viewport(mut self, width: u16, height: u16) -> Self {
        self.viewport_width = width;
        self.viewport_height = height;
        self
    }

    /// Set timeout.
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    /// Add a scripted event.
    pub fn with_event(mut self, event: ScriptedEvent) -> Self {
        self.events.push(event);
        self
    }

    /// Add multiple scripted events.
    pub fn with_events(mut self, events: Vec<ScriptedEvent>) -> Self {
        self.events.extend(events);
        self
    }

    /// Set an environment variable.
    pub fn with_env(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.env_vars.insert(key.into(), value.into());
        self
    }
}

// ── Scripted Events ──────────────────────────────────────────────────────

/// A user event to inject at a specific time during the run.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScriptedEvent {
    /// Delay from the start of the run (or from previous event).
    pub delay: Duration,
    /// The event to inject.
    pub kind: EventKind,
    /// Optional label for traceability.
    pub label: Option<String>,
}

impl ScriptedEvent {
    /// Create a new event with delay.
    pub fn new(delay: Duration, kind: EventKind) -> Self {
        Self {
            delay,
            kind,
            label: None,
        }
    }

    /// Add a label.
    pub fn with_label(mut self, label: impl Into<String>) -> Self {
        self.label = Some(label.into());
        self
    }
}

/// Types of events that can be injected.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum EventKind {
    /// Key press event.
    KeyPress { key: String, modifiers: Vec<String> },
    /// Text input (multiple characters).
    TextInput { text: String },
    /// Mouse click at (x, y).
    MouseClick { x: u16, y: u16, button: String },
    /// Mouse scroll.
    MouseScroll { x: u16, y: u16, delta: i16 },
    /// Viewport resize.
    Resize { width: u16, height: u16 },
    /// Wait for a specific condition or timeout.
    WaitForRender { timeout_ms: u64 },
    /// Take a snapshot at this point.
    Snapshot { label: String },
}

// ── Baseline Artifacts ───────────────────────────────────────────────────

/// A captured render frame from the fixture.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CapturedFrame {
    /// Frame index (0-based).
    pub index: usize,
    /// Timestamp offset from run start.
    pub offset_ms: u64,
    /// Viewport width at capture time.
    pub width: u16,
    /// Viewport height at capture time.
    pub height: u16,
    /// The rendered content as a 2D character grid.
    pub content: Vec<String>,
    /// SHA-256 of the frame content.
    pub content_hash: String,
    /// Event that triggered this frame (if any).
    pub trigger_event: Option<String>,
}

/// A state trace entry capturing component state at a point in time.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StateTrace {
    /// Trace index (0-based).
    pub index: usize,
    /// Timestamp offset from run start.
    pub offset_ms: u64,
    /// State snapshot as JSON value.
    pub state: serde_json::Value,
    /// SHA-256 of the serialized state.
    pub state_hash: String,
    /// Label or event that triggered this trace.
    pub trigger: String,
}

/// An interaction response recorded from the fixture.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InteractionResponse {
    /// Which event triggered this response.
    pub event_index: usize,
    /// Event label (if provided).
    pub event_label: Option<String>,
    /// Time from event injection to response capture.
    pub response_time_ms: u64,
    /// Whether the fixture acknowledged the event.
    pub acknowledged: bool,
    /// State delta caused by the event (if captured).
    pub state_delta: Option<serde_json::Value>,
}

// ── Baseline Run Result ──────────────────────────────────────────────────

/// Complete result of a baseline capture run.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BaselineResult {
    /// Run configuration used.
    pub run_id: String,
    /// Fixture path.
    pub fixture_path: String,
    /// Run status.
    pub status: RunStatus,
    /// Total run duration.
    pub duration_ms: u64,
    /// Captured render frames.
    pub frames: Vec<CapturedFrame>,
    /// State traces.
    pub state_traces: Vec<StateTrace>,
    /// Interaction responses.
    pub interactions: Vec<InteractionResponse>,
    /// Overall content hash (hash of all frame hashes).
    pub content_fingerprint: String,
    /// Overall state fingerprint (hash of all state hashes).
    pub state_fingerprint: String,
    /// Run metadata.
    pub metadata: RunMetadata,
}

/// Status of a baseline run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunStatus {
    /// Run completed successfully.
    Completed,
    /// Run timed out.
    TimedOut,
    /// Fixture crashed or errored.
    Failed,
    /// Run was cancelled.
    Cancelled,
}

/// Metadata about the run environment.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunMetadata {
    /// Timestamp when the run started.
    pub started_at: String,
    /// Timestamp when the run finished.
    pub finished_at: String,
    /// Viewport dimensions used.
    pub viewport: String,
    /// Number of events injected.
    pub events_injected: usize,
    /// Number of frames captured.
    pub frames_captured: usize,
    /// Number of state traces captured.
    pub state_traces_captured: usize,
    /// Environment variables (redacted values).
    pub env_vars: BTreeMap<String, String>,
}

impl BaselineResult {
    /// Whether the run completed successfully.
    #[must_use]
    pub fn is_success(&self) -> bool {
        self.status == RunStatus::Completed
    }

    /// Compute a deterministic fingerprint from frames and state.
    pub fn compute_fingerprints(&mut self) {
        let mut hasher = Sha256::new();
        for frame in &self.frames {
            hasher.update(frame.content_hash.as_bytes());
        }
        self.content_fingerprint = hex_encode(&hasher.finalize());

        let mut hasher = Sha256::new();
        for trace in &self.state_traces {
            hasher.update(trace.state_hash.as_bytes());
        }
        self.state_fingerprint = hex_encode(&hasher.finalize());
    }
}

// ── Harness Runner ───────────────────────────────────────────────────────

/// The baseline capture harness.
///
/// This provides the infrastructure for running fixture projects and capturing
/// their behavior artifacts. The actual fixture execution is delegated to
/// the caller via the `FixtureRunner` trait.
pub struct Harness {
    config: HarnessConfig,
}

/// Trait for fixture execution backends.
///
/// Implementors provide the actual mechanism for running a fixture project
/// (e.g., spawning a Node.js process, running a Rust binary, etc.).
pub trait FixtureRunner {
    /// Initialize the fixture with the given config.
    fn init(&mut self, config: &HarnessConfig) -> Result<(), String>;

    /// Send an event to the running fixture.
    fn inject_event(&mut self, event: &EventKind) -> Result<(), String>;

    /// Capture the current render output.
    fn capture_frame(&self) -> Result<Vec<String>, String>;

    /// Capture the current state.
    fn capture_state(&self) -> Result<serde_json::Value, String>;

    /// Check if the fixture is still running.
    fn is_running(&self) -> bool;

    /// Shut down the fixture.
    fn shutdown(&mut self) -> Result<(), String>;
}

impl Harness {
    /// Create a new harness with the given config.
    pub fn new(config: HarnessConfig) -> Self {
        Self { config }
    }

    /// Execute the baseline capture run.
    pub fn run(&self, runner: &mut dyn FixtureRunner) -> BaselineResult {
        let started_at = chrono::Utc::now().to_rfc3339();
        let start_time = std::time::Instant::now();

        let mut frames = Vec::new();
        let mut state_traces = Vec::new();
        let mut interactions = Vec::new();
        let mut status = RunStatus::Completed;

        // Initialize the fixture.
        if let Err(e) = runner.init(&self.config) {
            return self.failed_result(
                &started_at,
                start_time.elapsed(),
                &format!("init failed: {e}"),
            );
        }

        // Capture initial frame.
        if self.config.capture_renders
            && let Ok(content) = runner.capture_frame()
        {
            frames.push(make_frame(0, 0, &self.config, &content, None));
        }

        // Capture initial state.
        if self.config.capture_state_traces
            && let Ok(state) = runner.capture_state()
        {
            state_traces.push(make_state_trace(0, 0, state, "init"));
        }

        // Inject events with timing.
        let mut cumulative_delay = Duration::ZERO;
        for (event_idx, event) in self.config.events.iter().enumerate() {
            cumulative_delay += event.delay;

            // Check timeout.
            if start_time.elapsed() > self.config.timeout {
                status = RunStatus::TimedOut;
                break;
            }

            // Check if fixture is still running.
            if !runner.is_running() {
                status = RunStatus::Failed;
                break;
            }

            // Inject the event.
            let event_start = start_time.elapsed();
            let inject_result = runner.inject_event(&event.kind);
            let response_time = start_time.elapsed() - event_start;

            let acknowledged = inject_result.is_ok();
            let mut state_delta = None;

            // Capture post-event state.
            if self.config.capture_state_traces
                && let Ok(new_state) = runner.capture_state()
            {
                state_delta = Some(new_state.clone());
                state_traces.push(make_state_trace(
                    state_traces.len(),
                    start_time.elapsed().as_millis() as u64,
                    new_state,
                    event
                        .label
                        .as_deref()
                        .unwrap_or(&format!("event_{event_idx}")),
                ));
            }

            // Capture post-event frame.
            if self.config.capture_renders
                && let Ok(content) = runner.capture_frame()
            {
                frames.push(make_frame(
                    frames.len(),
                    start_time.elapsed().as_millis() as u64,
                    &self.config,
                    &content,
                    event.label.as_deref(),
                ));
            }

            interactions.push(InteractionResponse {
                event_index: event_idx,
                event_label: event.label.clone(),
                response_time_ms: response_time.as_millis() as u64,
                acknowledged,
                state_delta,
            });
        }

        // Shutdown.
        let _ = runner.shutdown();

        let finished_at = chrono::Utc::now().to_rfc3339();
        let duration = start_time.elapsed();

        let metadata = RunMetadata {
            started_at: started_at.clone(),
            finished_at,
            viewport: format!(
                "{}x{}",
                self.config.viewport_width, self.config.viewport_height
            ),
            events_injected: interactions.len(),
            frames_captured: frames.len(),
            state_traces_captured: state_traces.len(),
            env_vars: self
                .config
                .env_vars
                .keys()
                .map(|k| (k.clone(), "[redacted]".into()))
                .collect(),
        };

        let mut result = BaselineResult {
            run_id: self.config.run_id.clone(),
            fixture_path: self.config.fixture_path.to_string_lossy().to_string(),
            status,
            duration_ms: duration.as_millis() as u64,
            frames,
            state_traces,
            interactions,
            content_fingerprint: String::new(),
            state_fingerprint: String::new(),
            metadata,
        };
        result.compute_fingerprints();
        result
    }

    fn failed_result(&self, started_at: &str, elapsed: Duration, _error: &str) -> BaselineResult {
        let finished_at = chrono::Utc::now().to_rfc3339();
        BaselineResult {
            run_id: self.config.run_id.clone(),
            fixture_path: self.config.fixture_path.to_string_lossy().to_string(),
            status: RunStatus::Failed,
            duration_ms: elapsed.as_millis() as u64,
            frames: vec![],
            state_traces: vec![],
            interactions: vec![],
            content_fingerprint: String::new(),
            state_fingerprint: String::new(),
            metadata: RunMetadata {
                started_at: started_at.to_string(),
                finished_at,
                viewport: format!(
                    "{}x{}",
                    self.config.viewport_width, self.config.viewport_height
                ),
                events_injected: 0,
                frames_captured: 0,
                state_traces_captured: 0,
                env_vars: BTreeMap::new(),
            },
        }
    }
}

// ── Differential Comparison ──────────────────────────────────────────────

/// Result of comparing two baseline runs.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiffResult {
    /// Run IDs being compared.
    pub baseline_run_id: String,
    pub candidate_run_id: String,
    /// Whether the runs are equivalent.
    pub equivalent: bool,
    /// Frame-level differences.
    pub frame_diffs: Vec<FrameDiff>,
    /// State-level differences.
    pub state_diffs: Vec<StateDiff>,
    /// Summary statistics.
    pub summary: DiffSummary,
}

/// A single frame difference.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FrameDiff {
    pub frame_index: usize,
    pub baseline_hash: String,
    pub candidate_hash: String,
    pub differs: bool,
}

/// A single state difference.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StateDiff {
    pub trace_index: usize,
    pub baseline_hash: String,
    pub candidate_hash: String,
    pub differs: bool,
}

/// Summary of differences.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiffSummary {
    pub total_frames_compared: usize,
    pub frames_identical: usize,
    pub frames_different: usize,
    pub total_states_compared: usize,
    pub states_identical: usize,
    pub states_different: usize,
    pub content_fingerprint_match: bool,
    pub state_fingerprint_match: bool,
}

/// Compare two baseline results for differential analysis.
pub fn compare_baselines(baseline: &BaselineResult, candidate: &BaselineResult) -> DiffResult {
    let max_frames = baseline.frames.len().max(candidate.frames.len());
    let mut frame_diffs = Vec::new();
    let mut frames_identical = 0;
    let mut frames_different = 0;

    for i in 0..max_frames {
        let bh = baseline
            .frames
            .get(i)
            .map(|f| f.content_hash.as_str())
            .unwrap_or("");
        let ch = candidate
            .frames
            .get(i)
            .map(|f| f.content_hash.as_str())
            .unwrap_or("");
        let differs = bh != ch;
        if differs {
            frames_different += 1;
        } else {
            frames_identical += 1;
        }
        frame_diffs.push(FrameDiff {
            frame_index: i,
            baseline_hash: bh.to_string(),
            candidate_hash: ch.to_string(),
            differs,
        });
    }

    let max_states = baseline
        .state_traces
        .len()
        .max(candidate.state_traces.len());
    let mut state_diffs = Vec::new();
    let mut states_identical = 0;
    let mut states_different = 0;

    for i in 0..max_states {
        let bh = baseline
            .state_traces
            .get(i)
            .map(|s| s.state_hash.as_str())
            .unwrap_or("");
        let ch = candidate
            .state_traces
            .get(i)
            .map(|s| s.state_hash.as_str())
            .unwrap_or("");
        let differs = bh != ch;
        if differs {
            states_different += 1;
        } else {
            states_identical += 1;
        }
        state_diffs.push(StateDiff {
            trace_index: i,
            baseline_hash: bh.to_string(),
            candidate_hash: ch.to_string(),
            differs,
        });
    }

    let equivalent = baseline.content_fingerprint == candidate.content_fingerprint
        && baseline.state_fingerprint == candidate.state_fingerprint;

    DiffResult {
        baseline_run_id: baseline.run_id.clone(),
        candidate_run_id: candidate.run_id.clone(),
        equivalent,
        frame_diffs,
        state_diffs,
        summary: DiffSummary {
            total_frames_compared: max_frames,
            frames_identical,
            frames_different,
            total_states_compared: max_states,
            states_identical,
            states_different,
            content_fingerprint_match: baseline.content_fingerprint
                == candidate.content_fingerprint,
            state_fingerprint_match: baseline.state_fingerprint == candidate.state_fingerprint,
        },
    }
}

// ── Helpers ──────────────────────────────────────────────────────────────

fn make_frame(
    index: usize,
    offset_ms: u64,
    config: &HarnessConfig,
    content: &[String],
    trigger: Option<&str>,
) -> CapturedFrame {
    let hash = content_hash(content);
    CapturedFrame {
        index,
        offset_ms,
        width: config.viewport_width,
        height: config.viewport_height,
        content: content.to_vec(),
        content_hash: hash,
        trigger_event: trigger.map(String::from),
    }
}

fn make_state_trace(
    index: usize,
    offset_ms: u64,
    state: serde_json::Value,
    trigger: &str,
) -> StateTrace {
    let serialized = serde_json::to_string(&state).unwrap_or_default();
    let hash = sha256_hex(serialized.as_bytes());
    StateTrace {
        index,
        offset_ms,
        state,
        state_hash: hash,
        trigger: trigger.to_string(),
    }
}

fn content_hash(lines: &[String]) -> String {
    let combined = lines.join("\n");
    sha256_hex(combined.as_bytes())
}

fn sha256_hex(data: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(data);
    hex_encode(&hasher.finalize())
}

fn hex_encode(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

// ══════════════════════════════════════════════════════════════════════════
// Tests
// ══════════════════════════════════════════════════════════════════════════

#[cfg(test)]
mod tests {
    use super::*;

    /// A mock fixture runner for testing.
    struct MockRunner {
        running: bool,
        frame_content: Vec<String>,
        state: serde_json::Value,
        event_log: Vec<String>,
        should_fail_init: bool,
    }

    impl MockRunner {
        fn new() -> Self {
            Self {
                running: false,
                frame_content: vec!["Hello, World!".to_string(), "Line 2".to_string()],
                state: serde_json::json!({"counter": 0, "active": true}),
                event_log: Vec::new(),
                should_fail_init: false,
            }
        }

        fn with_fail_init(mut self) -> Self {
            self.should_fail_init = true;
            self
        }
    }

    impl FixtureRunner for MockRunner {
        fn init(&mut self, _config: &HarnessConfig) -> Result<(), String> {
            if self.should_fail_init {
                return Err("mock init failure".into());
            }
            self.running = true;
            Ok(())
        }

        fn inject_event(&mut self, event: &EventKind) -> Result<(), String> {
            match event {
                EventKind::KeyPress { key, .. } => {
                    self.event_log.push(format!("key:{key}"));
                    // Simulate state change on key press.
                    if let Some(counter) = self.state.get_mut("counter") {
                        *counter = serde_json::json!(counter.as_i64().unwrap_or(0) + 1);
                    }
                    self.frame_content = vec![
                        format!("Counter: {}", self.state["counter"]),
                        "Updated".to_string(),
                    ];
                }
                EventKind::TextInput { text } => {
                    self.event_log.push(format!("text:{text}"));
                }
                EventKind::Resize { width, height } => {
                    self.event_log.push(format!("resize:{width}x{height}"));
                }
                EventKind::Snapshot { label } => {
                    self.event_log.push(format!("snapshot:{label}"));
                }
                _ => {}
            }
            Ok(())
        }

        fn capture_frame(&self) -> Result<Vec<String>, String> {
            Ok(self.frame_content.clone())
        }

        fn capture_state(&self) -> Result<serde_json::Value, String> {
            Ok(self.state.clone())
        }

        fn is_running(&self) -> bool {
            self.running
        }

        fn shutdown(&mut self) -> Result<(), String> {
            self.running = false;
            Ok(())
        }
    }

    // ── Config ───────────────────────────────────────────────────────────

    #[test]
    fn config_defaults() {
        let config = HarnessConfig::new("test-run", "/fixtures/counter");
        assert_eq!(config.run_id, "test-run");
        assert_eq!(config.viewport_width, 80);
        assert_eq!(config.viewport_height, 24);
        assert_eq!(config.timeout, Duration::from_secs(30));
        assert!(config.capture_renders);
        assert!(config.capture_state_traces);
        assert!(config.events.is_empty());
    }

    #[test]
    fn config_builder_pattern() {
        let config = HarnessConfig::new("run-1", "/fix")
            .with_viewport(120, 40)
            .with_timeout(Duration::from_secs(60))
            .with_env("NODE_ENV", "test")
            .with_event(ScriptedEvent::new(
                Duration::from_millis(100),
                EventKind::KeyPress {
                    key: "Enter".into(),
                    modifiers: vec![],
                },
            ));
        assert_eq!(config.viewport_width, 120);
        assert_eq!(config.viewport_height, 40);
        assert_eq!(config.timeout, Duration::from_secs(60));
        assert_eq!(config.env_vars["NODE_ENV"], "test");
        assert_eq!(config.events.len(), 1);
    }

    #[test]
    fn config_serializes_to_json() {
        let config = HarnessConfig::new("run-1", "/fixtures/app");
        let json = serde_json::to_string(&config).unwrap();
        let parsed: HarnessConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.run_id, "run-1");
    }

    // ── Event types ──────────────────────────────────────────────────────

    #[test]
    fn event_kind_serialization() {
        let events = vec![
            EventKind::KeyPress {
                key: "Enter".into(),
                modifiers: vec!["Ctrl".into()],
            },
            EventKind::TextInput {
                text: "hello".into(),
            },
            EventKind::MouseClick {
                x: 10,
                y: 5,
                button: "left".into(),
            },
            EventKind::Resize {
                width: 120,
                height: 40,
            },
            EventKind::Snapshot {
                label: "after_init".into(),
            },
        ];

        for event in events {
            let json = serde_json::to_string(&event).unwrap();
            let _: EventKind = serde_json::from_str(&json).unwrap();
        }
    }

    #[test]
    fn scripted_event_with_label() {
        let event = ScriptedEvent::new(
            Duration::from_millis(500),
            EventKind::KeyPress {
                key: "a".into(),
                modifiers: vec![],
            },
        )
        .with_label("type_a");
        assert_eq!(event.label.as_deref(), Some("type_a"));
    }

    // ── Harness run ──────────────────────────────────────────────────────

    #[test]
    fn basic_run_captures_initial_frame_and_state() {
        let config = HarnessConfig::new("test-1", "/fixture");
        let harness = Harness::new(config);
        let mut runner = MockRunner::new();

        let result = harness.run(&mut runner);
        assert_eq!(result.status, RunStatus::Completed);
        assert_eq!(result.run_id, "test-1");
        assert!(!result.frames.is_empty());
        assert!(!result.state_traces.is_empty());
        assert!(!result.content_fingerprint.is_empty());
        assert!(!result.state_fingerprint.is_empty());
    }

    #[test]
    fn run_with_events_captures_responses() {
        let config = HarnessConfig::new("test-2", "/fixture")
            .with_event(
                ScriptedEvent::new(
                    Duration::from_millis(10),
                    EventKind::KeyPress {
                        key: "Enter".into(),
                        modifiers: vec![],
                    },
                )
                .with_label("press_enter"),
            )
            .with_event(ScriptedEvent::new(
                Duration::from_millis(10),
                EventKind::TextInput {
                    text: "hello".into(),
                },
            ));

        let harness = Harness::new(config);
        let mut runner = MockRunner::new();

        let result = harness.run(&mut runner);
        assert_eq!(result.status, RunStatus::Completed);
        assert_eq!(result.interactions.len(), 2);

        // First interaction should be acknowledged.
        assert!(result.interactions[0].acknowledged);
        assert_eq!(
            result.interactions[0].event_label.as_deref(),
            Some("press_enter")
        );
    }

    #[test]
    fn run_failed_init_returns_failed_status() {
        let config = HarnessConfig::new("test-fail", "/fixture");
        let harness = Harness::new(config);
        let mut runner = MockRunner::new().with_fail_init();

        let result = harness.run(&mut runner);
        assert_eq!(result.status, RunStatus::Failed);
        assert!(result.frames.is_empty());
        assert!(result.state_traces.is_empty());
    }

    #[test]
    fn run_metadata_is_populated() {
        let config = HarnessConfig::new("test-meta", "/fixture")
            .with_viewport(120, 40)
            .with_env("TERM", "xterm-256color");

        let harness = Harness::new(config);
        let mut runner = MockRunner::new();

        let result = harness.run(&mut runner);
        assert_eq!(result.metadata.viewport, "120x40");
        assert!(!result.metadata.started_at.is_empty());
        assert!(!result.metadata.finished_at.is_empty());
        // Env vars should be redacted.
        assert_eq!(result.metadata.env_vars["TERM"], "[redacted]");
    }

    // ── Determinism ──────────────────────────────────────────────────────

    #[test]
    fn identical_runs_produce_same_fingerprints() {
        let make_config = || {
            HarnessConfig::new("det-test", "/fixture").with_event(ScriptedEvent::new(
                Duration::from_millis(10),
                EventKind::KeyPress {
                    key: "a".into(),
                    modifiers: vec![],
                },
            ))
        };

        let harness1 = Harness::new(make_config());
        let mut runner1 = MockRunner::new();
        let result1 = harness1.run(&mut runner1);

        let harness2 = Harness::new(make_config());
        let mut runner2 = MockRunner::new();
        let result2 = harness2.run(&mut runner2);

        assert_eq!(result1.content_fingerprint, result2.content_fingerprint);
        assert_eq!(result1.state_fingerprint, result2.state_fingerprint);
    }

    // ── Differential comparison ──────────────────────────────────────────

    #[test]
    fn compare_identical_baselines_shows_equivalent() {
        let config = HarnessConfig::new("base", "/fixture");
        let harness = Harness::new(config);
        let mut runner = MockRunner::new();
        let baseline = harness.run(&mut runner);

        let config2 = HarnessConfig::new("cand", "/fixture");
        let harness2 = Harness::new(config2);
        let mut runner2 = MockRunner::new();
        let candidate = harness2.run(&mut runner2);

        let diff = compare_baselines(&baseline, &candidate);
        assert!(diff.equivalent);
        assert_eq!(diff.summary.frames_different, 0);
        assert_eq!(diff.summary.states_different, 0);
        assert!(diff.summary.content_fingerprint_match);
        assert!(diff.summary.state_fingerprint_match);
    }

    #[test]
    fn compare_different_baselines_shows_differences() {
        let config1 = HarnessConfig::new("base", "/fixture");
        let harness1 = Harness::new(config1);
        let mut runner1 = MockRunner::new();
        let baseline = harness1.run(&mut runner1);

        // Create a different run with an event that changes state.
        let config2 = HarnessConfig::new("cand", "/fixture").with_event(ScriptedEvent::new(
            Duration::from_millis(10),
            EventKind::KeyPress {
                key: "a".into(),
                modifiers: vec![],
            },
        ));
        let harness2 = Harness::new(config2);
        let mut runner2 = MockRunner::new();
        let candidate = harness2.run(&mut runner2);

        let diff = compare_baselines(&baseline, &candidate);
        // They should differ because the candidate has more frames/states.
        assert!(!diff.equivalent);
    }

    #[test]
    fn diff_result_serializes_to_json() {
        let config = HarnessConfig::new("base", "/fixture");
        let harness = Harness::new(config);
        let mut runner = MockRunner::new();
        let baseline = harness.run(&mut runner);

        let diff = compare_baselines(&baseline, &baseline);
        let json = serde_json::to_string_pretty(&diff).unwrap();
        let parsed: DiffResult = serde_json::from_str(&json).unwrap();
        assert!(parsed.equivalent);
    }

    // ── Frame and state hashing ──────────────────────────────────────────

    #[test]
    fn frame_content_hash_is_deterministic() {
        let lines = vec!["Hello".to_string(), "World".to_string()];
        let h1 = content_hash(&lines);
        let h2 = content_hash(&lines);
        assert_eq!(h1, h2);
        assert_eq!(h1.len(), 64);
    }

    #[test]
    fn different_content_produces_different_hash() {
        let h1 = content_hash(&["Hello".to_string()]);
        let h2 = content_hash(&["World".to_string()]);
        assert_ne!(h1, h2);
    }

    // ── Result serialization ─────────────────────────────────────────────

    #[test]
    fn baseline_result_serializes_to_json() {
        let config = HarnessConfig::new("json-test", "/fixture");
        let harness = Harness::new(config);
        let mut runner = MockRunner::new();
        let result = harness.run(&mut runner);

        let json = serde_json::to_string_pretty(&result).unwrap();
        let parsed: BaselineResult = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.run_id, "json-test");
        assert!(parsed.is_success());
    }

    #[test]
    fn run_status_serialization() {
        for status in [
            RunStatus::Completed,
            RunStatus::TimedOut,
            RunStatus::Failed,
            RunStatus::Cancelled,
        ] {
            let json = serde_json::to_string(&status).unwrap();
            let parsed: RunStatus = serde_json::from_str(&json).unwrap();
            assert_eq!(parsed, status);
        }
    }

    // ── Env var redaction in metadata ─────────────────────────────────────

    #[test]
    fn metadata_redacts_env_var_values() {
        let config = HarnessConfig::new("env-test", "/fixture")
            .with_env("SECRET_KEY", "super-secret-value")
            .with_env("API_TOKEN", "tok_abc123");

        let harness = Harness::new(config);
        let mut runner = MockRunner::new();
        let result = harness.run(&mut runner);

        for value in result.metadata.env_vars.values() {
            assert_eq!(value, "[redacted]", "env var values must be redacted");
        }
        assert!(result.metadata.env_vars.contains_key("SECRET_KEY"));
        assert!(result.metadata.env_vars.contains_key("API_TOKEN"));
    }
}
