#![forbid(unsafe_code)]

//! Sequential allocation leak detection using CUSUM and e-process.
//!
//! This module monitors per-frame allocation counts/bytes as a time series
//! and detects sustained mean-shift regressions with formal guarantees.
//!
//! # Mathematical Model
//!
//! ## CUSUM (Cumulative Sum Control Chart)
//!
//! Tracks one-sided cumulative deviation from a reference mean `μ₀`:
//!
//! ```text
//! S_t⁺ = max(0, S_{t-1}⁺ + (x_t − μ₀ − k))   // detect upward shift
//! S_t⁻ = max(0, S_{t-1}⁻ + (μ₀ − k − x_t))   // detect downward shift
//! ```
//!
//! where `k` is the allowance (slack) parameter, typically `δ/2` for a
//! target shift of `δ`. Alert when `S_t⁺ ≥ h` or `S_t⁻ ≥ h`.
//!
//! CUSUM is quick to detect sustained shifts but is not anytime-valid:
//! it controls ARL (average run length) rather than Type I error.
//!
//! ## E-Process (Anytime-Valid Sequential Test)
//!
//! Maintains a wealth process over centered residuals `r_t = x_t − μ₀`:
//!
//! ```text
//! E_0 = 1
//! E_t = E_{t-1} × exp(λ × r_t − λ² × σ² / 2)
//! ```
//!
//! where:
//! - `σ²` is the assumed variance under H₀
//! - `λ` is the betting fraction (adaptive via GRAPA or fixed)
//!
//! Alert when `E_t ≥ 1/α`. This provides anytime-valid Type I control:
//! `P(∃t: E_t ≥ 1/α | H₀) ≤ α`.
//!
//! # Dual Detection Strategy
//!
//! | Detector | Speed | Guarantee | Use |
//! |----------|-------|-----------|-----|
//! | CUSUM | Fast (O(δ) frames) | ARL-based | Quick alerting |
//! | E-process | Moderate | Anytime-valid α | Formal confirmation |
//!
//! An alert fires when **both** detectors agree (reduces false positives)
//! or when the e-process alone exceeds threshold (formal guarantee).
//!
//! # Failure Modes
//!
//! | Condition | Behavior | Rationale |
//! |-----------|----------|-----------|
//! | `σ² = 0` | Clamp to `σ²_min` | Division by zero guard |
//! | `E_t` underflow | Clamp to `E_MIN` | Prevents permanent zero-lock |
//! | `E_t` overflow | Clamp to `E_MAX` | Numerical stability |
//! | No observations | No state change | Idle is not evidence |

use std::collections::VecDeque;

use crate::evidence_sink::{EVIDENCE_SCHEMA_VERSION, EvidenceSink};
/// Minimum wealth floor.
const E_MIN: f64 = 1e-15;
/// Maximum wealth ceiling.
const E_MAX: f64 = 1e15;
/// Minimum variance floor.
const SIGMA2_MIN: f64 = 1e-6;

fn default_budget_run_id() -> String {
    format!("budget-{}", std::process::id())
}

#[derive(Debug, Clone)]
pub struct EvidenceContext {
    run_id: String,
    screen_mode: String,
    cols: u16,
    rows: u16,
}

impl EvidenceContext {
    #[must_use]
    pub fn new(
        run_id: impl Into<String>,
        screen_mode: impl Into<String>,
        cols: u16,
        rows: u16,
    ) -> Self {
        Self {
            run_id: run_id.into(),
            screen_mode: screen_mode.into(),
            cols,
            rows,
        }
    }

    fn prefix(&self, event_idx: u64) -> String {
        format!(
            r#""schema_version":"{}","run_id":"{}","event_idx":{},"screen_mode":"{}","cols":{},"rows":{}"#,
            EVIDENCE_SCHEMA_VERSION,
            json_escape(&self.run_id),
            event_idx,
            json_escape(&self.screen_mode),
            self.cols,
            self.rows
        )
    }
}

#[inline]
fn json_escape(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for ch in value.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if c.is_control() => {
                use std::fmt::Write as _;
                let _ = write!(out, "\\u{:04X}", c as u32);
            }
            _ => out.push(ch),
        }
    }
    out
}

/// Configuration for the allocation budget monitor.
#[derive(Debug, Clone)]
pub struct BudgetConfig {
    /// Significance level α for e-process. Default: 0.05.
    pub alpha: f64,

    /// Reference mean μ₀ (expected allocations per frame under H₀).
    /// This should be calibrated from a stable baseline.
    pub mu_0: f64,

    /// Assumed variance σ² under H₀. Default: computed from baseline.
    pub sigma_sq: f64,

    /// CUSUM allowance parameter k. Default: δ/2 where δ = target_shift.
    pub cusum_k: f64,

    /// CUSUM threshold h. Default: 5.0.
    pub cusum_h: f64,

    /// Fixed betting fraction λ for e-process. Default: 0.1.
    pub lambda: f64,

    /// Window size for running statistics. Default: 100.
    pub window_size: usize,
}

impl Default for BudgetConfig {
    fn default() -> Self {
        Self {
            alpha: 0.05,
            mu_0: 0.0,
            sigma_sq: 1.0,
            cusum_k: 0.5,
            cusum_h: 5.0,
            lambda: 0.1,
            window_size: 100,
        }
    }
}

impl BudgetConfig {
    /// Create config calibrated for detecting a shift of `delta` allocations
    /// above a baseline mean `mu_0` with variance `sigma_sq`.
    pub fn calibrated(mu_0: f64, sigma_sq: f64, delta: f64, alpha: f64) -> Self {
        let sigma_sq = sigma_sq.max(SIGMA2_MIN);
        let lambda = (delta / sigma_sq).min(0.5); // conservative λ
        Self {
            alpha,
            mu_0,
            sigma_sq,
            cusum_k: delta / 2.0,
            cusum_h: 5.0,
            lambda,
            window_size: 100,
        }
    }

    /// Serialize configuration to JSONL format.
    #[must_use]
    pub(crate) fn to_jsonl(&self, context: &EvidenceContext, event_idx: u64) -> String {
        let prefix = context.prefix(event_idx);
        format!(
            r#"{{{prefix},"event":"allocation_budget_config","alpha":{:.6},"mu_0":{:.6},"sigma_sq":{:.6},"cusum_k":{:.6},"cusum_h":{:.6},"lambda":{:.6},"window_size":{}}}"#,
            self.alpha,
            self.mu_0,
            self.sigma_sq,
            self.cusum_k,
            self.cusum_h,
            self.lambda,
            self.window_size
        )
    }
}

/// CUSUM state for one direction.
#[derive(Debug, Clone, Default)]
struct CusumState {
    /// Cumulative sum statistic.
    s: f64,
    /// Number of consecutive frames above threshold.
    alarm_count: u64,
}

/// Evidence ledger entry for diagnostics.
#[derive(Debug, Clone)]
pub struct BudgetEvidence {
    /// Frame index.
    pub frame: u64,
    /// Observed allocation count/bytes.
    pub x: f64,
    /// Residual r_t = x - μ₀.
    pub residual: f64,
    /// CUSUM S⁺ after this observation.
    pub cusum_plus: f64,
    /// CUSUM S⁻ after this observation.
    pub cusum_minus: f64,
    /// E-process value after this observation.
    pub e_value: f64,
    /// Whether this observation triggered an alert.
    pub alert: bool,
}

impl BudgetEvidence {
    /// Serialize evidence to JSONL format.
    #[must_use]
    pub(crate) fn to_jsonl(&self, context: &EvidenceContext) -> String {
        let prefix = context.prefix(self.frame);
        format!(
            r#"{{{prefix},"event":"allocation_budget_evidence","frame":{},"x":{:.6},"residual":{:.6},"cusum_plus":{:.6},"cusum_minus":{:.6},"e_value":{:.6},"alert":{}}}"#,
            self.frame,
            self.x,
            self.residual,
            self.cusum_plus,
            self.cusum_minus,
            self.e_value,
            self.alert
        )
    }
}

/// Alert information when a leak/regression is detected.
#[derive(Debug, Clone)]
pub struct BudgetAlert {
    /// Frame at which alert fired.
    pub frame: u64,
    /// Estimated shift magnitude (running mean − μ₀).
    pub estimated_shift: f64,
    /// E-process value at alert time.
    pub e_value: f64,
    /// CUSUM S⁺ at alert time.
    pub cusum_plus: f64,
    /// Whether the e-process alone triggered (formal guarantee).
    pub e_process_triggered: bool,
    /// Whether CUSUM triggered.
    pub cusum_triggered: bool,
}

/// Allocation budget monitor with dual CUSUM + e-process detection.
#[derive(Debug, Clone)]
pub struct AllocationBudget {
    config: BudgetConfig,
    /// E-process wealth (log-space for numerical stability).
    log_e_value: f64,
    /// CUSUM upper (detect increase).
    cusum_plus: CusumState,
    /// CUSUM lower (detect decrease).
    cusum_minus: CusumState,
    /// Frame counter.
    frame: u64,
    /// Running window of recent observations for diagnostics.
    window: VecDeque<f64>,
    /// Total alerts fired.
    total_alerts: u64,
    /// Evidence ledger (bounded to last N entries).
    ledger: VecDeque<BudgetEvidence>,
    /// Max ledger size.
    ledger_max: usize,
    /// Evidence sink for JSONL logging.
    evidence_sink: Option<EvidenceSink>,
    /// Whether config has been logged to the sink.
    config_logged: bool,
    /// Evidence metadata for JSONL logs.
    evidence_context: EvidenceContext,
}

impl AllocationBudget {
    /// Create monitor with default config.
    pub fn new(config: BudgetConfig) -> Self {
        Self {
            config,
            log_e_value: 0.0,
            cusum_plus: CusumState::default(),
            cusum_minus: CusumState::default(),
            frame: 0,
            window: VecDeque::new(),
            total_alerts: 0,
            ledger: VecDeque::new(),
            ledger_max: 500,
            evidence_sink: None,
            config_logged: false,
            evidence_context: EvidenceContext::new(default_budget_run_id(), "unknown", 0, 0),
        }
    }

    /// Attach an evidence sink for JSONL logging.
    #[must_use]
    pub fn with_evidence_sink(mut self, sink: EvidenceSink) -> Self {
        self.evidence_sink = Some(sink);
        self.config_logged = false;
        self
    }

    /// Set evidence context fields for JSONL logs.
    #[must_use]
    pub fn with_evidence_context(
        mut self,
        run_id: impl Into<String>,
        screen_mode: impl Into<String>,
        cols: u16,
        rows: u16,
    ) -> Self {
        self.evidence_context = EvidenceContext::new(run_id, screen_mode, cols, rows);
        self
    }

    /// Set evidence context fields for JSONL logs.
    pub fn set_evidence_context(
        &mut self,
        run_id: impl Into<String>,
        screen_mode: impl Into<String>,
        cols: u16,
        rows: u16,
    ) {
        self.evidence_context = EvidenceContext::new(run_id, screen_mode, cols, rows);
    }

    /// Set or clear the evidence sink.
    pub fn set_evidence_sink(&mut self, sink: Option<EvidenceSink>) {
        self.evidence_sink = sink;
        self.config_logged = false;
    }

    /// Observe an allocation count/byte measurement for the current frame.
    /// Returns `Some(alert)` if a regression is detected.
    pub fn observe(&mut self, x: f64) -> Option<BudgetAlert> {
        if !x.is_finite()
            || !self.config.mu_0.is_finite()
            || !self.config.cusum_k.is_finite()
            || !self.config.cusum_h.is_finite()
            || !self.config.sigma_sq.is_finite()
            || !self.config.lambda.is_finite()
            || !self.config.alpha.is_finite()
            || !(0.0..1.0).contains(&self.config.alpha)
        {
            return None;
        }
        self.frame += 1;

        // Maintain running window.
        self.window.push_back(x);
        if self.window.len() > self.config.window_size {
            self.window.pop_front();
        }

        let residual = x - self.config.mu_0;

        // --- CUSUM update ---
        self.cusum_plus.s = (self.cusum_plus.s + residual - self.config.cusum_k).max(0.0);
        self.cusum_minus.s = (self.cusum_minus.s - residual - self.config.cusum_k).max(0.0);

        let cusum_plus_triggered = self.cusum_plus.s >= self.config.cusum_h;
        if cusum_plus_triggered {
            self.cusum_plus.alarm_count += 1;
        } else {
            self.cusum_plus.alarm_count = 0;
        }

        let cusum_minus_triggered = self.cusum_minus.s >= self.config.cusum_h;
        if cusum_minus_triggered {
            self.cusum_minus.alarm_count += 1;
        } else {
            self.cusum_minus.alarm_count = 0;
        }

        let cusum_triggered = cusum_plus_triggered || cusum_minus_triggered;

        // --- E-process update ---
        let sigma_sq = self.config.sigma_sq.max(SIGMA2_MIN);
        let lambda = self.config.lambda;
        let log_increment = lambda * residual - lambda * lambda * sigma_sq / 2.0;

        // Update in log-space to prevent precision loss and overflow.
        self.log_e_value = (self.log_e_value + log_increment).clamp(E_MIN.ln(), E_MAX.ln());

        let e_threshold = 1.0 / self.config.alpha;
        let e_process_triggered = self.log_e_value >= e_threshold.ln();

        // Alert if e-process alone triggers (formal guarantee)
        // or both CUSUM and e-process agree.
        let alert = e_process_triggered || (cusum_triggered && self.log_e_value > 0.0);

        // Record evidence.
        let entry = BudgetEvidence {
            frame: self.frame,
            x,
            residual,
            cusum_plus: self.cusum_plus.s,
            cusum_minus: self.cusum_minus.s,
            e_value: self.log_e_value.exp(),
            alert,
        };
        if let Some(ref sink) = self.evidence_sink {
            let context = &self.evidence_context;
            if !self.config_logged {
                let _ = sink.write_jsonl(&self.config.to_jsonl(context, 0));
                self.config_logged = true;
            }
            let _ = sink.write_jsonl(&entry.to_jsonl(context));
        }
        self.ledger.push_back(entry);
        if self.ledger.len() > self.ledger_max {
            self.ledger.pop_front();
        }

        if alert {
            self.total_alerts += 1;
            let estimated_shift = self.running_mean() - self.config.mu_0;
            let e_value_at_alert = self.log_e_value.exp();
            let cusum_plus_at_alert = self.cusum_plus.s;

            // Reset after alert.
            self.log_e_value = 0.0;
            self.cusum_plus.s = 0.0;
            self.cusum_minus.s = 0.0;
            self.cusum_plus.alarm_count = 0;
            self.cusum_minus.alarm_count = 0;

            Some(BudgetAlert {
                frame: self.frame,
                estimated_shift,
                e_value: e_value_at_alert,
                cusum_plus: cusum_plus_at_alert,
                e_process_triggered,
                cusum_triggered,
            })
        } else {
            None
        }
    }

    /// Running mean of the observation window.
    pub fn running_mean(&self) -> f64 {
        if self.window.is_empty() {
            return self.config.mu_0;
        }
        self.window.iter().sum::<f64>() / self.window.len() as f64
    }

    /// Current e-process value.
    pub fn e_value(&self) -> f64 {
        self.log_e_value.exp()
    }

    /// Current CUSUM S⁺ value.
    pub fn cusum_plus(&self) -> f64 {
        self.cusum_plus.s
    }

    /// Current CUSUM S⁻ value.
    pub fn cusum_minus(&self) -> f64 {
        self.cusum_minus.s
    }

    /// Total frames observed.
    pub fn frames(&self) -> u64 {
        self.frame
    }

    /// Total alerts fired.
    pub fn total_alerts(&self) -> u64 {
        self.total_alerts
    }

    /// Access the evidence ledger.
    pub fn ledger(&self) -> &VecDeque<BudgetEvidence> {
        &self.ledger
    }

    /// Reset all state (keep config).
    pub fn reset(&mut self) {
        self.log_e_value = 0.0;
        self.cusum_plus = CusumState::default();
        self.cusum_minus = CusumState::default();
        self.frame = 0;
        self.window.clear();
        self.total_alerts = 0;
        self.ledger.clear();
        self.config_logged = false;
    }

    /// Summary for diagnostics.
    pub fn summary(&self) -> BudgetSummary {
        BudgetSummary {
            frames: self.frame,
            total_alerts: self.total_alerts,
            e_value: self.log_e_value.exp(),
            cusum_plus: self.cusum_plus.s,
            cusum_minus: self.cusum_minus.s,
            running_mean: self.running_mean(),
            mu_0: self.config.mu_0,
            drift: self.running_mean() - self.config.mu_0,
        }
    }
}

/// Diagnostic summary.
#[derive(Debug, Clone)]
pub struct BudgetSummary {
    pub frames: u64,
    pub total_alerts: u64,
    pub e_value: f64,
    pub cusum_plus: f64,
    pub cusum_minus: f64,
    pub running_mean: f64,
    pub mu_0: f64,
    pub drift: f64,
}

impl BudgetSummary {
    /// Serialize summary to JSONL format.
    #[must_use]
    #[allow(dead_code)]
    pub(crate) fn to_jsonl(&self, context: &EvidenceContext, event_idx: u64) -> String {
        let prefix = context.prefix(event_idx);
        format!(
            r#"{{{prefix},"event":"allocation_budget_summary","frames":{},"total_alerts":{},"e_value":{:.6},"cusum_plus":{:.6},"cusum_minus":{:.6},"running_mean":{:.6},"mu_0":{:.6},"drift":{:.6}}}"#,
            self.frames,
            self.total_alerts,
            self.e_value,
            self.cusum_plus,
            self.cusum_minus,
            self.running_mean,
            self.mu_0,
            self.drift
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_context() -> EvidenceContext {
        EvidenceContext::new("budget-test", "inline", 80, 24)
    }

    // ─── CUSUM tests ──────────────────────────────────────────────

    #[test]
    fn unit_cusum_detects_shift() {
        // μ₀ = 10, shift to 15 (δ=5). k=2.5, h=5.
        let config = BudgetConfig {
            mu_0: 10.0,
            sigma_sq: 4.0,
            cusum_k: 2.5,
            cusum_h: 5.0,
            lambda: 0.1,
            alpha: 0.05,
            ..Default::default()
        };
        let mut monitor = AllocationBudget::new(config);

        // Feed stable data first.
        for _ in 0..20 {
            monitor.observe(10.0);
        }
        assert_eq!(monitor.cusum_plus(), 0.0, "no CUSUM drift under H₀");

        // Now inject shift: x=15 each frame.
        // residual = 5, increment = 5 - 2.5 = 2.5 per frame.
        // After 2 frames: S⁺ = 5.0 → should trigger CUSUM.
        let mut cusum_crossed = false;
        for _ in 0..5 {
            monitor.observe(15.0);
            if monitor.cusum_plus() >= 5.0 || monitor.total_alerts() > 0 {
                cusum_crossed = true;
                break;
            }
        }
        assert!(cusum_crossed, "CUSUM should detect shift from 10→15");
    }

    // ─── E-process tests ──────────────────────────────────────────

    #[test]
    fn unit_eprocess_threshold() {
        // λ=0.3, σ²=1, α=0.05, μ₀=0.
        // With x=2 each frame, residual=2.
        // log_inc = 0.3*2 - 0.3²*1/2 = 0.6 - 0.045 = 0.555
        // E grows as exp(0.555*t), threshold = 1/0.05 = 20.
        // Need t such that exp(0.555*t) ≥ 20 → t ≥ ln(20)/0.555 ≈ 5.4.
        let config = BudgetConfig {
            alpha: 0.05,
            mu_0: 0.0,
            sigma_sq: 1.0,
            lambda: 0.3,
            cusum_k: 1.0,
            cusum_h: 100.0, // high to prevent CUSUM from interfering
            ..Default::default()
        };
        let mut monitor = AllocationBudget::new(config);

        let mut alert_frame = None;
        for i in 0..20 {
            if let Some(_alert) = monitor.observe(2.0) {
                alert_frame = Some(i + 1);
                break;
            }
        }
        assert!(alert_frame.is_some(), "e-process should trigger");
        let frame = alert_frame.unwrap();
        // Should trigger around frame 6 (ceil of 5.4).
        assert!(
            frame <= 8,
            "should detect quickly: triggered at frame {frame}"
        );
    }

    #[test]
    fn eprocess_stays_bounded_under_null() {
        // Under H₀ (x = μ₀), e-process should stay near 1.
        let config = BudgetConfig {
            alpha: 0.05,
            mu_0: 50.0,
            sigma_sq: 10.0,
            lambda: 0.1,
            cusum_k: 2.0,
            cusum_h: 10.0,
            ..Default::default()
        };
        let mut monitor = AllocationBudget::new(config);

        // Feed exactly μ₀.
        for _ in 0..1000 {
            monitor.observe(50.0);
        }
        // E-process should not have triggered.
        assert_eq!(
            monitor.total_alerts(),
            0,
            "no alerts under H₀ with constant input"
        );
        // Under exact H₀, log_inc = λ*0 - λ²σ²/2 < 0 → E decays.
        assert!(monitor.e_value() <= 1.0, "E should decay under exact H₀");
    }

    #[test]
    fn eprocess_wealth_clamped() {
        let config = BudgetConfig {
            alpha: 0.05,
            mu_0: 0.0,
            sigma_sq: 1.0,
            lambda: 0.1,
            cusum_k: 0.5,
            cusum_h: 1000.0,
            ..Default::default()
        };
        let mut monitor = AllocationBudget::new(config);

        // Feed large negative residuals → E should decay but not underflow.
        for _ in 0..10000 {
            monitor.observe(-100.0);
        }
        assert!(
            monitor.e_value() >= E_MIN,
            "wealth should not underflow past E_MIN"
        );
    }

    // ─── FPR control test ─────────────────────────────────────────

    #[test]
    fn property_fpr_control() {
        // Run many stable sequences, count false positive rate.
        // Under H₀ with exact constant input, there should be 0 false positives.
        let alpha = 0.05;
        let n_runs = 100;
        let frames_per_run = 200;
        let mut false_positives = 0;

        for _ in 0..n_runs {
            let config = BudgetConfig {
                alpha,
                mu_0: 100.0,
                sigma_sq: 25.0,
                lambda: 0.1,
                cusum_k: 2.5,
                cusum_h: 10.0,
                ..Default::default()
            };
            let mut monitor = AllocationBudget::new(config);

            // Deterministic PRNG for reproducibility.
            let mut seed: u64 = 0xDEAD_BEEF_1234_5678;
            let mut had_alert = false;

            for _ in 0..frames_per_run {
                // LCG pseudo-random: mean≈100, small noise.
                seed = seed
                    .wrapping_mul(6364136223846793005)
                    .wrapping_add(1442695040888963407);
                let u = (seed >> 33) as f64 / (1u64 << 31) as f64; // [0, 1)
                let noise = (u - 0.5) * 10.0; // [-5, 5)
                let x = 100.0 + noise;

                if monitor.observe(x).is_some() {
                    had_alert = true;
                }
            }
            if had_alert {
                false_positives += 1;
            }
        }

        let fpr = false_positives as f64 / n_runs as f64;
        // Under anytime-valid guarantee, FPR ≤ α.
        // Allow tolerance for finite-sample effects.
        assert!(
            fpr <= alpha + 0.10,
            "FPR {fpr} exceeds α + tolerance ({alpha} + 0.10)"
        );
    }

    // ─── Synthetic leak injection ─────────────────────────────────

    #[test]
    fn e2e_synthetic_leak_injection() {
        // Baseline at 50, then leak injects +10 starting at frame 100.
        let config = BudgetConfig::calibrated(50.0, 4.0, 10.0, 0.05);
        let mut monitor = AllocationBudget::new(config);

        // Stable phase.
        for _ in 0..100 {
            let result = monitor.observe(50.0);
            assert!(result.is_none(), "no alert during stable phase");
        }

        // Leak phase: x = 60.
        let mut detected_at = None;
        for i in 0..100 {
            if let Some(_alert) = monitor.observe(60.0) {
                detected_at = Some(i + 1);
                break;
            }
        }
        assert!(detected_at.is_some(), "should detect leak injection of +10");
        let frames_to_detect = detected_at.unwrap();
        assert!(
            frames_to_detect <= 20,
            "detection too slow: {frames_to_detect} frames for δ=10"
        );
    }

    #[test]
    fn e2e_stable_run_no_alerts() {
        let config = BudgetConfig::calibrated(100.0, 16.0, 20.0, 0.05);
        let mut monitor = AllocationBudget::new(config);

        // Run 500 frames at exact baseline.
        for _ in 0..500 {
            let result = monitor.observe(100.0);
            assert!(result.is_none());
        }

        assert_eq!(monitor.total_alerts(), 0);
        // E should have decayed.
        assert!(monitor.e_value() < 1.0);
    }

    // ─── Evidence ledger tests ────────────────────────────────────

    #[test]
    fn ledger_records_observations() {
        let config = BudgetConfig {
            mu_0: 10.0,
            ..Default::default()
        };
        let mut monitor = AllocationBudget::new(config);

        for i in 0..5 {
            monitor.observe(10.0 + i as f64);
        }

        assert_eq!(monitor.ledger().len(), 5);
        assert_eq!(monitor.ledger()[0].frame, 1);
        assert_eq!(monitor.ledger()[4].frame, 5);
        assert!((monitor.ledger()[0].x - 10.0).abs() < 1e-10);
        assert!((monitor.ledger()[2].residual - 2.0).abs() < 1e-10);
    }

    #[test]
    fn ledger_bounded_size() {
        let mut monitor = AllocationBudget::new(BudgetConfig::default());
        monitor.ledger_max = 10;

        for i in 0..100 {
            monitor.observe(i as f64);
        }

        assert!(monitor.ledger().len() <= 10);
    }

    // ─── Reset test ───────────────────────────────────────────────

    #[test]
    fn reset_clears_state() {
        let config = BudgetConfig {
            mu_0: 0.0,
            ..Default::default()
        };
        let mut monitor = AllocationBudget::new(config);

        for _ in 0..50 {
            monitor.observe(5.0);
        }
        assert!(monitor.frames() > 0);

        monitor.reset();
        assert_eq!(monitor.frames(), 0);
        assert_eq!(monitor.total_alerts(), 0);
        assert!((monitor.e_value() - 1.0).abs() < 1e-10);
        assert_eq!(monitor.cusum_plus(), 0.0);
        assert_eq!(monitor.cusum_minus(), 0.0);
        assert!(monitor.ledger().is_empty());
    }

    // ─── Summary test ─────────────────────────────────────────────

    #[test]
    fn summary_reports_drift() {
        let config = BudgetConfig {
            mu_0: 10.0,
            cusum_h: 1000.0, // prevent alerts
            alpha: 1e-20,    // prevent e-process alerts
            ..Default::default()
        };
        let mut monitor = AllocationBudget::new(config);

        for _ in 0..100 {
            monitor.observe(15.0);
        }

        let summary = monitor.summary();
        assert!((summary.running_mean - 15.0).abs() < 1e-10);
        assert!((summary.drift - 5.0).abs() < 1e-10);
        assert!((summary.mu_0 - 10.0).abs() < 1e-10);
    }

    // ─── Calibrated config test ───────────────────────────────────

    #[test]
    fn calibrated_config_reasonable() {
        let config = BudgetConfig::calibrated(100.0, 25.0, 10.0, 0.05);
        assert!((config.mu_0 - 100.0).abs() < 1e-10);
        assert!((config.sigma_sq - 25.0).abs() < 1e-10);
        assert!((config.cusum_k - 5.0).abs() < 1e-10);
        assert!(config.lambda > 0.0 && config.lambda <= 0.5);
        assert!((config.alpha - 0.05).abs() < 1e-10);
    }

    // ─── Determinism test ─────────────────────────────────────────

    #[test]
    fn deterministic_under_same_input() {
        let run = || {
            let config = BudgetConfig::calibrated(50.0, 4.0, 5.0, 0.05);
            let mut monitor = AllocationBudget::new(config);
            let inputs = [50.0, 51.0, 49.0, 55.0, 48.0, 60.0, 50.0, 52.0, 47.0, 53.0];
            let mut e_values = Vec::new();
            for x in inputs {
                monitor.observe(x);
                e_values.push(monitor.e_value());
            }
            (e_values, monitor.cusum_plus(), monitor.cusum_minus())
        };

        let (ev1, cp1, cm1) = run();
        let (ev2, cp2, cm2) = run();
        assert_eq!(ev1, ev2);
        assert!((cp1 - cp2).abs() < 1e-15);
        assert!((cm1 - cm2).abs() < 1e-15);
    }

    // ─── JSONL schema tests ───────────────────────────────────────

    #[test]
    fn config_jsonl_parses_and_has_fields() {
        use serde_json::Value;

        let config = BudgetConfig::default();
        let context = test_context();
        let jsonl = config.to_jsonl(&context, 0);
        let value: Value = serde_json::from_str(&jsonl).expect("valid JSONL");

        assert_eq!(value["schema_version"], EVIDENCE_SCHEMA_VERSION);
        assert_eq!(value["run_id"], "budget-test");
        assert!(
            value["event_idx"].is_number(),
            "event_idx should be numeric"
        );
        assert_eq!(value["screen_mode"], "inline");
        assert!(value["cols"].is_number(), "cols should be numeric");
        assert!(value["rows"].is_number(), "rows should be numeric");
        assert_eq!(value["event"], "allocation_budget_config");
        for key in [
            "alpha",
            "mu_0",
            "sigma_sq",
            "cusum_k",
            "cusum_h",
            "lambda",
            "window_size",
        ] {
            assert!(value.get(key).is_some(), "missing config field {key}");
        }
    }

    #[test]
    fn evidence_jsonl_parses_and_has_fields() {
        use serde_json::Value;

        let evidence = BudgetEvidence {
            frame: 3,
            x: 12.0,
            residual: 2.0,
            cusum_plus: 1.5,
            cusum_minus: 0.5,
            e_value: 1.2,
            alert: false,
        };
        let context = test_context();
        let jsonl = evidence.to_jsonl(&context);
        let value: Value = serde_json::from_str(&jsonl).expect("valid JSONL");

        assert_eq!(value["schema_version"], EVIDENCE_SCHEMA_VERSION);
        assert_eq!(value["run_id"], "budget-test");
        assert!(
            value["event_idx"].is_number(),
            "event_idx should be numeric"
        );
        assert_eq!(value["screen_mode"], "inline");
        assert!(value["cols"].is_number(), "cols should be numeric");
        assert!(value["rows"].is_number(), "rows should be numeric");
        assert_eq!(value["event"], "allocation_budget_evidence");
        for key in [
            "frame",
            "x",
            "residual",
            "cusum_plus",
            "cusum_minus",
            "e_value",
            "alert",
        ] {
            assert!(value.get(key).is_some(), "missing evidence field {key}");
        }
    }

    #[test]
    fn summary_jsonl_parses_and_has_fields() {
        use serde_json::Value;

        let summary = BudgetSummary {
            frames: 5,
            total_alerts: 1,
            e_value: 2.0,
            cusum_plus: 3.0,
            cusum_minus: 1.0,
            running_mean: 11.0,
            mu_0: 10.0,
            drift: 1.0,
        };
        let context = test_context();
        let jsonl = summary.to_jsonl(&context, 5);
        let value: Value = serde_json::from_str(&jsonl).expect("valid JSONL");

        assert_eq!(value["schema_version"], EVIDENCE_SCHEMA_VERSION);
        assert_eq!(value["run_id"], "budget-test");
        assert!(
            value["event_idx"].is_number(),
            "event_idx should be numeric"
        );
        assert_eq!(value["screen_mode"], "inline");
        assert!(value["cols"].is_number(), "cols should be numeric");
        assert!(value["rows"].is_number(), "rows should be numeric");
        assert_eq!(value["event"], "allocation_budget_summary");
        for key in [
            "frames",
            "total_alerts",
            "e_value",
            "cusum_plus",
            "cusum_minus",
            "running_mean",
            "mu_0",
            "drift",
        ] {
            assert!(value.get(key).is_some(), "missing summary field {key}");
        }
    }

    #[test]
    fn evidence_jsonl_is_deterministic_for_fixed_inputs() {
        let config = BudgetConfig::calibrated(50.0, 4.0, 5.0, 0.05);
        let inputs = [50.0, 51.0, 49.0, 55.0, 48.0, 60.0, 50.0, 52.0, 47.0, 53.0];

        let run = || {
            let context = test_context();
            let mut monitor = AllocationBudget::new(config.clone()).with_evidence_context(
                "budget-test",
                "inline",
                80,
                24,
            );
            for x in inputs {
                monitor.observe(x);
            }
            monitor
                .ledger()
                .iter()
                .map(|entry| entry.to_jsonl(&context))
                .collect::<Vec<_>>()
        };

        let first = run();
        let second = run();
        assert_eq!(first, second);
    }

    // ── BudgetConfig defaults ───────────────────────────────────────

    #[test]
    fn budget_config_default_values() {
        let config = BudgetConfig::default();
        assert!((config.alpha - 0.05).abs() < f64::EPSILON);
        assert!((config.mu_0 - 0.0).abs() < f64::EPSILON);
        assert!((config.sigma_sq - 1.0).abs() < f64::EPSILON);
        assert!((config.cusum_k - 0.5).abs() < f64::EPSILON);
        assert!((config.cusum_h - 5.0).abs() < f64::EPSILON);
        assert!((config.lambda - 0.1).abs() < f64::EPSILON);
        assert_eq!(config.window_size, 100);
    }

    #[test]
    fn calibrated_clamps_tiny_sigma() {
        let config = BudgetConfig::calibrated(0.0, 0.0, 1.0, 0.05);
        assert!(config.sigma_sq >= SIGMA2_MIN);
    }

    #[test]
    fn calibrated_lambda_bounded() {
        let config = BudgetConfig::calibrated(0.0, 0.001, 1000.0, 0.05);
        assert!(config.lambda <= 0.5);
    }

    // ── json_escape ─────────────────────────────────────────────────

    #[test]
    fn json_escape_special_chars() {
        assert_eq!(json_escape("hello"), "hello");
        assert_eq!(json_escape("say \"hi\""), "say \\\"hi\\\"");
        assert_eq!(json_escape("back\\slash"), "back\\\\slash");
        assert_eq!(json_escape("new\nline"), "new\\nline");
        assert_eq!(json_escape("tab\there"), "tab\\there");
        assert_eq!(json_escape("cr\rhere"), "cr\\rhere");
    }

    #[test]
    fn json_escape_control_chars() {
        let s = "\x01\x02";
        let escaped = json_escape(s);
        assert!(escaped.contains("\\u0001"));
        assert!(escaped.contains("\\u0002"));
    }

    // ── EvidenceContext ──────────────────────────────────────────────

    #[test]
    fn evidence_context_prefix_format() {
        let ctx = EvidenceContext::new("run-42", "inline", 120, 30);
        let prefix = ctx.prefix(7);
        assert!(prefix.contains("\"run_id\":\"run-42\""));
        assert!(prefix.contains("\"event_idx\":7"));
        assert!(prefix.contains("\"screen_mode\":\"inline\""));
        assert!(prefix.contains("\"cols\":120"));
        assert!(prefix.contains("\"rows\":30"));
    }

    // ── AllocationBudget constructors / accessors ────────────────────

    #[test]
    fn new_monitor_initial_state() {
        let monitor = AllocationBudget::new(BudgetConfig::default());
        assert_eq!(monitor.frames(), 0);
        assert_eq!(monitor.total_alerts(), 0);
        assert!((monitor.e_value() - 1.0).abs() < f64::EPSILON);
        assert!((monitor.cusum_plus() - 0.0).abs() < f64::EPSILON);
        assert!((monitor.cusum_minus() - 0.0).abs() < f64::EPSILON);
        assert!(monitor.ledger().is_empty());
    }

    #[test]
    fn running_mean_empty_returns_mu0() {
        let config = BudgetConfig {
            mu_0: 42.0,
            ..Default::default()
        };
        let monitor = AllocationBudget::new(config);
        assert!((monitor.running_mean() - 42.0).abs() < f64::EPSILON);
    }

    #[test]
    fn running_mean_with_observations() {
        let mut monitor = AllocationBudget::new(BudgetConfig {
            mu_0: 0.0,
            cusum_h: 1000.0,
            alpha: 1e-20,
            ..Default::default()
        });
        monitor.observe(10.0);
        monitor.observe(20.0);
        monitor.observe(30.0);
        assert!((monitor.running_mean() - 20.0).abs() < 1e-10);
    }

    #[test]
    fn window_size_enforced() {
        let config = BudgetConfig {
            window_size: 5,
            mu_0: 0.0,
            cusum_h: 1000.0,
            alpha: 1e-20,
            ..Default::default()
        };
        let mut monitor = AllocationBudget::new(config);
        for i in 0..20 {
            monitor.observe(i as f64);
        }
        let expected_mean = (15.0 + 16.0 + 17.0 + 18.0 + 19.0) / 5.0;
        assert!((monitor.running_mean() - expected_mean).abs() < 1e-10);
    }

    // ── with_evidence_context / set_evidence_context ─────────────────

    #[test]
    fn with_evidence_context_builder() {
        let monitor = AllocationBudget::new(BudgetConfig::default()).with_evidence_context(
            "my-run",
            "fullscreen",
            200,
            50,
        );
        let summary = monitor.summary();
        let ctx = EvidenceContext::new("my-run", "fullscreen", 200, 50);
        let jsonl = summary.to_jsonl(&ctx, 0);
        assert!(jsonl.contains("\"run_id\":\"my-run\""));
        assert!(jsonl.contains("\"screen_mode\":\"fullscreen\""));
    }

    #[test]
    fn set_evidence_context_mutates() {
        let mut monitor = AllocationBudget::new(BudgetConfig::default());
        monitor.set_evidence_context("new-run", "alt", 160, 40);
        monitor.observe(1.0);
        assert_eq!(monitor.frames(), 1);
    }

    // ── Alert resets state ──────────────────────────────────────────

    #[test]
    fn alert_resets_cusum_and_evalue() {
        let config = BudgetConfig {
            alpha: 0.05,
            mu_0: 0.0,
            sigma_sq: 1.0,
            lambda: 0.5,
            cusum_k: 0.5,
            cusum_h: 100.0,
            ..Default::default()
        };
        let mut monitor = AllocationBudget::new(config);
        let mut alert_seen = false;
        for _ in 0..100 {
            if monitor.observe(10.0).is_some() {
                alert_seen = true;
                break;
            }
        }
        assert!(alert_seen, "should have triggered alert");
        assert!((monitor.e_value() - 1.0).abs() < f64::EPSILON);
        assert!((monitor.cusum_plus() - 0.0).abs() < f64::EPSILON);
        assert!((monitor.cusum_minus() - 0.0).abs() < f64::EPSILON);
    }

    #[test]
    fn alert_increments_total_alerts() {
        let config = BudgetConfig {
            alpha: 0.05,
            mu_0: 0.0,
            sigma_sq: 1.0,
            lambda: 0.5,
            cusum_k: 0.5,
            cusum_h: 100.0,
            ..Default::default()
        };
        let mut monitor = AllocationBudget::new(config);
        assert_eq!(monitor.total_alerts(), 0);
        for _ in 0..100 {
            if monitor.observe(10.0).is_some() {
                break;
            }
        }
        assert!(monitor.total_alerts() >= 1);
    }

    #[test]
    fn alert_contains_expected_fields() {
        let config = BudgetConfig {
            alpha: 0.05,
            mu_0: 0.0,
            sigma_sq: 1.0,
            lambda: 0.5,
            cusum_k: 0.5,
            cusum_h: 100.0,
            ..Default::default()
        };
        let mut monitor = AllocationBudget::new(config);
        let mut alert = None;
        for _ in 0..100 {
            if let Some(a) = monitor.observe(10.0) {
                alert = Some(a);
                break;
            }
        }
        let alert = alert.expect("should have triggered");
        assert!(alert.frame > 0);
        assert!(alert.e_process_triggered);
        assert!(alert.e_value >= 1.0 / 0.05);
        assert!(alert.estimated_shift > 0.0);
    }

    // ── CUSUM downward shift ────────────────────────────────────────

    #[test]
    fn cusum_minus_detects_decrease() {
        let config = BudgetConfig {
            mu_0: 100.0,
            sigma_sq: 4.0,
            cusum_k: 2.5,
            cusum_h: 5.0,
            lambda: 0.01,
            alpha: 1e-100,
            ..Default::default()
        };
        let mut monitor = AllocationBudget::new(config);
        for _ in 0..10 {
            monitor.observe(90.0);
        }
        assert!(
            monitor.cusum_minus() > 0.0,
            "CUSUM- should be positive for downward shift"
        );
    }

    // ── Summary fields ──────────────────────────────────────────────

    #[test]
    fn summary_initial_state() {
        let monitor = AllocationBudget::new(BudgetConfig {
            mu_0: 25.0,
            ..Default::default()
        });
        let summary = monitor.summary();
        assert_eq!(summary.frames, 0);
        assert_eq!(summary.total_alerts, 0);
        assert!((summary.e_value - 1.0).abs() < f64::EPSILON);
        assert!((summary.mu_0 - 25.0).abs() < f64::EPSILON);
        assert!((summary.drift - 0.0).abs() < f64::EPSILON);
    }

    #[test]
    fn budget_summary_clone_debug() {
        let summary = BudgetSummary {
            frames: 10,
            total_alerts: 1,
            e_value: 2.5,
            cusum_plus: 3.0,
            cusum_minus: 1.0,
            running_mean: 55.0,
            mu_0: 50.0,
            drift: 5.0,
        };
        let cloned = summary.clone();
        assert_eq!(cloned.frames, 10);
        assert!((cloned.drift - 5.0).abs() < f64::EPSILON);
        let dbg = format!("{:?}", summary);
        assert!(dbg.contains("BudgetSummary"));
    }

    #[test]
    fn budget_evidence_clone_debug() {
        let ev = BudgetEvidence {
            frame: 5,
            x: 12.0,
            residual: 2.0,
            cusum_plus: 1.5,
            cusum_minus: 0.3,
            e_value: 1.1,
            alert: false,
        };
        let cloned = ev.clone();
        assert_eq!(cloned.frame, 5);
        assert!(!cloned.alert);
        let dbg = format!("{:?}", ev);
        assert!(dbg.contains("BudgetEvidence"));
    }

    #[test]
    fn budget_alert_clone_debug() {
        let alert = BudgetAlert {
            frame: 50,
            estimated_shift: 3.5,
            e_value: 25.0,
            cusum_plus: 8.0,
            e_process_triggered: true,
            cusum_triggered: true,
        };
        let cloned = alert.clone();
        assert_eq!(cloned.frame, 50);
        assert!(cloned.e_process_triggered);
        let dbg = format!("{:?}", alert);
        assert!(dbg.contains("BudgetAlert"));
    }

    // ── Reset clears config_logged ──────────────────────────────────

    #[test]
    fn reset_allows_config_re_logging() {
        let mut monitor = AllocationBudget::new(BudgetConfig::default());
        monitor.observe(1.0);
        monitor.reset();
        monitor.observe(2.0);
        assert_eq!(monitor.frames(), 1);
        assert_eq!(monitor.ledger().len(), 1);
    }

    // ── frames counter ──────────────────────────────────────────────

    #[test]
    fn frames_increments_per_observe() {
        let mut monitor = AllocationBudget::new(BudgetConfig {
            cusum_h: 1000.0,
            alpha: 1e-20,
            ..Default::default()
        });
        for _ in 0..7 {
            monitor.observe(0.0);
        }
        assert_eq!(monitor.frames(), 7);
    }

    #[test]
    fn observe_with_nan_lambda_is_noop() {
        let mut monitor = AllocationBudget::new(BudgetConfig {
            lambda: f64::NAN,
            ..Default::default()
        });

        assert!(monitor.observe(10.0).is_none());
        assert_eq!(monitor.frames(), 0);
        assert!(monitor.ledger().is_empty());
        assert_eq!(monitor.e_value(), 1.0);
    }

    #[test]
    fn observe_with_alpha_out_of_range_is_noop() {
        let mut monitor = AllocationBudget::new(BudgetConfig {
            alpha: 2.0,
            ..Default::default()
        });

        assert!(monitor.observe(10.0).is_none());
        assert_eq!(monitor.frames(), 0);
        assert!(monitor.ledger().is_empty());
        assert_eq!(monitor.e_value(), 1.0);
    }
}
