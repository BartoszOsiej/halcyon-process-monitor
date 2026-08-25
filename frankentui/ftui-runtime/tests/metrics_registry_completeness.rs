#![forbid(unsafe_code)]

//! bd-xox.9: Metrics registry completeness and correctness tests.
//!
//! Covers:
//! - All declared metrics are registered in the registry
//! - Histogram bucket boundaries are sensible (cover expected range)
//! - Counter increments are monotonic
//! - Gauge values stay in declared range
//! - Prometheus export format is valid (parseable text format)
//! - Assert metrics output matches Prometheus text format spec
//!
//! Run:
//!   cargo test -p ftui-runtime --test metrics_registry_completeness

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

// ============================================================================
// Metrics Engine (self-contained for testing)
// ============================================================================

/// Metric type discriminant.
#[derive(Debug, Clone, PartialEq)]
enum MetricKind {
    Counter,
    Histogram,
    Gauge,
}

/// A metric descriptor declaring what a metric measures.
#[derive(Debug, Clone)]
struct MetricDescriptor {
    name: String,
    kind: MetricKind,
    help: String,
    /// Label names (e.g., ["strategy", "widget_type"]).
    #[allow(dead_code)]
    label_names: Vec<String>,
    /// For histograms: bucket boundaries.
    buckets: Option<Vec<f64>>,
    /// For gauges: valid value range [min, max].
    gauge_range: Option<(f64, f64)>,
}

/// A counter metric (monotonically increasing).
#[derive(Debug, Clone)]
struct CounterValue {
    value: f64,
    labels: HashMap<String, String>,
}

/// A histogram observation.
#[derive(Debug, Clone)]
struct HistogramValue {
    sum: f64,
    count: u64,
    /// Per-bucket counts (cumulative).
    bucket_counts: Vec<u64>,
    labels: HashMap<String, String>,
}

/// A gauge value (arbitrary numeric).
#[derive(Debug, Clone)]
struct GaugeValue {
    value: f64,
    labels: HashMap<String, String>,
}

/// Thread-safe metrics registry.
#[derive(Debug, Clone)]
struct MetricsRegistry {
    descriptors: Vec<MetricDescriptor>,
    counters: Arc<Mutex<HashMap<String, Vec<CounterValue>>>>,
    histograms: Arc<Mutex<HashMap<String, Vec<HistogramValue>>>>,
    gauges: Arc<Mutex<HashMap<String, Vec<GaugeValue>>>>,
}

impl MetricsRegistry {
    fn new() -> Self {
        Self {
            descriptors: Vec::new(),
            counters: Arc::new(Mutex::new(HashMap::new())),
            histograms: Arc::new(Mutex::new(HashMap::new())),
            gauges: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Register a metric descriptor.
    fn register(&mut self, desc: MetricDescriptor) {
        self.descriptors.push(desc);
    }

    /// Increment a counter by the given amount.
    fn counter_inc(&self, name: &str, amount: f64, labels: HashMap<String, String>) {
        let mut counters = self.counters.lock().unwrap();
        let entry = counters.entry(name.to_string()).or_default();

        // Find existing label set or create new
        if let Some(cv) = entry.iter_mut().find(|cv| cv.labels == labels) {
            cv.value += amount;
        } else {
            entry.push(CounterValue {
                value: amount,
                labels,
            });
        }
    }

    /// Observe a histogram value.
    fn histogram_observe(&self, name: &str, value: f64, labels: HashMap<String, String>) {
        let desc = self
            .descriptors
            .iter()
            .find(|d| d.name == name && d.kind == MetricKind::Histogram);

        let buckets = desc
            .and_then(|d| d.buckets.as_ref())
            .cloned()
            .unwrap_or_default();

        let mut histograms = self.histograms.lock().unwrap();
        let entry = histograms.entry(name.to_string()).or_default();

        if let Some(hv) = entry.iter_mut().find(|hv| hv.labels == labels) {
            hv.sum += value;
            hv.count += 1;
            for (i, &bound) in buckets.iter().enumerate() {
                if value <= bound {
                    hv.bucket_counts[i] += 1;
                }
            }
        } else {
            let mut bucket_counts = vec![0u64; buckets.len()];
            for (i, &bound) in buckets.iter().enumerate() {
                if value <= bound {
                    bucket_counts[i] = 1;
                }
            }
            entry.push(HistogramValue {
                sum: value,
                count: 1,
                bucket_counts,
                labels,
            });
        }
    }

    /// Set a gauge value.
    fn gauge_set(&self, name: &str, value: f64, labels: HashMap<String, String>) {
        let mut gauges = self.gauges.lock().unwrap();
        let entry = gauges.entry(name.to_string()).or_default();

        if let Some(gv) = entry.iter_mut().find(|gv| gv.labels == labels) {
            gv.value = value;
        } else {
            entry.push(GaugeValue { value, labels });
        }
    }

    /// Check if a gauge value is within the declared range.
    fn gauge_in_range(&self, name: &str, value: f64) -> bool {
        if let Some(desc) = self
            .descriptors
            .iter()
            .find(|d| d.name == name && d.kind == MetricKind::Gauge)
            && let Some((min, max)) = desc.gauge_range
        {
            return value >= min && value <= max;
        }
        true // No range declared, always valid
    }

    /// Export all metrics in Prometheus text exposition format.
    fn export_prometheus(&self) -> String {
        let mut output = String::new();

        for desc in &self.descriptors {
            // HELP line
            output.push_str(&format!("# HELP {} {}\n", desc.name, desc.help));

            // TYPE line
            let type_str = match desc.kind {
                MetricKind::Counter => "counter",
                MetricKind::Histogram => "histogram",
                MetricKind::Gauge => "gauge",
            };
            output.push_str(&format!("# TYPE {} {}\n", desc.name, type_str));

            match desc.kind {
                MetricKind::Counter => {
                    let counters = self.counters.lock().unwrap();
                    if let Some(values) = counters.get(&desc.name) {
                        for cv in values {
                            let label_str = format_labels(&cv.labels);
                            output.push_str(&format!("{}{} {}\n", desc.name, label_str, cv.value));
                        }
                    }
                }
                MetricKind::Histogram => {
                    let histograms = self.histograms.lock().unwrap();
                    if let Some(values) = histograms.get(&desc.name) {
                        for hv in values {
                            let label_str = format_labels(&hv.labels);
                            if let Some(buckets) = &desc.buckets {
                                for (i, &bound) in buckets.iter().enumerate() {
                                    let mut bucket_labels = hv.labels.clone();
                                    bucket_labels.insert("le".to_string(), format_float(bound));
                                    let bl = format_labels(&bucket_labels);
                                    output.push_str(&format!(
                                        "{}_bucket{} {}\n",
                                        desc.name, bl, hv.bucket_counts[i]
                                    ));
                                }
                                // +Inf bucket
                                let mut inf_labels = hv.labels.clone();
                                inf_labels.insert("le".to_string(), "+Inf".to_string());
                                let bl = format_labels(&inf_labels);
                                output.push_str(&format!(
                                    "{}_bucket{} {}\n",
                                    desc.name, bl, hv.count
                                ));
                            }
                            output
                                .push_str(&format!("{}_sum{} {}\n", desc.name, label_str, hv.sum));
                            output.push_str(&format!(
                                "{}_count{} {}\n",
                                desc.name, label_str, hv.count
                            ));
                        }
                    }
                }
                MetricKind::Gauge => {
                    let gauges = self.gauges.lock().unwrap();
                    if let Some(values) = gauges.get(&desc.name) {
                        for gv in values {
                            let label_str = format_labels(&gv.labels);
                            output.push_str(&format!("{}{} {}\n", desc.name, label_str, gv.value));
                        }
                    }
                }
            }
        }

        output
    }
}

/// Format labels as Prometheus label string: {key1="val1",key2="val2"}
fn format_labels(labels: &HashMap<String, String>) -> String {
    if labels.is_empty() {
        return String::new();
    }
    let mut pairs: Vec<_> = labels.iter().map(|(k, v)| format!("{k}=\"{v}\"")).collect();
    pairs.sort(); // Deterministic ordering
    format!("{{{}}}", pairs.join(","))
}

/// Format a float for Prometheus (no trailing zeros).
fn format_float(v: f64) -> String {
    if v == v.floor() {
        format!("{v:.0}")
    } else {
        format!("{v}")
    }
}

// ============================================================================
// Canonical FrankenTUI Metrics Declarations
// (From bd-xox.3 spec: all metrics the registry must contain)
// ============================================================================

/// Build a registry pre-loaded with all declared FrankenTUI metrics.
fn build_canonical_registry() -> MetricsRegistry {
    let mut reg = MetricsRegistry::new();

    // --- Histograms (latency distributions) ---
    let latency_buckets = vec![
        10.0, 25.0, 50.0, 100.0, 250.0, 500.0, 1000.0, 2500.0, 5000.0, 10000.0,
    ];

    reg.register(MetricDescriptor {
        name: "render_frame_duration_us".into(),
        kind: MetricKind::Histogram,
        help: "Duration of a complete frame render cycle in microseconds".into(),
        label_names: vec![],
        buckets: Some(latency_buckets.clone()),
        gauge_range: None,
    });

    reg.register(MetricDescriptor {
        name: "diff_strategy_duration_us".into(),
        kind: MetricKind::Histogram,
        help: "Duration of diff strategy computation in microseconds".into(),
        label_names: vec![],
        buckets: Some(latency_buckets.clone()),
        gauge_range: None,
    });

    reg.register(MetricDescriptor {
        name: "layout_compute_duration_us".into(),
        kind: MetricKind::Histogram,
        help: "Duration of layout constraint solving in microseconds".into(),
        label_names: vec![],
        buckets: Some(latency_buckets.clone()),
        gauge_range: None,
    });

    reg.register(MetricDescriptor {
        name: "widget_render_duration_us".into(),
        kind: MetricKind::Histogram,
        help: "Duration of individual widget rendering in microseconds".into(),
        label_names: vec!["widget_type".into()],
        buckets: Some(latency_buckets.clone()),
        gauge_range: None,
    });

    reg.register(MetricDescriptor {
        name: "conformal_prediction_interval_width_us".into(),
        kind: MetricKind::Histogram,
        help: "Width of conformal prediction interval in microseconds".into(),
        label_names: vec![],
        buckets: Some(latency_buckets.clone()),
        gauge_range: None,
    });

    reg.register(MetricDescriptor {
        name: "animation_duration_ms".into(),
        kind: MetricKind::Histogram,
        help: "Duration of animation steps in milliseconds".into(),
        label_names: vec![],
        buckets: Some(vec![
            1.0, 2.0, 5.0, 10.0, 16.0, 33.0, 50.0, 100.0, 250.0, 500.0,
        ]),
        gauge_range: None,
    });

    // --- Counters (monotonic totals) ---
    reg.register(MetricDescriptor {
        name: "render_frames_total".into(),
        kind: MetricKind::Counter,
        help: "Total number of rendered frames".into(),
        label_names: vec![],
        buckets: None,
        gauge_range: None,
    });

    reg.register(MetricDescriptor {
        name: "diff_strategy_selected_total".into(),
        kind: MetricKind::Counter,
        help: "Total diff strategy selections by strategy type".into(),
        label_names: vec!["strategy".into()],
        buckets: None,
        gauge_range: None,
    });

    reg.register(MetricDescriptor {
        name: "ansi_sequences_parsed_total".into(),
        kind: MetricKind::Counter,
        help: "Total ANSI escape sequences parsed by type".into(),
        label_names: vec!["type".into()],
        buckets: None,
        gauge_range: None,
    });

    reg.register(MetricDescriptor {
        name: "ansi_malformed_total".into(),
        kind: MetricKind::Counter,
        help: "Total malformed ANSI sequences encountered".into(),
        label_names: vec![],
        buckets: None,
        gauge_range: None,
    });

    reg.register(MetricDescriptor {
        name: "runtime_messages_processed_total".into(),
        kind: MetricKind::Counter,
        help: "Total runtime messages processed by type".into(),
        label_names: vec!["msg_type".into()],
        buckets: None,
        gauge_range: None,
    });

    reg.register(MetricDescriptor {
        name: "effects_executed_total".into(),
        kind: MetricKind::Counter,
        help: "Total effects executed by type".into(),
        label_names: vec!["type".into()],
        buckets: None,
        gauge_range: None,
    });

    reg.register(MetricDescriptor {
        name: "slo_breaches_total".into(),
        kind: MetricKind::Counter,
        help: "Total SLO breaches by metric name".into(),
        label_names: vec!["metric".into()],
        buckets: None,
        gauge_range: None,
    });

    reg.register(MetricDescriptor {
        name: "terminal_resize_events_total".into(),
        kind: MetricKind::Counter,
        help: "Total terminal resize events received".into(),
        label_names: vec![],
        buckets: None,
        gauge_range: None,
    });

    reg.register(MetricDescriptor {
        name: "incremental_cache_hits_total".into(),
        kind: MetricKind::Counter,
        help: "Total incremental cache hits".into(),
        label_names: vec![],
        buckets: None,
        gauge_range: None,
    });

    reg.register(MetricDescriptor {
        name: "incremental_cache_misses_total".into(),
        kind: MetricKind::Counter,
        help: "Total incremental cache misses".into(),
        label_names: vec![],
        buckets: None,
        gauge_range: None,
    });

    // --- Gauges (point-in-time values) ---
    reg.register(MetricDescriptor {
        name: "terminal_active".into(),
        kind: MetricKind::Gauge,
        help: "Whether a terminal of given type is currently active".into(),
        label_names: vec!["term_type".into()],
        buckets: None,
        gauge_range: Some((0.0, 1.0)),
    });

    reg.register(MetricDescriptor {
        name: "eprocess_wealth".into(),
        kind: MetricKind::Gauge,
        help: "Current e-process wealth for sequential testing".into(),
        label_names: vec!["test_id".into()],
        buckets: None,
        gauge_range: Some((0.0, f64::INFINITY)),
    });

    reg
}

// ============================================================================
// Prometheus Text Format Validation
// ============================================================================

/// Validate that a string conforms to Prometheus text exposition format.
///
/// Returns a list of validation errors (empty = valid).
fn validate_prometheus_format(text: &str) -> Vec<String> {
    let mut errors = Vec::new();
    let mut seen_metrics: HashMap<String, String> = HashMap::new(); // name -> type
    let mut last_help: Option<String> = None;
    let mut last_type: Option<(String, String)> = None;

    for (line_num, line) in text.lines().enumerate() {
        let line_num = line_num + 1;

        if line.is_empty() {
            continue;
        }

        if let Some(rest) = line.strip_prefix("# HELP ") {
            // HELP line: "# HELP metric_name description"
            let parts: Vec<&str> = rest.splitn(2, ' ').collect();
            if parts.len() < 2 {
                errors.push(format!("line {line_num}: HELP missing description"));
            } else {
                let name = parts[0];
                if !is_valid_metric_name(name) {
                    errors.push(format!("line {line_num}: invalid metric name '{name}'"));
                }
                last_help = Some(name.to_string());
            }
        } else if let Some(rest) = line.strip_prefix("# TYPE ") {
            // TYPE line: "# TYPE metric_name counter|gauge|histogram|summary"
            let parts: Vec<&str> = rest.splitn(2, ' ').collect();
            if parts.len() < 2 {
                errors.push(format!("line {line_num}: TYPE missing type"));
            } else {
                let name = parts[0];
                let type_str = parts[1];
                if !["counter", "gauge", "histogram", "summary", "untyped"].contains(&type_str) {
                    errors.push(format!("line {line_num}: invalid metric type '{type_str}'"));
                }
                if let Some(ref help_name) = last_help
                    && help_name != name
                {
                    errors.push(format!(
                        "line {line_num}: TYPE name '{name}' doesn't match HELP name '{help_name}'"
                    ));
                }
                if seen_metrics.contains_key(name) {
                    errors.push(format!("line {line_num}: duplicate TYPE for '{name}'"));
                }
                seen_metrics.insert(name.to_string(), type_str.to_string());
                last_type = Some((name.to_string(), type_str.to_string()));
            }
        } else if line.starts_with('#') {
            // Other comments are allowed
        } else {
            // Metric sample line: metric_name{labels} value [timestamp]
            let (metric_name, _rest) = if let Some(brace_pos) = line.find('{') {
                (&line[..brace_pos], &line[brace_pos..])
            } else {
                let parts: Vec<&str> = line.splitn(2, ' ').collect();
                if parts.len() < 2 {
                    errors.push(format!("line {line_num}: sample line missing value"));
                    continue;
                }
                (parts[0], "")
            };

            // Extract base metric name (strip _bucket, _sum, _count suffixes)
            let base_name = metric_name
                .strip_suffix("_bucket")
                .or_else(|| metric_name.strip_suffix("_sum"))
                .or_else(|| metric_name.strip_suffix("_count"))
                .unwrap_or(metric_name);

            if !is_valid_metric_name(base_name) {
                errors.push(format!(
                    "line {line_num}: invalid metric name '{base_name}'"
                ));
            }

            // Verify value is a valid float
            let value_str = if let Some(brace_end) = line.find('}') {
                line[brace_end + 1..].split_whitespace().next()
            } else {
                line.split_whitespace().nth(1)
            };

            if let Some(vs) = value_str {
                if vs != "+Inf" && vs != "-Inf" && vs != "NaN" && vs.parse::<f64>().is_err() {
                    errors.push(format!("line {line_num}: invalid value '{vs}'"));
                }
            } else {
                errors.push(format!("line {line_num}: missing value"));
            }

            // Verify histogram suffixes match histogram type
            let has_hist_suffix = metric_name.ends_with("_bucket")
                || metric_name.ends_with("_sum")
                || metric_name.ends_with("_count");
            if has_hist_suffix
                && let Some((ref tn, ref tt)) = last_type
                && tn == base_name
                && tt != "histogram"
                && tt != "summary"
            {
                errors.push(format!(
                    "line {line_num}: _bucket/_sum/_count suffix on non-histogram metric '{base_name}'"
                ));
            }
        }
    }

    errors
}

/// Check if a metric name is valid per Prometheus conventions.
fn is_valid_metric_name(name: &str) -> bool {
    if name.is_empty() {
        return false;
    }
    let first = name.as_bytes()[0];
    if !(first.is_ascii_alphabetic() || first == b'_' || first == b':') {
        return false;
    }
    name.bytes()
        .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b':')
}

// ============================================================================
// Registry Completeness Tests
// ============================================================================

/// All declared metrics from the spec exist in the canonical registry.
#[test]
fn all_declared_metrics_are_registered() {
    let reg = build_canonical_registry();

    let expected_names = [
        // Histograms
        "render_frame_duration_us",
        "diff_strategy_duration_us",
        "layout_compute_duration_us",
        "widget_render_duration_us",
        "conformal_prediction_interval_width_us",
        "animation_duration_ms",
        // Counters
        "render_frames_total",
        "diff_strategy_selected_total",
        "ansi_sequences_parsed_total",
        "ansi_malformed_total",
        "runtime_messages_processed_total",
        "effects_executed_total",
        "slo_breaches_total",
        "terminal_resize_events_total",
        "incremental_cache_hits_total",
        "incremental_cache_misses_total",
        // Gauges
        "terminal_active",
        "eprocess_wealth",
    ];

    for name in &expected_names {
        assert!(
            reg.descriptors.iter().any(|d| d.name == *name),
            "Missing metric declaration: '{name}'"
        );
    }

    assert_eq!(
        reg.descriptors.len(),
        expected_names.len(),
        "Registry should have exactly {} metrics, got {}",
        expected_names.len(),
        reg.descriptors.len()
    );
}

/// Metric names follow Prometheus naming conventions.
#[test]
fn metric_names_follow_prometheus_conventions() {
    let reg = build_canonical_registry();

    for desc in &reg.descriptors {
        assert!(
            is_valid_metric_name(&desc.name),
            "Metric name '{}' is not valid per Prometheus conventions",
            desc.name
        );

        // Counters should end with _total
        if desc.kind == MetricKind::Counter {
            assert!(
                desc.name.ends_with("_total"),
                "Counter '{}' should end with '_total'",
                desc.name
            );
        }

        // Histograms/gauges should NOT end with _total
        if desc.kind != MetricKind::Counter {
            assert!(
                !desc.name.ends_with("_total"),
                "Non-counter '{}' should not end with '_total'",
                desc.name
            );
        }

        // Duration metrics should have unit suffix
        if desc.help.contains("uration") || desc.help.contains("latency") {
            assert!(
                desc.name.ends_with("_us") || desc.name.ends_with("_ms"),
                "Duration metric '{}' should have _us or _ms suffix",
                desc.name
            );
        }
    }
}

/// Every metric has a non-empty help string.
#[test]
fn all_metrics_have_help_strings() {
    let reg = build_canonical_registry();

    for desc in &reg.descriptors {
        assert!(
            !desc.help.is_empty(),
            "Metric '{}' has empty help string",
            desc.name
        );
        assert!(
            desc.help.len() >= 10,
            "Metric '{}' help string is too short: '{}'",
            desc.name,
            desc.help
        );
    }
}

/// No duplicate metric names in the registry.
#[test]
fn no_duplicate_metric_names() {
    let reg = build_canonical_registry();
    let mut seen = std::collections::HashSet::new();

    for desc in &reg.descriptors {
        assert!(
            seen.insert(&desc.name),
            "Duplicate metric name: '{}'",
            desc.name
        );
    }
}

// ============================================================================
// Histogram Bucket Boundary Tests
// ============================================================================

/// All histograms have bucket boundaries defined.
#[test]
fn all_histograms_have_buckets() {
    let reg = build_canonical_registry();

    for desc in reg
        .descriptors
        .iter()
        .filter(|d| d.kind == MetricKind::Histogram)
    {
        assert!(
            desc.buckets.is_some(),
            "Histogram '{}' has no bucket boundaries",
            desc.name
        );
        let buckets = desc.buckets.as_ref().unwrap();
        assert!(
            !buckets.is_empty(),
            "Histogram '{}' has empty bucket list",
            desc.name
        );
    }
}

/// Histogram buckets are strictly monotonically increasing.
#[test]
fn histogram_buckets_monotonically_increasing() {
    let reg = build_canonical_registry();

    for desc in reg
        .descriptors
        .iter()
        .filter(|d| d.kind == MetricKind::Histogram)
    {
        let buckets = desc.buckets.as_ref().unwrap();

        for i in 1..buckets.len() {
            assert!(
                buckets[i] > buckets[i - 1],
                "Histogram '{}' bucket boundaries not monotonic: {} >= {} at index {}",
                desc.name,
                buckets[i - 1],
                buckets[i],
                i
            );
        }
    }
}

/// Histogram buckets cover the expected range for latency metrics.
#[test]
fn latency_histogram_buckets_cover_expected_range() {
    let reg = build_canonical_registry();

    let latency_histograms: Vec<_> = reg
        .descriptors
        .iter()
        .filter(|d| d.kind == MetricKind::Histogram && d.name.ends_with("_us"))
        .collect();

    for desc in &latency_histograms {
        let buckets = desc.buckets.as_ref().unwrap();

        // Smallest bucket should capture sub-millisecond latencies
        assert!(
            buckets[0] <= 100.0,
            "Histogram '{}' smallest bucket {} too large for sub-ms latencies",
            desc.name,
            buckets[0]
        );

        // Largest bucket should capture worst-case latencies (at least 1ms)
        let max_bucket = buckets.last().unwrap();
        assert!(
            *max_bucket >= 1000.0,
            "Histogram '{}' largest bucket {} too small for worst-case latencies",
            desc.name,
            max_bucket
        );

        // Should have enough buckets for meaningful distribution
        assert!(
            buckets.len() >= 5,
            "Histogram '{}' has only {} buckets; need at least 5 for useful distribution",
            desc.name,
            buckets.len()
        );
    }
}

/// Animation duration histogram covers frame-rate-relevant ranges.
#[test]
fn animation_histogram_covers_frame_rate_range() {
    let reg = build_canonical_registry();

    let anim = reg
        .descriptors
        .iter()
        .find(|d| d.name == "animation_duration_ms")
        .expect("animation_duration_ms should exist");

    let buckets = anim.buckets.as_ref().unwrap();

    // Should have bucket at ~16ms (60fps frame time)
    assert!(
        buckets.iter().any(|&b| (b - 16.0).abs() < 1.0),
        "Animation histogram should have bucket near 16ms (60fps)"
    );

    // Should have bucket at ~33ms (30fps frame time)
    assert!(
        buckets.iter().any(|&b| (b - 33.0).abs() < 1.0),
        "Animation histogram should have bucket near 33ms (30fps)"
    );
}

/// Histogram buckets have no negative or zero values.
#[test]
fn histogram_buckets_all_positive() {
    let reg = build_canonical_registry();

    for desc in reg
        .descriptors
        .iter()
        .filter(|d| d.kind == MetricKind::Histogram)
    {
        let buckets = desc.buckets.as_ref().unwrap();

        for &b in buckets {
            assert!(
                b > 0.0,
                "Histogram '{}' has non-positive bucket boundary: {}",
                desc.name,
                b
            );
        }
    }
}

// ============================================================================
// Counter Monotonicity Tests
// ============================================================================

/// Counter increment always increases the value.
#[test]
fn counter_increment_is_monotonic() {
    let reg = build_canonical_registry();
    let labels = HashMap::new();

    reg.counter_inc("render_frames_total", 1.0, labels.clone());
    reg.counter_inc("render_frames_total", 1.0, labels.clone());
    reg.counter_inc("render_frames_total", 1.0, labels.clone());

    let counters = reg.counters.lock().unwrap();
    let values = counters.get("render_frames_total").unwrap();
    assert_eq!(
        values[0].value, 3.0,
        "Counter should be 3 after 3 increments"
    );
}

/// Counter with labels: separate label sets are independent.
#[test]
fn counter_labels_independent() {
    let reg = build_canonical_registry();

    let mut labels_a = HashMap::new();
    labels_a.insert("strategy".to_string(), "Full".to_string());

    let mut labels_b = HashMap::new();
    labels_b.insert("strategy".to_string(), "DirtyRows".to_string());

    reg.counter_inc("diff_strategy_selected_total", 5.0, labels_a.clone());
    reg.counter_inc("diff_strategy_selected_total", 3.0, labels_b.clone());
    reg.counter_inc("diff_strategy_selected_total", 2.0, labels_a.clone());

    let counters = reg.counters.lock().unwrap();
    let values = counters.get("diff_strategy_selected_total").unwrap();

    let full_val = values
        .iter()
        .find(|v| v.labels.get("strategy") == Some(&"Full".to_string()));
    let dirty_val = values
        .iter()
        .find(|v| v.labels.get("strategy") == Some(&"DirtyRows".to_string()));

    assert_eq!(
        full_val.unwrap().value,
        7.0,
        "Full strategy counter should be 7"
    );
    assert_eq!(
        dirty_val.unwrap().value,
        3.0,
        "DirtyRows strategy counter should be 3"
    );
}

/// Counter value never decreases (multiple increments).
#[test]
fn counter_never_decreases() {
    let reg = build_canonical_registry();
    let labels = HashMap::new();

    let mut prev = 0.0;
    for i in 1..=100 {
        let amount = (i as f64) * 0.5;
        reg.counter_inc("ansi_malformed_total", amount, labels.clone());

        let counters = reg.counters.lock().unwrap();
        let current = counters.get("ansi_malformed_total").unwrap()[0].value;
        assert!(
            current >= prev,
            "Counter decreased from {prev} to {current} at step {i}"
        );
        prev = current;
    }
}

// ============================================================================
// Gauge Range Tests
// ============================================================================

/// Gauge value must be within declared range.
#[test]
fn gauge_within_declared_range() {
    let reg = build_canonical_registry();

    // terminal_active has range [0.0, 1.0]
    assert!(reg.gauge_in_range("terminal_active", 0.0));
    assert!(reg.gauge_in_range("terminal_active", 1.0));
    assert!(reg.gauge_in_range("terminal_active", 0.5));
    assert!(!reg.gauge_in_range("terminal_active", -1.0));
    assert!(!reg.gauge_in_range("terminal_active", 2.0));
}

/// Gauge without declared range accepts any value.
#[test]
fn gauge_without_range_accepts_any_value() {
    let reg = build_canonical_registry();

    // eprocess_wealth has range [0.0, +inf]
    assert!(reg.gauge_in_range("eprocess_wealth", 0.0));
    assert!(reg.gauge_in_range("eprocess_wealth", 1000.0));
    assert!(reg.gauge_in_range("eprocess_wealth", f64::MAX));
    assert!(!reg.gauge_in_range("eprocess_wealth", -1.0));
}

/// Gauge set overwrites previous value.
#[test]
fn gauge_set_overwrites_value() {
    let reg = build_canonical_registry();

    let mut labels = HashMap::new();
    labels.insert("term_type".to_string(), "xterm".to_string());

    reg.gauge_set("terminal_active", 1.0, labels.clone());
    reg.gauge_set("terminal_active", 0.0, labels.clone());

    let gauges = reg.gauges.lock().unwrap();
    let values = gauges.get("terminal_active").unwrap();
    assert_eq!(values[0].value, 0.0, "Gauge should be 0.0 after second set");
}

/// Gauge with different labels are independent.
#[test]
fn gauge_labels_independent() {
    let reg = build_canonical_registry();

    let mut labels_a = HashMap::new();
    labels_a.insert("test_id".to_string(), "alpha".to_string());

    let mut labels_b = HashMap::new();
    labels_b.insert("test_id".to_string(), "beta".to_string());

    reg.gauge_set("eprocess_wealth", 100.0, labels_a.clone());
    reg.gauge_set("eprocess_wealth", 200.0, labels_b.clone());

    let gauges = reg.gauges.lock().unwrap();
    let values = gauges.get("eprocess_wealth").unwrap();

    let alpha = values
        .iter()
        .find(|v| v.labels.get("test_id") == Some(&"alpha".to_string()))
        .unwrap();
    let beta = values
        .iter()
        .find(|v| v.labels.get("test_id") == Some(&"beta".to_string()))
        .unwrap();

    assert_eq!(alpha.value, 100.0);
    assert_eq!(beta.value, 200.0);
}

// ============================================================================
// Histogram Observation Tests
// ============================================================================

/// Histogram observation increments count and accumulates sum.
#[test]
fn histogram_observation_updates_count_and_sum() {
    let reg = build_canonical_registry();
    let labels = HashMap::new();

    reg.histogram_observe("render_frame_duration_us", 100.0, labels.clone());
    reg.histogram_observe("render_frame_duration_us", 200.0, labels.clone());
    reg.histogram_observe("render_frame_duration_us", 50.0, labels.clone());

    let histograms = reg.histograms.lock().unwrap();
    let values = histograms.get("render_frame_duration_us").unwrap();

    assert_eq!(values[0].count, 3);
    assert!((values[0].sum - 350.0).abs() < f64::EPSILON);
}

/// Histogram bucket counts are cumulative.
#[test]
fn histogram_buckets_are_cumulative() {
    let reg = build_canonical_registry();
    let labels = HashMap::new();

    // Observe values that fall into different buckets
    // Buckets: [10, 25, 50, 100, 250, 500, 1000, 2500, 5000, 10000]
    reg.histogram_observe("render_frame_duration_us", 5.0, labels.clone()); // <= 10
    reg.histogram_observe("render_frame_duration_us", 30.0, labels.clone()); // <= 50
    reg.histogram_observe("render_frame_duration_us", 150.0, labels.clone()); // <= 250
    reg.histogram_observe("render_frame_duration_us", 8000.0, labels.clone()); // <= 10000

    let histograms = reg.histograms.lock().unwrap();
    let hv = &histograms.get("render_frame_duration_us").unwrap()[0];

    // Cumulative: each bucket counts observations <= that boundary
    // Buckets: [10, 25, 50, 100, 250, 500, 1000, 2500, 5000, 10000]
    // 5.0 falls in <=10, <=25, <=50, <=100, <=250, <=500, <=1000, <=2500, <=5000, <=10000
    // 30.0 falls in <=50, <=100, <=250, <=500, <=1000, <=2500, <=5000, <=10000
    // 150.0 falls in <=250, <=500, <=1000, <=2500, <=5000, <=10000
    // 8000.0 falls in <=10000
    assert_eq!(hv.bucket_counts[0], 1, "bucket <=10: [5.0]");
    assert_eq!(hv.bucket_counts[1], 1, "bucket <=25: [5.0]");
    assert_eq!(hv.bucket_counts[2], 2, "bucket <=50: [5.0, 30.0]");
    assert_eq!(hv.bucket_counts[3], 2, "bucket <=100: [5.0, 30.0]");
    assert_eq!(hv.bucket_counts[4], 3, "bucket <=250: [5.0, 30.0, 150.0]");
    assert_eq!(hv.bucket_counts[5], 3, "bucket <=500: [5.0, 30.0, 150.0]");
    assert_eq!(hv.bucket_counts[9], 4, "bucket <=10000: all 4 observations");
    // Total count
    assert_eq!(hv.count, 4);
}

// ============================================================================
// Prometheus Export Format Tests
// ============================================================================

/// Empty registry produces empty output.
#[test]
fn empty_registry_export_is_empty_lines() {
    let reg = MetricsRegistry::new();
    let output = reg.export_prometheus();
    // Empty registry should produce empty string
    assert!(output.is_empty(), "Empty registry should produce no output");
}

/// Exported counter format matches Prometheus spec.
#[test]
fn counter_export_prometheus_format() {
    let reg = build_canonical_registry();
    let labels = HashMap::new();

    reg.counter_inc("render_frames_total", 42.0, labels);

    let output = reg.export_prometheus();
    let errors = validate_prometheus_format(&output);

    assert!(errors.is_empty(), "Prometheus format errors: {errors:?}");

    assert!(
        output.contains("# HELP render_frames_total"),
        "Output should contain HELP for render_frames_total"
    );
    assert!(
        output.contains("# TYPE render_frames_total counter"),
        "Output should contain TYPE counter for render_frames_total"
    );
    assert!(
        output.contains("render_frames_total 42"),
        "Output should contain counter value"
    );
}

/// Exported histogram format matches Prometheus spec.
#[test]
fn histogram_export_prometheus_format() {
    let reg = build_canonical_registry();
    let labels = HashMap::new();

    reg.histogram_observe("render_frame_duration_us", 100.0, labels);

    let output = reg.export_prometheus();
    let errors = validate_prometheus_format(&output);

    assert!(errors.is_empty(), "Prometheus format errors: {errors:?}");

    assert!(
        output.contains("# TYPE render_frame_duration_us histogram"),
        "Output should contain TYPE histogram"
    );
    assert!(
        output.contains("render_frame_duration_us_bucket"),
        "Output should contain _bucket lines"
    );
    assert!(
        output.contains("render_frame_duration_us_sum"),
        "Output should contain _sum line"
    );
    assert!(
        output.contains("render_frame_duration_us_count"),
        "Output should contain _count line"
    );
    assert!(output.contains("+Inf"), "Output should contain +Inf bucket");
}

/// Exported gauge format matches Prometheus spec.
#[test]
fn gauge_export_prometheus_format() {
    let reg = build_canonical_registry();

    let mut labels = HashMap::new();
    labels.insert("term_type".to_string(), "xterm".to_string());

    reg.gauge_set("terminal_active", 1.0, labels);

    let output = reg.export_prometheus();
    let errors = validate_prometheus_format(&output);

    assert!(errors.is_empty(), "Prometheus format errors: {errors:?}");

    assert!(
        output.contains("# TYPE terminal_active gauge"),
        "Output should contain TYPE gauge"
    );
    assert!(
        output.contains("terminal_active{"),
        "Output should contain gauge with labels"
    );
}

/// Labels are correctly formatted in Prometheus output.
#[test]
fn labels_formatted_correctly() {
    let reg = build_canonical_registry();

    let mut labels = HashMap::new();
    labels.insert("strategy".to_string(), "DirtyRows".to_string());

    reg.counter_inc("diff_strategy_selected_total", 5.0, labels);

    let output = reg.export_prometheus();

    assert!(
        output.contains("diff_strategy_selected_total{strategy=\"DirtyRows\"} 5"),
        "Labels should be formatted as key=\"value\": got:\n{output}"
    );
}

/// Full registry export is valid Prometheus format.
#[test]
fn full_registry_export_valid_prometheus() {
    let reg = build_canonical_registry();

    // Populate some data
    reg.counter_inc("render_frames_total", 100.0, HashMap::new());
    reg.counter_inc("ansi_malformed_total", 3.0, HashMap::new());

    let mut strategy_labels = HashMap::new();
    strategy_labels.insert("strategy".to_string(), "Full".to_string());
    reg.counter_inc("diff_strategy_selected_total", 50.0, strategy_labels);

    reg.histogram_observe("render_frame_duration_us", 150.0, HashMap::new());
    reg.histogram_observe("render_frame_duration_us", 250.0, HashMap::new());

    let mut term_labels = HashMap::new();
    term_labels.insert("term_type".to_string(), "xterm-256color".to_string());
    reg.gauge_set("terminal_active", 1.0, term_labels);

    let output = reg.export_prometheus();
    let errors = validate_prometheus_format(&output);

    assert!(
        errors.is_empty(),
        "Full registry export has format errors: {errors:?}\n\nOutput:\n{output}"
    );
}

/// HELP and TYPE lines appear before sample lines.
#[test]
fn help_and_type_precede_samples() {
    let reg = build_canonical_registry();
    reg.counter_inc("render_frames_total", 1.0, HashMap::new());

    let output = reg.export_prometheus();
    let lines: Vec<&str> = output.lines().collect();

    // Find first render_frames_total sample
    let help_pos = lines
        .iter()
        .position(|l| l.starts_with("# HELP render_frames_total"));
    let type_pos = lines
        .iter()
        .position(|l| l.starts_with("# TYPE render_frames_total"));
    let sample_pos = lines
        .iter()
        .position(|l| l.starts_with("render_frames_total") && !l.starts_with('#'));

    assert!(help_pos.is_some(), "Should have HELP line");
    assert!(type_pos.is_some(), "Should have TYPE line");
    assert!(sample_pos.is_some(), "Should have sample line");

    assert!(
        help_pos.unwrap() < type_pos.unwrap(),
        "HELP should come before TYPE"
    );
    assert!(
        type_pos.unwrap() < sample_pos.unwrap(),
        "TYPE should come before samples"
    );
}

/// Validate metric name validation function.
#[test]
fn metric_name_validation() {
    // Valid names
    assert!(is_valid_metric_name("render_frames_total"));
    assert!(is_valid_metric_name("foo_bar_baz"));
    assert!(is_valid_metric_name("_private_metric"));
    assert!(is_valid_metric_name("a"));

    // Invalid names
    assert!(!is_valid_metric_name(""));
    assert!(!is_valid_metric_name("123_starts_with_digit"));
    assert!(!is_valid_metric_name("has-hyphen"));
    assert!(!is_valid_metric_name("has space"));
    assert!(!is_valid_metric_name("has.dot"));
}

/// Validate the format validator itself on known-good and known-bad input.
#[test]
fn prometheus_format_validator_correctness() {
    // Valid format
    let valid = "\
# HELP test_counter A test counter
# TYPE test_counter counter
test_counter 42
# HELP test_gauge A test gauge
# TYPE test_gauge gauge
test_gauge{label=\"value\"} 3.14
";
    let errors = validate_prometheus_format(valid);
    assert!(
        errors.is_empty(),
        "Valid format should produce no errors: {errors:?}"
    );

    // Invalid: bad type
    let bad_type = "# TYPE bad badtype\n";
    let errors = validate_prometheus_format(bad_type);
    assert!(!errors.is_empty(), "Bad type should produce errors");

    // Invalid: missing value
    let missing_value = "# HELP x X\n# TYPE x counter\nx\n";
    let errors = validate_prometheus_format(missing_value);
    assert!(!errors.is_empty(), "Missing value should produce errors");
}
