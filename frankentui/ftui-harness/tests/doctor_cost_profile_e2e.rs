//! E2E integration tests for doctor workflow cost profiling (bd-qbbkv).
//!
//! Validates the canonical doctor cost model produces actionable evidence
//! for downstream optimization beads.

#![forbid(unsafe_code)]

use ftui_harness::doctor_cost_profile::{
    CostEntry, CostLane, DoctorCostProfile, OptimizationImpact, WorkflowStage,
};

// ============================================================================
// Canonical profile validation
// ============================================================================

#[test]
fn e2e_canonical_profile_captures_complete_workflow() {
    let profile = DoctorCostProfile::canonical();
    let report = profile.finalize();

    // Must cover all 5 workflow stages
    for stage in WorkflowStage::ALL {
        let entries = report.by_stage(*stage);
        assert!(
            !entries.is_empty(),
            "missing entries for stage '{}'",
            stage.label()
        );
    }

    // Must cover all 5 cost lanes
    for lane in CostLane::ALL {
        let entries = report.by_lane(*lane);
        assert!(
            !entries.is_empty(),
            "missing entries for lane '{}'",
            lane.label()
        );
    }
}

#[test]
fn e2e_capture_stage_is_dominant_cost_center() {
    let profile = DoctorCostProfile::canonical();
    let report = profile.finalize();

    let capture_ms = report.stage_total(WorkflowStage::Capture);
    let total_ms = report.grand_total_ms;

    let capture_pct = (capture_ms as f64 / total_ms as f64) * 100.0;

    // Capture should be > 40% of total (it's the VHS recording bottleneck)
    assert!(
        capture_pct > 40.0,
        "capture should dominate: {:.1}% of total",
        capture_pct
    );
}

#[test]
fn e2e_subprocess_lane_dominates_blocking_time() {
    let profile = DoctorCostProfile::canonical();
    let report = profile.finalize();

    let subprocess_ms = report.lane_total(CostLane::Subprocess);
    let total_ms = report.grand_total_ms;

    let subprocess_pct = (subprocess_ms as f64 / total_ms as f64) * 100.0;

    // Subprocess should be > 50% of total (VHS + docker + ffmpeg)
    assert!(
        subprocess_pct > 50.0,
        "subprocess lane should dominate: {:.1}% of total",
        subprocess_pct
    );
}

#[test]
fn e2e_blocking_operations_dominate() {
    let profile = DoctorCostProfile::canonical();
    let report = profile.finalize();

    // Blocking ops should be > 80% of total wall-clock
    assert!(
        report.blocking_pct() > 80.0,
        "blocking should dominate: {:.1}%",
        report.blocking_pct()
    );
}

// ============================================================================
// Optimization target quality
// ============================================================================

#[test]
fn e2e_optimization_targets_include_critical_items() {
    let profile = DoctorCostProfile::canonical();
    let report = profile.finalize();

    let critical: Vec<_> = report
        .optimization_targets
        .iter()
        .filter(|t| t.impact == OptimizationImpact::Critical)
        .collect();

    assert!(
        !critical.is_empty(),
        "should have at least one critical optimization target"
    );

    // VHS recording should be critical
    let has_vhs = critical.iter().any(|t| t.operation.contains("vhs"));
    assert!(has_vhs, "VHS recording should be a critical target");
}

#[test]
fn e2e_optimization_targets_are_sorted_by_impact() {
    let profile = DoctorCostProfile::canonical();
    let report = profile.finalize();

    for window in report.optimization_targets.windows(2) {
        assert!(
            window[0].impact >= window[1].impact,
            "targets not sorted: {} ({:?}) before {} ({:?})",
            window[0].operation,
            window[0].impact,
            window[1].operation,
            window[1].impact
        );
    }
}

#[test]
fn e2e_every_target_has_rationale() {
    let profile = DoctorCostProfile::canonical();
    let report = profile.finalize();

    for target in &report.optimization_targets {
        assert!(
            !target.rationale.is_empty(),
            "target '{}' missing rationale",
            target.operation
        );
    }
}

// ============================================================================
// Redundancy analysis
// ============================================================================

#[test]
fn e2e_redundant_operations_identified() {
    let profile = DoctorCostProfile::canonical();
    let report = profile.finalize();

    // Should identify some non-essential operations
    assert!(
        report.redundant_total_ms > 0,
        "should have identified redundant operations"
    );

    // Redundant should be < 10% of total (most work is essential)
    assert!(
        report.redundant_pct() < 10.0,
        "redundant should be small: {:.1}%",
        report.redundant_pct()
    );
}

// ============================================================================
// Report serialization
// ============================================================================

#[test]
fn e2e_json_report_is_well_formed() {
    let profile = DoctorCostProfile::canonical();
    let report = profile.finalize();
    let json = report.to_json();

    // Structural checks
    assert!(json.contains("\"schema_version\": 1"));
    assert!(json.contains("\"grand_total_ms\":"));
    assert!(json.contains("\"blocking_total_ms\":"));
    assert!(json.contains("\"blocking_pct\":"));
    assert!(json.contains("\"redundant_total_ms\":"));
    assert!(json.contains("\"stage_breakdown\":"));
    assert!(json.contains("\"lane_breakdown\":"));
    assert!(json.contains("\"optimization_targets\":"));

    // All stages should appear in breakdown
    for stage in WorkflowStage::ALL {
        assert!(
            json.contains(&format!("\"stage\": \"{}\"", stage.label())),
            "JSON missing stage '{}'",
            stage.label()
        );
    }

    // All lanes should appear in breakdown
    for lane in CostLane::ALL {
        assert!(
            json.contains(&format!("\"lane\": \"{}\"", lane.label())),
            "JSON missing lane '{}'",
            lane.label()
        );
    }
}

#[test]
fn e2e_summary_is_operator_readable() {
    let profile = DoctorCostProfile::canonical();
    let report = profile.finalize();
    let summary = report.summary();

    assert!(summary.contains("Doctor Workflow Cost Profile"));
    assert!(summary.contains("Total:"));
    assert!(summary.contains("Blocking:"));
    assert!(summary.contains("Stage breakdown:"));
    assert!(summary.contains("capture"));
    assert!(summary.contains("Top optimization targets:"));
}

// ============================================================================
// Custom profile integration
// ============================================================================

#[test]
fn e2e_custom_profile_accumulates_correctly() {
    let mut profile = DoctorCostProfile::new();

    profile.record(
        CostEntry::new(WorkflowStage::Capture, CostLane::Subprocess)
            .operation("vhs_record")
            .wall_clock_ms(50000)
            .blocking(true)
            .essential(true)
            .impact(OptimizationImpact::Critical),
    );

    profile.record(
        CostEntry::new(WorkflowStage::Seed, CostLane::Network)
            .operation("rpc_call")
            .wall_clock_ms(1000)
            .blocking(true)
            .essential(true)
            .impact(OptimizationImpact::Moderate),
    );

    profile.record(
        CostEntry::new(WorkflowStage::Report, CostLane::FileIo)
            .operation("write_html")
            .wall_clock_ms(200)
            .blocking(false)
            .essential(false)
            .impact(OptimizationImpact::TailOnly),
    );

    let report = profile.finalize();

    assert_eq!(report.grand_total_ms, 51200);
    assert_eq!(report.blocking_total_ms, 51000);
    assert_eq!(report.redundant_total_ms, 200);
    assert_eq!(report.stage_total(WorkflowStage::Capture), 50000);
    assert_eq!(report.stage_total(WorkflowStage::Seed), 1000);
    assert_eq!(report.stage_total(WorkflowStage::Report), 200);
    assert_eq!(report.optimization_targets.len(), 2); // Critical + Moderate
}
