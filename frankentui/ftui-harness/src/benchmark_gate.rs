#![forbid(unsafe_code)]

//! Benchmark gate enforcement with structured evidence.
//!
//! Loads baseline performance thresholds, compares measured values, and emits
//! pass/fail evidence in JSONL format. This module provides the programmatic
//! backbone for CI performance regression gating.
//!
//! # Design
//!
//! A [`BenchmarkGate`] is configured with a set of [`Threshold`]s (metric name,
//! budget, tolerance). After collecting [`Measurement`]s, calling
//! [`evaluate`](BenchmarkGate::evaluate) produces a [`GateResult`] with
//! per-metric verdicts and an overall pass/fail.
//!
//! # Example
//!
//! ```ignore
//! use ftui_harness::benchmark_gate::{BenchmarkGate, Measurement, Threshold};
//!
//! let gate = BenchmarkGate::new("render_perf")
//!     .threshold(Threshold::new("frame_render_p99_us", 2000.0).tolerance_pct(10.0))
//!     .threshold(Threshold::new("diff_compute_p99_us", 500.0).tolerance_pct(15.0));
//!
//! let measurements = vec![
//!     Measurement::new("frame_render_p99_us", 1850.0),
//!     Measurement::new("diff_compute_p99_us", 480.0),
//! ];
//!
//! let result = gate.evaluate(&measurements);
//! assert!(result.passed());
//! ```
//!
//! # Baseline JSON Format
//!
//! Thresholds can be loaded from a JSON file matching the format used by
//! `scripts/perf_regression_gate.sh`:
//!
//! ```json
//! {
//!   "frame_render_p99_us": { "budget": 2000.0, "tolerance_pct": 10.0 },
//!   "diff_compute_p99_us": { "budget": 500.0 }
//! }
//! ```

use std::collections::BTreeMap;

use crate::determinism::{JsonValue, TestJsonlLogger};

// ============================================================================
// Threshold
// ============================================================================

/// A single performance threshold for gating.
#[derive(Debug, Clone)]
pub struct Threshold {
    /// Metric name (must match a [`Measurement`] name).
    pub metric: String,
    /// Budget value (upper bound for the metric).
    pub budget: f64,
    /// Tolerance as a percentage (0.0–100.0). A measurement is allowed to
    /// exceed `budget` by up to `budget * tolerance_pct / 100`.
    pub tolerance_pct: f64,
}

impl Threshold {
    /// Create a threshold with zero tolerance.
    pub fn new(metric: &str, budget: f64) -> Self {
        Self {
            metric: metric.to_string(),
            budget,
            tolerance_pct: 0.0,
        }
    }

    /// Set the tolerance percentage.
    #[must_use]
    pub fn tolerance_pct(mut self, pct: f64) -> Self {
        self.tolerance_pct = pct;
        self
    }

    /// Effective ceiling = budget × (1 + tolerance / 100).
    #[must_use]
    pub fn ceiling(&self) -> f64 {
        self.budget * (1.0 + self.tolerance_pct / 100.0)
    }
}

// ============================================================================
// Measurement
// ============================================================================

/// A single performance measurement to check against a threshold.
#[derive(Debug, Clone)]
pub struct Measurement {
    /// Metric name (should match a [`Threshold`] metric).
    pub metric: String,
    /// Measured value.
    pub value: f64,
    /// Optional unit label for evidence output (e.g., "μs", "bytes").
    pub unit: Option<String>,
}

impl Measurement {
    /// Create a measurement.
    pub fn new(metric: &str, value: f64) -> Self {
        Self {
            metric: metric.to_string(),
            value,
            unit: None,
        }
    }

    /// Set the unit label.
    #[must_use]
    pub fn unit(mut self, unit: &str) -> Self {
        self.unit = Some(unit.to_string());
        self
    }
}

// ============================================================================
// MetricVerdict
// ============================================================================

/// Verdict for a single metric check.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MetricVerdict {
    /// Measured value is within budget (including tolerance).
    Pass,
    /// Measured value exceeds budget + tolerance.
    Fail,
    /// No threshold defined for this metric (informational only).
    Unchecked,
}

/// Detailed result for a single metric evaluation.
#[derive(Debug, Clone)]
pub struct MetricResult {
    /// Metric name.
    pub metric: String,
    /// Measured value.
    pub value: f64,
    /// Budget (if a threshold was defined).
    pub budget: Option<f64>,
    /// Effective ceiling (budget + tolerance).
    pub ceiling: Option<f64>,
    /// Tolerance percentage applied.
    pub tolerance_pct: Option<f64>,
    /// How much the value exceeds the budget as a percentage.
    /// Negative means under budget.
    pub overshoot_pct: Option<f64>,
    /// Per-metric verdict.
    pub verdict: MetricVerdict,
    /// Unit label (if provided).
    pub unit: Option<String>,
}

// ============================================================================
// GateResult
// ============================================================================

/// Overall result of a benchmark gate evaluation.
#[derive(Debug, Clone)]
pub struct GateResult {
    /// Gate name.
    pub gate_name: String,
    /// Per-metric results (sorted by metric name).
    pub metrics: Vec<MetricResult>,
    /// Number of metrics that passed.
    pub pass_count: usize,
    /// Number of metrics that failed.
    pub fail_count: usize,
    /// Number of metrics with no threshold (unchecked).
    pub unchecked_count: usize,
}

impl GateResult {
    /// True if no metric failed.
    #[must_use]
    pub fn passed(&self) -> bool {
        self.fail_count == 0
    }

    /// Return only the failed metrics.
    pub fn failures(&self) -> Vec<&MetricResult> {
        self.metrics
            .iter()
            .filter(|m| m.verdict == MetricVerdict::Fail)
            .collect()
    }

    /// Format a human-readable summary.
    #[must_use]
    pub fn summary(&self) -> String {
        let status = if self.passed() { "PASS" } else { "FAIL" };
        let mut out = format!(
            "Gate '{}': {} ({} passed, {} failed, {} unchecked)\n",
            self.gate_name, status, self.pass_count, self.fail_count, self.unchecked_count
        );
        for m in &self.metrics {
            let icon = match m.verdict {
                MetricVerdict::Pass => "  ok",
                MetricVerdict::Fail => "FAIL",
                MetricVerdict::Unchecked => "  --",
            };
            let unit = m.unit.as_deref().unwrap_or("");
            if let Some(budget) = m.budget {
                let overshoot = m.overshoot_pct.unwrap_or(0.0);
                out.push_str(&format!(
                    "  [{icon}] {}: {:.1}{unit} (budget: {:.1}{unit}, overshoot: {overshoot:+.1}%)\n",
                    m.metric, m.value, budget
                ));
            } else {
                out.push_str(&format!(
                    "  [{icon}] {}: {:.1}{unit} (no threshold)\n",
                    m.metric, m.value
                ));
            }
        }
        out
    }
}

// ============================================================================
// BenchmarkGate
// ============================================================================

/// Benchmark gate that compares measurements against thresholds.
#[derive(Debug, Clone)]
pub struct BenchmarkGate {
    /// Gate name for evidence output.
    gate_name: String,
    /// Thresholds keyed by metric name.
    thresholds: BTreeMap<String, Threshold>,
}

impl BenchmarkGate {
    /// Create a new benchmark gate.
    pub fn new(gate_name: &str) -> Self {
        Self {
            gate_name: gate_name.to_string(),
            thresholds: BTreeMap::new(),
        }
    }

    /// Add a threshold.
    #[must_use]
    pub fn threshold(mut self, threshold: Threshold) -> Self {
        self.thresholds.insert(threshold.metric.clone(), threshold);
        self
    }

    /// Load thresholds from a simple JSON map.
    ///
    /// Expected format:
    /// ```json
    /// {
    ///   "metric_name": { "budget": 123.0, "tolerance_pct": 10.0 }
    /// }
    /// ```
    ///
    /// Returns `None` if parsing fails.
    #[must_use]
    pub fn load_json(gate_name: &str, json: &str) -> Option<Self> {
        let parsed: serde_json::Value = serde_json::from_str(json).ok()?;
        let obj = parsed.as_object()?;
        let mut gate = Self::new(gate_name);
        for (metric, value) in obj {
            let budget = value.get("budget")?.as_f64()?;
            let tolerance_pct = value
                .get("tolerance_pct")
                .and_then(|v| v.as_f64())
                .unwrap_or(0.0);
            gate.thresholds.insert(
                metric.clone(),
                Threshold {
                    metric: metric.clone(),
                    budget,
                    tolerance_pct,
                },
            );
        }
        Some(gate)
    }

    /// Load thresholds from FrankenTUI's `tests/baseline.json` format.
    ///
    /// This format uses percentile budgets (`p99_ns`) and `threshold_pct`:
    /// ```json
    /// {
    ///   "frame_render": {
    ///     "p99_ns": 2000000,
    ///     "threshold_pct": 10
    ///   }
    /// }
    /// ```
    ///
    /// Entries whose keys start with `_` are skipped (metadata comments).
    /// The `percentile` parameter selects which budget to use (e.g., `"p99_ns"`).
    ///
    /// Returns `None` if the JSON is malformed.
    #[must_use]
    pub fn load_baseline_json(gate_name: &str, json: &str, percentile: &str) -> Option<Self> {
        let parsed: serde_json::Value = serde_json::from_str(json).ok()?;
        let obj = parsed.as_object()?;
        let mut gate = Self::new(gate_name);
        for (metric, value) in obj {
            // Skip metadata keys (e.g., _comment, _format)
            if metric.starts_with('_') {
                continue;
            }
            let budget = value.get(percentile).and_then(|v| v.as_f64())?;
            let tolerance_pct = value
                .get("threshold_pct")
                .and_then(|v| v.as_f64())
                .unwrap_or(0.0);
            gate.thresholds.insert(
                metric.clone(),
                Threshold {
                    metric: metric.clone(),
                    budget,
                    tolerance_pct,
                },
            );
        }
        Some(gate)
    }

    /// Evaluate measurements against thresholds.
    ///
    /// Metrics with no matching threshold get [`MetricVerdict::Unchecked`].
    /// Emits structured JSONL evidence via [`TestJsonlLogger`].
    pub fn evaluate(&self, measurements: &[Measurement]) -> GateResult {
        let mut logger = TestJsonlLogger::new_with(&format!("{}_gate", self.gate_name), 0, true, 0);
        logger.add_context_str("gate_name", &self.gate_name);

        logger.log(
            "gate.start",
            &[
                ("gate_name", JsonValue::str(&self.gate_name)),
                (
                    "threshold_count",
                    JsonValue::u64(self.thresholds.len() as u64),
                ),
                (
                    "measurement_count",
                    JsonValue::u64(measurements.len() as u64),
                ),
            ],
        );

        let mut metrics = Vec::new();
        let mut pass_count = 0usize;
        let mut fail_count = 0usize;
        let mut unchecked_count = 0usize;

        for measurement in measurements {
            let result = if let Some(threshold) = self.thresholds.get(&measurement.metric) {
                let ceiling = threshold.ceiling();
                let overshoot_pct = if threshold.budget > 0.0 {
                    (measurement.value - threshold.budget) / threshold.budget * 100.0
                } else {
                    0.0
                };
                let verdict = if measurement.value <= ceiling {
                    MetricVerdict::Pass
                } else {
                    MetricVerdict::Fail
                };
                MetricResult {
                    metric: measurement.metric.clone(),
                    value: measurement.value,
                    budget: Some(threshold.budget),
                    ceiling: Some(ceiling),
                    tolerance_pct: Some(threshold.tolerance_pct),
                    overshoot_pct: Some(overshoot_pct),
                    verdict,
                    unit: measurement.unit.clone(),
                }
            } else {
                MetricResult {
                    metric: measurement.metric.clone(),
                    value: measurement.value,
                    budget: None,
                    ceiling: None,
                    tolerance_pct: None,
                    overshoot_pct: None,
                    verdict: MetricVerdict::Unchecked,
                    unit: measurement.unit.clone(),
                }
            };

            // Log per-metric evidence
            let verdict_str = match result.verdict {
                MetricVerdict::Pass => "pass",
                MetricVerdict::Fail => "fail",
                MetricVerdict::Unchecked => "unchecked",
            };

            let mut fields: Vec<(&str, JsonValue)> = vec![
                ("metric", JsonValue::str(&result.metric)),
                ("value", JsonValue::raw(format!("{:.6}", result.value))),
                ("verdict", JsonValue::str(verdict_str)),
            ];
            if let Some(budget) = result.budget {
                fields.push(("budget", JsonValue::raw(format!("{budget:.6}"))));
            }
            if let Some(ceiling) = result.ceiling {
                fields.push(("ceiling", JsonValue::raw(format!("{ceiling:.6}"))));
            }
            if let Some(overshoot) = result.overshoot_pct {
                fields.push(("overshoot_pct", JsonValue::raw(format!("{overshoot:.2}"))));
            }
            logger.log("gate.metric", &fields);

            match result.verdict {
                MetricVerdict::Pass => pass_count += 1,
                MetricVerdict::Fail => fail_count += 1,
                MetricVerdict::Unchecked => unchecked_count += 1,
            }

            metrics.push(result);
        }

        // Sort by metric name for stable output
        metrics.sort_by(|a, b| a.metric.cmp(&b.metric));

        let overall = if fail_count == 0 { "pass" } else { "fail" };
        logger.log(
            "gate.result",
            &[
                ("gate_name", JsonValue::str(&self.gate_name)),
                ("verdict", JsonValue::str(overall)),
                ("pass_count", JsonValue::u64(pass_count as u64)),
                ("fail_count", JsonValue::u64(fail_count as u64)),
                ("unchecked_count", JsonValue::u64(unchecked_count as u64)),
            ],
        );

        GateResult {
            gate_name: self.gate_name.clone(),
            metrics,
            pass_count,
            fail_count,
            unchecked_count,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn threshold_ceiling_with_tolerance() {
        let t = Threshold::new("render_p99", 2000.0).tolerance_pct(10.0);
        assert!((t.ceiling() - 2200.0).abs() < f64::EPSILON);
    }

    #[test]
    fn threshold_ceiling_zero_tolerance() {
        let t = Threshold::new("render_p99", 1000.0);
        assert!((t.ceiling() - 1000.0).abs() < f64::EPSILON);
    }

    #[test]
    fn gate_pass_within_budget() {
        let gate = BenchmarkGate::new("test_gate")
            .threshold(Threshold::new("metric_a", 100.0).tolerance_pct(10.0));

        let result = gate.evaluate(&[Measurement::new("metric_a", 95.0)]);
        assert!(result.passed());
        assert_eq!(result.pass_count, 1);
        assert_eq!(result.fail_count, 0);
    }

    #[test]
    fn gate_pass_within_tolerance() {
        let gate = BenchmarkGate::new("test_gate")
            .threshold(Threshold::new("metric_a", 100.0).tolerance_pct(10.0));

        // 105 is above budget (100) but within tolerance (110)
        let result = gate.evaluate(&[Measurement::new("metric_a", 105.0)]);
        assert!(result.passed());
    }

    #[test]
    fn gate_fail_exceeds_tolerance() {
        let gate = BenchmarkGate::new("test_gate")
            .threshold(Threshold::new("metric_a", 100.0).tolerance_pct(10.0));

        // 115 exceeds ceiling of 110
        let result = gate.evaluate(&[Measurement::new("metric_a", 115.0)]);
        assert!(!result.passed());
        assert_eq!(result.fail_count, 1);
    }

    #[test]
    fn gate_unchecked_metric() {
        let gate = BenchmarkGate::new("test_gate").threshold(Threshold::new("metric_a", 100.0));

        let result = gate.evaluate(&[
            Measurement::new("metric_a", 90.0),
            Measurement::new("metric_b", 999.0),
        ]);
        assert!(result.passed());
        assert_eq!(result.unchecked_count, 1);
    }

    #[test]
    fn gate_multiple_metrics_mixed() {
        let gate = BenchmarkGate::new("test_gate")
            .threshold(Threshold::new("fast", 100.0))
            .threshold(Threshold::new("slow", 200.0).tolerance_pct(5.0));

        let result = gate.evaluate(&[
            Measurement::new("fast", 80.0),
            Measurement::new("slow", 250.0), // exceeds 210 ceiling
        ]);
        assert!(!result.passed());
        assert_eq!(result.pass_count, 1);
        assert_eq!(result.fail_count, 1);

        let failures = result.failures();
        assert_eq!(failures.len(), 1);
        assert_eq!(failures[0].metric, "slow");
    }

    #[test]
    fn gate_load_json() {
        let json = r#"{
            "render_p99": { "budget": 2000.0, "tolerance_pct": 10.0 },
            "diff_p99": { "budget": 500.0 }
        }"#;
        let gate = BenchmarkGate::load_json("perf_gate", json).expect("valid JSON");
        let result = gate.evaluate(&[
            Measurement::new("render_p99", 1800.0),
            Measurement::new("diff_p99", 480.0),
        ]);
        assert!(result.passed());
    }

    #[test]
    fn gate_load_json_invalid() {
        assert!(BenchmarkGate::load_json("bad", "not json").is_none());
    }

    #[test]
    fn gate_load_baseline_json_format() {
        let json = r#"{
            "_comment": "Performance baseline",
            "_format": "p50/p95/p99/p999 in nanoseconds",
            "frame_render": {
                "p50_ns": 500000,
                "p95_ns": 1000000,
                "p99_ns": 2000000,
                "p999_ns": 5000000,
                "threshold_pct": 10
            },
            "diff_strategy": {
                "p50_ns": 50000,
                "p99_ns": 200000,
                "threshold_pct": 10
            }
        }"#;
        let gate = BenchmarkGate::load_baseline_json("perf_gate", json, "p99_ns")
            .expect("baseline JSON should parse");

        // Under budget
        let result = gate.evaluate(&[
            Measurement::new("frame_render", 1_800_000.0).unit("ns"),
            Measurement::new("diff_strategy", 190_000.0).unit("ns"),
        ]);
        assert!(result.passed(), "gate should pass: {}", result.summary());

        // Over budget + tolerance
        let result = gate.evaluate(&[
            Measurement::new("frame_render", 2_500_000.0).unit("ns"), // >2.2M ceiling
            Measurement::new("diff_strategy", 190_000.0).unit("ns"),
        ]);
        assert!(!result.passed(), "gate should fail on regression");
    }

    #[test]
    fn gate_load_baseline_json_skips_metadata() {
        let json = r#"{
            "_comment": "ignored",
            "metric_a": { "p99_ns": 100.0, "threshold_pct": 5 }
        }"#;
        let gate =
            BenchmarkGate::load_baseline_json("meta_test", json, "p99_ns").expect("should parse");
        let result = gate.evaluate(&[Measurement::new("metric_a", 95.0)]);
        assert!(result.passed());
        // The _comment entry should not appear as a threshold
        assert_eq!(result.metrics.len(), 1);
    }

    #[test]
    fn gate_summary_format() {
        let gate = BenchmarkGate::new("summary_test").threshold(Threshold::new("metric_a", 100.0));
        let result = gate.evaluate(&[Measurement::new("metric_a", 90.0).unit("μs")]);
        let summary = result.summary();
        assert!(summary.contains("PASS"));
        assert!(summary.contains("metric_a"));
        assert!(summary.contains("μs"));
    }

    #[test]
    fn gate_overshoot_pct_negative_when_under_budget() {
        let gate =
            BenchmarkGate::new("overshoot_test").threshold(Threshold::new("metric_a", 100.0));
        let result = gate.evaluate(&[Measurement::new("metric_a", 80.0)]);
        let m = &result.metrics[0];
        assert!(m.overshoot_pct.unwrap() < 0.0);
    }

    #[test]
    fn gate_empty_measurements() {
        let gate = BenchmarkGate::new("empty_test").threshold(Threshold::new("metric_a", 100.0));
        let result = gate.evaluate(&[]);
        assert!(result.passed());
        assert_eq!(result.pass_count, 0);
        assert_eq!(result.fail_count, 0);
    }

    // =========================================================================
    // Runtime benchmark gate tests (bd-1vb19)
    // =========================================================================

    #[test]
    fn load_baseline_includes_runtime_benchmarks() {
        let json = include_str!("../../../tests/baseline.json");
        let gate = BenchmarkGate::load_baseline_json("runtime_gate", json, "p99_ns")
            .expect("baseline.json should parse");

        // Verify runtime benchmarks were loaded
        let metrics: Vec<&str> = gate
            .thresholds
            .keys()
            .filter(|k| k.starts_with("runtime_"))
            .map(|k| k.as_str())
            .collect();
        assert!(
            metrics.contains(&"runtime_shutdown_latency"),
            "shutdown_latency baseline should be loaded"
        );
        assert!(
            metrics.contains(&"runtime_first_frame"),
            "first_frame baseline should be loaded"
        );
        assert!(
            metrics.contains(&"runtime_command_roundtrip"),
            "command_roundtrip baseline should be loaded"
        );
        assert!(
            metrics.contains(&"runtime_effect_queue_drain"),
            "effect_queue_drain baseline should be loaded"
        );
    }

    #[test]
    fn runtime_gate_passes_within_budget() {
        let json = include_str!("../../../tests/baseline.json");
        let gate = BenchmarkGate::load_baseline_json("runtime_gate", json, "p99_ns")
            .expect("baseline.json should parse");

        // Simulate measurements well within budget
        let measurements = vec![
            Measurement::new("runtime_shutdown_latency", 1_000_000.0).unit("ns"),
            Measurement::new("runtime_first_frame", 5_000_000.0).unit("ns"),
            Measurement::new("runtime_command_roundtrip", 100_000.0).unit("ns"),
            Measurement::new("runtime_effect_queue_drain", 500_000.0).unit("ns"),
        ];
        let result = gate.evaluate(&measurements);
        assert!(
            result.passed(),
            "all runtime metrics should pass: {}",
            result.summary()
        );
    }

    #[test]
    fn runtime_gate_fails_on_regression() {
        let json = include_str!("../../../tests/baseline.json");
        let gate = BenchmarkGate::load_baseline_json("runtime_gate", json, "p99_ns")
            .expect("baseline.json should parse");

        // Simulate a severe regression on shutdown latency
        let measurements = vec![
            Measurement::new("runtime_shutdown_latency", 100_000_000.0).unit("ns"), // 100ms, way over 5ms budget
            Measurement::new("runtime_first_frame", 5_000_000.0).unit("ns"),
        ];
        let result = gate.evaluate(&measurements);
        assert!(!result.passed(), "regression should fail the gate");
        assert!(result.fail_count >= 1);

        let failures = result.failures();
        assert!(
            failures
                .iter()
                .any(|f| f.metric == "runtime_shutdown_latency"),
            "shutdown latency should be the failing metric"
        );
    }

    #[test]
    fn runtime_gate_summary_readable() {
        let json = include_str!("../../../tests/baseline.json");
        let gate = BenchmarkGate::load_baseline_json("runtime_gate", json, "p99_ns")
            .expect("baseline.json should parse");

        let measurements =
            vec![Measurement::new("runtime_shutdown_latency", 4_000_000.0).unit("ns")];
        let result = gate.evaluate(&measurements);
        let summary = result.summary();
        assert!(summary.contains("runtime_shutdown_latency"));
        assert!(summary.contains("PASS") || summary.contains("ok"));
    }
}
