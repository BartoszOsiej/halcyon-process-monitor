#![forbid(unsafe_code)]

//! Failure signatures and replay-friendly log quality contracts (bd-xl1c4).
//!
//! This module defines:
//! - **Canonical failure reason codes** for runtime and doctor failures
//! - **Required log fields** per failure class for machine-readable triage
//! - **Log quality validators** that fail when logs become too weak for replay
//!
//! # Design rationale
//!
//! A concurrency migration generates huge logs that are useless if they don't
//! summarize the right failure signatures. This module provides:
//! 1. Machine-readable reason codes that can route to artifact bundles
//! 2. Required field contracts that ensure enough context for reproduction
//! 3. Quality checks that catch log regression before it reaches operators
//!
//! # Failure taxonomy
//!
//! ```text
//! FailureClass
//! ├── Mismatch         — shadow-run output divergence
//! ├── Timeout          — deadline or per-operation timeout exceeded
//! ├── Cancellation     — cooperative cancellation completed or forced
//! ├── QueueOverload    — effect queue backpressure or drop
//! ├── ProcessFailure   — child process non-zero exit or crash
//! ├── Rollback         — rollout policy triggered rollback
//! ├── ShadowDivergence — shadow lane produced different output
//! ├── PanicCaught      — subscription or effect thread panicked
//! └── NetworkFailure   — RPC/HTTP connection or protocol error
//! ```

use std::collections::HashSet;

/// Canonical failure class for runtime and doctor failures.
///
/// Each class has a set of required log fields that must be present
/// for meaningful triage and replay.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FailureClass {
    /// Shadow-run output divergence (frame hash mismatch).
    Mismatch,
    /// Deadline or per-operation timeout exceeded.
    Timeout,
    /// Cooperative cancellation completed or forced.
    Cancellation,
    /// Effect queue backpressure or drop.
    QueueOverload,
    /// Child process non-zero exit or crash.
    ProcessFailure,
    /// Rollout policy triggered rollback.
    Rollback,
    /// Shadow lane produced different output vs primary.
    ShadowDivergence,
    /// Subscription or effect thread panicked (caught).
    PanicCaught,
    /// RPC/HTTP connection or protocol error.
    NetworkFailure,
}

impl FailureClass {
    /// Machine-readable reason code string.
    ///
    /// These codes are stable across versions and suitable for
    /// indexing, filtering, and routing in dashboards and scripts.
    #[must_use]
    pub const fn reason_code(&self) -> &'static str {
        match self {
            Self::Mismatch => "MISMATCH",
            Self::Timeout => "TIMEOUT",
            Self::Cancellation => "CANCELLATION",
            Self::QueueOverload => "QUEUE_OVERLOAD",
            Self::ProcessFailure => "PROCESS_FAILURE",
            Self::Rollback => "ROLLBACK",
            Self::ShadowDivergence => "SHADOW_DIVERGENCE",
            Self::PanicCaught => "PANIC_CAUGHT",
            Self::NetworkFailure => "NETWORK_FAILURE",
        }
    }

    /// Required log fields for this failure class.
    ///
    /// These are the minimum structured fields that MUST appear in
    /// the log event for this failure type to support triage and replay.
    #[must_use]
    pub const fn required_fields(&self) -> &'static [&'static str] {
        match self {
            Self::Mismatch => &[
                "reason",        // MISMATCH
                "frame_idx",     // which frame diverged
                "expected_hash", // baseline frame hash
                "actual_hash",   // candidate frame hash
                "scenario",      // scenario name for reproduction
                "seed",          // random seed for deterministic replay
            ],
            Self::Timeout => &[
                "reason",     // TIMEOUT
                "timeout_ms", // configured timeout
                "elapsed_ms", // actual elapsed time
                "operation",  // what timed out (e.g., "vhs_capture", "seed_rpc")
            ],
            Self::Cancellation => &[
                "reason",     // CANCELLATION
                "trigger",    // what triggered (e.g., "stop_signal", "deadline")
                "elapsed_ms", // time before cancellation
                "pending",    // number of pending operations
            ],
            Self::QueueOverload => &[
                "reason",      // QUEUE_OVERLOAD
                "queue_depth", // current queue depth
                "high_water",  // peak queue depth
                "dropped",     // total dropped count
            ],
            Self::ProcessFailure => &[
                "reason",    // PROCESS_FAILURE
                "program",   // process name/command
                "exit_code", // process exit code
                "sub_id",    // subscription ID
            ],
            Self::Rollback => &[
                "reason",          // ROLLBACK
                "previous_lane",   // lane before rollback
                "rollback_lane",   // lane after rollback
                "rollback_reason", // human-readable reason
            ],
            Self::ShadowDivergence => &[
                "reason",          // SHADOW_DIVERGENCE
                "diverged_count",  // number of diverged frames
                "total_frames",    // total frames compared
                "baseline_label",  // label for primary run
                "candidate_label", // label for shadow run
            ],
            Self::PanicCaught => &[
                "reason",      // PANIC_CAUGHT
                "sub_id",      // subscription or effect ID
                "panic_msg",   // panic message
                "effect_type", // "subscription" or "command"
            ],
            Self::NetworkFailure => &[
                "reason",     // NETWORK_FAILURE
                "url",        // target URL
                "stage",      // RPC stage name
                "attempts",   // retry attempts made
                "last_error", // final error message
            ],
        }
    }

    /// Human-readable summary template for operator triage.
    ///
    /// Contains `{field}` placeholders that should be filled from log fields.
    #[must_use]
    pub const fn summary_template(&self) -> &'static str {
        match self {
            Self::Mismatch => {
                "Frame {frame_idx} diverged: expected {expected_hash}, got {actual_hash} \
                 (scenario={scenario}, seed={seed})"
            }
            Self::Timeout => "{operation} timed out after {elapsed_ms}ms (limit: {timeout_ms}ms)",
            Self::Cancellation => {
                "Cancelled by {trigger} after {elapsed_ms}ms ({pending} operations pending)"
            }
            Self::QueueOverload => {
                "Queue overloaded: depth={queue_depth}, high_water={high_water}, dropped={dropped}"
            }
            Self::ProcessFailure => {
                "Process '{program}' exited with code {exit_code} (sub_id={sub_id})"
            }
            Self::Rollback => {
                "Rolled back from {previous_lane} to {rollback_lane}: {rollback_reason}"
            }
            Self::ShadowDivergence => {
                "Shadow diverged: {diverged_count}/{total_frames} frames differ \
                 ({baseline_label} vs {candidate_label})"
            }
            Self::PanicCaught => "Panic caught in {effect_type} (id={sub_id}): {panic_msg}",
            Self::NetworkFailure => {
                "Network failure at stage '{stage}' ({url}): {last_error} \
                 after {attempts} attempts"
            }
        }
    }

    /// All failure classes.
    pub const ALL: &'static [FailureClass] = &[
        Self::Mismatch,
        Self::Timeout,
        Self::Cancellation,
        Self::QueueOverload,
        Self::ProcessFailure,
        Self::Rollback,
        Self::ShadowDivergence,
        Self::PanicCaught,
        Self::NetworkFailure,
    ];
}

/// A structured log entry for validation.
#[derive(Debug, Clone)]
pub struct LogEntry {
    /// The failure class (parsed from `reason` field).
    pub class: FailureClass,
    /// Fields present in this log entry.
    pub fields: HashSet<String>,
}

/// Result of validating a log entry against its failure class contract.
#[derive(Debug, Clone)]
pub struct ValidationResult {
    /// The failure class that was validated.
    pub class: FailureClass,
    /// Fields that are required but missing.
    pub missing_fields: Vec<String>,
    /// Whether the entry passes the quality contract.
    pub passes: bool,
}

/// Validate a log entry against the failure signature contract.
///
/// Returns a validation result indicating whether all required fields
/// are present for the given failure class.
#[must_use]
pub fn validate_log_entry(entry: &LogEntry) -> ValidationResult {
    let required = entry.class.required_fields();
    let missing: Vec<String> = required
        .iter()
        .filter(|f| !entry.fields.contains(**f))
        .map(|f| (*f).to_string())
        .collect();
    let passes = missing.is_empty();
    ValidationResult {
        class: entry.class,
        missing_fields: missing,
        passes,
    }
}

/// Validate a batch of log entries and return all failures.
///
/// Returns only entries that fail validation (missing required fields).
#[must_use]
pub fn validate_log_batch(entries: &[LogEntry]) -> Vec<ValidationResult> {
    entries
        .iter()
        .map(validate_log_entry)
        .filter(|r| !r.passes)
        .collect()
}

/// Parse a failure class from a reason code string.
///
/// Returns `None` if the reason code is not recognized.
#[must_use]
pub fn parse_reason_code(code: &str) -> Option<FailureClass> {
    FailureClass::ALL
        .iter()
        .find(|c| c.reason_code() == code)
        .copied()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_classes_have_unique_reason_codes() {
        let mut codes = HashSet::new();
        for class in FailureClass::ALL {
            assert!(
                codes.insert(class.reason_code()),
                "duplicate reason code: {}",
                class.reason_code()
            );
        }
    }

    #[test]
    fn all_classes_have_required_fields() {
        for class in FailureClass::ALL {
            let fields = class.required_fields();
            assert!(
                !fields.is_empty(),
                "{} must have at least one required field",
                class.reason_code()
            );
            // Every class must require a "reason" field.
            assert!(
                fields.contains(&"reason"),
                "{} must include 'reason' in required fields",
                class.reason_code()
            );
        }
    }

    #[test]
    fn all_classes_have_summary_templates() {
        for class in FailureClass::ALL {
            let template = class.summary_template();
            assert!(
                !template.is_empty(),
                "{} must have a non-empty summary template",
                class.reason_code()
            );
        }
    }

    #[test]
    fn reason_codes_are_uppercase_snake_case() {
        for class in FailureClass::ALL {
            let code = class.reason_code();
            assert!(
                code.chars().all(|c| c.is_ascii_uppercase() || c == '_'),
                "reason code '{}' must be UPPER_SNAKE_CASE",
                code
            );
        }
    }

    #[test]
    fn parse_reason_code_roundtrips() {
        for class in FailureClass::ALL {
            let code = class.reason_code();
            let parsed = parse_reason_code(code);
            assert_eq!(
                parsed,
                Some(*class),
                "parse_reason_code('{}') should roundtrip",
                code
            );
        }
    }

    #[test]
    fn parse_unknown_reason_code_returns_none() {
        assert_eq!(parse_reason_code("UNKNOWN_CODE"), None);
        assert_eq!(parse_reason_code(""), None);
    }

    #[test]
    fn validate_passing_entry() {
        let entry = LogEntry {
            class: FailureClass::Timeout,
            fields: ["reason", "timeout_ms", "elapsed_ms", "operation"]
                .iter()
                .map(|s| s.to_string())
                .collect(),
        };
        let result = validate_log_entry(&entry);
        assert!(result.passes, "entry with all required fields should pass");
        assert!(result.missing_fields.is_empty());
    }

    #[test]
    fn validate_failing_entry() {
        let entry = LogEntry {
            class: FailureClass::Timeout,
            fields: ["reason"].iter().map(|s| s.to_string()).collect(),
        };
        let result = validate_log_entry(&entry);
        assert!(!result.passes, "entry missing fields should fail");
        assert!(
            result.missing_fields.contains(&"timeout_ms".to_string()),
            "should report missing timeout_ms"
        );
        assert!(
            result.missing_fields.contains(&"elapsed_ms".to_string()),
            "should report missing elapsed_ms"
        );
        assert!(
            result.missing_fields.contains(&"operation".to_string()),
            "should report missing operation"
        );
    }

    #[test]
    fn validate_batch_returns_only_failures() {
        let good = LogEntry {
            class: FailureClass::Cancellation,
            fields: ["reason", "trigger", "elapsed_ms", "pending"]
                .iter()
                .map(|s| s.to_string())
                .collect(),
        };
        let bad = LogEntry {
            class: FailureClass::ProcessFailure,
            fields: ["reason"].iter().map(|s| s.to_string()).collect(),
        };
        let results = validate_log_batch(&[good, bad]);
        assert_eq!(results.len(), 1, "only the bad entry should fail");
        assert_eq!(results[0].class, FailureClass::ProcessFailure);
    }

    #[test]
    fn mismatch_requires_replay_fields() {
        let fields = FailureClass::Mismatch.required_fields();
        assert!(
            fields.contains(&"scenario"),
            "mismatch needs scenario for replay"
        );
        assert!(
            fields.contains(&"seed"),
            "mismatch needs seed for deterministic replay"
        );
        assert!(
            fields.contains(&"frame_idx"),
            "mismatch needs frame_idx for pinpointing"
        );
    }

    #[test]
    fn shadow_divergence_requires_comparison_context() {
        let fields = FailureClass::ShadowDivergence.required_fields();
        assert!(fields.contains(&"diverged_count"));
        assert!(fields.contains(&"total_frames"));
        assert!(fields.contains(&"baseline_label"));
        assert!(fields.contains(&"candidate_label"));
    }

    #[test]
    fn network_failure_requires_retry_context() {
        let fields = FailureClass::NetworkFailure.required_fields();
        assert!(fields.contains(&"attempts"), "need retry count for triage");
        assert!(
            fields.contains(&"last_error"),
            "need final error for diagnosis"
        );
        assert!(fields.contains(&"stage"), "need RPC stage for routing");
    }

    #[test]
    fn panic_caught_requires_effect_context() {
        let fields = FailureClass::PanicCaught.required_fields();
        assert!(fields.contains(&"panic_msg"), "need panic message");
        assert!(
            fields.contains(&"effect_type"),
            "need to know if sub or cmd"
        );
        assert!(fields.contains(&"sub_id"), "need ID for correlation");
    }

    #[test]
    fn queue_overload_requires_capacity_context() {
        let fields = FailureClass::QueueOverload.required_fields();
        assert!(fields.contains(&"queue_depth"));
        assert!(fields.contains(&"high_water"));
        assert!(fields.contains(&"dropped"));
    }

    #[test]
    fn all_failure_classes_covered() {
        assert_eq!(
            FailureClass::ALL.len(),
            9,
            "taxonomy should have exactly 9 failure classes"
        );
    }

    #[test]
    fn summary_templates_reference_required_fields() {
        for class in FailureClass::ALL {
            let template = class.summary_template();
            let required = class.required_fields();
            // At least some required fields should appear in the template
            // (the 'reason' field is implicit in the class itself).
            let non_reason_fields: Vec<_> = required.iter().filter(|f| **f != "reason").collect();
            let referenced_count = non_reason_fields
                .iter()
                .filter(|f| template.contains(&format!("{{{}}}", f)))
                .count();
            assert!(
                referenced_count > 0,
                "{} summary template should reference at least one required field \
                 (template: '{}', fields: {:?})",
                class.reason_code(),
                template,
                non_reason_fields
            );
        }
    }

    #[test]
    fn extra_fields_dont_cause_validation_failure() {
        let entry = LogEntry {
            class: FailureClass::Timeout,
            fields: [
                "reason",
                "timeout_ms",
                "elapsed_ms",
                "operation",
                "extra_context",
                "trace_id",
            ]
            .iter()
            .map(|s| s.to_string())
            .collect(),
        };
        let result = validate_log_entry(&entry);
        assert!(
            result.passes,
            "extra fields should not cause validation failure"
        );
    }
}
