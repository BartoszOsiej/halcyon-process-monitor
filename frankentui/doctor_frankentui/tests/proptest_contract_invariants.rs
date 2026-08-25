//! Property-based invariant tests for the contract engine modules.
//!
//! These tests verify structural invariants that must hold for **any** valid
//! parameter combination:
//!
//! 1. Posterior mean is monotonically increasing with successes.
//! 2. Posterior mean is monotonically decreasing with failures.
//! 3. Posterior variance is monotonically decreasing with total evidence.
//! 4. Posterior mean is always in [0, 1].
//! 5. Credible interval lower <= mean <= upper.
//! 6. Credible interval bounds are in [0, 1].
//! 7. Decision transitions are monotonic across confidence thresholds.
//! 8. Expected loss values are non-negative.
//! 9. Same inputs produce identical posterior (determinism).
//! 10. Same inputs produce identical decision (determinism).
//! 11. Round-trip serialization is stable for all computed values.
//! 12. Evidence manifest JSONL record count equals stage count.
//! 13. Policy matrix planner/certification row counts equal catalog size.
//! 14. Semantic contract clause lookup is consistent.

use doctor_frankentui::semantic_contract::{
    BayesianPosterior, MigrationDecision, load_builtin_confidence_model,
    load_builtin_evidence_manifest, load_builtin_semantic_contract,
    load_builtin_transformation_policy_matrix,
};
use proptest::prelude::*;

// ── Strategies ────────────────────────────────────────────────────────────

/// Successes/failures for posterior computation (reasonable range to avoid overflow).
fn evidence_pair() -> impl Strategy<Value = (u32, u32)> {
    (0u32..500, 0u32..500)
}

/// Posterior mean values spanning the full [0, 1] range.
fn confidence_value() -> impl Strategy<Value = f64> {
    (0u32..=1000).prop_map(|v| f64::from(v) / 1000.0)
}

/// Build a synthetic BayesianPosterior from a given mean value.
fn synthetic_posterior(mean: f64) -> BayesianPosterior {
    BayesianPosterior {
        alpha: mean * 100.0 + 1.0,
        beta: (1.0 - mean) * 100.0 + 1.0,
        mean,
        variance: 0.01,
        credible_lower: (mean - 0.05).max(0.0),
        credible_upper: (mean + 0.05).min(1.0),
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// 1. Posterior mean monotonically increases with successes
// ═══════════════════════════════════════════════════════════════════════════

proptest! {
    #[test]
    fn posterior_mean_monotone_increasing_with_successes(
        base_s in 0u32..100,
        base_f in 0u32..100,
        extra in 1u32..100,
    ) {
        let model = load_builtin_confidence_model().unwrap();
        let p_low = model.compute_posterior(base_s, base_f);
        let p_high = model.compute_posterior(base_s + extra, base_f);
        prop_assert!(
            p_high.mean >= p_low.mean,
            "adding {} successes should not decrease mean: {:.6} < {:.6}",
            extra, p_high.mean, p_low.mean
        );
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// 2. Posterior mean monotonically decreases with failures
// ═══════════════════════════════════════════════════════════════════════════

proptest! {
    #[test]
    fn posterior_mean_monotone_decreasing_with_failures(
        base_s in 0u32..100,
        base_f in 0u32..100,
        extra in 1u32..100,
    ) {
        let model = load_builtin_confidence_model().unwrap();
        let p_low = model.compute_posterior(base_s, base_f);
        let p_high = model.compute_posterior(base_s, base_f + extra);
        prop_assert!(
            p_high.mean <= p_low.mean,
            "adding {} failures should not increase mean: {:.6} > {:.6}",
            extra, p_high.mean, p_low.mean
        );
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// 3. Posterior variance is bounded by Beta distribution upper bound
//    and decreases when evidence is added proportionally
// ═══════════════════════════════════════════════════════════════════════════

proptest! {
    #[test]
    fn posterior_variance_bounded_by_theoretical_maximum((s, f) in evidence_pair()) {
        let model = load_builtin_confidence_model().unwrap();
        let p = model.compute_posterior(s, f);
        // Beta(alpha, beta) variance is always <= 1 / (4 * (alpha + beta + 1))
        let upper_bound = 0.25 / (p.alpha + p.beta + 1.0);
        prop_assert!(
            p.variance <= upper_bound + 1e-15,
            "variance {:.10} exceeds theoretical upper bound {:.10} for s={}, f={}",
            p.variance, upper_bound, s, f
        );
    }

    #[test]
    fn posterior_variance_decreases_with_proportional_evidence(
        s in 1u32..50,
        f in 1u32..50,
        scale in 2u32..10,
    ) {
        let model = load_builtin_confidence_model().unwrap();
        let p_small = model.compute_posterior(s, f);
        let p_large = model.compute_posterior(s * scale, f * scale);
        prop_assert!(
            p_large.variance <= p_small.variance,
            "proportionally scaled evidence should not increase variance: {:.8} > {:.8}",
            p_large.variance, p_small.variance
        );
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// 4. Posterior mean is always in [0, 1]
// ═══════════════════════════════════════════════════════════════════════════

proptest! {
    #[test]
    fn posterior_mean_always_in_unit_interval((s, f) in evidence_pair()) {
        let model = load_builtin_confidence_model().unwrap();
        let p = model.compute_posterior(s, f);
        prop_assert!(
            (0.0..=1.0).contains(&p.mean),
            "posterior mean {:.6} not in [0, 1] for s={}, f={}",
            p.mean, s, f
        );
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// 5. Credible interval: lower <= mean <= upper
// ═══════════════════════════════════════════════════════════════════════════

proptest! {
    #[test]
    fn credible_interval_contains_mean((s, f) in evidence_pair()) {
        let model = load_builtin_confidence_model().unwrap();
        let p = model.compute_posterior(s, f);
        prop_assert!(
            p.credible_lower <= p.mean,
            "credible_lower ({:.6}) > mean ({:.6})",
            p.credible_lower, p.mean
        );
        prop_assert!(
            p.credible_upper >= p.mean,
            "credible_upper ({:.6}) < mean ({:.6})",
            p.credible_upper, p.mean
        );
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// 6. Credible interval bounds are in [0, 1]
// ═══════════════════════════════════════════════════════════════════════════

proptest! {
    #[test]
    fn credible_interval_bounds_in_unit_interval((s, f) in evidence_pair()) {
        let model = load_builtin_confidence_model().unwrap();
        let p = model.compute_posterior(s, f);
        prop_assert!(
            p.credible_lower >= 0.0,
            "credible_lower ({:.6}) < 0.0",
            p.credible_lower
        );
        prop_assert!(
            p.credible_upper <= 1.0,
            "credible_upper ({:.6}) > 1.0",
            p.credible_upper
        );
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// 7. Decision transitions are monotonic: higher mean => less conservative
// ═══════════════════════════════════════════════════════════════════════════

fn decision_severity(d: MigrationDecision) -> u8 {
    match d {
        MigrationDecision::ConservativeFallback => 0,
        MigrationDecision::Rollback => 1,
        MigrationDecision::HardReject => 2,
        MigrationDecision::Reject => 3,
        MigrationDecision::HumanReview => 4,
        MigrationDecision::AutoApprove => 5,
    }
}

proptest! {
    #[test]
    fn decision_monotone_with_confidence(
        low_val in 0u32..500,
        high_delta in 1u32..500,
    ) {
        let model = load_builtin_confidence_model().unwrap();
        let low = f64::from(low_val) / 1000.0;
        let high = (f64::from(low_val) + f64::from(high_delta)) / 1000.0;
        let high = high.min(1.0);

        let p_low = synthetic_posterior(low);
        let p_high = synthetic_posterior(high);

        let d_low = model.decide(&p_low);
        let d_high = model.decide(&p_high);

        prop_assert!(
            decision_severity(d_high) >= decision_severity(d_low),
            "higher confidence ({:.3}) should not yield more conservative decision than lower ({:.3}): {:?} vs {:?}",
            high, low, d_high, d_low
        );
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// 8. Expected loss values are non-negative
// ═══════════════════════════════════════════════════════════════════════════

proptest! {
    #[test]
    fn expected_loss_values_are_nonnegative((s, f) in evidence_pair()) {
        let model = load_builtin_confidence_model().unwrap();
        let posterior = model.compute_posterior(s, f);
        let result = model.expected_loss_decision(&posterior, None, None);
        prop_assert!(
            result.expected_loss_accept >= 0.0,
            "EL(accept) = {:.4} < 0",
            result.expected_loss_accept
        );
        prop_assert!(
            result.expected_loss_reject >= 0.0,
            "EL(reject) = {:.4} < 0",
            result.expected_loss_reject
        );
        prop_assert!(
            result.expected_loss_hold >= 0.0,
            "EL(hold) = {:.4} < 0",
            result.expected_loss_hold
        );
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// 9. Posterior computation is deterministic (same inputs → same output)
// ═══════════════════════════════════════════════════════════════════════════

proptest! {
    #[test]
    fn posterior_is_deterministic((s, f) in evidence_pair()) {
        let model = load_builtin_confidence_model().unwrap();
        let p1 = model.compute_posterior(s, f);
        let p2 = model.compute_posterior(s, f);
        prop_assert_eq!(p1, p2, "posterior must be deterministic for s={}, f={}", s, f);
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// 10. Decision is deterministic (same inputs → same decision)
// ═══════════════════════════════════════════════════════════════════════════

proptest! {
    #[test]
    fn decision_is_deterministic(mean_val in confidence_value()) {
        let model = load_builtin_confidence_model().unwrap();
        let p = synthetic_posterior(mean_val);
        let d1 = model.decide(&p);
        let d2 = model.decide(&p);
        prop_assert_eq!(d1, d2, "decision must be deterministic for mean={:.3}", mean_val);
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// 11. Round-trip serialization is stable for computed values
// ═══════════════════════════════════════════════════════════════════════════

proptest! {
    #[test]
    fn expected_loss_result_round_trips((s, f) in evidence_pair()) {
        let model = load_builtin_confidence_model().unwrap();
        let posterior = model.compute_posterior(s, f);
        let result = model.expected_loss_decision(&posterior, None, None);
        let json = serde_json::to_string(&result).unwrap();
        let deser: doctor_frankentui::semantic_contract::ExpectedLossResult =
            serde_json::from_str(&json).unwrap();
        prop_assert_eq!(result.decision, deser.decision);
        prop_assert_eq!(result.rationale, deser.rationale);
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// 12. Evidence manifest JSONL count equals stage count
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn evidence_manifest_jsonl_count_equals_stage_count() {
    let manifest = load_builtin_evidence_manifest().unwrap();
    let records = manifest.to_evidence_records();
    assert_eq!(
        records.len(),
        manifest.stages.len(),
        "evidence records count must equal stage count"
    );
    let jsonl = manifest.evidence_jsonl();
    assert_eq!(
        jsonl.lines().count(),
        manifest.stages.len(),
        "JSONL line count must equal stage count"
    );
}

// ═══════════════════════════════════════════════════════════════════════════
// 13. Policy matrix row counts equal catalog size
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn policy_matrix_row_counts_equal_catalog_size() {
    let matrix = load_builtin_transformation_policy_matrix().unwrap();
    assert_eq!(
        matrix.planner_rows().len(),
        matrix.construct_catalog.len(),
        "planner rows count must equal catalog size"
    );
    assert_eq!(
        matrix.certification_rows().len(),
        matrix.construct_catalog.len(),
        "certification rows count must equal catalog size"
    );
}

// ═══════════════════════════════════════════════════════════════════════════
// 14. Semantic contract clause lookup is consistent
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn semantic_contract_clause_lookup_is_exhaustive() {
    let contract = load_builtin_semantic_contract().unwrap();
    for clause in &contract.clauses {
        assert!(
            contract.clause(&clause.clause_id).is_some(),
            "clause '{}' must be findable via lookup",
            clause.clause_id
        );
    }
    assert!(
        contract.clause("NONEXISTENT-CLAUSE-ID").is_none(),
        "nonexistent clause must return None"
    );
}
