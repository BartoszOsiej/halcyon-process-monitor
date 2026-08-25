//! E2E contract gate validation script with detailed structured logging.
//!
//! This test suite simulates a complete contract validation pipeline:
//! 1. Load all 5 contract schemas (semantic equivalence, transformation policy,
//!    evidence manifest, confidence model, licensing provenance).
//! 2. Validate a representative migration manifest against all contracts.
//! 3. Produce a per-clause pass/fail report with structured JSONL logging.
//! 4. Block on any critical violation (fail-safe behavior).
//! 5. Include at least one intentionally failing fixture for red-path diagnostics.
//!
//! All runs use deterministic run IDs and hashes for replay-friendliness.

use doctor_frankentui::semantic_contract::{
    EvidenceManifest, IpArtifactRecord, IpArtifactStatus, MigrationDecision, ProvenanceAction,
    ProvenanceChainRecord, VerdictOutcome, load_builtin_confidence_model,
    load_builtin_evidence_manifest, load_builtin_licensing_provenance,
    load_builtin_semantic_contract, load_builtin_transformation_policy_matrix,
};

// ── Deterministic Run Context ─────────────────────────────────────────────

const E2E_RUN_ID: &str = "e2e_gate_20260225_120000";

fn timestamp() -> String {
    "2026-02-25T12:00:00Z".to_string()
}

// ── JSONL Structured Logging ──────────────────────────────────────────────

#[derive(Debug, serde::Serialize)]
struct GateLogEntry {
    event: String,
    run_id: String,
    timestamp: String,
    stage: String,
    clause_id: Option<String>,
    status: String,
    detail: String,
}

fn log_entry(stage: &str, status: &str, detail: &str, clause_id: Option<&str>) -> String {
    let entry = GateLogEntry {
        event: "contract_gate".to_string(),
        run_id: E2E_RUN_ID.to_string(),
        timestamp: timestamp(),
        stage: stage.to_string(),
        clause_id: clause_id.map(|s| s.to_string()),
        status: status.to_string(),
        detail: detail.to_string(),
    };
    serde_json::to_string(&entry).expect("log entry must serialize")
}

// ── Gate Report ───────────────────────────────────────────────────────────

#[derive(Debug, serde::Serialize)]
struct GateReport {
    run_id: String,
    verdict: String,
    stages_passed: Vec<String>,
    stages_failed: Vec<String>,
    clause_results: Vec<ClauseResult>,
    risk_flags: Vec<String>,
    execution_trace: Vec<String>,
}

#[derive(Debug, serde::Serialize)]
struct ClauseResult {
    clause_id: String,
    status: String,
    detail: String,
}

// ── Test Fixtures ─────────────────────────────────────────────────────────

fn valid_evidence_manifest() -> EvidenceManifest {
    load_builtin_evidence_manifest().expect("builtin evidence manifest should parse")
}

fn build_valid_provenance_chain() -> Vec<ProvenanceChainRecord> {
    vec![
        ProvenanceChainRecord {
            stage_id: "source_snapshot".into(),
            input_hash: "sha256:aaa".into(),
            output_hash: "sha256:bbb".into(),
            tool_version: "opentui-snapshot 1.0".into(),
            timestamp: "2026-02-25T12:00:00Z".into(),
        },
        ProvenanceChainRecord {
            stage_id: "extraction".into(),
            input_hash: "sha256:bbb".into(),
            output_hash: "sha256:ccc".into(),
            tool_version: "opentui-extractor 1.0".into(),
            timestamp: "2026-02-25T12:00:05Z".into(),
        },
        ProvenanceChainRecord {
            stage_id: "ir_normalization".into(),
            input_hash: "sha256:ccc".into(),
            output_hash: "sha256:ddd".into(),
            tool_version: "opentui-ir 1.0".into(),
            timestamp: "2026-02-25T12:00:10Z".into(),
        },
        ProvenanceChainRecord {
            stage_id: "translation".into(),
            input_hash: "sha256:ddd".into(),
            output_hash: "sha256:eee".into(),
            tool_version: "opentui-translator 1.0".into(),
            timestamp: "2026-02-25T12:00:15Z".into(),
        },
        ProvenanceChainRecord {
            stage_id: "generated_output".into(),
            input_hash: "sha256:eee".into(),
            output_hash: "sha256:fff".into(),
            tool_version: "opentui-formatter 1.0".into(),
            timestamp: "2026-02-25T12:00:20Z".into(),
        },
    ]
}

fn build_clean_ip_artifacts() -> Vec<IpArtifactRecord> {
    vec![
        IpArtifactRecord {
            artifact_id: "react".into(),
            license_spdx: Some("MIT".into()),
            license_class: "permissive".into(),
            status: IpArtifactStatus::Clear,
            risk_flags: vec![],
            design_around_notes: None,
        },
        IpArtifactRecord {
            artifact_id: "lodash".into(),
            license_spdx: Some("MIT".into()),
            license_class: "permissive".into(),
            status: IpArtifactStatus::Clear,
            risk_flags: vec![],
            design_around_notes: None,
        },
    ]
}

// ═══════════════════════════════════════════════════════════════════════════
// E2E Test 1: Full green-path validation pipeline
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn e2e_contract_gate_green_path() {
    let mut trace = Vec::new();
    let mut stages_passed = Vec::new();
    let stages_failed: Vec<String> = Vec::new();
    let mut clause_results = Vec::new();

    // Stage 1: Load all contracts
    trace.push(log_entry(
        "load_contracts",
        "start",
        "Loading all 5 contract schemas",
        None,
    ));

    let sem_contract = load_builtin_semantic_contract().expect("semantic contract should parse");
    trace.push(log_entry(
        "load_contracts",
        "ok",
        "Semantic equivalence contract loaded",
        None,
    ));

    let policy_matrix = load_builtin_transformation_policy_matrix().expect("policy should parse");
    trace.push(log_entry(
        "load_contracts",
        "ok",
        "Transformation policy matrix loaded",
        None,
    ));

    let manifest = valid_evidence_manifest();
    trace.push(log_entry(
        "load_contracts",
        "ok",
        "Evidence manifest loaded",
        None,
    ));

    let confidence = load_builtin_confidence_model().expect("confidence model should parse");
    trace.push(log_entry(
        "load_contracts",
        "ok",
        "Confidence model loaded",
        None,
    ));

    let licensing = load_builtin_licensing_provenance().expect("licensing contract should parse");
    trace.push(log_entry(
        "load_contracts",
        "ok",
        "Licensing/provenance contract loaded",
        None,
    ));

    stages_passed.push("load_contracts".into());

    // Stage 2: Validate evidence manifest
    trace.push(log_entry(
        "validate_manifest",
        "start",
        "Validating evidence manifest integrity",
        None,
    ));

    assert!(!manifest.stages.is_empty(), "manifest must have stages");
    for window in manifest.stages.windows(2) {
        assert_eq!(
            window[0].output_hash, window[1].input_hash,
            "hash chain integrity"
        );
    }
    let run_policy_id = manifest.stages[0].policy_id.clone();
    let run_trace_id = manifest.stages[0].trace_id.clone();
    for stage in &manifest.stages {
        assert!(
            !stage.claim_id.is_empty(),
            "stage claim_id must not be empty"
        );
        assert!(
            !stage.evidence_id.is_empty(),
            "stage evidence_id must not be empty"
        );
        assert_eq!(
            stage.policy_id, run_policy_id,
            "policy_id must be consistent across all stages"
        );
        assert_eq!(
            stage.trace_id, run_trace_id,
            "trace_id must be consistent across all stages"
        );
        trace.push(log_entry(
            "validate_manifest",
            "ok",
            &format!(
                "Stage '{}' (index {}) validated",
                stage.stage_id, stage.stage_index
            ),
            None,
        ));
    }
    stages_passed.push("validate_manifest".into());

    // Stage 3: Validate semantic clause coverage
    trace.push(log_entry(
        "clause_coverage",
        "start",
        "Checking semantic clause coverage",
        None,
    ));

    let covered = &manifest
        .certification_verdict
        .semantic_clause_coverage
        .covered;
    let uncovered = &manifest
        .certification_verdict
        .semantic_clause_coverage
        .uncovered;

    for clause in &sem_contract.clauses {
        let is_covered = covered.iter().any(|c| c == &clause.clause_id);
        let status = if is_covered { "pass" } else { "uncovered" };
        clause_results.push(ClauseResult {
            clause_id: clause.clause_id.clone(),
            status: status.into(),
            detail: format!("severity={}, category={}", clause.severity, clause.category),
        });
        trace.push(log_entry(
            "clause_coverage",
            status,
            &format!(
                "Clause {} [{}]: {}",
                clause.clause_id, clause.severity, clause.title
            ),
            Some(&clause.clause_id),
        ));
    }

    assert!(
        !covered.is_empty(),
        "green-path fixture must include covered clauses"
    );
    let covered_set = covered
        .iter()
        .cloned()
        .collect::<std::collections::BTreeSet<_>>();
    let uncovered_set = uncovered
        .iter()
        .cloned()
        .collect::<std::collections::BTreeSet<_>>();
    assert!(
        covered_set.is_disjoint(&uncovered_set),
        "covered and uncovered clause sets must be disjoint"
    );
    let declared = covered_set
        .union(&uncovered_set)
        .cloned()
        .collect::<std::collections::BTreeSet<_>>();
    let contract_clauses = sem_contract
        .clauses
        .iter()
        .map(|clause| clause.clause_id.clone())
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(
        declared, contract_clauses,
        "coverage partition must account for all contract clauses"
    );

    let contract_gate = manifest
        .execute_runtime_contract_gate(&sem_contract)
        .expect("runtime contract gate should pass");
    assert!(
        contract_gate.passed,
        "compiled validator gate should produce a passing report"
    );
    for validator in &contract_gate.validator_results {
        assert!(
            validator.passed,
            "validator '{}' should pass with builtin manifest coverage",
            validator.validator_id
        );
        trace.push(log_entry(
            "clause_coverage",
            "ok",
            &format!(
                "Validator '{}' executed with full claim linkage",
                validator.validator_id
            ),
            None,
        ));
    }

    stages_passed.push("clause_coverage".into());

    // Stage 4: Validate transformation policy
    trace.push(log_entry(
        "policy_validation",
        "start",
        "Checking policy matrix completeness",
        None,
    ));

    let planner_rows = policy_matrix.planner_rows();
    let cert_rows = policy_matrix.certification_rows();
    assert_eq!(planner_rows.len(), policy_matrix.construct_catalog.len());
    assert_eq!(cert_rows.len(), policy_matrix.construct_catalog.len());
    trace.push(log_entry(
        "policy_validation",
        "ok",
        &format!("{} constructs fully classified", planner_rows.len()),
        None,
    ));
    stages_passed.push("policy_validation".into());

    // Stage 5: Confidence model verdict
    trace.push(log_entry(
        "confidence_verdict",
        "start",
        "Computing Bayesian posterior",
        None,
    ));

    let test_pass = manifest.certification_verdict.test_pass_count;
    let test_fail = manifest.certification_verdict.test_fail_count;
    let posterior = confidence.compute_posterior(test_pass, test_fail);
    let decision = confidence.decide(&posterior);
    let cert_stage = manifest
        .stages
        .iter()
        .find(|stage| stage.stage_id == "certification")
        .expect("certification stage must exist");
    let el_result = confidence.expected_loss_decision(
        &posterior,
        Some(cert_stage.claim_id.clone()),
        Some(cert_stage.policy_id.clone()),
    );

    trace.push(log_entry(
        "confidence_verdict",
        match decision {
            MigrationDecision::AutoApprove => "pass",
            MigrationDecision::HumanReview => "review",
            _ => "fail",
        },
        &el_result.rationale,
        None,
    ));
    stages_passed.push("confidence_verdict".into());

    // Stage 6: Licensing/provenance check
    trace.push(log_entry(
        "licensing_check",
        "start",
        "Validating licensing and provenance",
        None,
    ));

    let chain = build_valid_provenance_chain();
    licensing
        .validate_provenance_chain(&chain)
        .expect("valid chain should pass");

    let artifacts = build_clean_ip_artifacts();
    let ip_report = licensing.assess_ip_artifacts(E2E_RUN_ID, &chain, &artifacts);
    let ip_action = licensing.fail_safe_action(ip_report.overall_status);

    trace.push(log_entry(
        "licensing_check",
        match ip_action {
            ProvenanceAction::Accept => "pass",
            ProvenanceAction::Hold => "review",
            ProvenanceAction::Reject => "fail",
        },
        &format!(
            "overall_status={:?}, flags={:?}",
            ip_report.overall_status, ip_report.unresolved_risk_flags
        ),
        None,
    ));
    assert_eq!(ip_action, ProvenanceAction::Accept);
    stages_passed.push("licensing_check".into());

    // Build final report
    let report = GateReport {
        run_id: E2E_RUN_ID.to_string(),
        verdict: "pass".into(),
        stages_passed: stages_passed.clone(),
        stages_failed: stages_failed.clone(),
        clause_results,
        risk_flags: vec![],
        execution_trace: trace.clone(),
    };

    // Verify report structure
    let report_json = serde_json::to_string_pretty(&report).expect("report must serialize");
    assert!(!report_json.is_empty());
    assert!(
        stages_failed.is_empty(),
        "green path should have zero failures"
    );
    assert_eq!(stages_passed.len(), 6, "all 6 stages should pass");

    // Verify all trace entries are valid JSON
    for line in &trace {
        let _: serde_json::Value =
            serde_json::from_str(line).expect("trace line must be valid JSON");
    }

    // Verify deterministic run ID in all trace entries
    for line in &trace {
        assert!(
            line.contains(E2E_RUN_ID),
            "all trace entries must contain run_id"
        );
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// E2E Test 2: Red-path - broken hash chain in evidence manifest
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn e2e_contract_gate_red_path_broken_hash_chain() {
    let mut trace = Vec::new();

    trace.push(log_entry(
        "validate_manifest",
        "start",
        "Validating manifest with broken hash chain",
        None,
    ));

    let mut manifest = valid_evidence_manifest();
    // Intentionally break the hash chain
    if manifest.stages.len() >= 2 {
        manifest.stages[1].input_hash = "sha256:INTENTIONALLY_BROKEN".into();
    }

    let raw = serde_json::to_string(&manifest).expect("should serialize");
    let result = EvidenceManifest::parse_and_validate(&raw);

    assert!(result.is_err(), "broken hash chain must be rejected");
    let error = result.unwrap_err();
    let error_msg = error.to_string();

    trace.push(log_entry(
        "validate_manifest",
        "fail",
        &format!("REJECTED: {error_msg}"),
        None,
    ));

    assert!(
        error_msg.contains("hash chain"),
        "error must mention hash chain, got: {error_msg}"
    );

    // Verify diagnostic trace is complete
    assert!(trace.len() >= 2, "trace must have start and fail entries");
    let last = &trace[trace.len() - 1];
    assert!(last.contains("fail"), "last trace entry must show failure");
    assert!(
        last.contains("REJECTED"),
        "last trace entry must show rejection"
    );
}

// ═══════════════════════════════════════════════════════════════════════════
// E2E Test 3: Red-path - blocked license in IP artifacts
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn e2e_contract_gate_red_path_blocked_license() {
    let mut trace = Vec::new();
    let licensing = load_builtin_licensing_provenance().expect("licensing should parse");

    trace.push(log_entry(
        "licensing_check",
        "start",
        "Validating artifacts with blocked license",
        None,
    ));

    let artifacts = vec![
        IpArtifactRecord {
            artifact_id: "react".into(),
            license_spdx: Some("MIT".into()),
            license_class: "permissive".into(),
            status: IpArtifactStatus::Clear,
            risk_flags: vec![],
            design_around_notes: None,
        },
        IpArtifactRecord {
            artifact_id: "gpl-library".into(),
            license_spdx: Some("GPL-3.0".into()),
            license_class: "strong_copyleft".into(),
            status: IpArtifactStatus::Blocked,
            risk_flags: vec!["lp-copyleft-contamination".into()],
            design_around_notes: Some("Remove or replace with MIT alternative".into()),
        },
    ];

    let report = licensing.assess_ip_artifacts(E2E_RUN_ID, &[], &artifacts);
    let action = licensing.fail_safe_action(report.overall_status);

    trace.push(log_entry(
        "licensing_check",
        "fail",
        &format!(
            "BLOCKED: overall_status={:?}, risk_flags={:?}, remediation={}",
            report.overall_status,
            report.unresolved_risk_flags,
            "Remove or replace blocked dependencies"
        ),
        None,
    ));

    assert_eq!(report.overall_status, IpArtifactStatus::Blocked);
    assert_eq!(action, ProvenanceAction::Reject);
    assert!(
        report
            .unresolved_risk_flags
            .contains(&"lp-copyleft-contamination".into())
    );

    // Verify trace includes remediation hints
    let last = &trace[trace.len() - 1];
    assert!(
        last.contains("remediation"),
        "trace must include remediation hints"
    );
}

// ═══════════════════════════════════════════════════════════════════════════
// E2E Test 4: Red-path - accept verdict with failing tests
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn e2e_contract_gate_red_path_accept_with_failures() {
    let mut trace = Vec::new();

    trace.push(log_entry(
        "validate_manifest",
        "start",
        "Validating manifest with inconsistent verdict",
        None,
    ));

    let mut manifest = valid_evidence_manifest();
    manifest.certification_verdict.verdict = VerdictOutcome::Accept;
    manifest.certification_verdict.test_fail_count = 5;

    let raw = serde_json::to_string(&manifest).expect("should serialize");
    let result = EvidenceManifest::parse_and_validate(&raw);

    assert!(result.is_err(), "accept with failures must be rejected");
    let error_msg = result.unwrap_err().to_string();

    trace.push(log_entry(
        "validate_manifest",
        "fail",
        &format!("REJECTED: {error_msg}"),
        None,
    ));

    assert!(
        error_msg.contains("failing tests"),
        "error must mention failing tests, got: {error_msg}"
    );
}

// ═══════════════════════════════════════════════════════════════════════════
// E2E Test 5: Red-path - low confidence triggers rejection
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn e2e_contract_gate_red_path_low_confidence() {
    let mut trace = Vec::new();
    let confidence = load_builtin_confidence_model().expect("confidence should parse");

    trace.push(log_entry(
        "confidence_verdict",
        "start",
        "Computing posterior for low-confidence scenario",
        None,
    ));

    // Simulate 2 passes, 50 failures
    let posterior = confidence.compute_posterior(2, 50);
    let decision = confidence.decide(&posterior);
    let el_result = confidence.expected_loss_decision(&posterior, None, None);

    trace.push(log_entry(
        "confidence_verdict",
        "fail",
        &format!("REJECTED: {}", el_result.rationale),
        None,
    ));

    // Very low confidence should trigger reject, hard_reject, rollback, or conservative_fallback
    assert!(
        matches!(
            decision,
            MigrationDecision::Reject
                | MigrationDecision::HardReject
                | MigrationDecision::Rollback
                | MigrationDecision::ConservativeFallback
        ),
        "low confidence must not auto-approve or human-review, got {:?}",
        decision
    );

    // Verify rationale includes diagnostic info
    assert!(el_result.rationale.contains("posterior_mean"));
    assert!(el_result.rationale.contains("EL(accept)"));
}

// ═══════════════════════════════════════════════════════════════════════════
// E2E Test 6: Red-path - broken provenance chain
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn e2e_contract_gate_red_path_broken_provenance() {
    let mut trace = Vec::new();
    let licensing = load_builtin_licensing_provenance().expect("licensing should parse");

    trace.push(log_entry(
        "licensing_check",
        "start",
        "Validating broken provenance chain",
        None,
    ));

    let mut chain = build_valid_provenance_chain();
    if chain.len() >= 2 {
        chain[1].input_hash = "sha256:BROKEN".into();
    }

    let result = licensing.validate_provenance_chain(&chain);
    assert!(result.is_err(), "broken provenance must be rejected");

    let error_msg = result.unwrap_err().to_string();
    trace.push(log_entry(
        "licensing_check",
        "fail",
        &format!("REJECTED: {error_msg}"),
        None,
    ));

    assert!(error_msg.contains("chain broken"));
}

// ═══════════════════════════════════════════════════════════════════════════
// E2E Test 7: Red-path - missing provenance stage
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn e2e_contract_gate_red_path_missing_provenance_stage() {
    let mut trace = Vec::new();
    let licensing = load_builtin_licensing_provenance().expect("licensing should parse");

    trace.push(log_entry(
        "licensing_check",
        "start",
        "Validating provenance with missing stage",
        None,
    ));

    // Only provide 2 of the 5 required stages
    let partial_chain = vec![
        ProvenanceChainRecord {
            stage_id: "source_snapshot".into(),
            input_hash: "sha256:aaa".into(),
            output_hash: "sha256:bbb".into(),
            tool_version: "opentui-snapshot 1.0".into(),
            timestamp: "2026-02-25T12:00:00Z".into(),
        },
        ProvenanceChainRecord {
            stage_id: "extraction".into(),
            input_hash: "sha256:bbb".into(),
            output_hash: "sha256:ccc".into(),
            tool_version: "opentui-extractor 1.0".into(),
            timestamp: "2026-02-25T12:00:05Z".into(),
        },
    ];

    let result = licensing.validate_provenance_chain(&partial_chain);
    assert!(result.is_err(), "incomplete provenance must be rejected");

    let error_msg = result.unwrap_err().to_string();
    trace.push(log_entry(
        "licensing_check",
        "fail",
        &format!("REJECTED: {error_msg}"),
        None,
    ));

    assert!(error_msg.contains("missing required stage"));
}

// ═══════════════════════════════════════════════════════════════════════════
// E2E Test 8: Verify JSONL trace is machine-parseable and complete
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn e2e_contract_gate_jsonl_trace_is_well_formed() {
    // Run a mini pipeline and verify all trace entries are valid JSONL
    let trace = [
        log_entry("load", "ok", "contracts loaded", None),
        log_entry("manifest", "ok", "manifest valid", None),
        log_entry("clauses", "ok", "all clauses covered", Some("ST-001")),
        log_entry("policy", "ok", "all constructs classified", None),
        log_entry("confidence", "pass", "auto-approve", None),
        log_entry("licensing", "pass", "all clear", None),
    ];

    // Every trace entry must be valid JSON
    for (i, line) in trace.iter().enumerate() {
        let parsed: serde_json::Value =
            serde_json::from_str(line).unwrap_or_else(|e| panic!("trace[{i}] invalid JSON: {e}"));
        assert_eq!(
            parsed["event"], "contract_gate",
            "trace[{i}] event must be 'contract_gate'"
        );
        assert_eq!(
            parsed["run_id"], E2E_RUN_ID,
            "trace[{i}] must have deterministic run_id"
        );
        assert!(
            !parsed["stage"].as_str().unwrap_or("").is_empty(),
            "trace[{i}] must have non-empty stage"
        );
        assert!(
            !parsed["status"].as_str().unwrap_or("").is_empty(),
            "trace[{i}] must have non-empty status"
        );
    }

    // The JSONL output must be one valid JSON per line (no trailing commas, etc.)
    let jsonl = trace.join("\n");
    let lines: Vec<&str> = jsonl.lines().collect();
    assert_eq!(lines.len(), trace.len());
    for line in lines {
        let _: serde_json::Value =
            serde_json::from_str(line).expect("each line must be valid JSON");
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// E2E Test 9: Full gate report is deterministic across runs
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn e2e_contract_gate_report_is_deterministic() {
    // Run the same pipeline twice and verify identical output
    let run = || -> String {
        let sem = load_builtin_semantic_contract().unwrap();
        let _pol = load_builtin_transformation_policy_matrix().unwrap();
        let man = valid_evidence_manifest();
        let conf = load_builtin_confidence_model().unwrap();
        let lic = load_builtin_licensing_provenance().unwrap();

        let posterior = conf.compute_posterior(
            man.certification_verdict.test_pass_count,
            man.certification_verdict.test_fail_count,
        );
        let decision = conf.decide(&posterior);
        let el = conf.expected_loss_decision(&posterior, None, None);

        let chain = build_valid_provenance_chain();
        lic.validate_provenance_chain(&chain).unwrap();
        let artifacts = build_clean_ip_artifacts();
        let ip_report = lic.assess_ip_artifacts(E2E_RUN_ID, &chain, &artifacts);

        let report = GateReport {
            run_id: E2E_RUN_ID.into(),
            verdict: match decision {
                MigrationDecision::AutoApprove => "pass",
                MigrationDecision::HumanReview => "review",
                _ => "fail",
            }
            .into(),
            stages_passed: vec![
                "load_contracts".into(),
                "validate_manifest".into(),
                "clause_coverage".into(),
                "policy_validation".into(),
                "confidence_verdict".into(),
                "licensing_check".into(),
            ],
            stages_failed: vec![],
            clause_results: sem
                .clauses
                .iter()
                .map(|c| ClauseResult {
                    clause_id: c.clause_id.clone(),
                    status: "pass".into(),
                    detail: format!("severity={}", c.severity),
                })
                .collect(),
            risk_flags: ip_report.unresolved_risk_flags,
            execution_trace: vec![el.rationale.clone()],
        };

        serde_json::to_string(&report).unwrap()
    };

    let run1 = run();
    let run2 = run();
    assert_eq!(
        run1, run2,
        "gate report must be deterministic across identical runs"
    );
}

// ═══════════════════════════════════════════════════════════════════════════
// E2E Test 10: Cross-contract consistency check
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn e2e_contract_gate_cross_contract_consistency() {
    let sem = load_builtin_semantic_contract().unwrap();
    let pol = load_builtin_transformation_policy_matrix().unwrap();
    let man = valid_evidence_manifest();
    let conf = load_builtin_confidence_model().unwrap();
    let lic = load_builtin_licensing_provenance().unwrap();

    // Semantic contract clause IDs referenced in policy matrix must exist
    let clause_ids: std::collections::BTreeSet<_> =
        sem.clauses.iter().map(|c| c.clause_id.as_str()).collect();
    for cell in &pol.policy_cells {
        for link in &cell.semantic_clause_links {
            assert!(
                clause_ids.contains(link.as_str()),
                "policy cell '{}' references unknown clause '{}'",
                cell.construct_signature,
                link
            );
        }
    }

    // Evidence manifest covered clauses must exist in semantic contract
    for covered in &man.certification_verdict.semantic_clause_coverage.covered {
        assert!(
            clause_ids.contains(covered.as_str()),
            "manifest covered clause '{}' not in semantic contract",
            covered
        );
    }

    // Confidence model decision space must include expected actions
    for action in ["accept", "hold", "reject", "rollback"] {
        assert!(
            conf.decision_space.actions.iter().any(|a| a == action),
            "confidence model missing action '{action}'"
        );
    }

    // Licensing contract must define classes referenced in policy
    assert!(!lic.licensing_policy.license_class_definitions.is_empty());
    assert!(lic.provenance_chain_policy.chain_must_be_unbroken);
}
