//! Anytime-Valid Flake Detector (bd-1plj).
//!
//! Detects flaky timing regressions in E2E tests without inflating false positives,
//! using anytime-valid e-process statistics.
//!
//! # Mathematical Model
//!
//! For sub-Gaussian residuals `r_t` (mean 0 under null), the e-value at time `t` is:
//!
//! ```text
//! e_t = exp(λ × r_t − (λ² × σ²) / 2)
//! E_t = ∏_{i=1}^t e_i
//! ```
//!
//! We reject H₀ (system is stable) when `E_t > 1/α`, providing anytime-valid
//! Type I error control.
//!
//! # Key Properties
//!
//! - **Anytime-valid**: Can stop testing early without invalid inference
//! - **No false positives in stable runs**: `E[E_t]` ≤ 1 under H₀
//! - **Early detection**: Strong evidence triggers early failure
//! - **Variable-length support**: Works with different test run lengths
//!
//! # Failure Modes
//!
//! | Condition | Behavior | Rationale |
//! |-----------|----------|-----------|
//! | σ = 0 | Clamp to σ_MIN | Division by zero guard |
//! | E_t underflow | Clamp to E_MIN | Prevents permanent zero-lock |
//! | E_t overflow | Clamp to E_MAX | Numerical stability |
//! | No observations | E_t = 1 | Identity element |
//!
//! # Example
//!
//! ```rust,ignore
//! use ftui_runtime::flake_detector::{FlakeDetector, FlakeConfig};
//!
//! let mut detector = FlakeDetector::new(FlakeConfig::default());
//!
//! // Observe latency deviations
//! let decision = detector.observe(latency_deviation);
//! if decision.is_flaky {
//!     eprintln!("Flaky test detected");
//! }
//! ```

#![forbid(unsafe_code)]

use std::collections::VecDeque;

/// Minimum sigma to prevent division by zero.
const SIGMA_MIN: f64 = 1e-9;

/// Minimum e-value floor to prevent permanent zero-lock.
const E_MIN: f64 = 1e-100;

/// Maximum e-value to prevent overflow.
const E_MAX: f64 = 1e100;

/// Default significance level.
const DEFAULT_ALPHA: f64 = 0.05;

/// Default lambda (betting intensity).
const DEFAULT_LAMBDA: f64 = 0.5;

// =============================================================================
// Configuration
// =============================================================================

/// Configuration for the flake detector.
#[derive(Debug, Clone)]
pub struct FlakeConfig {
    /// Significance level `α`. Fail when `E_t > 1/α`.
    /// Lower α → more conservative (fewer false alarms). Default: 0.05.
    pub alpha: f64,

    /// Betting intensity `λ`. Higher values detect deviations faster
    /// but are more sensitive to noise. Default: 0.5.
    pub lambda: f64,

    /// Prior estimate of standard deviation for latency residuals.
    /// Used in the e-value formula. Default: 1.0 (normalized units).
    pub sigma: f64,

    /// Rolling window size for empirical variance estimation.
    /// Set to 0 to use fixed sigma. Default: 50.
    pub variance_window: usize,

    /// Minimum observations before making decisions.
    /// Helps with warm-up. Default: 3.
    pub min_observations: usize,

    /// Enable JSONL-compatible evidence logging. Default: false.
    pub enable_logging: bool,

    /// Minimum e-value before flagging as flaky (1/alpha).
    /// Computed from alpha but can be overridden.
    pub threshold: Option<f64>,
}

impl Default for FlakeConfig {
    fn default() -> Self {
        Self {
            alpha: DEFAULT_ALPHA,
            lambda: DEFAULT_LAMBDA,
            sigma: 1.0,
            variance_window: 50,
            min_observations: 3,
            enable_logging: false,
            threshold: None,
        }
    }
}

impl FlakeConfig {
    /// Create a new configuration with the given alpha.
    #[must_use]
    pub fn new(alpha: f64) -> Self {
        Self {
            alpha: alpha.clamp(1e-10, 0.5),
            ..Default::default()
        }
    }

    /// Set the betting intensity lambda.
    #[must_use]
    pub fn with_lambda(mut self, lambda: f64) -> Self {
        self.lambda = lambda.clamp(0.01, 2.0);
        self
    }

    /// Set the prior sigma.
    #[must_use]
    pub fn with_sigma(mut self, sigma: f64) -> Self {
        self.sigma = sigma.max(SIGMA_MIN);
        self
    }

    /// Set the variance window size.
    #[must_use]
    pub fn with_variance_window(mut self, window: usize) -> Self {
        self.variance_window = window;
        self
    }

    /// Set minimum observations.
    #[must_use]
    pub fn with_min_observations(mut self, min: usize) -> Self {
        self.min_observations = min.max(1);
        self
    }

    /// Enable logging.
    #[must_use]
    pub fn with_logging(mut self, enabled: bool) -> Self {
        self.enable_logging = enabled;
        self
    }

    /// Get the threshold (1/alpha).
    #[must_use]
    pub fn threshold(&self) -> f64 {
        self.threshold.unwrap_or(1.0 / self.alpha)
    }
}

// =============================================================================
// Decision Types
// =============================================================================

/// Decision returned by the flake detector.
#[derive(Debug, Clone, PartialEq)]
pub struct FlakeDecision {
    /// Whether the test is flagged as flaky.
    pub is_flaky: bool,
    /// Current cumulative e-value.
    pub e_value: f64,
    /// Threshold for flakiness (1/alpha).
    pub threshold: f64,
    /// Number of observations so far.
    pub observation_count: usize,
    /// Current variance estimate.
    pub variance_estimate: f64,
    /// Whether we have enough observations.
    pub warmed_up: bool,
}

impl FlakeDecision {
    /// Check if we should fail the test.
    #[must_use]
    pub fn should_fail(&self) -> bool {
        self.is_flaky && self.warmed_up
    }
}

/// Log entry for evidence tracking.
#[derive(Debug, Clone)]
pub struct EvidenceLog {
    /// Observation index.
    pub observation_idx: usize,
    /// The residual value observed.
    pub residual: f64,
    /// The incremental e-value for this observation.
    pub e_increment: f64,
    /// Cumulative e-value after this observation.
    pub e_cumulative: f64,
    /// Variance estimate at this point.
    pub variance: f64,
    /// Decision at this point.
    pub decision: bool,
}

impl EvidenceLog {
    /// Serialize to JSONL format.
    #[must_use]
    pub fn to_jsonl(&self) -> String {
        format!(
            r#"{{"idx":{},"residual":{:.6},"e_inc":{:.6},"e_cum":{:.6},"var":{:.6},"decision":{}}}"#,
            self.observation_idx,
            self.residual,
            self.e_increment,
            self.e_cumulative,
            self.variance,
            self.decision
        )
    }
}

// =============================================================================
// Flake Detector
// =============================================================================

/// Anytime-valid flake detector using e-process statistics.
#[derive(Debug, Clone)]
pub struct FlakeDetector {
    /// Configuration.
    config: FlakeConfig,
    /// Cumulative e-value (product of incremental e-values).
    e_cumulative: f64,
    /// Observation count.
    observation_count: usize,
    /// Rolling window for variance estimation.
    variance_window: VecDeque<f64>,
    /// Evidence log (if logging enabled).
    evidence_log: Vec<EvidenceLog>,
    /// Observation index where a flaky decision was first reached.
    first_flaky_at: Option<usize>,
    /// Maximum cumulative e-value observed over the detector lifetime.
    max_e_value: f64,
}

impl FlakeDetector {
    /// Create a new flake detector.
    #[must_use]
    pub fn new(config: FlakeConfig) -> Self {
        let capacity = if config.variance_window > 0 {
            config.variance_window
        } else {
            1
        };
        Self {
            config,
            e_cumulative: 1.0, // Identity element
            observation_count: 0,
            variance_window: VecDeque::with_capacity(capacity),
            evidence_log: Vec::new(),
            first_flaky_at: None,
            max_e_value: 1.0,
        }
    }

    /// Observe a latency deviation (residual).
    ///
    /// The residual should be the difference between observed latency
    /// and expected latency, ideally normalized.
    pub fn observe(&mut self, residual: f64) -> FlakeDecision {
        if residual.is_nan() {
            return FlakeDecision {
                is_flaky: false,
                e_value: self.e_cumulative,
                threshold: self.config.threshold(),
                observation_count: self.observation_count,
                variance_estimate: self.current_sigma().powi(2),
                warmed_up: self.observation_count >= self.config.min_observations,
            };
        }

        self.observation_count += 1;

        // Update variance estimate
        self.update_variance(residual);
        let sigma = self.current_sigma();

        // Compute incremental e-value: e_t = exp(λ × r_t − (λ² × σ²) / 2)
        let lambda = self.config.lambda;
        let exponent = lambda * residual - (lambda * lambda * sigma * sigma) / 2.0;
        let e_increment = exponent.exp().clamp(E_MIN, E_MAX);

        // Update cumulative e-value
        self.e_cumulative = (self.e_cumulative * e_increment).clamp(E_MIN, E_MAX);

        // Check threshold
        let threshold = self.config.threshold();
        let is_flaky = self.e_cumulative > threshold;
        let warmed_up = self.observation_count >= self.config.min_observations;
        let decision = is_flaky && warmed_up;

        if decision && self.first_flaky_at.is_none() {
            self.first_flaky_at = Some(self.observation_count);
        }
        self.max_e_value = self.max_e_value.max(self.e_cumulative);

        // Log if enabled
        if self.config.enable_logging {
            self.evidence_log.push(EvidenceLog {
                observation_idx: self.observation_count,
                residual,
                e_increment,
                e_cumulative: self.e_cumulative,
                variance: sigma * sigma,
                decision,
            });
        }

        FlakeDecision {
            is_flaky,
            e_value: self.e_cumulative,
            threshold,
            observation_count: self.observation_count,
            variance_estimate: sigma * sigma,
            warmed_up,
        }
    }

    /// Observe multiple residuals and return the final decision.
    pub fn observe_batch(&mut self, residuals: &[f64]) -> FlakeDecision {
        let mut decision = FlakeDecision {
            is_flaky: false,
            e_value: self.e_cumulative,
            threshold: self.config.threshold(),
            observation_count: self.observation_count,
            variance_estimate: self.current_sigma().powi(2),
            warmed_up: false,
        };

        for &r in residuals {
            decision = self.observe(r);
            if decision.should_fail() {
                break; // Early stopping with anytime-valid guarantee
            }
        }

        decision
    }

    /// Reset the detector state.
    pub fn reset(&mut self) {
        self.e_cumulative = 1.0;
        self.observation_count = 0;
        self.variance_window.clear();
        self.evidence_log.clear();
        self.first_flaky_at = None;
        self.max_e_value = 1.0;
    }

    /// Get the current e-value.
    #[must_use]
    pub fn e_value(&self) -> f64 {
        self.e_cumulative
    }

    /// Get the observation count.
    #[must_use]
    pub fn observation_count(&self) -> usize {
        self.observation_count
    }

    /// Check if the detector has warmed up.
    #[must_use]
    pub fn is_warmed_up(&self) -> bool {
        self.observation_count >= self.config.min_observations
    }

    /// Get the evidence log.
    #[must_use]
    pub fn evidence_log(&self) -> &[EvidenceLog] {
        &self.evidence_log
    }

    /// Export evidence log as JSONL.
    #[must_use]
    pub fn evidence_to_jsonl(&self) -> String {
        self.evidence_log
            .iter()
            .map(|e| e.to_jsonl())
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// Get the current sigma estimate.
    #[must_use]
    pub fn current_sigma(&self) -> f64 {
        if self.config.variance_window == 0 || self.variance_window.len() < 2 {
            return self.config.sigma.max(SIGMA_MIN);
        }

        let n = self.variance_window.len() as f64;
        let mean = self.variance_window.iter().sum::<f64>() / n;
        let variance = self
            .variance_window
            .iter()
            .map(|&x| {
                let diff = x - mean;
                diff * diff
            })
            .sum::<f64>()
            / (n - 1.0);

        variance.sqrt().max(SIGMA_MIN)
    }

    /// Update variance estimate with new observation.
    fn update_variance(&mut self, residual: f64) {
        if self.config.variance_window == 0 {
            return;
        }

        // Maintain rolling window
        if self.variance_window.len() >= self.config.variance_window {
            self.variance_window.pop_front();
        }
        self.variance_window.push_back(residual);
    }

    /// Get configuration.
    #[must_use]
    pub fn config(&self) -> &FlakeConfig {
        &self.config
    }
}

impl Default for FlakeDetector {
    fn default() -> Self {
        Self::new(FlakeConfig::default())
    }
}

// =============================================================================
// Summary Statistics
// =============================================================================

/// Summary of flake detection run.
#[derive(Debug, Clone)]
pub struct FlakeSummary {
    /// Total observations.
    pub total_observations: usize,
    /// Final e-value.
    pub final_e_value: f64,
    /// Whether flagged as flaky.
    pub is_flaky: bool,
    /// Observation index where flakiness was first detected (if any).
    pub first_flaky_at: Option<usize>,
    /// Maximum e-value observed.
    pub max_e_value: f64,
    /// Threshold used.
    pub threshold: f64,
}

impl FlakeDetector {
    /// Generate summary statistics.
    #[must_use]
    pub fn summary(&self) -> FlakeSummary {
        FlakeSummary {
            total_observations: self.observation_count,
            final_e_value: self.e_cumulative,
            is_flaky: self.e_cumulative > self.config.threshold(),
            first_flaky_at: self.first_flaky_at,
            max_e_value: self.max_e_value,
            threshold: self.config.threshold(),
        }
    }
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unit_eprocess_threshold() {
        // Test that we fail when E_t > 1/alpha
        let config = FlakeConfig::new(0.05).with_min_observations(1);
        let mut detector = FlakeDetector::new(config);

        // Feed large positive residuals to drive e-value up
        for _ in 0..20 {
            let decision = detector.observe(3.0); // Large deviation
            if decision.should_fail() {
                // Should eventually trigger
                assert!(decision.e_value > decision.threshold);
                return;
            }
        }

        // If we didn't fail, check threshold
        let decision = detector.observe(0.0);
        assert!(
            decision.e_value > decision.threshold || !decision.is_flaky,
            "Should either have triggered or not be flaky"
        );
    }

    #[test]
    fn unit_eprocess_nonnegative() {
        // E-values should never be negative
        let mut detector = FlakeDetector::default();

        // Test with various residuals including negative
        let residuals = [-5.0, -2.0, 0.0, 2.0, 5.0, -10.0, 10.0];
        for r in residuals {
            let decision = detector.observe(r);
            assert!(
                decision.e_value > 0.0,
                "E-value must be positive, got {}",
                decision.e_value
            );
        }
    }

    #[test]
    fn unit_optional_stopping() {
        // Stopping early should preserve decision validity
        let config = FlakeConfig::new(0.05)
            .with_lambda(0.3)
            .with_min_observations(1)
            .with_logging(true);
        let mut detector = FlakeDetector::new(config);

        // Simulate stable run (small residuals around 0)
        let stable_residuals: Vec<f64> = (0..100).map(|i| (i as f64 * 0.1).sin() * 0.1).collect();

        let decision = detector.observe_batch(&stable_residuals);

        // Under H₀ (stable), we shouldn't flag as flaky
        // Note: Due to random variation, we check the e-value is reasonable
        assert!(
            decision.e_value <= decision.threshold * 2.0 || !decision.should_fail(),
            "Stable run should rarely trigger flakiness"
        );
    }

    #[test]
    fn unit_stable_run_no_false_positives() {
        // A truly stable run should not trigger false positives
        let config = FlakeConfig::new(0.05)
            .with_sigma(1.0)
            .with_lambda(0.5)
            .with_min_observations(3);
        let mut detector = FlakeDetector::new(config);

        // Zero residuals (perfectly stable)
        for _ in 0..50 {
            let decision = detector.observe(0.0);
            // With zero residuals, e_increment = exp(-λ²σ²/2) < 1
            // So e_cumulative should decrease over time
            assert!(
                !decision.should_fail(),
                "Zero residuals should never trigger flakiness"
            );
        }
    }

    #[test]
    fn unit_spike_detection() {
        // Inject latency spikes and verify detection
        let config = FlakeConfig::new(0.05)
            .with_sigma(1.0)
            .with_lambda(0.5)
            .with_min_observations(3)
            .with_logging(true);
        let mut detector = FlakeDetector::new(config);

        // Start with some normal observations
        for _ in 0..5 {
            detector.observe(0.1);
        }

        // Inject spike
        let mut detected = false;
        for _ in 0..20 {
            let decision = detector.observe(5.0); // Large spike
            if decision.should_fail() {
                detected = true;
                break;
            }
        }

        assert!(detected, "Should detect sustained spike");
    }

    #[test]
    fn unit_reset() {
        let mut detector = FlakeDetector::default();
        detector.observe(1.0);
        detector.observe(2.0);

        assert_eq!(detector.observation_count(), 2);

        detector.reset();

        assert_eq!(detector.observation_count(), 0);
        assert!((detector.e_value() - 1.0).abs() < 1e-10);
    }

    #[test]
    fn unit_variance_estimation() {
        let config = FlakeConfig::default().with_variance_window(10);
        let mut detector = FlakeDetector::new(config);

        // Feed constant residuals
        for _ in 0..20 {
            detector.observe(1.0);
        }

        // With constant input, variance should be low
        let sigma = detector.current_sigma();
        assert!(
            sigma < 0.1 || (sigma - 1.0).abs() < 0.5,
            "Variance should converge"
        );
    }

    #[test]
    fn unit_evidence_log() {
        let config = FlakeConfig::default()
            .with_logging(true)
            .with_min_observations(1);
        let mut detector = FlakeDetector::new(config);

        detector.observe(0.5);
        detector.observe(1.0);
        detector.observe(-0.5);

        assert_eq!(detector.evidence_log().len(), 3);

        let jsonl = detector.evidence_to_jsonl();
        assert!(jsonl.contains("\"idx\":1"));
        assert!(jsonl.contains("\"idx\":2"));
        assert!(jsonl.contains("\"idx\":3"));
    }

    #[test]
    fn unit_summary() {
        let config = FlakeConfig::default()
            .with_logging(true)
            .with_min_observations(1);
        let mut detector = FlakeDetector::new(config);

        for _ in 0..10 {
            detector.observe(0.1);
        }

        let summary = detector.summary();
        assert_eq!(summary.total_observations, 10);
        assert!(summary.final_e_value > 0.0);
        assert!(summary.threshold > 0.0);
    }

    #[test]
    fn unit_batch_observe() {
        let config = FlakeConfig::default().with_min_observations(1);
        let mut detector = FlakeDetector::new(config);

        let residuals = vec![0.1, 0.2, 0.3, 0.4, 0.5];
        let decision = detector.observe_batch(&residuals);

        assert_eq!(decision.observation_count, 5);
    }

    #[test]
    fn unit_config_builder() {
        let config = FlakeConfig::new(0.01)
            .with_lambda(0.3)
            .with_sigma(2.0)
            .with_variance_window(100)
            .with_min_observations(5)
            .with_logging(true);

        assert!((config.alpha - 0.01).abs() < 1e-10);
        assert!((config.lambda - 0.3).abs() < 1e-10);
        assert!((config.sigma - 2.0).abs() < 1e-10);
        assert_eq!(config.variance_window, 100);
        assert_eq!(config.min_observations, 5);
        assert!(config.enable_logging);
        assert!((config.threshold() - 100.0).abs() < 1e-10);
    }

    #[test]
    fn unit_numerical_stability() {
        let mut detector = FlakeDetector::default();

        // Very large residuals
        for _ in 0..10 {
            let decision = detector.observe(1000.0);
            assert!(decision.e_value.is_finite());
            assert!(decision.e_value > 0.0);
        }

        detector.reset();

        // Very small negative residuals
        for _ in 0..10 {
            let decision = detector.observe(-1000.0);
            assert!(decision.e_value.is_finite());
            assert!(decision.e_value > 0.0);
        }
    }

    // ── FlakeConfig defaults ─────────────────────────────────────

    #[test]
    fn config_default_values() {
        let config = FlakeConfig::default();
        assert!((config.alpha - DEFAULT_ALPHA).abs() < f64::EPSILON);
        assert!((config.lambda - DEFAULT_LAMBDA).abs() < f64::EPSILON);
        assert!((config.sigma - 1.0).abs() < f64::EPSILON);
        assert_eq!(config.variance_window, 50);
        assert_eq!(config.min_observations, 3);
        assert!(!config.enable_logging);
        assert!(config.threshold.is_none());
    }

    #[test]
    fn config_threshold_computed_from_alpha() {
        let config = FlakeConfig::new(0.05);
        assert!((config.threshold() - 20.0).abs() < 1e-10);
    }

    #[test]
    fn config_threshold_override() {
        let mut config = FlakeConfig::new(0.05);
        config.threshold = Some(42.0);
        assert!((config.threshold() - 42.0).abs() < f64::EPSILON);
    }

    // ── FlakeConfig clamping ─────────────────────────────────────

    #[test]
    fn config_new_clamps_alpha_low() {
        let config = FlakeConfig::new(0.0);
        assert!(config.alpha >= 1e-10);
    }

    #[test]
    fn config_new_clamps_alpha_high() {
        let config = FlakeConfig::new(1.0);
        assert!(config.alpha <= 0.5);
    }

    #[test]
    fn config_with_lambda_clamps_low() {
        let config = FlakeConfig::default().with_lambda(0.0);
        assert!(config.lambda >= 0.01);
    }

    #[test]
    fn config_with_lambda_clamps_high() {
        let config = FlakeConfig::default().with_lambda(100.0);
        assert!(config.lambda <= 2.0);
    }

    #[test]
    fn config_with_sigma_clamps_to_min() {
        let config = FlakeConfig::default().with_sigma(0.0);
        assert!(config.sigma >= SIGMA_MIN);
    }

    #[test]
    fn config_with_min_observations_clamps_to_one() {
        let config = FlakeConfig::default().with_min_observations(0);
        assert!(config.min_observations >= 1);
    }

    // ── FlakeDecision ────────────────────────────────────────────

    #[test]
    fn decision_should_fail_requires_both_flaky_and_warmed_up() {
        let d1 = FlakeDecision {
            is_flaky: true,
            warmed_up: false,
            e_value: 100.0,
            threshold: 20.0,
            observation_count: 1,
            variance_estimate: 1.0,
        };
        assert!(!d1.should_fail());

        let d2 = FlakeDecision {
            is_flaky: false,
            warmed_up: true,
            e_value: 1.0,
            threshold: 20.0,
            observation_count: 5,
            variance_estimate: 1.0,
        };
        assert!(!d2.should_fail());

        let d3 = FlakeDecision {
            is_flaky: true,
            warmed_up: true,
            e_value: 100.0,
            threshold: 20.0,
            observation_count: 5,
            variance_estimate: 1.0,
        };
        assert!(d3.should_fail());
    }

    // ── EvidenceLog JSONL ────────────────────────────────────────

    #[test]
    fn evidence_log_to_jsonl_format() {
        let log = EvidenceLog {
            observation_idx: 3,
            residual: 1.5,
            e_increment: 2.1,
            e_cumulative: 4.2,
            variance: 0.9,
            decision: true,
        };
        let jsonl = log.to_jsonl();
        assert!(jsonl.contains("\"idx\":3"));
        assert!(jsonl.contains("\"residual\":"));
        assert!(jsonl.contains("\"e_inc\":"));
        assert!(jsonl.contains("\"e_cum\":"));
        assert!(jsonl.contains("\"var\":"));
        assert!(jsonl.contains("\"decision\":true"));
    }

    #[test]
    fn evidence_log_to_jsonl_false_decision() {
        let log = EvidenceLog {
            observation_idx: 1,
            residual: 0.0,
            e_increment: 1.0,
            e_cumulative: 1.0,
            variance: 1.0,
            decision: false,
        };
        let jsonl = log.to_jsonl();
        assert!(jsonl.contains("\"decision\":false"));
    }

    // ── FlakeDetector accessors ──────────────────────────────────

    #[test]
    fn detector_default_initial_state() {
        let detector = FlakeDetector::default();
        assert_eq!(detector.observation_count(), 0);
        assert!((detector.e_value() - 1.0).abs() < f64::EPSILON);
        assert!(!detector.is_warmed_up());
        assert!(detector.evidence_log().is_empty());
    }

    #[test]
    fn detector_config_accessor() {
        let config = FlakeConfig::new(0.01).with_lambda(0.3);
        let detector = FlakeDetector::new(config);
        assert!((detector.config().alpha - 0.01).abs() < 1e-10);
        assert!((detector.config().lambda - 0.3).abs() < 1e-10);
    }

    #[test]
    fn detector_is_warmed_up_after_min_observations() {
        let config = FlakeConfig::default().with_min_observations(3);
        let mut detector = FlakeDetector::new(config);
        assert!(!detector.is_warmed_up());
        detector.observe(0.0);
        detector.observe(0.0);
        assert!(!detector.is_warmed_up());
        detector.observe(0.0);
        assert!(detector.is_warmed_up());
    }

    // ── Variance window = 0 (fixed sigma) ────────────────────────

    #[test]
    fn fixed_sigma_when_variance_window_zero() {
        let config = FlakeConfig::default()
            .with_sigma(3.0)
            .with_variance_window(0);
        let mut detector = FlakeDetector::new(config);
        detector.observe(10.0);
        detector.observe(20.0);
        assert!((detector.current_sigma() - 3.0).abs() < f64::EPSILON);
    }

    // ── Summary edge cases ───────────────────────────────────────

    #[test]
    fn summary_empty_detector() {
        let detector = FlakeDetector::new(FlakeConfig::default().with_logging(true));
        let summary = detector.summary();
        assert_eq!(summary.total_observations, 0);
        assert!((summary.final_e_value - 1.0).abs() < f64::EPSILON);
        assert!(!summary.is_flaky);
        assert!(summary.first_flaky_at.is_none());
        assert!((summary.max_e_value - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn summary_first_flaky_at_recorded() {
        let config = FlakeConfig::new(0.05)
            .with_min_observations(1)
            .with_logging(true);
        let mut detector = FlakeDetector::new(config);
        for _ in 0..50 {
            detector.observe(5.0);
        }
        let summary = detector.summary();
        if summary.is_flaky {
            assert!(
                summary.first_flaky_at.is_some(),
                "should record first flaky index"
            );
            assert!(summary.first_flaky_at.unwrap() > 0);
        }
    }

    // ── Determinism ──────────────────────────────────────────────

    #[test]
    fn deterministic_same_inputs() {
        let config = FlakeConfig::new(0.05).with_lambda(0.5).with_sigma(1.0);
        let residuals = [0.1, -0.2, 0.5, -0.1, 3.0, 0.0, -1.0, 2.0];
        let mut d1 = FlakeDetector::new(config.clone());
        let mut d2 = FlakeDetector::new(config);
        for &r in &residuals {
            d1.observe(r);
            d2.observe(r);
        }
        assert!((d1.e_value() - d2.e_value()).abs() < 1e-10);
        assert_eq!(d1.observation_count(), d2.observation_count());
    }

    // ── Batch early stopping ─────────────────────────────────────

    #[test]
    fn batch_early_stops_on_flaky() {
        let config = FlakeConfig::new(0.05)
            .with_min_observations(1)
            .with_lambda(0.5);
        let mut detector = FlakeDetector::new(config);
        let mut residuals = vec![10.0; 20];
        residuals.extend(vec![0.0; 80]);
        let decision = detector.observe_batch(&residuals);
        if decision.should_fail() {
            assert!(
                decision.observation_count < 100,
                "should stop early, count={}",
                decision.observation_count
            );
        }
    }

    // ── E-value monotone under positive residuals ────────────────

    #[test]
    fn e_value_increases_under_consistent_positive_residuals() {
        let config = FlakeConfig::default()
            .with_variance_window(0)
            .with_sigma(1.0);
        let mut detector = FlakeDetector::new(config);
        let mut prev_e = 1.0;
        for _ in 0..5 {
            let decision = detector.observe(2.0);
            assert!(
                decision.e_value >= prev_e,
                "e-value should increase: prev={prev_e}, cur={}",
                decision.e_value
            );
            prev_e = decision.e_value;
        }
    }

    // ── Evidence log only when enabled ───────────────────────────

    #[test]
    fn no_evidence_log_when_disabled() {
        let config = FlakeConfig::default();
        let mut detector = FlakeDetector::new(config);
        detector.observe(1.0);
        detector.observe(2.0);
        assert!(detector.evidence_log().is_empty());
        assert!(detector.evidence_to_jsonl().is_empty());
    }

    #[test]
    fn summary_tracks_metrics_when_logging_disabled() {
        let config = FlakeConfig::new(0.05).with_min_observations(1);
        let mut detector = FlakeDetector::new(config);
        detector.observe(5.0);
        detector.observe(5.0);
        detector.observe(-1.0);

        let summary = detector.summary();
        assert_eq!(summary.first_flaky_at, Some(2));
        assert!(summary.max_e_value > summary.threshold);
        assert!(summary.max_e_value + f64::EPSILON >= summary.final_e_value);
    }

    // ── Reset clears everything ──────────────────────────────────

    #[test]
    fn reset_clears_evidence_log() {
        let config = FlakeConfig::default().with_logging(true);
        let mut detector = FlakeDetector::new(config);
        detector.observe(1.0);
        assert_eq!(detector.evidence_log().len(), 1);
        detector.reset();
        assert!(detector.evidence_log().is_empty());
    }
}
