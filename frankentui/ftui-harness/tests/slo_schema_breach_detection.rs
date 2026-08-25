#![forbid(unsafe_code)]

//! bd-2xj.8: slo.yaml schema validation and breach detection tests.
//!
//! Covers:
//! - slo.yaml schema validation (malformed YAML rejected)
//! - Breach detection for all metric types (latency, memory, error rate)
//! - Safe-mode trigger logic
//! - Assert `slo.check` span with correct fields
//! - Assert WARN for breach, ERROR for safe-mode
//!
//! Run:
//!   cargo test -p ftui-harness --test slo_schema_breach_detection

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::registry::LookupSpan;

// ============================================================================
// SLO Schema Types
// ============================================================================

/// Metric type for SLO enforcement.
#[derive(Debug, Clone, PartialEq)]
enum MetricType {
    /// Latency in microseconds (p50, p95, p99).
    Latency,
    /// Memory usage in bytes.
    Memory,
    /// Error rate as a fraction (0.0-1.0).
    ErrorRate,
}

/// A metric observation with its type.
#[derive(Debug, Clone)]
#[allow(dead_code)]
struct MetricObservation {
    name: String,
    metric_type: MetricType,
    value: f64,
}

/// Per-metric SLO definition in the schema.
#[derive(Debug, Clone)]
struct MetricSloSchema {
    metric_type: MetricType,
    /// Maximum absolute value allowed.
    max_value: Option<f64>,
    /// Maximum ratio vs baseline before breach.
    max_ratio: Option<f64>,
    /// If true, breaching this metric triggers safe-mode.
    safe_mode_trigger: bool,
}

/// Validated SLO configuration.
#[derive(Debug, Clone)]
struct SloSchema {
    /// Global regression threshold as fraction (e.g., 0.10 = 10%).
    regression_threshold: f64,
    /// Global noise tolerance as fraction.
    noise_tolerance: f64,
    /// Per-metric SLO definitions.
    metrics: HashMap<String, MetricSloSchema>,
    /// Number of simultaneous breaches that triggers safe-mode.
    safe_mode_breach_count: usize,
    /// Error rate above which safe-mode is triggered regardless.
    safe_mode_error_rate: f64,
}

/// Schema validation error.
#[derive(Debug, Clone, PartialEq)]
enum SloSchemaError {
    /// A threshold value is out of range.
    InvalidThreshold { field: String, value: f64 },
    /// A required field is missing.
    #[allow(dead_code)]
    MissingField(String),
    /// A value failed to parse.
    ParseError { field: String, reason: String },
    /// Unknown metric type.
    UnknownMetricType(String),
    /// Duplicate metric definition.
    DuplicateMetric(String),
    /// General malformed YAML structure.
    MalformedStructure(String),
}

/// Result of a breach check.
#[derive(Debug, Clone)]
struct BreachResult {
    metric_name: String,
    metric_type: MetricType,
    baseline: f64,
    current: f64,
    ratio: f64,
    severity: BreachSeverity,
    safe_mode_trigger: bool,
}

/// Severity of a detected breach.
#[derive(Debug, Clone, PartialEq)]
enum BreachSeverity {
    /// No breach detected.
    None,
    /// Within noise tolerance.
    Noise,
    /// Exceeded regression threshold.
    Breach,
    /// Absolute SLO value exceeded.
    AbsoluteBreach,
}

/// Safe-mode decision result.
#[derive(Debug, Clone, PartialEq)]
enum SafeModeDecision {
    /// Normal operation continues.
    Normal,
    /// Safe-mode triggered with reason.
    Triggered(String),
}

// ============================================================================
// Schema Validation
// ============================================================================

impl Default for SloSchema {
    fn default() -> Self {
        Self {
            regression_threshold: 0.10,
            noise_tolerance: 0.05,
            metrics: HashMap::new(),
            safe_mode_breach_count: 3,
            safe_mode_error_rate: 0.10,
        }
    }
}

/// Parse and validate slo.yaml content.
///
/// Returns a validated `SloSchema` or a list of validation errors.
fn validate_slo_yaml(yaml: &str) -> Result<SloSchema, Vec<SloSchemaError>> {
    let mut schema = SloSchema::default();
    let mut errors = Vec::new();
    let mut in_metrics = false;
    let mut current_metric: Option<String> = None;
    let mut current_slo = MetricSloSchema {
        metric_type: MetricType::Latency,
        max_value: None,
        max_ratio: None,
        safe_mode_trigger: false,
    };
    let mut seen_metrics = std::collections::HashSet::new();

    for (line_num, line) in yaml.lines().enumerate() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }

        // Check for invalid YAML-like structure
        if trimmed.contains('\t') {
            errors.push(SloSchemaError::MalformedStructure(format!(
                "line {}: tabs not allowed, use spaces",
                line_num + 1
            )));
            continue;
        }

        if let Some(value) = trimmed.strip_prefix("regression_threshold:") {
            let value = value.trim();
            match value.parse::<f64>() {
                Ok(v) if (0.0..=1.0).contains(&v) => {
                    schema.regression_threshold = v;
                }
                Ok(v) => {
                    errors.push(SloSchemaError::InvalidThreshold {
                        field: "regression_threshold".into(),
                        value: v,
                    });
                }
                Err(e) => {
                    errors.push(SloSchemaError::ParseError {
                        field: "regression_threshold".into(),
                        reason: e.to_string(),
                    });
                }
            }
        } else if let Some(value) = trimmed.strip_prefix("noise_tolerance:") {
            let value = value.trim();
            match value.parse::<f64>() {
                Ok(v) if (0.0..=1.0).contains(&v) => {
                    schema.noise_tolerance = v;
                }
                Ok(v) => {
                    errors.push(SloSchemaError::InvalidThreshold {
                        field: "noise_tolerance".into(),
                        value: v,
                    });
                }
                Err(e) => {
                    errors.push(SloSchemaError::ParseError {
                        field: "noise_tolerance".into(),
                        reason: e.to_string(),
                    });
                }
            }
        } else if let Some(value) = trimmed.strip_prefix("safe_mode_breach_count:") {
            let value = value.trim();
            match value.parse::<usize>() {
                Ok(v) if v > 0 => {
                    schema.safe_mode_breach_count = v;
                }
                Ok(_) => {
                    errors.push(SloSchemaError::InvalidThreshold {
                        field: "safe_mode_breach_count".into(),
                        value: 0.0,
                    });
                }
                Err(e) => {
                    errors.push(SloSchemaError::ParseError {
                        field: "safe_mode_breach_count".into(),
                        reason: e.to_string(),
                    });
                }
            }
        } else if let Some(value) = trimmed.strip_prefix("safe_mode_error_rate:") {
            let value = value.trim();
            match value.parse::<f64>() {
                Ok(v) if (0.0..=1.0).contains(&v) => {
                    schema.safe_mode_error_rate = v;
                }
                Ok(v) => {
                    errors.push(SloSchemaError::InvalidThreshold {
                        field: "safe_mode_error_rate".into(),
                        value: v,
                    });
                }
                Err(e) => {
                    errors.push(SloSchemaError::ParseError {
                        field: "safe_mode_error_rate".into(),
                        reason: e.to_string(),
                    });
                }
            }
        } else if trimmed == "metrics:" {
            in_metrics = true;
        } else if in_metrics
            && trimmed.ends_with(':')
            && !trimmed.starts_with("max_")
            && !trimmed.starts_with("metric_type:")
            && !trimmed.starts_with("safe_mode")
        {
            // Flush previous metric
            if let Some(ref name) = current_metric {
                schema.metrics.insert(name.clone(), current_slo.clone());
            }
            let metric_name = trimmed.trim_end_matches(':').to_string();
            if !seen_metrics.insert(metric_name.clone()) {
                errors.push(SloSchemaError::DuplicateMetric(metric_name.clone()));
            }
            current_metric = Some(metric_name);
            current_slo = MetricSloSchema {
                metric_type: MetricType::Latency,
                max_value: None,
                max_ratio: None,
                safe_mode_trigger: false,
            };
        } else if let Some(value) = trimmed.strip_prefix("metric_type:") {
            let value = value.trim();
            match value {
                "latency" => current_slo.metric_type = MetricType::Latency,
                "memory" => current_slo.metric_type = MetricType::Memory,
                "error_rate" => current_slo.metric_type = MetricType::ErrorRate,
                other => {
                    errors.push(SloSchemaError::UnknownMetricType(other.to_string()));
                }
            }
        } else if let Some(value) = trimmed.strip_prefix("max_value:") {
            match value.trim().parse::<f64>() {
                Ok(v) => current_slo.max_value = Some(v),
                Err(e) => {
                    errors.push(SloSchemaError::ParseError {
                        field: "max_value".into(),
                        reason: e.to_string(),
                    });
                }
            }
        } else if let Some(value) = trimmed.strip_prefix("max_ratio:") {
            match value.trim().parse::<f64>() {
                Ok(v) => current_slo.max_ratio = Some(v),
                Err(e) => {
                    errors.push(SloSchemaError::ParseError {
                        field: "max_ratio".into(),
                        reason: e.to_string(),
                    });
                }
            }
        } else if let Some(value) = trimmed.strip_prefix("safe_mode_trigger:") {
            match value.trim() {
                "true" => current_slo.safe_mode_trigger = true,
                "false" => current_slo.safe_mode_trigger = false,
                other => {
                    errors.push(SloSchemaError::ParseError {
                        field: "safe_mode_trigger".into(),
                        reason: format!("expected 'true' or 'false', got '{other}'"),
                    });
                }
            }
        }
    }

    // Flush last metric
    if let Some(ref name) = current_metric {
        schema.metrics.insert(name.clone(), current_slo);
    }

    // Cross-field validation
    if schema.noise_tolerance >= schema.regression_threshold {
        errors.push(SloSchemaError::InvalidThreshold {
            field: "noise_tolerance".into(),
            value: schema.noise_tolerance,
        });
    }

    if errors.is_empty() {
        Ok(schema)
    } else {
        Err(errors)
    }
}

// ============================================================================
// Breach Detection Engine
// ============================================================================

/// Check a single metric observation against its SLO.
fn check_breach(
    metric_name: &str,
    baseline: f64,
    current: f64,
    schema: &SloSchema,
) -> BreachResult {
    let ratio = if baseline > 0.0 {
        current / baseline
    } else {
        1.0
    };

    let metric_slo = schema.metrics.get(metric_name);
    let metric_type = metric_slo
        .map(|s| s.metric_type.clone())
        .unwrap_or(MetricType::Latency);
    let safe_mode_trigger = metric_slo.map(|s| s.safe_mode_trigger).unwrap_or(false);

    // Check absolute threshold first
    if let Some(slo) = metric_slo {
        if let Some(max_val) = slo.max_value
            && current > max_val
        {
            return BreachResult {
                metric_name: metric_name.to_string(),
                metric_type,
                baseline,
                current,
                ratio,
                severity: BreachSeverity::AbsoluteBreach,
                safe_mode_trigger,
            };
        }
        if let Some(max_ratio) = slo.max_ratio
            && ratio > max_ratio
        {
            return BreachResult {
                metric_name: metric_name.to_string(),
                metric_type,
                baseline,
                current,
                ratio,
                severity: BreachSeverity::Breach,
                safe_mode_trigger,
            };
        }
    }

    // Global threshold check
    let change_pct = ratio - 1.0;
    let severity = if change_pct > schema.regression_threshold {
        BreachSeverity::Breach
    } else if change_pct > schema.noise_tolerance {
        BreachSeverity::Noise
    } else {
        BreachSeverity::None
    };

    BreachResult {
        metric_name: metric_name.to_string(),
        metric_type,
        baseline,
        current,
        ratio,
        severity,
        safe_mode_trigger,
    }
}

/// Evaluate safe-mode trigger from a batch of breach results.
fn check_safe_mode(breaches: &[BreachResult], schema: &SloSchema) -> SafeModeDecision {
    // Check for explicit safe-mode triggers
    for b in breaches {
        if b.safe_mode_trigger
            && (b.severity == BreachSeverity::Breach
                || b.severity == BreachSeverity::AbsoluteBreach)
        {
            return SafeModeDecision::Triggered(format!(
                "metric '{}' breached with safe_mode_trigger=true (ratio={:.3})",
                b.metric_name, b.ratio
            ));
        }
    }

    // Check error rate threshold
    for b in breaches {
        if b.metric_type == MetricType::ErrorRate && b.current > schema.safe_mode_error_rate {
            return SafeModeDecision::Triggered(format!(
                "error rate '{}' at {:.3} exceeds safe_mode_error_rate {:.3}",
                b.metric_name, b.current, schema.safe_mode_error_rate
            ));
        }
    }

    // Check simultaneous breach count
    let breach_count = breaches
        .iter()
        .filter(|b| {
            b.severity == BreachSeverity::Breach || b.severity == BreachSeverity::AbsoluteBreach
        })
        .count();

    if breach_count >= schema.safe_mode_breach_count {
        return SafeModeDecision::Triggered(format!(
            "{breach_count} simultaneous breaches (threshold: {})",
            schema.safe_mode_breach_count
        ));
    }

    SafeModeDecision::Normal
}

/// Emit a tracing span for SLO check and log appropriate events.
fn emit_slo_check(breach: &BreachResult, safe_mode: &SafeModeDecision) {
    let span = tracing::info_span!(
        "slo.check",
        metric_name = breach.metric_name.as_str(),
        metric_type = ?breach.metric_type,
        baseline = breach.baseline,
        current = breach.current,
        ratio = breach.ratio,
        severity = ?breach.severity,
    );
    let _guard = span.enter();

    match safe_mode {
        SafeModeDecision::Triggered(reason) => {
            tracing::error!(
                metric = breach.metric_name.as_str(),
                ratio = breach.ratio,
                reason = reason.as_str(),
                "safe-mode triggered"
            );
        }
        SafeModeDecision::Normal => match breach.severity {
            BreachSeverity::Breach | BreachSeverity::AbsoluteBreach => {
                tracing::warn!(
                    metric = breach.metric_name.as_str(),
                    baseline = breach.baseline,
                    current = breach.current,
                    ratio = breach.ratio,
                    severity = ?breach.severity,
                    "SLO breach detected"
                );
            }
            BreachSeverity::Noise => {
                tracing::debug!(
                    metric = breach.metric_name.as_str(),
                    ratio = breach.ratio,
                    "noise-level change within tolerance"
                );
            }
            BreachSeverity::None => {
                tracing::trace!(
                    metric = breach.metric_name.as_str(),
                    ratio = breach.ratio,
                    "metric within SLO"
                );
            }
        },
    }
}

// ============================================================================
// Tracing Capture Infrastructure
// ============================================================================

#[derive(Debug, Clone)]
#[allow(dead_code)]
struct CapturedSpan {
    name: String,
    level: tracing::Level,
    fields: HashMap<String, String>,
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
struct CapturedEvent {
    level: tracing::Level,
    message: Option<String>,
    fields: HashMap<String, String>,
    parent_span_name: Option<String>,
}

struct SloCapture {
    spans: Arc<Mutex<Vec<CapturedSpan>>>,
    events: Arc<Mutex<Vec<CapturedEvent>>>,
}

type SloCaptureInit = (
    SloCapture,
    Arc<Mutex<Vec<CapturedSpan>>>,
    Arc<Mutex<Vec<CapturedEvent>>>,
);

impl SloCapture {
    fn new() -> SloCaptureInit {
        let spans = Arc::new(Mutex::new(Vec::new()));
        let events = Arc::new(Mutex::new(Vec::new()));
        (
            Self {
                spans: spans.clone(),
                events: events.clone(),
            },
            spans,
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

impl<S> tracing_subscriber::Layer<S> for SloCapture
where
    S: tracing::Subscriber + for<'a> LookupSpan<'a>,
{
    fn on_new_span(
        &self,
        attrs: &tracing::span::Attributes<'_>,
        _id: &tracing::span::Id,
        _ctx: tracing_subscriber::layer::Context<'_, S>,
    ) {
        let mut visitor = FieldVisitor(Vec::new());
        attrs.record(&mut visitor);

        // Also capture fields declared via metadata (Empty fields)
        let mut fields: HashMap<String, String> = HashMap::new();
        for field in attrs.metadata().fields() {
            fields.entry(field.name().to_string()).or_default();
        }
        for (k, v) in visitor.0 {
            fields.insert(k, v);
        }

        self.spans.lock().unwrap().push(CapturedSpan {
            name: attrs.metadata().name().to_string(),
            level: *attrs.metadata().level(),
            fields,
        });
    }

    fn on_event(&self, event: &tracing::Event<'_>, ctx: tracing_subscriber::layer::Context<'_, S>) {
        let mut visitor = FieldVisitor(Vec::new());
        event.record(&mut visitor);

        let fields: HashMap<String, String> = visitor.0.into_iter().collect();
        let message = fields.get("message").cloned();

        let parent_span_name = ctx.event_span(event).map(|span| span.name().to_string());

        self.events.lock().unwrap().push(CapturedEvent {
            level: *event.metadata().level(),
            message,
            fields,
            parent_span_name,
        });
    }
}

fn with_slo_capture<F>(f: F) -> (Vec<CapturedSpan>, Vec<CapturedEvent>)
where
    F: FnOnce(),
{
    ensure_global_trace_level();
    let (layer, spans, events) = SloCapture::new();
    let subscriber = tracing_subscriber::registry().with(layer);
    tracing::subscriber::with_default(subscriber, || {
        tracing::callsite::rebuild_interest_cache();
        f()
    });
    let s = spans.lock().unwrap().clone();
    let e = events.lock().unwrap().clone();
    (s, e)
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
// Schema Validation Tests
// ============================================================================

#[test]
fn valid_schema_parses_successfully() {
    let yaml = r#"
regression_threshold: 0.10
noise_tolerance: 0.05
safe_mode_breach_count: 3
safe_mode_error_rate: 0.10
metrics:
  frame_pipeline_80x24:
    metric_type: latency
    max_value: 500.0
    max_ratio: 1.15
    safe_mode_trigger: false
  heap_usage:
    metric_type: memory
    max_value: 104857600.0
    max_ratio: 1.50
    safe_mode_trigger: true
  request_error_rate:
    metric_type: error_rate
    max_value: 0.05
    safe_mode_trigger: true
"#;

    let schema = validate_slo_yaml(yaml).expect("valid YAML should parse");

    assert!((schema.regression_threshold - 0.10).abs() < f64::EPSILON);
    assert!((schema.noise_tolerance - 0.05).abs() < f64::EPSILON);
    assert_eq!(schema.safe_mode_breach_count, 3);
    assert_eq!(schema.metrics.len(), 3);

    let frame = schema.metrics.get("frame_pipeline_80x24").unwrap();
    assert_eq!(frame.metric_type, MetricType::Latency);
    assert_eq!(frame.max_value, Some(500.0));
    assert_eq!(frame.max_ratio, Some(1.15));
    assert!(!frame.safe_mode_trigger);

    let heap = schema.metrics.get("heap_usage").unwrap();
    assert_eq!(heap.metric_type, MetricType::Memory);
    assert!(heap.safe_mode_trigger);

    let err_rate = schema.metrics.get("request_error_rate").unwrap();
    assert_eq!(err_rate.metric_type, MetricType::ErrorRate);
    assert!(err_rate.safe_mode_trigger);
}

#[test]
fn malformed_threshold_out_of_range_rejected() {
    let yaml = r#"
regression_threshold: 1.5
noise_tolerance: -0.1
"#;

    let errors = validate_slo_yaml(yaml).unwrap_err();

    assert!(
        errors.iter().any(|e| matches!(
            e,
            SloSchemaError::InvalidThreshold { field, value }
            if field == "regression_threshold" && (*value - 1.5).abs() < f64::EPSILON
        )),
        "Should reject regression_threshold > 1.0: {errors:?}"
    );
    assert!(
        errors.iter().any(|e| matches!(
            e,
            SloSchemaError::InvalidThreshold { field, .. }
            if field == "noise_tolerance"
        )),
        "Should reject negative noise_tolerance: {errors:?}"
    );
}

#[test]
fn malformed_non_numeric_threshold_rejected() {
    let yaml = r#"
regression_threshold: abc
noise_tolerance: 0.05
"#;

    let errors = validate_slo_yaml(yaml).unwrap_err();

    assert!(
        errors.iter().any(|e| matches!(
            e,
            SloSchemaError::ParseError { field, .. }
            if field == "regression_threshold"
        )),
        "Should reject non-numeric regression_threshold: {errors:?}"
    );
}

#[test]
fn malformed_unknown_metric_type_rejected() {
    let yaml = r#"
regression_threshold: 0.10
noise_tolerance: 0.05
metrics:
  bad_metric:
    metric_type: throughput
    max_value: 100.0
"#;

    let errors = validate_slo_yaml(yaml).unwrap_err();

    assert!(
        errors.iter().any(|e| matches!(
            e,
            SloSchemaError::UnknownMetricType(t) if t == "throughput"
        )),
        "Should reject unknown metric type: {errors:?}"
    );
}

#[test]
fn malformed_duplicate_metric_rejected() {
    let yaml = r#"
regression_threshold: 0.10
noise_tolerance: 0.05
metrics:
  frame_pipeline:
    metric_type: latency
    max_value: 500.0
  frame_pipeline:
    metric_type: latency
    max_value: 600.0
"#;

    let errors = validate_slo_yaml(yaml).unwrap_err();

    assert!(
        errors.iter().any(|e| matches!(
            e,
            SloSchemaError::DuplicateMetric(name) if name == "frame_pipeline"
        )),
        "Should reject duplicate metric: {errors:?}"
    );
}

#[test]
fn malformed_tabs_rejected() {
    let yaml = "regression_threshold:\t0.10\nnoise_tolerance: 0.05\n";

    let errors = validate_slo_yaml(yaml).unwrap_err();

    assert!(
        errors.iter().any(|e| matches!(
            e,
            SloSchemaError::MalformedStructure(msg) if msg.contains("tabs")
        )),
        "Should reject tabs: {errors:?}"
    );
}

#[test]
fn noise_tolerance_gte_regression_threshold_rejected() {
    let yaml = r#"
regression_threshold: 0.05
noise_tolerance: 0.10
"#;

    let errors = validate_slo_yaml(yaml).unwrap_err();

    assert!(
        errors.iter().any(|e| matches!(
            e,
            SloSchemaError::InvalidThreshold { field, .. }
            if field == "noise_tolerance"
        )),
        "Should reject noise_tolerance >= regression_threshold: {errors:?}"
    );
}

#[test]
fn empty_yaml_uses_defaults() {
    let schema = validate_slo_yaml("").expect("empty should use defaults");

    assert!((schema.regression_threshold - 0.10).abs() < f64::EPSILON);
    assert!((schema.noise_tolerance - 0.05).abs() < f64::EPSILON);
    assert_eq!(schema.safe_mode_breach_count, 3);
    assert!(schema.metrics.is_empty());
}

#[test]
fn comments_and_blanks_are_ignored() {
    let yaml = r#"
# This is a comment
regression_threshold: 0.12

# Another comment
noise_tolerance: 0.03

# Metrics section
metrics:
  # A metric
  test_metric:
    metric_type: latency
    max_value: 200.0
"#;

    let schema = validate_slo_yaml(yaml).expect("comments should be ignored");

    assert!((schema.regression_threshold - 0.12).abs() < f64::EPSILON);
    assert!((schema.noise_tolerance - 0.03).abs() < f64::EPSILON);
    assert_eq!(schema.metrics.len(), 1);
}

#[test]
fn malformed_safe_mode_trigger_value_rejected() {
    let yaml = r#"
regression_threshold: 0.10
noise_tolerance: 0.05
metrics:
  bad_trigger:
    metric_type: latency
    safe_mode_trigger: yes
"#;

    let errors = validate_slo_yaml(yaml).unwrap_err();

    assert!(
        errors.iter().any(|e| matches!(
            e,
            SloSchemaError::ParseError { field, reason }
            if field == "safe_mode_trigger" && reason.contains("yes")
        )),
        "Should reject 'yes' for safe_mode_trigger: {errors:?}"
    );
}

// ============================================================================
// Breach Detection Tests: Latency
// ============================================================================

#[test]
fn latency_breach_detected_above_threshold() {
    let schema = SloSchema {
        regression_threshold: 0.10,
        noise_tolerance: 0.05,
        metrics: {
            let mut m = HashMap::new();
            m.insert(
                "render_p99".into(),
                MetricSloSchema {
                    metric_type: MetricType::Latency,
                    max_value: Some(500.0),
                    max_ratio: Some(1.15),
                    safe_mode_trigger: false,
                },
            );
            m
        },
        ..SloSchema::default()
    };

    // Absolute breach: 520us > 500us max
    let result = check_breach("render_p99", 400.0, 520.0, &schema);
    assert_eq!(result.severity, BreachSeverity::AbsoluteBreach);
    assert_eq!(result.metric_type, MetricType::Latency);
}

#[test]
fn latency_ratio_breach_below_absolute() {
    let schema = SloSchema {
        metrics: {
            let mut m = HashMap::new();
            m.insert(
                "render_p99".into(),
                MetricSloSchema {
                    metric_type: MetricType::Latency,
                    max_value: Some(1000.0),
                    max_ratio: Some(1.10),
                    safe_mode_trigger: false,
                },
            );
            m
        },
        ..SloSchema::default()
    };

    // Ratio breach: 1.20 > 1.10 max_ratio, but 480 < 1000 absolute
    let result = check_breach("render_p99", 400.0, 480.0, &schema);
    assert_eq!(result.severity, BreachSeverity::Breach);
}

#[test]
fn latency_within_slo_no_breach() {
    let schema = SloSchema {
        metrics: {
            let mut m = HashMap::new();
            m.insert(
                "render_p99".into(),
                MetricSloSchema {
                    metric_type: MetricType::Latency,
                    max_value: Some(500.0),
                    max_ratio: Some(1.15),
                    safe_mode_trigger: false,
                },
            );
            m
        },
        ..SloSchema::default()
    };

    // Within bounds: 404 < 500, ratio 1.01 < 1.15, change 1% < 5% noise
    let result = check_breach("render_p99", 400.0, 404.0, &schema);
    assert_eq!(result.severity, BreachSeverity::None);
}

// ============================================================================
// Breach Detection Tests: Memory
// ============================================================================

#[test]
fn memory_absolute_breach_detected() {
    let schema = SloSchema {
        metrics: {
            let mut m = HashMap::new();
            m.insert(
                "heap_bytes".into(),
                MetricSloSchema {
                    metric_type: MetricType::Memory,
                    max_value: Some(100_000_000.0), // 100MB
                    max_ratio: None,
                    safe_mode_trigger: true,
                },
            );
            m
        },
        ..SloSchema::default()
    };

    let result = check_breach("heap_bytes", 80_000_000.0, 120_000_000.0, &schema);
    assert_eq!(result.severity, BreachSeverity::AbsoluteBreach);
    assert_eq!(result.metric_type, MetricType::Memory);
    assert!(result.safe_mode_trigger);
}

#[test]
fn memory_within_limit_no_breach() {
    let schema = SloSchema {
        metrics: {
            let mut m = HashMap::new();
            m.insert(
                "heap_bytes".into(),
                MetricSloSchema {
                    metric_type: MetricType::Memory,
                    max_value: Some(100_000_000.0),
                    max_ratio: None,
                    safe_mode_trigger: false,
                },
            );
            m
        },
        ..SloSchema::default()
    };

    // 82M < 100M limit, ratio 1.025, change 2.5% < 5% noise tolerance
    let result = check_breach("heap_bytes", 80_000_000.0, 82_000_000.0, &schema);
    assert_eq!(result.severity, BreachSeverity::None);
}

#[test]
fn memory_ratio_breach_detected() {
    let schema = SloSchema {
        metrics: {
            let mut m = HashMap::new();
            m.insert(
                "alloc_count".into(),
                MetricSloSchema {
                    metric_type: MetricType::Memory,
                    max_value: None,
                    max_ratio: Some(1.50),
                    safe_mode_trigger: false,
                },
            );
            m
        },
        ..SloSchema::default()
    };

    // 2x increase: ratio 2.0 > 1.5
    let result = check_breach("alloc_count", 1000.0, 2000.0, &schema);
    assert_eq!(result.severity, BreachSeverity::Breach);
    assert_eq!(result.metric_type, MetricType::Memory);
}

// ============================================================================
// Breach Detection Tests: Error Rate
// ============================================================================

#[test]
fn error_rate_absolute_breach_detected() {
    let schema = SloSchema {
        metrics: {
            let mut m = HashMap::new();
            m.insert(
                "frame_error_rate".into(),
                MetricSloSchema {
                    metric_type: MetricType::ErrorRate,
                    max_value: Some(0.05), // 5% max error rate
                    max_ratio: None,
                    safe_mode_trigger: true,
                },
            );
            m
        },
        ..SloSchema::default()
    };

    let result = check_breach("frame_error_rate", 0.01, 0.08, &schema);
    assert_eq!(result.severity, BreachSeverity::AbsoluteBreach);
    assert_eq!(result.metric_type, MetricType::ErrorRate);
    assert!(result.safe_mode_trigger);
}

#[test]
fn error_rate_within_slo_no_breach() {
    let schema = SloSchema {
        metrics: {
            let mut m = HashMap::new();
            m.insert(
                "frame_error_rate".into(),
                MetricSloSchema {
                    metric_type: MetricType::ErrorRate,
                    max_value: Some(0.05),
                    max_ratio: None,
                    safe_mode_trigger: false,
                },
            );
            m
        },
        ..SloSchema::default()
    };

    // 0.011 < 0.05 max, ratio 1.01/0.01 = 1.1 → change 10% at boundary
    // Use values with smaller ratio: 0.040 vs 0.041, ratio 1.025
    let result = check_breach("frame_error_rate", 0.040, 0.041, &schema);
    assert_eq!(result.severity, BreachSeverity::None);
}

#[test]
fn error_rate_global_threshold_fallback() {
    let schema = SloSchema::default();

    // Metric not in config, uses global 10% regression threshold
    // Ratio 0.08/0.01 = 8.0, change = 7.0 = 700% — well above 10%
    let result = check_breach("unknown_error", 0.01, 0.08, &schema);
    assert_eq!(result.severity, BreachSeverity::Breach);
}

// ============================================================================
// Safe-Mode Trigger Logic Tests
// ============================================================================

#[test]
fn safe_mode_triggered_by_explicit_trigger_flag() {
    let schema = SloSchema::default();

    let breaches = vec![BreachResult {
        metric_name: "critical_latency".into(),
        metric_type: MetricType::Latency,
        baseline: 200.0,
        current: 600.0,
        ratio: 3.0,
        severity: BreachSeverity::Breach,
        safe_mode_trigger: true,
    }];

    let decision = check_safe_mode(&breaches, &schema);
    assert!(
        matches!(decision, SafeModeDecision::Triggered(ref reason) if reason.contains("safe_mode_trigger=true")),
        "Should trigger safe-mode for flagged metric: {decision:?}"
    );
}

#[test]
fn safe_mode_triggered_by_error_rate_threshold() {
    let schema = SloSchema {
        safe_mode_error_rate: 0.10,
        ..SloSchema::default()
    };

    let breaches = vec![BreachResult {
        metric_name: "api_errors".into(),
        metric_type: MetricType::ErrorRate,
        baseline: 0.02,
        current: 0.15, // 15% > 10% safe_mode_error_rate
        ratio: 7.5,
        severity: BreachSeverity::Breach,
        safe_mode_trigger: false,
    }];

    let decision = check_safe_mode(&breaches, &schema);
    assert!(
        matches!(decision, SafeModeDecision::Triggered(ref reason) if reason.contains("error rate")),
        "Should trigger safe-mode for high error rate: {decision:?}"
    );
}

#[test]
fn safe_mode_triggered_by_breach_count() {
    let schema = SloSchema {
        safe_mode_breach_count: 2,
        ..SloSchema::default()
    };

    let breaches = vec![
        BreachResult {
            metric_name: "metric_a".into(),
            metric_type: MetricType::Latency,
            baseline: 100.0,
            current: 200.0,
            ratio: 2.0,
            severity: BreachSeverity::Breach,
            safe_mode_trigger: false,
        },
        BreachResult {
            metric_name: "metric_b".into(),
            metric_type: MetricType::Memory,
            baseline: 1000.0,
            current: 3000.0,
            ratio: 3.0,
            severity: BreachSeverity::AbsoluteBreach,
            safe_mode_trigger: false,
        },
    ];

    let decision = check_safe_mode(&breaches, &schema);
    assert!(
        matches!(decision, SafeModeDecision::Triggered(ref reason) if reason.contains("simultaneous breaches")),
        "Should trigger safe-mode for multiple breaches: {decision:?}"
    );
}

#[test]
fn safe_mode_not_triggered_below_thresholds() {
    let schema = SloSchema::default();

    let breaches = vec![
        BreachResult {
            metric_name: "metric_a".into(),
            metric_type: MetricType::Latency,
            baseline: 100.0,
            current: 115.0,
            ratio: 1.15,
            severity: BreachSeverity::Breach,
            safe_mode_trigger: false,
        },
        BreachResult {
            metric_name: "metric_b".into(),
            metric_type: MetricType::Latency,
            baseline: 200.0,
            current: 210.0,
            ratio: 1.05,
            severity: BreachSeverity::None,
            safe_mode_trigger: false,
        },
    ];

    let decision = check_safe_mode(&breaches, &schema);
    assert_eq!(
        decision,
        SafeModeDecision::Normal,
        "1 breach < 3 threshold, should be Normal"
    );
}

#[test]
fn safe_mode_not_triggered_for_noise_only() {
    let schema = SloSchema {
        safe_mode_breach_count: 1,
        ..SloSchema::default()
    };

    let breaches = vec![BreachResult {
        metric_name: "minor".into(),
        metric_type: MetricType::Latency,
        baseline: 100.0,
        current: 107.0,
        ratio: 1.07,
        severity: BreachSeverity::Noise,
        safe_mode_trigger: false,
    }];

    let decision = check_safe_mode(&breaches, &schema);
    assert_eq!(
        decision,
        SafeModeDecision::Normal,
        "Noise-level changes should not trigger safe-mode"
    );
}

#[test]
fn safe_mode_trigger_flag_only_fires_on_actual_breach() {
    let schema = SloSchema::default();

    // safe_mode_trigger=true but severity is Noise (not a real breach)
    let breaches = vec![BreachResult {
        metric_name: "flagged_but_ok".into(),
        metric_type: MetricType::Latency,
        baseline: 100.0,
        current: 107.0,
        ratio: 1.07,
        severity: BreachSeverity::Noise,
        safe_mode_trigger: true,
    }];

    let decision = check_safe_mode(&breaches, &schema);
    assert_eq!(
        decision,
        SafeModeDecision::Normal,
        "safe_mode_trigger flag should only activate on Breach/AbsoluteBreach, not Noise"
    );
}

// ============================================================================
// slo.check Span Tests
// ============================================================================

#[test]
fn slo_check_span_emitted_with_correct_fields() {
    let breach = BreachResult {
        metric_name: "render_p99".into(),
        metric_type: MetricType::Latency,
        baseline: 200.0,
        current: 250.0,
        ratio: 1.25,
        severity: BreachSeverity::Breach,
        safe_mode_trigger: false,
    };

    let (spans, _events) = with_slo_capture(|| {
        emit_slo_check(&breach, &SafeModeDecision::Normal);
    });

    let slo_spans: Vec<_> = spans.iter().filter(|s| s.name == "slo.check").collect();
    assert_eq!(slo_spans.len(), 1, "Should emit exactly 1 slo.check span");

    let span = &slo_spans[0];
    assert!(
        span.fields.contains_key("metric_name"),
        "slo.check span must have metric_name field"
    );
    assert!(
        span.fields.contains_key("metric_type"),
        "slo.check span must have metric_type field"
    );
    assert!(
        span.fields.contains_key("baseline"),
        "slo.check span must have baseline field"
    );
    assert!(
        span.fields.contains_key("current"),
        "slo.check span must have current field"
    );
    assert!(
        span.fields.contains_key("ratio"),
        "slo.check span must have ratio field"
    );
    assert!(
        span.fields.contains_key("severity"),
        "slo.check span must have severity field"
    );
}

#[test]
fn slo_check_span_field_values_match() {
    let breach = BreachResult {
        metric_name: "heap_bytes".into(),
        metric_type: MetricType::Memory,
        baseline: 50_000_000.0,
        current: 120_000_000.0,
        ratio: 2.4,
        severity: BreachSeverity::AbsoluteBreach,
        safe_mode_trigger: true,
    };

    let (spans, _events) = with_slo_capture(|| {
        emit_slo_check(&breach, &SafeModeDecision::Normal);
    });

    let span = spans
        .iter()
        .find(|s| s.name == "slo.check")
        .expect("should have slo.check span");

    assert_eq!(
        span.fields.get("metric_name").map(|s| s.as_str()),
        Some("heap_bytes"),
        "metric_name should match"
    );
    assert!(
        span.fields
            .get("metric_type")
            .map(|s| s.contains("Memory"))
            .unwrap_or(false),
        "metric_type should contain Memory"
    );
}

#[test]
fn warn_emitted_for_breach_inside_slo_check_span() {
    let breach = BreachResult {
        metric_name: "render_p99".into(),
        metric_type: MetricType::Latency,
        baseline: 200.0,
        current: 300.0,
        ratio: 1.5,
        severity: BreachSeverity::Breach,
        safe_mode_trigger: false,
    };

    let (_spans, events) = with_slo_capture(|| {
        emit_slo_check(&breach, &SafeModeDecision::Normal);
    });

    let warn_events: Vec<_> = events
        .iter()
        .filter(|e| e.level == tracing::Level::WARN)
        .collect();

    assert_eq!(warn_events.len(), 1, "Should emit 1 WARN for breach");

    let warn = &warn_events[0];
    assert!(
        warn.message.as_deref().unwrap_or("").contains("SLO breach"),
        "WARN message should mention SLO breach"
    );
    assert_eq!(
        warn.parent_span_name.as_deref(),
        Some("slo.check"),
        "WARN should be inside slo.check span"
    );
    assert!(
        warn.fields.contains_key("metric"),
        "WARN should have 'metric' field"
    );
    assert!(
        warn.fields.contains_key("ratio"),
        "WARN should have 'ratio' field"
    );
}

#[test]
fn error_emitted_for_safe_mode_inside_slo_check_span() {
    let breach = BreachResult {
        metric_name: "critical_path".into(),
        metric_type: MetricType::Latency,
        baseline: 100.0,
        current: 500.0,
        ratio: 5.0,
        severity: BreachSeverity::AbsoluteBreach,
        safe_mode_trigger: true,
    };

    let safe_mode =
        SafeModeDecision::Triggered("critical_path breached with safe_mode_trigger=true".into());

    let (_spans, events) = with_slo_capture(|| {
        emit_slo_check(&breach, &safe_mode);
    });

    let error_events: Vec<_> = events
        .iter()
        .filter(|e| e.level == tracing::Level::ERROR)
        .collect();

    assert_eq!(
        error_events.len(),
        1,
        "Should emit 1 ERROR for safe-mode trigger"
    );

    let error = &error_events[0];
    assert!(
        error.message.as_deref().unwrap_or("").contains("safe-mode"),
        "ERROR message should mention safe-mode"
    );
    assert_eq!(
        error.parent_span_name.as_deref(),
        Some("slo.check"),
        "ERROR should be inside slo.check span"
    );
    assert!(
        error.fields.contains_key("reason"),
        "ERROR should have 'reason' field"
    );
}

#[test]
fn no_warn_or_error_for_noise_in_slo_check() {
    let breach = BreachResult {
        metric_name: "minor_metric".into(),
        metric_type: MetricType::Latency,
        baseline: 100.0,
        current: 107.0,
        ratio: 1.07,
        severity: BreachSeverity::Noise,
        safe_mode_trigger: false,
    };

    let (_spans, events) = with_slo_capture(|| {
        emit_slo_check(&breach, &SafeModeDecision::Normal);
    });

    let warn_or_error: Vec<_> = events
        .iter()
        .filter(|e| e.level == tracing::Level::WARN || e.level == tracing::Level::ERROR)
        .collect();

    assert!(
        warn_or_error.is_empty(),
        "Noise-level change should not emit WARN or ERROR"
    );

    let debug_events: Vec<_> = events
        .iter()
        .filter(|e| e.level == tracing::Level::DEBUG)
        .collect();
    assert_eq!(debug_events.len(), 1, "Should emit DEBUG for noise");
}

#[test]
fn no_warn_or_error_for_none_severity() {
    let breach = BreachResult {
        metric_name: "stable_metric".into(),
        metric_type: MetricType::Latency,
        baseline: 100.0,
        current: 100.0,
        ratio: 1.0,
        severity: BreachSeverity::None,
        safe_mode_trigger: false,
    };

    let (_spans, events) = with_slo_capture(|| {
        emit_slo_check(&breach, &SafeModeDecision::Normal);
    });

    let warn_or_error: Vec<_> = events
        .iter()
        .filter(|e| e.level == tracing::Level::WARN || e.level == tracing::Level::ERROR)
        .collect();

    assert!(
        warn_or_error.is_empty(),
        "Stable metric should not emit WARN or ERROR"
    );

    let trace_events: Vec<_> = events
        .iter()
        .filter(|e| e.level == tracing::Level::TRACE)
        .collect();
    assert_eq!(trace_events.len(), 1, "Should emit TRACE for stable metric");
}

// ============================================================================
// End-to-End Batch Check with Tracing
// ============================================================================

#[test]
fn batch_slo_check_mixed_severities() {
    let yaml = r#"
regression_threshold: 0.10
noise_tolerance: 0.05
safe_mode_breach_count: 5
metrics:
  fast_render:
    metric_type: latency
    max_value: 500.0
    max_ratio: 1.15
    safe_mode_trigger: false
  heap_usage:
    metric_type: memory
    max_value: 100000000.0
    safe_mode_trigger: false
  error_rate:
    metric_type: error_rate
    max_value: 0.05
    safe_mode_trigger: true
"#;

    let schema = validate_slo_yaml(yaml).expect("valid yaml");

    let checks = vec![
        ("fast_render", 300.0, 310.0), // 3.3% — None
        ("heap_usage", 80e6, 82e6),    // 2.5% — None (within memory limit and noise)
        ("error_rate", 0.040, 0.041),  // 2.5% ratio, 0.041 < 0.05 — None
    ];

    let mut breaches = Vec::new();
    for (name, baseline, current) in &checks {
        breaches.push(check_breach(name, *baseline, *current, &schema));
    }

    let safe_mode = check_safe_mode(&breaches, &schema);
    assert_eq!(safe_mode, SafeModeDecision::Normal);

    let (spans, events) = with_slo_capture(|| {
        for b in &breaches {
            emit_slo_check(b, &safe_mode);
        }
    });

    let slo_spans: Vec<_> = spans.iter().filter(|s| s.name == "slo.check").collect();
    assert_eq!(slo_spans.len(), 3, "Should emit 3 slo.check spans");

    // No WARN or ERROR for this batch (all within tolerance)
    let warn_or_error: Vec<_> = events
        .iter()
        .filter(|e| e.level == tracing::Level::WARN || e.level == tracing::Level::ERROR)
        .collect();
    assert!(
        warn_or_error.is_empty(),
        "No WARN/ERROR for this batch: {warn_or_error:?}"
    );
}

#[test]
fn batch_slo_check_with_safe_mode_trigger() {
    let yaml = r#"
regression_threshold: 0.10
noise_tolerance: 0.05
safe_mode_breach_count: 5
safe_mode_error_rate: 0.10
metrics:
  fast_render:
    metric_type: latency
    max_value: 500.0
    safe_mode_trigger: false
  critical_error_rate:
    metric_type: error_rate
    max_value: 0.05
    safe_mode_trigger: true
"#;

    let schema = validate_slo_yaml(yaml).expect("valid yaml");

    let checks = vec![
        ("fast_render", 300.0, 550.0),       // AbsoluteBreach (550 > 500)
        ("critical_error_rate", 0.01, 0.08), // AbsoluteBreach (0.08 > 0.05) + safe_mode_trigger
    ];

    let mut breaches = Vec::new();
    for (name, baseline, current) in &checks {
        breaches.push(check_breach(name, *baseline, *current, &schema));
    }

    let safe_mode = check_safe_mode(&breaches, &schema);
    assert!(
        matches!(safe_mode, SafeModeDecision::Triggered(_)),
        "Should trigger safe-mode"
    );

    let (_spans, events) = with_slo_capture(|| {
        for b in &breaches {
            emit_slo_check(b, &safe_mode);
        }
    });

    // Both metrics should emit ERROR because safe_mode is triggered
    let error_events: Vec<_> = events
        .iter()
        .filter(|e| e.level == tracing::Level::ERROR)
        .collect();
    assert_eq!(
        error_events.len(),
        2,
        "Both metrics should emit ERROR when safe-mode is triggered"
    );

    for err in &error_events {
        assert!(
            err.message.as_deref().unwrap_or("").contains("safe-mode"),
            "ERROR message should mention safe-mode"
        );
    }
}

#[test]
fn zero_baseline_no_panic_in_breach_check() {
    let schema = SloSchema::default();

    let result = check_breach("zero_metric", 0.0, 5.0, &schema);

    // Should not panic; ratio defaults to 1.0 when baseline is 0
    assert!((result.ratio - 1.0).abs() < f64::EPSILON);
    assert_eq!(result.severity, BreachSeverity::None);
}

#[test]
fn improvement_not_flagged_as_breach() {
    let schema = SloSchema::default();

    let result = check_breach("improving", 200.0, 150.0, &schema);

    assert_eq!(result.severity, BreachSeverity::None);
    assert!(result.ratio < 1.0);
}
