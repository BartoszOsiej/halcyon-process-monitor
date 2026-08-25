#![forbid(unsafe_code)]

//! bd-3fc.8: Performance regression detection accuracy tests.
//!
//! Verify that performance regression detection correctly identifies true
//! regressions and avoids false positives from noise:
//! - Known 15% regression is detected
//! - Known 5% noise is NOT flagged
//! - Baseline loading and comparison works correctly
//! - slo.yaml threshold enforcement
//! - Assert WARN for true regression
//! - Assert no false-positive WARN for noise-level changes
//!
//! Run:
//!   cargo test -p ftui-harness --test regression_detection_accuracy

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::registry::LookupSpan;

// ============================================================================
// Regression Detection Engine (self-contained for testing)
// ============================================================================

/// A performance metric baseline entry.
#[derive(Debug, Clone)]
#[allow(dead_code)]
struct BaselineMetric {
    name: String,
    p50_us: f64,
    p95_us: f64,
    p99_us: f64,
}

/// Configuration loaded from slo.yaml format.
#[derive(Debug, Clone)]
struct SloConfig {
    /// Regression threshold as a fraction (e.g., 0.10 = 10%).
    regression_threshold: f64,
    /// Noise tolerance as a fraction (e.g., 0.05 = 5%).
    noise_tolerance: f64,
    /// Metric-specific thresholds (overrides global).
    metric_thresholds: HashMap<String, MetricSlo>,
}

/// Per-metric SLO definition.
#[derive(Debug, Clone)]
struct MetricSlo {
    /// Maximum allowed p99 latency in microseconds.
    max_p99_us: Option<f64>,
    /// Maximum allowed regression ratio (e.g., 1.15 = 15% increase).
    max_regression_ratio: Option<f64>,
}

/// Result of a regression check.
#[derive(Debug, Clone)]
struct RegressionResult {
    metric_name: String,
    baseline_p99_us: f64,
    current_p99_us: f64,
    ratio: f64,
    is_regression: bool,
    severity: RegressionSeverity,
}

#[derive(Debug, Clone, PartialEq)]
enum RegressionSeverity {
    /// No regression detected.
    None,
    /// Within noise tolerance — not flagged.
    Noise,
    /// Significant regression detected.
    Regression,
    /// SLO absolute threshold breached.
    SloBreach,
}

impl Default for SloConfig {
    fn default() -> Self {
        Self {
            regression_threshold: 0.10,
            noise_tolerance: 0.05,
            metric_thresholds: HashMap::new(),
        }
    }
}

/// Parse a minimal slo.yaml-style configuration from a YAML-like string.
///
/// Format:
/// ```yaml
/// regression_threshold: 0.10
/// noise_tolerance: 0.05
/// metrics:
///   frame_pipeline_80x24:
///     max_p99_us: 500.0
///     max_regression_ratio: 1.15
/// ```
fn parse_slo_config(yaml: &str) -> SloConfig {
    let mut config = SloConfig::default();
    let mut current_metric: Option<String> = None;
    let mut current_slo = MetricSlo {
        max_p99_us: None,
        max_regression_ratio: None,
    };

    for line in yaml.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }

        if let Some(value) = trimmed.strip_prefix("regression_threshold:") {
            config.regression_threshold = value.trim().parse().unwrap_or(0.10);
        } else if let Some(value) = trimmed.strip_prefix("noise_tolerance:") {
            config.noise_tolerance = value.trim().parse().unwrap_or(0.05);
        } else if trimmed == "metrics:" {
            // Start of metrics section
        } else if trimmed.ends_with(':') && !trimmed.starts_with("max_") {
            // Flush previous metric
            if let Some(ref name) = current_metric {
                config
                    .metric_thresholds
                    .insert(name.clone(), current_slo.clone());
            }
            current_metric = Some(trimmed.trim_end_matches(':').to_string());
            current_slo = MetricSlo {
                max_p99_us: None,
                max_regression_ratio: None,
            };
        } else if let Some(value) = trimmed.strip_prefix("max_p99_us:") {
            current_slo.max_p99_us = value.trim().parse().ok();
        } else if let Some(value) = trimmed.strip_prefix("max_regression_ratio:") {
            current_slo.max_regression_ratio = value.trim().parse().ok();
        }
    }

    // Flush last metric
    if let Some(ref name) = current_metric {
        config.metric_thresholds.insert(name.clone(), current_slo);
    }

    config
}

/// Compare current metrics against baseline and return regression results.
fn check_regression(
    baseline: &BaselineMetric,
    current: &BaselineMetric,
    config: &SloConfig,
) -> RegressionResult {
    let ratio = if baseline.p99_us > 0.0 {
        current.p99_us / baseline.p99_us
    } else {
        1.0
    };

    // Check metric-specific SLO
    if let Some(slo) = config.metric_thresholds.get(&baseline.name) {
        if let Some(max_p99) = slo.max_p99_us
            && current.p99_us > max_p99
        {
            return RegressionResult {
                metric_name: baseline.name.clone(),
                baseline_p99_us: baseline.p99_us,
                current_p99_us: current.p99_us,
                ratio,
                is_regression: true,
                severity: RegressionSeverity::SloBreach,
            };
        }
        if let Some(max_ratio) = slo.max_regression_ratio
            && ratio > max_ratio
        {
            return RegressionResult {
                metric_name: baseline.name.clone(),
                baseline_p99_us: baseline.p99_us,
                current_p99_us: current.p99_us,
                ratio,
                is_regression: true,
                severity: RegressionSeverity::Regression,
            };
        }
    }

    // Check against global threshold
    let severity = if ratio > 1.0 + config.regression_threshold {
        RegressionSeverity::Regression
    } else if ratio > 1.0 + config.noise_tolerance {
        // Between noise tolerance and regression threshold — could be noise
        // but flagged as potential issue
        RegressionSeverity::Noise
    } else {
        RegressionSeverity::None
    };

    let is_regression =
        severity == RegressionSeverity::Regression || severity == RegressionSeverity::SloBreach;

    RegressionResult {
        metric_name: baseline.name.clone(),
        baseline_p99_us: baseline.p99_us,
        current_p99_us: current.p99_us,
        ratio,
        is_regression,
        severity,
    }
}

/// Emit tracing events for regression results.
fn emit_regression_event(result: &RegressionResult) {
    match result.severity {
        RegressionSeverity::Regression | RegressionSeverity::SloBreach => {
            tracing::warn!(
                metric = result.metric_name.as_str(),
                baseline_p99_us = result.baseline_p99_us,
                current_p99_us = result.current_p99_us,
                ratio = result.ratio,
                severity = ?result.severity,
                "performance regression detected"
            );
        }
        RegressionSeverity::Noise => {
            tracing::debug!(
                metric = result.metric_name.as_str(),
                baseline_p99_us = result.baseline_p99_us,
                current_p99_us = result.current_p99_us,
                ratio = result.ratio,
                "noise-level performance change (within tolerance)"
            );
        }
        RegressionSeverity::None => {
            tracing::trace!(
                metric = result.metric_name.as_str(),
                ratio = result.ratio,
                "performance within baseline"
            );
        }
    }
}

// ============================================================================
// Tracing Capture Infrastructure
// ============================================================================

#[derive(Debug, Clone)]
#[allow(dead_code)]
struct CapturedEvent {
    level: tracing::Level,
    message: Option<String>,
    fields: HashMap<String, String>,
}

struct EventCapture {
    events: Arc<Mutex<Vec<CapturedEvent>>>,
}

impl EventCapture {
    fn new() -> (Self, Arc<Mutex<Vec<CapturedEvent>>>) {
        let events = Arc::new(Mutex::new(Vec::new()));
        (
            Self {
                events: events.clone(),
            },
            events,
        )
    }
}

struct FieldVisitor(Vec<(String, String)>);

impl tracing::field::Visit for FieldVisitor {
    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        self.0
            .push((field.name().to_string(), format!("{value:?}")));
    }

    fn record_u64(&mut self, field: &tracing::field::Field, value: u64) {
        self.0.push((field.name().to_string(), value.to_string()));
    }

    fn record_f64(&mut self, field: &tracing::field::Field, value: f64) {
        self.0.push((field.name().to_string(), value.to_string()));
    }

    fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
        self.0.push((field.name().to_string(), value.to_string()));
    }

    fn record_bool(&mut self, field: &tracing::field::Field, value: bool) {
        self.0.push((field.name().to_string(), value.to_string()));
    }
}

impl<S> tracing_subscriber::Layer<S> for EventCapture
where
    S: tracing::Subscriber + for<'a> LookupSpan<'a>,
{
    fn on_event(
        &self,
        event: &tracing::Event<'_>,
        _ctx: tracing_subscriber::layer::Context<'_, S>,
    ) {
        let mut visitor = FieldVisitor(Vec::new());
        event.record(&mut visitor);

        let fields: HashMap<String, String> = visitor.0.clone().into_iter().collect();
        let message = fields.get("message").cloned();

        self.events.lock().unwrap().push(CapturedEvent {
            level: *event.metadata().level(),
            message,
            fields,
        });
    }
}

fn with_captured_events<F>(f: F) -> Vec<CapturedEvent>
where
    F: FnOnce(),
{
    ensure_global_trace_level();
    let (layer, events) = EventCapture::new();
    let subscriber = tracing_subscriber::registry().with(layer);
    tracing::subscriber::with_default(subscriber, || {
        tracing::callsite::rebuild_interest_cache();
        f()
    });
    events.lock().unwrap().clone()
}

fn ensure_global_trace_level() {
    static INIT: std::sync::Once = std::sync::Once::new();
    INIT.call_once(|| {
        let subscriber =
            tracing_subscriber::registry().with(tracing_subscriber::filter::LevelFilter::TRACE);
        let _ = tracing::subscriber::set_global_default(subscriber);
    });
}

// ============================================================================
// Regression Detection Accuracy Tests
// ============================================================================

/// Known 15% regression must be detected.
#[test]
fn detects_15_percent_regression() {
    let config = SloConfig::default(); // 10% threshold

    let baseline = BaselineMetric {
        name: "frame_pipeline_80x24".into(),
        p50_us: 100.0,
        p95_us: 150.0,
        p99_us: 200.0,
    };

    let current = BaselineMetric {
        name: "frame_pipeline_80x24".into(),
        p50_us: 115.0,
        p95_us: 172.5,
        p99_us: 230.0, // 15% increase
    };

    let result = check_regression(&baseline, &current, &config);

    assert!(
        result.is_regression,
        "A 15% p99 increase (200 → 230) must be detected as regression. Got ratio: {:.2}",
        result.ratio
    );
    assert_eq!(
        result.severity,
        RegressionSeverity::Regression,
        "Severity should be Regression, got {:?}",
        result.severity
    );
    assert!(
        (result.ratio - 1.15).abs() < 0.001,
        "Ratio should be ~1.15, got {}",
        result.ratio
    );
}

/// Known 5% noise must NOT be flagged as regression.
///
/// With default config (noise_tolerance=5%, regression_threshold=10%):
/// - 0-5% → None (within noise)
/// - 5.01-10% → Noise (above noise, below regression)
/// - >10% → Regression
#[test]
fn does_not_flag_5_percent_noise() {
    let config = SloConfig::default(); // 10% threshold, 5% noise tolerance

    let baseline = BaselineMetric {
        name: "diff_engine_80x24_25pct".into(),
        p50_us: 50.0,
        p95_us: 80.0,
        p99_us: 100.0,
    };

    // Exactly 5% increase (at noise tolerance boundary): classified as None
    let current_5pct = BaselineMetric {
        name: "diff_engine_80x24_25pct".into(),
        p50_us: 52.5,
        p95_us: 84.0,
        p99_us: 105.0,
    };

    let result = check_regression(&baseline, &current_5pct, &config);
    assert!(
        !result.is_regression,
        "A 5% p99 increase must NOT be flagged as regression. Got ratio: {:.2}",
        result.ratio
    );
    assert_eq!(
        result.severity,
        RegressionSeverity::None,
        "5% increase (at boundary) should be None, got {:?}",
        result.severity
    );

    // 7% increase (above noise, below regression): classified as Noise
    let current_7pct = BaselineMetric {
        name: "diff_engine_80x24_25pct".into(),
        p50_us: 53.5,
        p95_us: 85.6,
        p99_us: 107.0,
    };

    let result = check_regression(&baseline, &current_7pct, &config);
    assert!(
        !result.is_regression,
        "A 7% p99 increase must NOT be flagged as regression. Got ratio: {:.2}",
        result.ratio
    );
    assert_eq!(
        result.severity,
        RegressionSeverity::Noise,
        "7% increase should be classified as Noise, got {:?}",
        result.severity
    );
}

/// Exactly at the threshold boundary (10%) should be classified as regression.
#[test]
fn boundary_at_threshold_is_regression() {
    let config = SloConfig::default(); // 10% threshold

    let baseline = BaselineMetric {
        name: "layout_10_widgets".into(),
        p50_us: 20.0,
        p95_us: 30.0,
        p99_us: 100.0,
    };

    // Just over 10%: 100 * 1.101 = 110.1
    let current = BaselineMetric {
        name: "layout_10_widgets".into(),
        p50_us: 22.0,
        p95_us: 33.0,
        p99_us: 110.1,
    };

    let result = check_regression(&baseline, &current, &config);

    assert!(
        result.is_regression,
        "At 10.1% increase, should be flagged. Ratio: {:.4}",
        result.ratio
    );
}

/// Just under the threshold (9.9%) should NOT be flagged.
#[test]
fn just_under_threshold_is_not_regression() {
    let config = SloConfig::default(); // 10% threshold

    let baseline = BaselineMetric {
        name: "layout_10_widgets".into(),
        p50_us: 20.0,
        p95_us: 30.0,
        p99_us: 100.0,
    };

    let current = BaselineMetric {
        name: "layout_10_widgets".into(),
        p50_us: 22.0,
        p95_us: 33.0,
        p99_us: 109.9, // 9.9% increase
    };

    let result = check_regression(&baseline, &current, &config);

    assert!(
        !result.is_regression,
        "At 9.9% increase, should NOT be flagged. Ratio: {:.4}",
        result.ratio
    );
}

/// Performance improvement (negative regression) should not be flagged.
#[test]
fn performance_improvement_not_flagged() {
    let config = SloConfig::default();

    let baseline = BaselineMetric {
        name: "diff_engine_200x60_full".into(),
        p50_us: 500.0,
        p95_us: 800.0,
        p99_us: 1000.0,
    };

    let current = BaselineMetric {
        name: "diff_engine_200x60_full".into(),
        p50_us: 350.0,
        p95_us: 560.0,
        p99_us: 700.0, // 30% improvement
    };

    let result = check_regression(&baseline, &current, &config);

    assert!(
        !result.is_regression,
        "Performance improvement should not be flagged. Ratio: {:.2}",
        result.ratio
    );
    assert_eq!(result.severity, RegressionSeverity::None);
    assert!(result.ratio < 1.0, "Ratio should be < 1.0 for improvement");
}

/// Identical performance (no change) should not be flagged.
#[test]
fn identical_performance_not_flagged() {
    let config = SloConfig::default();

    let baseline = BaselineMetric {
        name: "frame_pipeline_120x40".into(),
        p50_us: 200.0,
        p95_us: 350.0,
        p99_us: 500.0,
    };

    let current = baseline.clone();

    let result = check_regression(&baseline, &current, &config);

    assert!(!result.is_regression);
    assert_eq!(result.severity, RegressionSeverity::None);
    assert!((result.ratio - 1.0).abs() < f64::EPSILON);
}

/// Zero baseline should not cause divide-by-zero.
#[test]
fn zero_baseline_no_panic() {
    let config = SloConfig::default();

    let baseline = BaselineMetric {
        name: "zero_metric".into(),
        p50_us: 0.0,
        p95_us: 0.0,
        p99_us: 0.0,
    };

    let current = BaselineMetric {
        name: "zero_metric".into(),
        p50_us: 1.0,
        p95_us: 2.0,
        p99_us: 5.0,
    };

    let result = check_regression(&baseline, &current, &config);

    // Should not panic, and ratio should default to 1.0
    assert!((result.ratio - 1.0).abs() < f64::EPSILON);
    assert!(!result.is_regression);
}

// ============================================================================
// slo.yaml Schema Tests
// ============================================================================

/// Parse a valid slo.yaml configuration.
#[test]
fn parse_valid_slo_yaml() {
    let yaml = r#"
regression_threshold: 0.10
noise_tolerance: 0.05
metrics:
  frame_pipeline_80x24:
    max_p99_us: 500.0
    max_regression_ratio: 1.15
  diff_engine_120x40:
    max_p99_us: 200.0
    max_regression_ratio: 1.20
"#;

    let config = parse_slo_config(yaml);

    assert!((config.regression_threshold - 0.10).abs() < f64::EPSILON);
    assert!((config.noise_tolerance - 0.05).abs() < f64::EPSILON);

    let frame_slo = config
        .metric_thresholds
        .get("frame_pipeline_80x24")
        .expect("should have frame_pipeline_80x24");
    assert_eq!(frame_slo.max_p99_us, Some(500.0));
    assert_eq!(frame_slo.max_regression_ratio, Some(1.15));

    let diff_slo = config
        .metric_thresholds
        .get("diff_engine_120x40")
        .expect("should have diff_engine_120x40");
    assert_eq!(diff_slo.max_p99_us, Some(200.0));
    assert_eq!(diff_slo.max_regression_ratio, Some(1.20));
}

/// slo.yaml with comments and blank lines should be parsed correctly.
#[test]
fn parse_slo_yaml_with_comments() {
    let yaml = r#"
# Global thresholds
regression_threshold: 0.15

# Noise tolerance
noise_tolerance: 0.08

# Metric definitions
metrics:
  layout_50_widgets:
    # Max p99 latency for 50-widget layout
    max_p99_us: 1000.0
"#;

    let config = parse_slo_config(yaml);

    assert!((config.regression_threshold - 0.15).abs() < f64::EPSILON);
    assert!((config.noise_tolerance - 0.08).abs() < f64::EPSILON);

    let layout_slo = config
        .metric_thresholds
        .get("layout_50_widgets")
        .expect("should have layout_50_widgets");
    assert_eq!(layout_slo.max_p99_us, Some(1000.0));
}

/// Empty slo.yaml uses defaults.
#[test]
fn parse_empty_slo_yaml_uses_defaults() {
    let config = parse_slo_config("");

    assert!((config.regression_threshold - 0.10).abs() < f64::EPSILON);
    assert!((config.noise_tolerance - 0.05).abs() < f64::EPSILON);
    assert!(config.metric_thresholds.is_empty());
}

/// slo.yaml absolute threshold enforcement.
#[test]
fn slo_absolute_threshold_breach() {
    let yaml = r#"
regression_threshold: 0.10
noise_tolerance: 0.05
metrics:
  frame_pipeline_80x24:
    max_p99_us: 500.0
"#;

    let config = parse_slo_config(yaml);

    let baseline = BaselineMetric {
        name: "frame_pipeline_80x24".into(),
        p50_us: 300.0,
        p95_us: 450.0,
        p99_us: 480.0,
    };

    // Current breaches absolute SLO (> 500us) even though ratio is only 1.06
    let current = BaselineMetric {
        name: "frame_pipeline_80x24".into(),
        p50_us: 320.0,
        p95_us: 475.0,
        p99_us: 510.0, // Above 500us SLO
    };

    let result = check_regression(&baseline, &current, &config);

    assert!(result.is_regression, "Should flag SLO breach");
    assert_eq!(
        result.severity,
        RegressionSeverity::SloBreach,
        "Should be SloBreach severity"
    );
}

/// slo.yaml per-metric regression ratio enforcement.
#[test]
fn slo_metric_specific_regression_ratio() {
    let yaml = r#"
regression_threshold: 0.20
noise_tolerance: 0.05
metrics:
  critical_path:
    max_regression_ratio: 1.05
"#;

    let config = parse_slo_config(yaml);

    let baseline = BaselineMetric {
        name: "critical_path".into(),
        p50_us: 100.0,
        p95_us: 150.0,
        p99_us: 200.0,
    };

    // 8% increase: below global 20% but above metric-specific 5%
    let current = BaselineMetric {
        name: "critical_path".into(),
        p50_us: 108.0,
        p95_us: 162.0,
        p99_us: 216.0,
    };

    let result = check_regression(&baseline, &current, &config);

    assert!(
        result.is_regression,
        "Per-metric 5% threshold should catch 8% increase even though global is 20%"
    );
}

/// Metric without specific SLO uses global threshold.
#[test]
fn metric_without_slo_uses_global() {
    let yaml = r#"
regression_threshold: 0.10
noise_tolerance: 0.05
metrics:
  other_metric:
    max_p99_us: 1000.0
"#;

    let config = parse_slo_config(yaml);

    let baseline = BaselineMetric {
        name: "unconfigured_metric".into(),
        p50_us: 100.0,
        p95_us: 150.0,
        p99_us: 200.0,
    };

    // 15% increase: above global 10%
    let current = BaselineMetric {
        name: "unconfigured_metric".into(),
        p50_us: 115.0,
        p95_us: 172.5,
        p99_us: 230.0,
    };

    let result = check_regression(&baseline, &current, &config);

    assert!(
        result.is_regression,
        "Unconfigured metric should use global 10% threshold"
    );
}

// ============================================================================
// Baseline Loading Tests
// ============================================================================

/// Baseline comparison with multiple metrics.
#[test]
fn multi_metric_baseline_comparison() {
    let config = SloConfig::default();

    let metrics = vec![
        ("frame_pipeline_80x24", 200.0, 230.0),    // 15% regression
        ("diff_engine_80x24", 100.0, 107.0),       // 7% noise (above 5%, below 10%)
        ("layout_10_widgets", 50.0, 45.0),         // 10% improvement
        ("frame_pipeline_200x60", 1000.0, 1200.0), // 20% regression
    ];

    let mut regressions = Vec::new();
    let mut noise = Vec::new();
    let mut ok = Vec::new();

    for (name, baseline_p99, current_p99) in &metrics {
        let baseline = BaselineMetric {
            name: name.to_string(),
            p50_us: baseline_p99 * 0.5,
            p95_us: baseline_p99 * 0.8,
            p99_us: *baseline_p99,
        };
        let current = BaselineMetric {
            name: name.to_string(),
            p50_us: current_p99 * 0.5,
            p95_us: current_p99 * 0.8,
            p99_us: *current_p99,
        };
        let result = check_regression(&baseline, &current, &config);
        match result.severity {
            RegressionSeverity::Regression | RegressionSeverity::SloBreach => {
                regressions.push(result);
            }
            RegressionSeverity::Noise => {
                noise.push(result);
            }
            RegressionSeverity::None => {
                ok.push(result);
            }
        }
    }

    assert_eq!(
        regressions.len(),
        2,
        "Should detect 2 regressions (15% and 20%)"
    );
    assert_eq!(noise.len(), 1, "Should have 1 noise-level change (7%)");
    assert_eq!(ok.len(), 1, "Should have 1 ok result (improvement)");

    // Verify specific regression names
    assert!(
        regressions
            .iter()
            .any(|r| r.metric_name == "frame_pipeline_80x24")
    );
    assert!(
        regressions
            .iter()
            .any(|r| r.metric_name == "frame_pipeline_200x60")
    );
}

/// Baseline with many small changes (statistical noise profile).
#[test]
fn statistical_noise_profile_not_flagged() {
    let config = SloConfig::default();
    let noise_levels = [
        0.01, 0.02, 0.03, 0.04, 0.05, -0.01, -0.02, -0.03, -0.04, -0.05,
    ];

    let mut false_positives = 0;
    for (i, &noise_pct) in noise_levels.iter().enumerate() {
        let baseline_p99 = 100.0 + (i as f64 * 50.0);
        let current_p99 = baseline_p99 * (1.0 + noise_pct);

        let baseline = BaselineMetric {
            name: format!("metric_{i}"),
            p50_us: baseline_p99 * 0.5,
            p95_us: baseline_p99 * 0.8,
            p99_us: baseline_p99,
        };
        let current = BaselineMetric {
            name: format!("metric_{i}"),
            p50_us: current_p99 * 0.5,
            p95_us: current_p99 * 0.8,
            p99_us: current_p99,
        };

        let result = check_regression(&baseline, &current, &config);
        if result.is_regression {
            false_positives += 1;
        }
    }

    assert_eq!(
        false_positives, 0,
        "Noise within ±5% should produce zero false positives"
    );
}

// ============================================================================
// Tracing Event Tests (WARN for regression, no WARN for noise)
// ============================================================================

/// WARN emitted for true regression.
#[test]
fn warn_emitted_for_true_regression() {
    let config = SloConfig::default();

    let baseline = BaselineMetric {
        name: "critical_render".into(),
        p50_us: 100.0,
        p95_us: 150.0,
        p99_us: 200.0,
    };

    let current = BaselineMetric {
        name: "critical_render".into(),
        p50_us: 120.0,
        p95_us: 180.0,
        p99_us: 240.0, // 20% regression
    };

    let result = check_regression(&baseline, &current, &config);

    let events = with_captured_events(|| {
        emit_regression_event(&result);
    });

    let warn_events: Vec<_> = events
        .iter()
        .filter(|e| e.level == tracing::Level::WARN)
        .collect();

    assert_eq!(
        warn_events.len(),
        1,
        "Should emit exactly 1 WARN for true regression"
    );

    let warn = &warn_events[0];
    assert!(
        warn.message.as_deref().unwrap_or("").contains("regression"),
        "WARN message should mention 'regression'"
    );
    assert!(
        warn.fields.contains_key("metric"),
        "WARN should include 'metric' field"
    );
    assert!(
        warn.fields.contains_key("ratio"),
        "WARN should include 'ratio' field"
    );
    assert!(
        warn.fields.contains_key("baseline_p99_us"),
        "WARN should include 'baseline_p99_us' field"
    );
    assert!(
        warn.fields.contains_key("current_p99_us"),
        "WARN should include 'current_p99_us' field"
    );
}

/// No WARN emitted for noise-level change (in Noise band: 5-10%).
#[test]
fn no_warn_for_noise_level_change() {
    let config = SloConfig::default();

    let baseline = BaselineMetric {
        name: "minor_path".into(),
        p50_us: 50.0,
        p95_us: 75.0,
        p99_us: 100.0,
    };

    let current = BaselineMetric {
        name: "minor_path".into(),
        p50_us: 54.0,
        p95_us: 81.0,
        p99_us: 108.0, // 8% change — in Noise band (above 5%, below 10%)
    };

    let result = check_regression(&baseline, &current, &config);

    let events = with_captured_events(|| {
        emit_regression_event(&result);
    });

    let warn_events: Vec<_> = events
        .iter()
        .filter(|e| e.level == tracing::Level::WARN)
        .collect();

    assert_eq!(
        warn_events.len(),
        0,
        "Should NOT emit WARN for noise-level change (8%)"
    );

    // Should emit DEBUG for Noise-band changes
    let debug_events: Vec<_> = events
        .iter()
        .filter(|e| e.level == tracing::Level::DEBUG)
        .collect();
    assert_eq!(
        debug_events.len(),
        1,
        "Should emit DEBUG for noise-band change"
    );
}

/// No WARN emitted for identical performance.
#[test]
fn no_warn_for_identical_performance() {
    let config = SloConfig::default();

    let baseline = BaselineMetric {
        name: "stable_metric".into(),
        p50_us: 100.0,
        p95_us: 150.0,
        p99_us: 200.0,
    };

    let current = baseline.clone();

    let result = check_regression(&baseline, &current, &config);

    let events = with_captured_events(|| {
        emit_regression_event(&result);
    });

    let warn_events: Vec<_> = events
        .iter()
        .filter(|e| e.level == tracing::Level::WARN)
        .collect();

    assert_eq!(
        warn_events.len(),
        0,
        "Should NOT emit WARN for identical performance"
    );

    // Should emit TRACE for stable metrics
    let trace_events: Vec<_> = events
        .iter()
        .filter(|e| e.level == tracing::Level::TRACE)
        .collect();
    assert_eq!(
        trace_events.len(),
        1,
        "Should emit TRACE for stable performance"
    );
}

/// WARN emitted for SLO breach.
#[test]
fn warn_emitted_for_slo_breach() {
    let yaml = r#"
regression_threshold: 0.10
noise_tolerance: 0.05
metrics:
  hot_path:
    max_p99_us: 500.0
"#;

    let config = parse_slo_config(yaml);

    let baseline = BaselineMetric {
        name: "hot_path".into(),
        p50_us: 300.0,
        p95_us: 450.0,
        p99_us: 490.0,
    };

    let current = BaselineMetric {
        name: "hot_path".into(),
        p50_us: 310.0,
        p95_us: 460.0,
        p99_us: 520.0, // Breaches 500us SLO
    };

    let result = check_regression(&baseline, &current, &config);

    let events = with_captured_events(|| {
        emit_regression_event(&result);
    });

    let warn_events: Vec<_> = events
        .iter()
        .filter(|e| e.level == tracing::Level::WARN)
        .collect();

    assert_eq!(warn_events.len(), 1, "Should emit WARN for SLO breach");
}

/// Multi-metric batch: only regressions emit WARN.
#[test]
fn batch_check_only_regressions_emit_warn() {
    let config = SloConfig::default();

    let checks = vec![
        ("fast_path", 100.0, 102.0),  // 2% noise
        ("slow_path", 500.0, 600.0),  // 20% regression
        ("improving", 300.0, 250.0),  // improvement
        ("stable", 200.0, 200.0),     // unchanged
        ("borderline", 100.0, 112.0), // 12% regression
    ];

    let mut results = Vec::new();
    for (name, baseline_p99, current_p99) in &checks {
        let baseline = BaselineMetric {
            name: name.to_string(),
            p50_us: baseline_p99 * 0.5,
            p95_us: baseline_p99 * 0.8,
            p99_us: *baseline_p99,
        };
        let current = BaselineMetric {
            name: name.to_string(),
            p50_us: current_p99 * 0.5,
            p95_us: current_p99 * 0.8,
            p99_us: *current_p99,
        };
        results.push(check_regression(&baseline, &current, &config));
    }

    let events = with_captured_events(|| {
        for result in &results {
            emit_regression_event(result);
        }
    });

    let warn_events: Vec<_> = events
        .iter()
        .filter(|e| e.level == tracing::Level::WARN)
        .collect();

    assert_eq!(
        warn_events.len(),
        2,
        "Only 2 regressions (20% and 12%) should emit WARN, got {}",
        warn_events.len()
    );
}

// ============================================================================
// Regression Ratio Calculation Tests
// ============================================================================

/// Verify ratio calculation is correct for various scenarios.
#[test]
fn ratio_calculation_correct() {
    let config = SloConfig::default();

    let test_cases = vec![
        (100.0, 115.0, 1.15),  // 15% increase
        (100.0, 100.0, 1.00),  // No change
        (100.0, 50.0, 0.50),   // 50% improvement
        (100.0, 200.0, 2.00),  // 100% regression
        (100.0, 100.1, 1.001), // 0.1% noise
        (1000.0, 1001.0, 1.001),
    ];

    for (baseline_p99, current_p99, expected_ratio) in test_cases {
        let baseline = BaselineMetric {
            name: "test".into(),
            p50_us: baseline_p99 * 0.5,
            p95_us: baseline_p99 * 0.8,
            p99_us: baseline_p99,
        };
        let current = BaselineMetric {
            name: "test".into(),
            p50_us: current_p99 * 0.5,
            p95_us: current_p99 * 0.8,
            p99_us: current_p99,
        };

        let result = check_regression(&baseline, &current, &config);
        assert!(
            (result.ratio - expected_ratio).abs() < 0.01,
            "For baseline={baseline_p99}, current={current_p99}: expected ratio {expected_ratio}, got {}",
            result.ratio
        );
    }
}

/// Custom threshold configuration changes detection sensitivity.
#[test]
fn custom_threshold_changes_sensitivity() {
    // Strict config: 3% threshold
    let strict_config = SloConfig {
        regression_threshold: 0.03,
        noise_tolerance: 0.01,
        metric_thresholds: HashMap::new(),
    };

    // Relaxed config: 25% threshold
    let relaxed_config = SloConfig {
        regression_threshold: 0.25,
        noise_tolerance: 0.10,
        metric_thresholds: HashMap::new(),
    };

    let baseline = BaselineMetric {
        name: "test_metric".into(),
        p50_us: 100.0,
        p95_us: 150.0,
        p99_us: 200.0,
    };

    // 15% increase
    let current = BaselineMetric {
        name: "test_metric".into(),
        p50_us: 115.0,
        p95_us: 172.5,
        p99_us: 230.0,
    };

    let strict_result = check_regression(&baseline, &current, &strict_config);
    let relaxed_result = check_regression(&baseline, &current, &relaxed_config);

    assert!(
        strict_result.is_regression,
        "Strict 3% threshold should flag 15% regression"
    );
    assert!(
        !relaxed_result.is_regression,
        "Relaxed 25% threshold should NOT flag 15% change"
    );
}
