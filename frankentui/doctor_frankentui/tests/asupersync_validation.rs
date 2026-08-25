//! Asupersync migration validation suite for doctor_frankentui (bd-1s6w5).
//!
//! These tests validate the subprocess orchestration contracts that must be
//! preserved when migrating to Asupersync-style structured cancellation.
//!
//! # Test Categories
//!
//! - **V1: RunMeta Evidence** — metadata contracts for subprocess outcomes
//! - **V2: DecisionRecord Ledger** — JSONL evidence chain correctness
//! - **V3: Subprocess Timeout** — timeout enforcement contracts
//! - **V4: Fallback Classification** — error vs timeout vs success paths
//! - **V5: Determinism** — identical inputs produce identical artifacts

use doctor_frankentui::runmeta::{DecisionRecord, RunMeta};
use std::time::{Duration, Instant};
use tempfile::tempdir;

// ============================================================================
// V1: RunMeta Evidence — metadata contracts for subprocess outcomes
// ============================================================================

/// V1.1: RunMeta serializes all subprocess outcome fields.
#[test]
fn v1_runmeta_serializes_subprocess_fields() {
    let meta = RunMeta {
        status: "completed".into(),
        seed_exit_code: Some(0),
        vhs_exit_code: Some(0),
        host_vhs_exit_code: Some(0),
        fallback_active: Some(false),
        fallback_reason: None,
        capture_error_reason: None,
        trace_id: Some("trace-001".into()),
        policy_id: Some("default".into()),
        evidence_ledger: Some("/tmp/ledger.jsonl".into()),
        ..RunMeta::default()
    };

    let json = serde_json::to_string_pretty(&meta).expect("serialize");

    assert!(json.contains("seed_exit_code"));
    assert!(json.contains("vhs_exit_code"));
    assert!(json.contains("host_vhs_exit_code"));
    assert!(json.contains("fallback_active"));
    assert!(json.contains("trace_id"));
    assert!(json.contains("policy_id"));
    assert!(json.contains("evidence_ledger"));
}

/// V1.2: RunMeta round-trips through file I/O.
#[test]
fn v1_runmeta_roundtrip_file_io() {
    let temp = tempdir().expect("tempdir");
    let path = temp.path().join("meta.json");

    let original = RunMeta {
        status: "completed".into(),
        profile: "test-profile".into(),
        seed_exit_code: Some(42),
        vhs_exit_code: Some(1),
        fallback_active: Some(true),
        fallback_reason: Some("timeout".into()),
        trace_id: Some("abc-123".into()),
        ..RunMeta::default()
    };

    original.write_to_path(&path).expect("write");
    let loaded = RunMeta::from_path(&path).expect("read");

    assert_eq!(loaded.status, "completed");
    assert_eq!(loaded.profile, "test-profile");
    assert_eq!(loaded.seed_exit_code, Some(42));
    assert_eq!(loaded.vhs_exit_code, Some(1));
    assert_eq!(loaded.fallback_active, Some(true));
    assert_eq!(loaded.fallback_reason.as_deref(), Some("timeout"));
    assert_eq!(loaded.trace_id.as_deref(), Some("abc-123"));
}

/// V1.3: RunMeta default has empty/None subprocess fields.
#[test]
fn v1_runmeta_default_clean() {
    let meta = RunMeta::default();

    assert!(meta.status.is_empty());
    assert!(meta.seed_exit_code.is_none());
    assert!(meta.vhs_exit_code.is_none());
    assert!(meta.host_vhs_exit_code.is_none());
    assert!(meta.fallback_active.is_none());
    assert!(meta.fallback_reason.is_none());
    assert!(meta.capture_error_reason.is_none());
    assert!(meta.trace_id.is_none());
    assert!(meta.policy_id.is_none());
}

/// V1.4: RunMeta tolerates unknown fields on deserialization (forward compat).
#[test]
fn v1_runmeta_tolerates_extra_fields() {
    let json = r#"{
        "status": "ok",
        "profile": "test",
        "future_field_that_does_not_exist": true,
        "another_future_field": 42
    }"#;

    let meta: RunMeta = serde_json::from_str(json).expect("should tolerate extra fields");
    assert_eq!(meta.status, "ok");
    assert_eq!(meta.profile, "test");
}

// ============================================================================
// V2: DecisionRecord Ledger — JSONL evidence chain
// ============================================================================

/// V2.1: DecisionRecord serializes to single-line JSON.
#[test]
fn v2_decision_record_single_line_json() {
    let record = DecisionRecord {
        timestamp: "2026-03-16T00:00:00Z".into(),
        trace_id: "trace-001".into(),
        decision_id: "d-001".into(),
        action: "spawn_vhs".into(),
        evidence_terms: vec!["vhs_available".into(), "host_mode".into()],
        fallback_active: false,
        fallback_reason: None,
        policy_id: "default".into(),
    };

    let json = serde_json::to_string(&record).expect("serialize");
    assert!(!json.contains('\n'), "JSONL must be single-line");
    assert!(json.contains("spawn_vhs"));
    assert!(json.contains("trace-001"));
}

/// V2.2: DecisionRecord appends to JSONL file correctly.
#[test]
fn v2_decision_record_append_jsonl() {
    let temp = tempdir().expect("tempdir");
    let path = temp.path().join("decisions.jsonl");

    let r1 = DecisionRecord {
        timestamp: "2026-03-16T00:00:00Z".into(),
        trace_id: "t1".into(),
        decision_id: "d1".into(),
        action: "spawn".into(),
        evidence_terms: vec!["available".into()],
        fallback_active: false,
        fallback_reason: None,
        policy_id: "default".into(),
    };

    let r2 = DecisionRecord {
        timestamp: "2026-03-16T00:00:01Z".into(),
        trace_id: "t1".into(),
        decision_id: "d2".into(),
        action: "complete".into(),
        evidence_terms: vec!["exit_0".into()],
        fallback_active: false,
        fallback_reason: None,
        policy_id: "default".into(),
    };

    r1.append_jsonl(&path).expect("append r1");
    r2.append_jsonl(&path).expect("append r2");

    let content = std::fs::read_to_string(&path).expect("read");
    let lines: Vec<&str> = content.lines().collect();
    assert_eq!(lines.len(), 2, "should have 2 JSONL lines");

    let parsed1: DecisionRecord = serde_json::from_str(lines[0]).expect("parse line 1");
    let parsed2: DecisionRecord = serde_json::from_str(lines[1]).expect("parse line 2");
    assert_eq!(parsed1.action, "spawn");
    assert_eq!(parsed2.action, "complete");
    assert_eq!(
        parsed1.trace_id, parsed2.trace_id,
        "trace_id must be consistent"
    );
}

/// V2.3: DecisionRecord captures fallback state.
#[test]
fn v2_decision_record_fallback_state() {
    let record = DecisionRecord {
        timestamp: "2026-03-16T00:00:00Z".into(),
        trace_id: "t1".into(),
        decision_id: "d-fb".into(),
        action: "fallback_to_docker".into(),
        evidence_terms: vec!["host_vhs_failed".into(), "docker_available".into()],
        fallback_active: true,
        fallback_reason: Some("host VHS EOF".into()),
        policy_id: "default".into(),
    };

    let json = serde_json::to_string(&record).expect("serialize");
    let parsed: DecisionRecord = serde_json::from_str(&json).expect("roundtrip");

    assert!(parsed.fallback_active);
    assert_eq!(parsed.fallback_reason.as_deref(), Some("host VHS EOF"));
    assert_eq!(parsed.evidence_terms.len(), 2);
}

// ============================================================================
// V3: Subprocess Timeout — timeout enforcement contracts
// ============================================================================

/// V3.1: Process timeout kills within reasonable bound.
#[test]
fn v3_subprocess_timeout_kills_promptly() {
    use std::process::{Command, Stdio};
    use wait_timeout::ChildExt;

    let mut child = Command::new("sleep")
        .arg("60")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn sleep");

    let start = Instant::now();
    let timeout = Duration::from_millis(200);

    match child.wait_timeout(timeout).expect("wait_timeout") {
        Some(_status) => panic!("sleep should not exit in 200ms"),
        None => {
            // Timed out as expected — kill it
            child.kill().expect("kill");
            child.wait().expect("reap");
        }
    }

    let elapsed = start.elapsed();
    assert!(
        elapsed < Duration::from_secs(2),
        "timeout + kill must complete promptly, took {elapsed:?}"
    );
}

/// V3.2: Process that exits before timeout is captured without kill.
#[test]
fn v3_subprocess_exits_before_timeout() {
    use std::process::{Command, Stdio};
    use wait_timeout::ChildExt;

    let mut child = Command::new("true")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn true");

    let timeout = Duration::from_secs(5);
    let status = child
        .wait_timeout(timeout)
        .expect("wait_timeout")
        .expect("should exit before timeout");

    assert!(status.success(), "true should exit 0");
}

/// V3.3: Process exit code preserved through wait_timeout.
#[test]
fn v3_exit_code_preserved() {
    use std::process::{Command, Stdio};
    use wait_timeout::ChildExt;

    let mut child = Command::new("sh")
        .arg("-c")
        .arg("exit 7")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn sh");

    let status = child
        .wait_timeout(Duration::from_secs(5))
        .expect("wait_timeout")
        .expect("should exit");

    assert_eq!(status.code(), Some(7), "exit code must be preserved");
}

// ============================================================================
// V4: Fallback Classification — error vs timeout vs success
// ============================================================================

/// V4.1: RunMeta correctly represents success path.
#[test]
fn v4_success_path_classification() {
    let meta = RunMeta {
        status: "completed".into(),
        seed_exit_code: Some(0),
        vhs_exit_code: Some(0),
        fallback_active: Some(false),
        capture_error_reason: None,
        ..RunMeta::default()
    };

    assert_eq!(meta.status, "completed");
    assert_eq!(meta.seed_exit_code, Some(0));
    assert_eq!(meta.fallback_active, Some(false));
    assert!(meta.capture_error_reason.is_none());
}

/// V4.2: RunMeta correctly represents fallback path.
#[test]
fn v4_fallback_path_classification() {
    let meta = RunMeta {
        status: "completed_with_fallback".into(),
        host_vhs_exit_code: Some(124),
        vhs_exit_code: Some(0),
        fallback_active: Some(true),
        fallback_reason: Some("host VHS ttyd EOF".into()),
        ..RunMeta::default()
    };

    assert!(meta.fallback_active.unwrap());
    assert!(meta.fallback_reason.is_some());
    assert_eq!(meta.host_vhs_exit_code, Some(124));
    // Docker fallback succeeded
    assert_eq!(meta.vhs_exit_code, Some(0));
}

/// V4.3: RunMeta correctly represents error path.
#[test]
fn v4_error_path_classification() {
    let meta = RunMeta {
        status: "failed".into(),
        capture_error_reason: Some("timeout exceeded".into()),
        fallback_active: Some(true),
        fallback_reason: Some("both host and docker VHS failed".into()),
        ..RunMeta::default()
    };

    assert_eq!(meta.status, "failed");
    assert!(meta.capture_error_reason.is_some());
}

// ============================================================================
// V5: Determinism — identical inputs produce identical artifacts
// ============================================================================

/// V5.1: RunMeta serialization is deterministic.
#[test]
fn v5_runmeta_serialization_deterministic() {
    let make_meta = || RunMeta {
        status: "completed".into(),
        profile: "test".into(),
        seed_exit_code: Some(0),
        vhs_exit_code: Some(0),
        fallback_active: Some(false),
        trace_id: Some("determinism-test".into()),
        ..RunMeta::default()
    };

    let json1 = serde_json::to_string_pretty(&make_meta()).expect("serialize 1");
    let json2 = serde_json::to_string_pretty(&make_meta()).expect("serialize 2");
    let json3 = serde_json::to_string_pretty(&make_meta()).expect("serialize 3");

    assert_eq!(json1, json2, "serialization must be deterministic");
    assert_eq!(json2, json3);
}

/// V5.2: DecisionRecord serialization is deterministic.
#[test]
fn v5_decision_record_deterministic() {
    let make_record = || DecisionRecord {
        timestamp: "2026-03-16T00:00:00Z".into(),
        trace_id: "det-test".into(),
        decision_id: "d-det".into(),
        action: "test".into(),
        evidence_terms: vec!["a".into(), "b".into()],
        fallback_active: false,
        fallback_reason: None,
        policy_id: "default".into(),
    };

    let j1 = serde_json::to_string(&make_record()).expect("s1");
    let j2 = serde_json::to_string(&make_record()).expect("s2");
    assert_eq!(
        j1, j2,
        "decision record serialization must be deterministic"
    );
}

/// V5.3: JSONL append order is deterministic.
#[test]
fn v5_jsonl_append_order_deterministic() {
    fn write_ledger() -> String {
        let temp = tempdir().expect("tempdir");
        let path = temp.path().join("ledger.jsonl");

        for i in 0..5 {
            let record = DecisionRecord {
                timestamp: format!("2026-03-16T00:00:{i:02}Z"),
                trace_id: "order-test".into(),
                decision_id: format!("d-{i}"),
                action: format!("step-{i}"),
                evidence_terms: vec![],
                fallback_active: false,
                fallback_reason: None,
                policy_id: "default".into(),
            };
            record.append_jsonl(&path).expect("append");
        }

        std::fs::read_to_string(&path).expect("read")
    }

    let l1 = write_ledger();
    let l2 = write_ledger();
    assert_eq!(l1, l2, "JSONL append order must be deterministic");
}
