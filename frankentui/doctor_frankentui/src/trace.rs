//! Canonical interaction trace capture and replay format.
//!
//! Defines a versioned interaction trace schema usable for both source and
//! translated app replay, with deterministic scheduling and drift detection.
//!
//! # Trace Format
//!
//! A trace is a timestamped sequence of `TraceEvent` entries, each carrying:
//! - A monotonic offset from trace start
//! - An event payload (keyboard, mouse, resize, render, state)
//! - Optional metadata (label, component target, etc.)
//!
//! # Replay
//!
//! The `TraceReplayer` consumes a trace and injects events with original timing,
//! detecting drift when actual response times diverge from recorded baselines.

use std::collections::BTreeMap;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

// ── Schema Version ───────────────────────────────────────────────────────

/// Current trace schema version. Increment on breaking changes.
pub const TRACE_SCHEMA_VERSION: &str = "interaction-trace-v1";

// ── Trace Types ──────────────────────────────────────────────────────────

/// A complete interaction trace recording.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InteractionTrace {
    /// Schema version for forward compatibility.
    pub schema_version: String,
    /// Unique trace identifier.
    pub trace_id: String,
    /// Run ID this trace belongs to.
    pub run_id: String,
    /// Viewport dimensions at trace start.
    pub initial_viewport: Viewport,
    /// Ordered sequence of trace events.
    pub events: Vec<TraceEvent>,
    /// Total duration of the trace.
    pub duration_ms: u64,
    /// SHA-256 of the serialized event sequence (for integrity checking).
    pub events_hash: String,
    /// Optional metadata.
    pub metadata: BTreeMap<String, String>,
}

/// Viewport dimensions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Viewport {
    pub width: u16,
    pub height: u16,
}

/// A single timestamped event in the trace.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TraceEvent {
    /// Monotonic offset from trace start.
    pub offset_ms: u64,
    /// Sequential index in the trace.
    pub sequence: u32,
    /// The event payload.
    pub payload: TracePayload,
    /// Optional human-readable label.
    pub label: Option<String>,
}

/// Event payload variants.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum TracePayload {
    /// Keyboard event.
    Key {
        key: String,
        modifiers: Vec<String>,
        action: KeyAction,
    },
    /// Text input (multiple characters, e.g. paste).
    TextInput { text: String },
    /// Mouse event.
    Mouse {
        x: u16,
        y: u16,
        button: MouseButton,
        action: MouseAction,
    },
    /// Mouse scroll event.
    Scroll {
        x: u16,
        y: u16,
        delta_x: i16,
        delta_y: i16,
    },
    /// Viewport resize event.
    Resize { width: u16, height: u16 },
    /// Render frame capture (output event, not input).
    RenderCapture {
        frame_index: u32,
        content_hash: String,
    },
    /// State snapshot capture (output event, not input).
    StateCapture {
        state_hash: String,
        component: Option<String>,
    },
    /// Marker event for replay synchronization.
    Marker { name: String },
}

/// Key press/release action.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum KeyAction {
    Press,
    Release,
    Repeat,
}

/// Mouse button identifier.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MouseButton {
    Left,
    Right,
    Middle,
    None,
}

/// Mouse action type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MouseAction {
    Press,
    Release,
    Move,
    Drag,
}

// ── Trace Builder ────────────────────────────────────────────────────────

/// Builder for constructing interaction traces.
pub struct TraceBuilder {
    trace_id: String,
    run_id: String,
    viewport: Viewport,
    events: Vec<TraceEvent>,
    metadata: BTreeMap<String, String>,
    sequence: u32,
}

impl TraceBuilder {
    /// Create a new trace builder.
    pub fn new(trace_id: impl Into<String>, run_id: impl Into<String>, viewport: Viewport) -> Self {
        Self {
            trace_id: trace_id.into(),
            run_id: run_id.into(),
            viewport,
            events: Vec::new(),
            metadata: BTreeMap::new(),
            sequence: 0,
        }
    }

    /// Add a metadata key-value pair.
    pub fn with_metadata(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.metadata.insert(key.into(), value.into());
        self
    }

    /// Record an event at the given offset.
    pub fn record(&mut self, offset_ms: u64, payload: TracePayload) -> &mut Self {
        self.record_labeled(offset_ms, payload, None)
    }

    /// Record a labeled event at the given offset.
    pub fn record_labeled(
        &mut self,
        offset_ms: u64,
        payload: TracePayload,
        label: Option<String>,
    ) -> &mut Self {
        let event = TraceEvent {
            offset_ms,
            sequence: self.sequence,
            payload,
            label,
        };
        self.events.push(event);
        self.sequence += 1;
        self
    }

    /// Finalize and build the trace.
    pub fn build(self) -> InteractionTrace {
        let duration_ms = self.events.last().map_or(0, |e| e.offset_ms);
        let events_hash = compute_events_hash(&self.events);

        InteractionTrace {
            schema_version: TRACE_SCHEMA_VERSION.to_string(),
            trace_id: self.trace_id,
            run_id: self.run_id,
            initial_viewport: self.viewport,
            events: self.events,
            duration_ms,
            events_hash,
            metadata: self.metadata,
        }
    }
}

// ── Trace Validation ─────────────────────────────────────────────────────

/// Errors that can occur during trace validation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TraceValidationError {
    /// Schema version mismatch.
    SchemaVersionMismatch { expected: String, actual: String },
    /// Events are not monotonically ordered by offset.
    NonMonotonicTimestamps { at_sequence: u32 },
    /// Sequence numbers are not contiguous.
    NonContiguousSequence { expected: u32, actual: u32 },
    /// Events hash does not match.
    IntegrityCheckFailed { expected: String, actual: String },
    /// Trace has no events.
    EmptyTrace,
}

impl std::fmt::Display for TraceValidationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::SchemaVersionMismatch { expected, actual } => {
                write!(
                    f,
                    "schema version mismatch: expected {expected}, got {actual}"
                )
            }
            Self::NonMonotonicTimestamps { at_sequence } => {
                write!(f, "non-monotonic timestamps at sequence {at_sequence}")
            }
            Self::NonContiguousSequence { expected, actual } => {
                write!(
                    f,
                    "non-contiguous sequence: expected {expected}, got {actual}"
                )
            }
            Self::IntegrityCheckFailed { expected, actual } => {
                write!(
                    f,
                    "integrity check failed: expected {expected}, got {actual}"
                )
            }
            Self::EmptyTrace => write!(f, "trace contains no events"),
        }
    }
}

/// Validate a trace for structural correctness.
pub fn validate_trace(trace: &InteractionTrace) -> Result<(), TraceValidationError> {
    // Check schema version.
    if trace.schema_version != TRACE_SCHEMA_VERSION {
        return Err(TraceValidationError::SchemaVersionMismatch {
            expected: TRACE_SCHEMA_VERSION.to_string(),
            actual: trace.schema_version.clone(),
        });
    }

    // Check non-empty.
    if trace.events.is_empty() {
        return Err(TraceValidationError::EmptyTrace);
    }

    // Check monotonic timestamps and contiguous sequences.
    let mut prev_offset = 0u64;
    for (i, event) in trace.events.iter().enumerate() {
        let expected_seq = i as u32;
        if event.sequence != expected_seq {
            return Err(TraceValidationError::NonContiguousSequence {
                expected: expected_seq,
                actual: event.sequence,
            });
        }
        if event.offset_ms < prev_offset {
            return Err(TraceValidationError::NonMonotonicTimestamps {
                at_sequence: event.sequence,
            });
        }
        prev_offset = event.offset_ms;
    }

    // Check integrity hash.
    let computed_hash = compute_events_hash(&trace.events);
    if computed_hash != trace.events_hash {
        return Err(TraceValidationError::IntegrityCheckFailed {
            expected: trace.events_hash.clone(),
            actual: computed_hash,
        });
    }

    Ok(())
}

// ── Replay ───────────────────────────────────────────────────────────────

/// Configuration for trace replay.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReplayConfig {
    /// Maximum allowed drift between expected and actual timing.
    pub max_drift_ms: u64,
    /// Whether to fail on drift exceeding the threshold.
    pub fail_on_drift: bool,
    /// Speed multiplier (1.0 = real-time, 2.0 = 2x speed).
    pub speed_factor: f64,
    /// Whether to inject render captures as verification checkpoints.
    pub verify_renders: bool,
}

impl Default for ReplayConfig {
    fn default() -> Self {
        Self {
            max_drift_ms: 50,
            fail_on_drift: false,
            speed_factor: 1.0,
            verify_renders: true,
        }
    }
}

/// A scheduled replay step.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReplayStep {
    /// The trace event to replay.
    pub event: TraceEvent,
    /// Scheduled delay from the previous step (adjusted by speed factor).
    pub scheduled_delay: Duration,
    /// Whether this is an input event (key, mouse, text, resize) or
    /// an output checkpoint (render, state).
    pub is_input: bool,
}

/// Result of a replay run.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReplayResult {
    /// Trace ID that was replayed.
    pub trace_id: String,
    /// Whether replay completed successfully.
    pub success: bool,
    /// Total steps executed.
    pub steps_executed: usize,
    /// Total steps in the trace.
    pub steps_total: usize,
    /// Drift measurements per step.
    pub drift_log: Vec<DriftEntry>,
    /// Maximum observed drift.
    pub max_drift_ms: u64,
    /// Average drift.
    pub avg_drift_ms: f64,
    /// Number of steps that exceeded the drift threshold.
    pub drift_violations: usize,
}

/// A single drift measurement during replay.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DriftEntry {
    /// Event sequence number.
    pub sequence: u32,
    /// Expected offset from trace.
    pub expected_ms: u64,
    /// Actual offset during replay.
    pub actual_ms: u64,
    /// Drift (actual - expected).
    pub drift_ms: i64,
    /// Whether this exceeded the threshold.
    pub exceeded_threshold: bool,
}

/// Plan a replay from a trace.
pub fn plan_replay(trace: &InteractionTrace, config: &ReplayConfig) -> Vec<ReplayStep> {
    let mut steps = Vec::with_capacity(trace.events.len());
    let mut prev_offset_ms = 0u64;

    for event in &trace.events {
        let delta_ms = event.offset_ms.saturating_sub(prev_offset_ms);
        let adjusted_ms = if config.speed_factor > 0.0 {
            (delta_ms as f64 / config.speed_factor) as u64
        } else {
            delta_ms
        };

        let is_input = matches!(
            event.payload,
            TracePayload::Key { .. }
                | TracePayload::TextInput { .. }
                | TracePayload::Mouse { .. }
                | TracePayload::Scroll { .. }
                | TracePayload::Resize { .. }
        );

        steps.push(ReplayStep {
            event: event.clone(),
            scheduled_delay: Duration::from_millis(adjusted_ms),
            is_input,
        });

        prev_offset_ms = event.offset_ms;
    }

    steps
}

/// Simulate a replay and compute drift measurements.
///
/// This is a dry-run simulation that computes what the drift would be
/// given a set of actual timestamps. Real replay would use `plan_replay`
/// with actual event injection.
pub fn compute_replay_drift(
    trace: &InteractionTrace,
    actual_offsets_ms: &[u64],
    config: &ReplayConfig,
) -> ReplayResult {
    let mut drift_log = Vec::new();
    let mut max_drift: u64 = 0;
    let mut total_drift: i64 = 0;
    let mut violations = 0;
    let mut success = true;

    let steps = actual_offsets_ms.len().min(trace.events.len());

    for (trace_event, &actual) in trace
        .events
        .iter()
        .zip(actual_offsets_ms.iter())
        .take(steps)
    {
        let expected = trace_event.offset_ms;
        let drift = actual as i64 - expected as i64;
        let abs_drift = drift.unsigned_abs();

        let exceeded = abs_drift > config.max_drift_ms;
        if exceeded {
            violations += 1;
            if config.fail_on_drift {
                success = false;
            }
        }

        max_drift = max_drift.max(abs_drift);
        total_drift += drift.abs();

        drift_log.push(DriftEntry {
            sequence: trace_event.sequence,
            expected_ms: expected,
            actual_ms: actual,
            drift_ms: drift,
            exceeded_threshold: exceeded,
        });
    }

    let avg_drift = if steps > 0 {
        total_drift as f64 / steps as f64
    } else {
        0.0
    };

    ReplayResult {
        trace_id: trace.trace_id.clone(),
        success,
        steps_executed: steps,
        steps_total: trace.events.len(),
        drift_log,
        max_drift_ms: max_drift,
        avg_drift_ms: avg_drift,
        drift_violations: violations,
    }
}

// ── Helpers ──────────────────────────────────────────────────────────────

fn compute_events_hash(events: &[TraceEvent]) -> String {
    let serialized = serde_json::to_string(events).unwrap_or_default();
    sha256_hex(serialized.as_bytes())
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

    fn sample_viewport() -> Viewport {
        Viewport {
            width: 80,
            height: 24,
        }
    }

    fn sample_trace() -> InteractionTrace {
        let mut builder = TraceBuilder::new("trace-1", "run-1", sample_viewport());
        builder.record(
            0,
            TracePayload::Marker {
                name: "start".into(),
            },
        );
        builder.record(
            100,
            TracePayload::Key {
                key: "a".into(),
                modifiers: vec![],
                action: KeyAction::Press,
            },
        );
        builder.record(
            200,
            TracePayload::TextInput {
                text: "hello".into(),
            },
        );
        builder.record(
            300,
            TracePayload::Mouse {
                x: 10,
                y: 5,
                button: MouseButton::Left,
                action: MouseAction::Press,
            },
        );
        builder.record(
            400,
            TracePayload::Resize {
                width: 120,
                height: 40,
            },
        );
        builder.record(
            500,
            TracePayload::RenderCapture {
                frame_index: 0,
                content_hash: "abc123".into(),
            },
        );
        builder.record(
            600,
            TracePayload::StateCapture {
                state_hash: "def456".into(),
                component: Some("counter".into()),
            },
        );
        builder.build()
    }

    // ── Schema ───────────────────────────────────────────────────────────

    #[test]
    fn schema_version_is_set() {
        let trace = sample_trace();
        assert_eq!(trace.schema_version, TRACE_SCHEMA_VERSION);
    }

    #[test]
    fn trace_has_correct_event_count() {
        let trace = sample_trace();
        assert_eq!(trace.events.len(), 7);
    }

    #[test]
    fn trace_duration_is_last_event_offset() {
        let trace = sample_trace();
        assert_eq!(trace.duration_ms, 600);
    }

    // ── Builder ──────────────────────────────────────────────────────────

    #[test]
    fn builder_assigns_sequential_sequence_numbers() {
        let trace = sample_trace();
        for (i, event) in trace.events.iter().enumerate() {
            assert_eq!(event.sequence, i as u32);
        }
    }

    #[test]
    fn builder_with_metadata() {
        let trace = TraceBuilder::new("t-1", "r-1", sample_viewport())
            .with_metadata("source", "original")
            .with_metadata("fixture", "counter-app")
            .build();
        // Empty trace is fine for metadata testing — build succeeds.
        assert_eq!(trace.metadata["source"], "original");
        assert_eq!(trace.metadata["fixture"], "counter-app");
    }

    #[test]
    fn builder_labeled_events() {
        let mut builder = TraceBuilder::new("t-1", "r-1", sample_viewport());
        builder.record_labeled(
            0,
            TracePayload::Key {
                key: "Enter".into(),
                modifiers: vec![],
                action: KeyAction::Press,
            },
            Some("submit_form".into()),
        );
        let trace = builder.build();
        assert_eq!(trace.events[0].label.as_deref(), Some("submit_form"));
    }

    // ── Serialization ────────────────────────────────────────────────────

    #[test]
    fn trace_round_trips_through_json() {
        let trace = sample_trace();
        let json = serde_json::to_string_pretty(&trace).unwrap();
        let parsed: InteractionTrace = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.trace_id, trace.trace_id);
        assert_eq!(parsed.events.len(), trace.events.len());
        assert_eq!(parsed.events_hash, trace.events_hash);
    }

    #[test]
    fn payload_variants_serialize_with_type_tag() {
        let payloads = vec![
            TracePayload::Key {
                key: "a".into(),
                modifiers: vec!["ctrl".into()],
                action: KeyAction::Press,
            },
            TracePayload::TextInput { text: "hi".into() },
            TracePayload::Mouse {
                x: 1,
                y: 2,
                button: MouseButton::Left,
                action: MouseAction::Press,
            },
            TracePayload::Scroll {
                x: 0,
                y: 0,
                delta_x: 0,
                delta_y: -3,
            },
            TracePayload::Resize {
                width: 120,
                height: 40,
            },
            TracePayload::RenderCapture {
                frame_index: 0,
                content_hash: "hash".into(),
            },
            TracePayload::StateCapture {
                state_hash: "hash".into(),
                component: None,
            },
            TracePayload::Marker {
                name: "checkpoint".into(),
            },
        ];

        for payload in payloads {
            let json = serde_json::to_string(&payload).unwrap();
            assert!(
                json.contains("\"type\""),
                "payload must have type tag: {json}"
            );
            let _: TracePayload = serde_json::from_str(&json).unwrap();
        }
    }

    #[test]
    fn enum_variants_serialize_correctly() {
        assert_eq!(
            serde_json::to_string(&KeyAction::Press).unwrap(),
            "\"press\""
        );
        assert_eq!(
            serde_json::to_string(&MouseButton::Left).unwrap(),
            "\"left\""
        );
        assert_eq!(
            serde_json::to_string(&MouseAction::Drag).unwrap(),
            "\"drag\""
        );
    }

    // ── Validation ───────────────────────────────────────────────────────

    #[test]
    fn valid_trace_passes_validation() {
        let trace = sample_trace();
        assert!(validate_trace(&trace).is_ok());
    }

    #[test]
    fn wrong_schema_version_fails_validation() {
        let mut trace = sample_trace();
        trace.schema_version = "wrong-v99".into();
        let err = validate_trace(&trace).unwrap_err();
        assert!(matches!(
            err,
            TraceValidationError::SchemaVersionMismatch { .. }
        ));
    }

    #[test]
    fn empty_trace_fails_validation() {
        let trace = TraceBuilder::new("t-1", "r-1", sample_viewport()).build();
        let err = validate_trace(&trace).unwrap_err();
        assert!(matches!(err, TraceValidationError::EmptyTrace));
    }

    #[test]
    fn non_monotonic_timestamps_fail_validation() {
        let mut trace = sample_trace();
        // Swap timestamps to create non-monotonic order.
        trace.events[2].offset_ms = 50; // Was 200, now before event[1]=100.
        // Recompute hash so integrity check doesn't fail first.
        trace.events_hash = compute_events_hash(&trace.events);
        let err = validate_trace(&trace).unwrap_err();
        assert!(matches!(
            err,
            TraceValidationError::NonMonotonicTimestamps { .. }
        ));
    }

    #[test]
    fn non_contiguous_sequence_fails_validation() {
        let mut trace = sample_trace();
        trace.events[1].sequence = 99;
        trace.events_hash = compute_events_hash(&trace.events);
        let err = validate_trace(&trace).unwrap_err();
        assert!(matches!(
            err,
            TraceValidationError::NonContiguousSequence { .. }
        ));
    }

    #[test]
    fn tampered_hash_fails_validation() {
        let mut trace = sample_trace();
        trace.events_hash =
            "0000000000000000000000000000000000000000000000000000000000000000".into();
        let err = validate_trace(&trace).unwrap_err();
        assert!(matches!(
            err,
            TraceValidationError::IntegrityCheckFailed { .. }
        ));
    }

    // ── Events hash determinism ──────────────────────────────────────────

    #[test]
    fn events_hash_is_deterministic() {
        let t1 = sample_trace();
        let t2 = sample_trace();
        assert_eq!(t1.events_hash, t2.events_hash);
    }

    // ── Replay planning ──────────────────────────────────────────────────

    #[test]
    fn plan_replay_produces_correct_steps() {
        let trace = sample_trace();
        let config = ReplayConfig::default();
        let steps = plan_replay(&trace, &config);

        assert_eq!(steps.len(), trace.events.len());

        // First step has 0 delay.
        assert_eq!(steps[0].scheduled_delay, Duration::ZERO);

        // Second step has 100ms delay (events at 0 and 100).
        assert_eq!(steps[1].scheduled_delay, Duration::from_millis(100));

        // Input events should be marked as input.
        assert!(!steps[0].is_input); // Marker
        assert!(steps[1].is_input); // Key
        assert!(steps[2].is_input); // TextInput
        assert!(steps[3].is_input); // Mouse
        assert!(steps[4].is_input); // Resize
        assert!(!steps[5].is_input); // RenderCapture
        assert!(!steps[6].is_input); // StateCapture
    }

    #[test]
    fn plan_replay_applies_speed_factor() {
        let trace = sample_trace();
        let config = ReplayConfig {
            speed_factor: 2.0,
            ..Default::default()
        };
        let steps = plan_replay(&trace, &config);

        // At 2x speed, 100ms delay becomes 50ms.
        assert_eq!(steps[1].scheduled_delay, Duration::from_millis(50));
    }

    // ── Drift detection ──────────────────────────────────────────────────

    #[test]
    fn perfect_replay_has_zero_drift() {
        let trace = sample_trace();
        let actual: Vec<u64> = trace.events.iter().map(|e| e.offset_ms).collect();
        let config = ReplayConfig::default();
        let result = compute_replay_drift(&trace, &actual, &config);

        assert!(result.success);
        assert_eq!(result.max_drift_ms, 0);
        assert_eq!(result.drift_violations, 0);
    }

    #[test]
    fn drift_within_threshold_succeeds() {
        let trace = sample_trace();
        // Add small drift (within 50ms default threshold).
        let actual: Vec<u64> = trace.events.iter().map(|e| e.offset_ms + 10).collect();
        let config = ReplayConfig::default();
        let result = compute_replay_drift(&trace, &actual, &config);

        assert!(result.success);
        assert_eq!(result.max_drift_ms, 10);
        assert_eq!(result.drift_violations, 0);
    }

    #[test]
    fn drift_exceeding_threshold_is_detected() {
        let trace = sample_trace();
        let mut actual: Vec<u64> = trace.events.iter().map(|e| e.offset_ms).collect();
        actual[3] += 100; // Add 100ms drift to one event.
        let config = ReplayConfig {
            max_drift_ms: 50,
            fail_on_drift: true,
            ..Default::default()
        };
        let result = compute_replay_drift(&trace, &actual, &config);

        assert!(!result.success);
        assert!(result.drift_violations > 0);
        assert!(result.max_drift_ms >= 100);
    }

    #[test]
    fn drift_detection_with_fail_on_drift_disabled() {
        let trace = sample_trace();
        let mut actual: Vec<u64> = trace.events.iter().map(|e| e.offset_ms).collect();
        actual[3] += 100;
        let config = ReplayConfig {
            max_drift_ms: 50,
            fail_on_drift: false,
            ..Default::default()
        };
        let result = compute_replay_drift(&trace, &actual, &config);

        // Should still succeed even with drift.
        assert!(result.success);
        assert!(result.drift_violations > 0);
    }

    // ── Replay result ────────────────────────────────────────────────────

    #[test]
    fn replay_result_serializes_to_json() {
        let trace = sample_trace();
        let actual: Vec<u64> = trace.events.iter().map(|e| e.offset_ms).collect();
        let result = compute_replay_drift(&trace, &actual, &ReplayConfig::default());

        let json = serde_json::to_string_pretty(&result).unwrap();
        let parsed: ReplayResult = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.trace_id, "trace-1");
        assert!(parsed.success);
    }

    // ── Viewport ─────────────────────────────────────────────────────────

    #[test]
    fn viewport_equality() {
        let v1 = Viewport {
            width: 80,
            height: 24,
        };
        let v2 = Viewport {
            width: 80,
            height: 24,
        };
        let v3 = Viewport {
            width: 120,
            height: 40,
        };
        assert_eq!(v1, v2);
        assert_ne!(v1, v3);
    }

    // ── Validation error display ─────────────────────────────────────────

    #[test]
    fn validation_errors_have_readable_display() {
        let errors = vec![
            TraceValidationError::SchemaVersionMismatch {
                expected: "v1".into(),
                actual: "v2".into(),
            },
            TraceValidationError::NonMonotonicTimestamps { at_sequence: 5 },
            TraceValidationError::NonContiguousSequence {
                expected: 3,
                actual: 7,
            },
            TraceValidationError::IntegrityCheckFailed {
                expected: "aaa".into(),
                actual: "bbb".into(),
            },
            TraceValidationError::EmptyTrace,
        ];

        for err in errors {
            let msg = err.to_string();
            assert!(
                !msg.is_empty(),
                "error display should not be empty: {err:?}"
            );
        }
    }

    // ── ReplayConfig defaults ────────────────────────────────────────────

    #[test]
    fn replay_config_defaults() {
        let config = ReplayConfig::default();
        assert_eq!(config.max_drift_ms, 50);
        assert!(!config.fail_on_drift);
        assert!((config.speed_factor - 1.0).abs() < f64::EPSILON);
        assert!(config.verify_renders);
    }
}
