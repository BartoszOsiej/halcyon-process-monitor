//! E2E integration tests for the fixture runner pipeline (bd-muv6p).
//!
//! These tests validate that the full `FixtureSpec → FixtureRunner → BaselineRecord`
//! pipeline works end-to-end, including determinism verification, multi-viewport
//! execution, and manifest generation.

#![forbid(unsafe_code)]

use ftui_harness::baseline_capture::StabilityClass;
use ftui_harness::fixture_runner::{FixtureRunResult, FixtureRunner};
use ftui_harness::fixture_suite::{
    FixtureRegistry, FixtureSpec, SuitePartition, TransitionPattern, ViewportSpec,
};

// ============================================================================
// Full pipeline E2E
// ============================================================================

#[test]
fn e2e_canonical_registry_all_fixtures_produce_baselines() {
    let reg = FixtureRegistry::canonical();
    let mut results: Vec<(String, FixtureRunResult)> = Vec::new();

    for spec in reg.all() {
        let result = FixtureRunner::run(spec);

        // Every fixture must complete all frames
        assert_eq!(
            result.frames_executed, spec.frame_count,
            "fixture {} incomplete: {}/{}",
            spec.id, result.frames_executed, spec.frame_count
        );

        // Every fixture must produce at least one metric
        assert!(
            !result.record.metrics.is_empty(),
            "fixture {} produced no metrics",
            spec.id
        );

        // Every fixture must produce frame checksums
        assert_eq!(
            result.frame_checksums.len(),
            spec.frame_count as usize,
            "fixture {} checksum count mismatch",
            spec.id
        );

        results.push((spec.id.clone(), result));
    }

    // Must have run at least 14 fixtures (the canonical set)
    assert!(
        results.len() >= 14,
        "expected at least 14 fixtures, got {}",
        results.len()
    );

    // Manifest generation should succeed
    let manifest = FixtureRunner::results_manifest(&results);
    assert!(manifest.contains("\"schema_version\": 1"));
    assert!(manifest.contains(&format!("\"run_count\": {}", results.len())));
}

// ============================================================================
// Determinism proofs
// ============================================================================

#[test]
fn e2e_all_canonical_fixtures_are_deterministic() {
    let reg = FixtureRegistry::canonical();

    for spec in reg.by_partition(SuitePartition::Canonical) {
        let verdict = FixtureRunner::verify_determinism(spec);
        assert!(
            verdict.deterministic,
            "canonical fixture {} is non-deterministic: {}",
            spec.id,
            verdict.summary()
        );
    }
}

#[test]
fn e2e_negative_controls_are_deterministic() {
    let reg = FixtureRegistry::canonical();

    for spec in reg.by_partition(SuitePartition::NegativeControl) {
        let verdict = FixtureRunner::verify_determinism(spec);
        assert!(
            verdict.deterministic,
            "negative control {} is non-deterministic: {}",
            spec.id,
            verdict.summary()
        );
    }
}

// ============================================================================
// Baseline quality
// ============================================================================

#[test]
fn e2e_render_baselines_have_expected_metrics() {
    let reg = FixtureRegistry::canonical();
    let expected_metrics = [
        "buffer_diff",
        "presenter_emit",
        "frame_pipeline_total",
        "ansi_bytes_per_frame",
        "cells_changed_per_frame",
        "cell_mutation",
    ];

    for spec in reg
        .all()
        .into_iter()
        .filter(|s| s.family == ftui_harness::baseline_capture::FixtureFamily::Render)
        .filter(|s| s.partition == SuitePartition::Canonical)
    {
        let result = FixtureRunner::run(spec);
        let metric_names: Vec<&str> = result
            .record
            .metrics
            .iter()
            .map(|m| m.metric.as_str())
            .collect();

        for expected in &expected_metrics {
            assert!(
                metric_names.contains(expected),
                "render fixture {} missing metric '{}'",
                spec.id,
                expected
            );
        }
    }
}

#[test]
fn e2e_baseline_records_serialize_to_valid_json() {
    let reg = FixtureRegistry::canonical();

    for spec in reg.all() {
        let result = FixtureRunner::run(spec);
        let json = result.record.to_json();

        // Basic structural checks (we don't have serde_json in this test
        // context for full parsing, but we validate key fields are present)
        assert!(
            json.contains("\"schema_version\":"),
            "fixture {} baseline JSON missing schema_version",
            spec.id
        );
        assert!(
            json.contains(&format!("\"fixture\": \"{}\"", spec.id)),
            "fixture {} baseline JSON missing fixture id",
            spec.id
        );
        assert!(
            json.contains("\"metrics\":"),
            "fixture {} baseline JSON missing metrics",
            spec.id
        );
        assert!(
            json.contains("\"stable\":"),
            "fixture {} baseline JSON missing stable flag",
            spec.id
        );
    }
}

// ============================================================================
// Multi-viewport coverage
// ============================================================================

#[test]
fn e2e_multi_viewport_fixtures_produce_separate_results() {
    let reg = FixtureRegistry::canonical();

    // The sparse diff fixture has extra viewports (MEDIUM, LARGE)
    let spec = reg.get("render_diff_sparse_80x24").unwrap();
    assert!(
        !spec.extra_viewports.is_empty(),
        "sparse diff fixture should have extra viewports"
    );

    let results = FixtureRunner::run_all_viewports(spec);

    // Should have primary + extras
    let expected_count = 1 + spec.extra_viewports.len();
    assert_eq!(results.len(), expected_count);

    // Each viewport should produce ANSI output
    for (vp, result) in &results {
        assert!(
            result.total_ansi_bytes > 0,
            "viewport {}x{} should produce ANSI output",
            vp.width,
            vp.height
        );
    }

    // All runs should complete all frames
    for (vp, result) in &results {
        assert_eq!(
            result.frames_executed, spec.frame_count,
            "viewport {}x{} incomplete",
            vp.width, vp.height
        );
    }
}

// ============================================================================
// Custom fixture execution
// ============================================================================

#[test]
fn e2e_custom_fixture_spec_executes() {
    let custom = FixtureSpec::new(
        "custom_e2e_test",
        "Custom E2E test fixture",
        ftui_harness::baseline_capture::FixtureFamily::Render,
    )
    .viewport(ViewportSpec::TINY)
    .transitions(vec![TransitionPattern::LargeInvalidation])
    .frame_count(25)
    .rationale("E2E test fixture for custom workload execution")
    .tests_hypothesis("Custom fixtures should execute correctly");

    let result = FixtureRunner::run(&custom);
    assert_eq!(result.frames_executed, 25);
    assert!(result.total_ansi_bytes > 0);
    assert!(!result.record.metrics.is_empty());
}

#[test]
fn e2e_partition_run_filters_correctly() {
    let reg = FixtureRegistry::canonical();
    let all = reg.all();

    let canonical_results = FixtureRunner::run_partition(&all, SuitePartition::Canonical);
    let challenge_results = FixtureRunner::run_partition(&all, SuitePartition::Challenge);
    let control_results = FixtureRunner::run_partition(&all, SuitePartition::NegativeControl);

    let canonical_count = reg.by_partition(SuitePartition::Canonical).len();
    let challenge_count = reg.by_partition(SuitePartition::Challenge).len();
    let control_count = reg.by_partition(SuitePartition::NegativeControl).len();

    assert_eq!(canonical_results.len(), canonical_count);
    assert_eq!(challenge_results.len(), challenge_count);
    assert_eq!(control_results.len(), control_count);
}

// ============================================================================
// Throughput metric
// ============================================================================

#[test]
fn e2e_throughput_metric_is_reasonable() {
    let reg = FixtureRegistry::canonical();
    let spec = reg.get("render_diff_sparse_80x24").unwrap();
    let result = FixtureRunner::run(spec);

    // Find the frames_per_second metric
    let fps_metric = result
        .record
        .metrics
        .iter()
        .find(|m| m.metric == "frames_per_second");

    assert!(fps_metric.is_some(), "should have fps metric");
    let fps = fps_metric.unwrap();

    // At 80x24 with sparse updates, we should achieve at least 100 fps
    // (these are synthetic workloads, not real terminal I/O)
    assert!(
        fps.mean > 100.0,
        "fps too low: {:.1} (expected > 100 for synthetic workload)",
        fps.mean
    );
}

// ============================================================================
// Stability classification
// ============================================================================

#[test]
fn e2e_output_cost_metrics_are_stable() {
    let reg = FixtureRegistry::canonical();
    let spec = reg.get("render_diff_sparse_80x24").unwrap();
    let result = FixtureRunner::run(spec);

    // Output cost metrics (cells changed, ansi bytes) should be deterministic
    // and therefore stable across frames with the same seed
    for m in &result.record.metrics {
        if m.metric == "cells_changed_per_frame" || m.metric == "ansi_bytes_per_frame" {
            assert!(
                m.stability != StabilityClass::Unstable,
                "output cost metric '{}' should not be unstable (cv={:.4})",
                m.metric,
                m.cv
            );
        }
    }
}
