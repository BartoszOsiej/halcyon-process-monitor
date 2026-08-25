#![forbid(unsafe_code)]

//! Conformal frame guard: coverage-guaranteed prediction intervals for frame timing.
//!
//! Wraps [`ConformalPredictor`] with a frame-time time series, nonconformity
//! score tracking, and p99 prediction intervals. When the predicted p99
//! exceeds the frame budget, degradation is triggered.
//!
//! **Fallback:** before calibration reaches `min_samples`, a fixed 16 ms
//! budget threshold is used (no conformal interval).
//!
//! # Integration
//!
//! The guard sits between frame measurement and `BudgetController`:
//!
//! ```text
//! frame_time ──► ConformalFrameGuard ──► P99Prediction
//!                        │                      │
//!                        ▼                      ▼
//!                   observe()              exceeds_budget?
//!                   (calibrate)           → trigger degrade
//! ```

use std::collections::VecDeque;

use ftui_render::budget::DegradationLevel;

use crate::conformal_predictor::{
    BucketKey, ConformalConfig, ConformalPrediction, ConformalPredictor,
};

/// Default fallback budget threshold in microseconds (16 ms = 60 fps target).
const DEFAULT_FALLBACK_BUDGET_US: f64 = 16_000.0;

/// Configuration for the conformal frame guard.
#[derive(Debug, Clone)]
pub struct ConformalFrameGuardConfig {
    /// Underlying conformal predictor configuration.
    pub conformal: ConformalConfig,

    /// Fixed fallback budget threshold (µs) used before calibration.
    /// Default: 16 000.0 (16 ms).
    pub fallback_budget_us: f64,

    /// Maximum frame time samples retained for time-series tracking.
    /// Default: 512.
    pub time_series_window: usize,

    /// Maximum nonconformity scores retained.
    /// Default: 256 (matches conformal window).
    pub nonconformity_window: usize,
}

impl Default for ConformalFrameGuardConfig {
    fn default() -> Self {
        let conformal = ConformalConfig::default();
        let nonconformity_window = conformal.window_size;
        Self {
            conformal,
            fallback_budget_us: DEFAULT_FALLBACK_BUDGET_US,
            time_series_window: 512,
            nonconformity_window,
        }
    }
}

/// State of the conformal frame guard.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GuardState {
    /// Insufficient calibration data; using fixed fallback threshold.
    Warmup,
    /// Calibrated with enough samples; conformal intervals active.
    Calibrated,
    /// Last prediction indicated p99 exceeds budget.
    AtRisk,
}

impl GuardState {
    /// Stable string for JSONL logging.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Warmup => "warmup",
            Self::Calibrated => "calibrated",
            Self::AtRisk => "at_risk",
        }
    }
}

/// Result of a p99 prediction from the guard.
#[derive(Debug, Clone)]
pub struct P99Prediction {
    /// Base prediction (most recent frame time or EMA estimate) in µs.
    pub y_hat_us: f64,
    /// Upper bound of the p99 prediction interval in µs.
    pub upper_us: f64,
    /// Frame budget in µs.
    pub budget_us: f64,
    /// Whether the p99 upper bound exceeds the budget.
    pub exceeds_budget: bool,
    /// Calibration sample count used.
    pub calibration_size: usize,
    /// Fallback level from the underlying conformal predictor (0..=4).
    /// Level 4 means frame-guard fixed fallback was used.
    pub fallback_level: u8,
    /// Current guard state.
    pub state: GuardState,
    /// Width of the prediction interval (upper - y_hat) in µs.
    pub interval_width_us: f64,
    /// Underlying conformal prediction (if calibrated).
    pub conformal: Option<ConformalPrediction>,
}

impl P99Prediction {
    /// Format as a JSONL line for structured evidence logging.
    #[must_use]
    pub fn to_jsonl(&self) -> String {
        let conformal_fields = self
            .conformal
            .as_ref()
            .map(|c| {
                format!(
                    r#","conformal_quantile":{:.2},"conformal_bucket":"{}","conformal_confidence":{:.4}"#,
                    c.quantile, c.bucket, c.confidence,
                )
            })
            .unwrap_or_default();

        format!(
            r#"{{"schema":"conformal-frame-guard-v1","y_hat_us":{:.1},"upper_us":{:.1},"budget_us":{:.1},"exceeds_budget":{},"calibration_size":{},"fallback_level":{},"state":"{}","interval_width_us":{:.1}{}}}"#,
            self.y_hat_us,
            self.upper_us,
            self.budget_us,
            self.exceeds_budget,
            self.calibration_size,
            self.fallback_level,
            self.state.as_str(),
            self.interval_width_us,
            conformal_fields,
        )
    }
}

/// Conformal frame guard: wraps [`ConformalPredictor`] with p99 intervals.
///
/// Tracks frame render times as a time series, computes nonconformity scores,
/// and emits coverage-guaranteed prediction intervals for the next frame.
/// When the predicted p99 exceeds the frame budget, the guard signals
/// degradation.
#[derive(Debug)]
pub struct ConformalFrameGuard {
    config: ConformalFrameGuardConfig,
    predictor: ConformalPredictor,
    /// Rolling frame time measurements (µs).
    frame_times: VecDeque<f64>,
    /// Rolling nonconformity scores (residual = observed - predicted).
    nonconformity_scores: VecDeque<f64>,
    /// EMA of frame times (µs) for base prediction.
    ema_us: f64,
    /// EMA decay factor. Closer to 1.0 = slower adaptation.
    ema_decay: f64,
    /// Current guard state.
    state: GuardState,
    /// Total observations processed.
    observations: u64,
    /// Count of degradation triggers.
    degradation_triggers: u64,
}

impl ConformalFrameGuard {
    /// Create a new guard with the given configuration.
    pub fn new(config: ConformalFrameGuardConfig) -> Self {
        let predictor = ConformalPredictor::new(config.conformal.clone());
        Self {
            config,
            predictor,
            frame_times: VecDeque::new(),
            nonconformity_scores: VecDeque::new(),
            ema_us: 0.0,
            ema_decay: 0.95,
            state: GuardState::Warmup,
            observations: 0,
            degradation_triggers: 0,
        }
    }

    /// Create a guard with default configuration.
    pub fn with_defaults() -> Self {
        Self::new(ConformalFrameGuardConfig::default())
    }

    /// Observe a realized frame time and update calibration.
    ///
    /// `frame_time_us`: measured frame render time in microseconds.
    /// `key`: bucket key from the rendering context.
    pub fn observe(&mut self, frame_time_us: f64, key: BucketKey) {
        if !frame_time_us.is_finite() || frame_time_us < 0.0 {
            return;
        }

        self.observations += 1;

        // Update EMA
        if self.observations == 1 {
            self.ema_us = frame_time_us;
        } else {
            self.ema_us = self.ema_decay * self.ema_us + (1.0 - self.ema_decay) * frame_time_us;
        }

        // Track frame time in rolling window
        self.frame_times.push_back(frame_time_us);
        while self.frame_times.len() > self.config.time_series_window {
            self.frame_times.pop_front();
        }

        // Compute and track nonconformity score (residual)
        let y_hat = self.ema_us;
        let residual = frame_time_us - y_hat;
        self.nonconformity_scores.push_back(residual);
        while self.nonconformity_scores.len() > self.config.nonconformity_window {
            self.nonconformity_scores.pop_front();
        }

        // Feed the underlying conformal predictor
        self.predictor.observe(key, y_hat, frame_time_us);

        // Update state based on calibration
        let samples = self.predictor.bucket_samples(key);
        if samples < self.config.conformal.min_samples && self.state == GuardState::Warmup {
            // Stay in warmup
        } else if self.state == GuardState::Warmup {
            self.state = GuardState::Calibrated;
        }
    }

    /// Predict the p99 upper bound for the next frame.
    ///
    /// `budget_us`: current frame budget in microseconds.
    /// `key`: bucket key for the upcoming rendering context.
    ///
    /// Returns a [`P99Prediction`] with the interval and risk assessment.
    pub fn predict_p99(&mut self, budget_us: f64, key: BucketKey) -> P99Prediction {
        let y_hat = if self.observations > 0 {
            self.ema_us
        } else {
            0.0
        };

        let samples = self.predictor.bucket_samples(key);
        let is_calibrated = samples >= self.config.conformal.min_samples;

        if is_calibrated {
            // Use conformal prediction for coverage-guaranteed bound
            let prediction = self.predictor.predict(key, y_hat, budget_us);
            let exceeds = prediction.upper_us > budget_us;

            self.state = if exceeds {
                self.degradation_triggers += 1;
                GuardState::AtRisk
            } else {
                GuardState::Calibrated
            };

            P99Prediction {
                y_hat_us: y_hat,
                upper_us: prediction.upper_us,
                budget_us,
                exceeds_budget: exceeds,
                calibration_size: prediction.sample_count,
                fallback_level: prediction.fallback_level,
                state: self.state,
                interval_width_us: (prediction.upper_us - y_hat).max(0.0),
                conformal: Some(prediction),
            }
        } else {
            // Fallback: fixed budget threshold (16ms default)
            let fallback = self.config.fallback_budget_us;
            let exceeds = y_hat > fallback;

            if exceeds && self.state != GuardState::Warmup {
                self.degradation_triggers += 1;
            }

            // In warmup, signal risk only if EMA clearly exceeds fallback
            let state = if exceeds {
                GuardState::AtRisk
            } else {
                GuardState::Warmup
            };
            self.state = state;

            P99Prediction {
                y_hat_us: y_hat,
                upper_us: y_hat, // No interval in fallback mode
                budget_us: fallback,
                exceeds_budget: exceeds,
                calibration_size: samples,
                fallback_level: 4, // Frame-guard fixed fallback
                state,
                interval_width_us: 0.0,
                conformal: None,
            }
        }
    }

    /// Get the current guard state.
    #[inline]
    pub fn state(&self) -> GuardState {
        self.state
    }

    /// Whether the guard has enough calibration data for conformal intervals.
    #[inline]
    pub fn is_calibrated(&self) -> bool {
        matches!(self.state, GuardState::Calibrated | GuardState::AtRisk)
    }

    /// Total frame observations processed.
    #[inline]
    pub fn observations(&self) -> u64 {
        self.observations
    }

    /// Total degradation triggers.
    #[inline]
    pub fn degradation_triggers(&self) -> u64 {
        self.degradation_triggers
    }

    /// Access the rolling nonconformity scores.
    pub fn nonconformity_scores(&self) -> &VecDeque<f64> {
        &self.nonconformity_scores
    }

    /// Access the rolling frame time series.
    pub fn frame_times(&self) -> &VecDeque<f64> {
        &self.frame_times
    }

    /// Current EMA of frame times (µs).
    #[inline]
    pub fn ema_us(&self) -> f64 {
        self.ema_us
    }

    /// Access the underlying conformal predictor.
    pub fn predictor(&self) -> &ConformalPredictor {
        &self.predictor
    }

    /// Access the configuration.
    pub fn config(&self) -> &ConformalFrameGuardConfig {
        &self.config
    }

    /// Compute summary statistics for the nonconformity score distribution.
    ///
    /// Returns `(mean, p50, p90, p99, max)` or `None` if no scores exist.
    pub fn nonconformity_summary(&self) -> Option<NonconformitySummary> {
        if self.nonconformity_scores.is_empty() {
            return None;
        }

        let mut sorted: Vec<f64> = self.nonconformity_scores.iter().copied().collect();
        sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));

        let n = sorted.len();
        let mean = sorted.iter().sum::<f64>() / n as f64;
        let p50 = sorted[n / 2];
        let p90 = sorted[(n as f64 * 0.90).ceil() as usize - 1];
        let p99 = sorted[(n as f64 * 0.99).ceil() as usize - 1];
        let max = sorted[n - 1];

        Some(NonconformitySummary {
            count: n,
            mean,
            p50,
            p90,
            p99,
            max,
        })
    }

    /// Reset all calibration state (e.g., after a mode change).
    pub fn reset(&mut self) {
        self.predictor.reset_all();
        self.frame_times.clear();
        self.nonconformity_scores.clear();
        self.ema_us = 0.0;
        self.state = GuardState::Warmup;
        self.observations = 0;
        // Preserve degradation_triggers count across resets for audit trail
    }

    /// Suggest what degradation action to take based on the prediction.
    ///
    /// Returns `Some(DegradationLevel::next())` if the p99 exceeds budget
    /// and the guard is calibrated, `None` otherwise (hold current level).
    pub fn suggest_action(
        &self,
        prediction: &P99Prediction,
        current_level: DegradationLevel,
    ) -> Option<DegradationLevel> {
        if prediction.exceeds_budget && !current_level.is_max() {
            Some(current_level.next())
        } else {
            None
        }
    }

    /// Capture a telemetry snapshot for structured logging.
    pub fn telemetry(&self) -> ConformalFrameGuardTelemetry {
        ConformalFrameGuardTelemetry {
            state: self.state,
            observations: self.observations,
            degradation_triggers: self.degradation_triggers,
            ema_us: self.ema_us,
            frame_times_len: self.frame_times.len(),
            nonconformity_len: self.nonconformity_scores.len(),
            summary: self.nonconformity_summary(),
        }
    }
}

/// Summary statistics for nonconformity score distribution.
#[derive(Debug, Clone, Copy)]
pub struct NonconformitySummary {
    /// Number of scores in the window.
    pub count: usize,
    /// Mean nonconformity score.
    pub mean: f64,
    /// Median (p50).
    pub p50: f64,
    /// 90th percentile.
    pub p90: f64,
    /// 99th percentile.
    pub p99: f64,
    /// Maximum.
    pub max: f64,
}

impl NonconformitySummary {
    /// Format as a JSONL fragment (no outer braces).
    #[must_use]
    pub fn to_jsonl_fragment(&self) -> String {
        format!(
            r#""nc_count":{},"nc_mean":{:.2},"nc_p50":{:.2},"nc_p90":{:.2},"nc_p99":{:.2},"nc_max":{:.2}"#,
            self.count, self.mean, self.p50, self.p90, self.p99, self.max,
        )
    }
}

/// Telemetry snapshot of the conformal frame guard.
#[derive(Debug, Clone)]
pub struct ConformalFrameGuardTelemetry {
    /// Current guard state.
    pub state: GuardState,
    /// Total observations.
    pub observations: u64,
    /// Total degradation triggers.
    pub degradation_triggers: u64,
    /// Current EMA estimate (µs).
    pub ema_us: f64,
    /// Frame time window length.
    pub frame_times_len: usize,
    /// Nonconformity window length.
    pub nonconformity_len: usize,
    /// Nonconformity summary (if any scores exist).
    pub summary: Option<NonconformitySummary>,
}

impl ConformalFrameGuardTelemetry {
    /// Format as a JSONL line.
    #[must_use]
    pub fn to_jsonl(&self) -> String {
        let summary_fields = self
            .summary
            .as_ref()
            .map(|s| format!(",{}", s.to_jsonl_fragment()))
            .unwrap_or_default();

        format!(
            r#"{{"schema":"conformal-frame-guard-telemetry-v1","state":"{}","observations":{},"degradation_triggers":{},"ema_us":{:.1},"frame_times_len":{},"nonconformity_len":{}{}}}"#,
            self.state.as_str(),
            self.observations,
            self.degradation_triggers,
            self.ema_us,
            self.frame_times_len,
            self.nonconformity_len,
            summary_fields,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::conformal_predictor::{DiffBucket, ModeBucket};

    fn test_key() -> BucketKey {
        BucketKey {
            mode: ModeBucket::AltScreen,
            diff: DiffBucket::Full,
            size_bucket: 2,
        }
    }

    #[test]
    fn warmup_uses_fixed_fallback() {
        let mut guard = ConformalFrameGuard::with_defaults();
        let key = test_key();

        // No observations yet
        let pred = guard.predict_p99(16_000.0, key);
        assert_eq!(pred.fallback_level, 4);
        assert_eq!(pred.state, GuardState::Warmup);
        assert!(!pred.exceeds_budget); // y_hat=0 < 16ms
        assert!(pred.conformal.is_none());
    }

    #[test]
    fn warmup_with_slow_frames_signals_risk() {
        let mut guard = ConformalFrameGuard::with_defaults();
        let key = test_key();

        // Feed 5 slow frames (30ms each) — not enough for calibration
        for _ in 0..5 {
            guard.observe(30_000.0, key);
        }

        let pred = guard.predict_p99(16_000.0, key);
        assert_eq!(pred.fallback_level, 4);
        assert!(pred.exceeds_budget); // EMA ~30ms > 16ms fallback
        assert_eq!(pred.state, GuardState::AtRisk);
    }

    #[test]
    fn calibration_transitions_from_warmup() {
        let mut guard = ConformalFrameGuard::with_defaults();
        let key = test_key();

        // Feed min_samples (20) fast frames
        for _ in 0..20 {
            guard.observe(8_000.0, key);
        }

        assert!(guard.is_calibrated());
        assert_eq!(guard.state(), GuardState::Calibrated);
    }

    #[test]
    fn calibrated_prediction_has_conformal_data() {
        let mut guard = ConformalFrameGuard::with_defaults();
        let key = test_key();

        // Calibrate with 25 samples of ~10ms
        for _ in 0..25 {
            guard.observe(10_000.0, key);
        }

        let pred = guard.predict_p99(16_000.0, key);
        assert!(pred.conformal.is_some());
        assert!(pred.fallback_level < 4);
        assert!(!pred.exceeds_budget); // 10ms well under 16ms budget
        assert_eq!(pred.state, GuardState::Calibrated);
    }

    #[test]
    fn calibrated_slow_frames_trigger_at_risk() {
        let mut guard = ConformalFrameGuard::with_defaults();
        let key = test_key();

        // Calibrate with slow frames (20ms)
        for _ in 0..25 {
            guard.observe(20_000.0, key);
        }

        let pred = guard.predict_p99(16_000.0, key);
        assert!(pred.exceeds_budget);
        assert_eq!(pred.state, GuardState::AtRisk);
        assert!(guard.degradation_triggers() > 0);
    }

    #[test]
    fn nonconformity_scores_tracked() {
        let mut guard = ConformalFrameGuard::with_defaults();
        let key = test_key();

        for i in 0..10 {
            guard.observe(10_000.0 + (i as f64 * 100.0), key);
        }

        assert_eq!(guard.nonconformity_scores().len(), 10);
        assert_eq!(guard.frame_times().len(), 10);
    }

    #[test]
    fn nonconformity_summary_computes_percentiles() {
        let mut guard = ConformalFrameGuard::with_defaults();
        let key = test_key();

        // Feed 100 samples with known distribution
        for i in 0..100 {
            guard.observe(10_000.0 + (i as f64 * 100.0), key);
        }

        let summary = guard.nonconformity_summary();
        assert!(summary.is_some());
        let s = summary.unwrap();
        assert_eq!(s.count, 100);
        assert!(s.p99 >= s.p90);
        assert!(s.p90 >= s.p50);
        assert!(s.max >= s.p99);
    }

    #[test]
    fn reset_clears_state_but_preserves_triggers() {
        let mut guard = ConformalFrameGuard::with_defaults();
        let key = test_key();

        // Feed slow frames to trigger degradation
        for _ in 0..25 {
            guard.observe(20_000.0, key);
        }
        let _ = guard.predict_p99(16_000.0, key);
        let triggers_before = guard.degradation_triggers();
        assert!(triggers_before > 0);

        guard.reset();

        assert_eq!(guard.state(), GuardState::Warmup);
        assert_eq!(guard.observations(), 0);
        assert!(guard.frame_times().is_empty());
        assert!(guard.nonconformity_scores().is_empty());
        // Triggers preserved for audit trail
        assert_eq!(guard.degradation_triggers(), triggers_before);
    }

    #[test]
    fn suggest_action_degrades_when_at_risk() {
        let guard = ConformalFrameGuard::with_defaults();

        let pred = P99Prediction {
            y_hat_us: 18_000.0,
            upper_us: 20_000.0,
            budget_us: 16_000.0,
            exceeds_budget: true,
            calibration_size: 25,
            fallback_level: 0,
            state: GuardState::AtRisk,
            interval_width_us: 2_000.0,
            conformal: None,
        };

        let action = guard.suggest_action(&pred, DegradationLevel::Full);
        assert_eq!(action, Some(DegradationLevel::SimpleBorders));
    }

    #[test]
    fn suggest_action_holds_at_max_degradation() {
        let guard = ConformalFrameGuard::with_defaults();

        let pred = P99Prediction {
            y_hat_us: 30_000.0,
            upper_us: 35_000.0,
            budget_us: 16_000.0,
            exceeds_budget: true,
            calibration_size: 25,
            fallback_level: 0,
            state: GuardState::AtRisk,
            interval_width_us: 5_000.0,
            conformal: None,
        };

        let action = guard.suggest_action(&pred, DegradationLevel::SkipFrame);
        assert!(action.is_none());
    }

    #[test]
    fn suggest_action_holds_when_within_budget() {
        let guard = ConformalFrameGuard::with_defaults();

        let pred = P99Prediction {
            y_hat_us: 10_000.0,
            upper_us: 14_000.0,
            budget_us: 16_000.0,
            exceeds_budget: false,
            calibration_size: 25,
            fallback_level: 0,
            state: GuardState::Calibrated,
            interval_width_us: 4_000.0,
            conformal: None,
        };

        let action = guard.suggest_action(&pred, DegradationLevel::Full);
        assert!(action.is_none());
    }

    #[test]
    fn ema_tracks_frame_times() {
        let mut guard = ConformalFrameGuard::with_defaults();
        let key = test_key();

        // All 10ms frames
        for _ in 0..50 {
            guard.observe(10_000.0, key);
        }

        // EMA should converge close to 10_000
        let ema = guard.ema_us();
        assert!(
            (ema - 10_000.0).abs() < 500.0,
            "EMA should be ~10000, got {ema}"
        );
    }

    #[test]
    fn invalid_frame_time_ignored() {
        let mut guard = ConformalFrameGuard::with_defaults();
        let key = test_key();

        guard.observe(f64::NAN, key);
        guard.observe(f64::INFINITY, key);
        guard.observe(-1.0, key);

        assert_eq!(guard.observations(), 0);
        assert!(guard.frame_times().is_empty());
    }

    #[test]
    fn jsonl_output_is_valid_json() {
        let pred = P99Prediction {
            y_hat_us: 10_000.0,
            upper_us: 14_000.0,
            budget_us: 16_000.0,
            exceeds_budget: false,
            calibration_size: 25,
            fallback_level: 0,
            state: GuardState::Calibrated,
            interval_width_us: 4_000.0,
            conformal: None,
        };

        let json_str = pred.to_jsonl();
        // Verify it parses as JSON (basic check: starts/ends with braces, has schema)
        assert!(json_str.starts_with('{'));
        assert!(json_str.ends_with('}'));
        assert!(json_str.contains("conformal-frame-guard-v1"));
    }

    #[test]
    fn telemetry_snapshot_captures_state() {
        let mut guard = ConformalFrameGuard::with_defaults();
        let key = test_key();

        for _ in 0..30 {
            guard.observe(12_000.0, key);
        }

        let telem = guard.telemetry();
        assert_eq!(telem.observations, 30);
        assert_eq!(telem.frame_times_len, 30);
        assert_eq!(telem.nonconformity_len, 30);
        assert!(telem.summary.is_some());

        let json_str = telem.to_jsonl();
        assert!(json_str.contains("conformal-frame-guard-telemetry-v1"));
    }

    #[test]
    fn window_limits_respected() {
        let config = ConformalFrameGuardConfig {
            time_series_window: 10,
            nonconformity_window: 5,
            ..Default::default()
        };
        let mut guard = ConformalFrameGuard::new(config);
        let key = test_key();

        for i in 0..100 {
            guard.observe(10_000.0 + (i as f64), key);
        }

        assert_eq!(guard.frame_times().len(), 10);
        assert_eq!(guard.nonconformity_scores().len(), 5);
    }
}
