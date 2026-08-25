//! Prometheus-compatible metrics registry (bd-xox.3).
//!
//! Provides counters, gauges, and histograms that can be exported in
//! Prometheus text exposition format via [`MetricsRegistry::render`].
//!
//! # Design
//!
//! - **Zero-allocation on the hot path**: All metric storage uses `AtomicU64`.
//! - **Lock-free reads**: Snapshot is a single pass over atomic loads.
//! - **Label support**: Metrics with labels use a fixed set of variants
//!   to avoid dynamic allocation and hash maps.
//! - **Histogram**: Uses fixed log-scale buckets for O(1) observe.
//!
//! # Usage
//!
//! ```ignore
//! use ftui_runtime::metrics_registry::{METRICS, BuiltinCounter, BuiltinGauge, BuiltinHistogram};
//!
//! // Increment a counter
//! METRICS.counter(BuiltinCounter::RenderFramesTotal).inc();
//!
//! // Set a gauge
//! METRICS.gauge(BuiltinGauge::TerminalActive).set(1);
//!
//! // Observe a histogram value
//! METRICS.histogram(BuiltinHistogram::RenderFrameDurationUs).observe(450);
//!
//! // Export Prometheus text format
//! let output = METRICS.render();
//! println!("{output}");
//! ```

use std::fmt;
use std::sync::atomic::{AtomicI64, AtomicU64, Ordering};

/// Global metrics registry instance.
pub static METRICS: MetricsRegistry = MetricsRegistry::new();

// ============================================================================
// Atomic metric primitives
// ============================================================================

/// A monotonic counter (can only increase).
#[derive(Debug)]
pub struct Counter(AtomicU64);

impl Counter {
    const fn new() -> Self {
        Self(AtomicU64::new(0))
    }

    /// Increment by 1.
    pub fn inc(&self) {
        self.0.fetch_add(1, Ordering::Relaxed);
    }

    /// Increment by `n`.
    pub fn inc_by(&self, n: u64) {
        self.0.fetch_add(n, Ordering::Relaxed);
    }

    /// Current value.
    #[must_use]
    pub fn get(&self) -> u64 {
        self.0.load(Ordering::Relaxed)
    }
}

/// A gauge (can go up or down).
#[derive(Debug)]
pub struct Gauge(AtomicI64);

impl Gauge {
    const fn new() -> Self {
        Self(AtomicI64::new(0))
    }

    /// Set the gauge to a specific value.
    pub fn set(&self, v: i64) {
        self.0.store(v, Ordering::Relaxed);
    }

    /// Increment by 1.
    pub fn inc(&self) {
        self.0.fetch_add(1, Ordering::Relaxed);
    }

    /// Decrement by 1.
    pub fn dec(&self) {
        self.0.fetch_sub(1, Ordering::Relaxed);
    }

    /// Current value.
    #[must_use]
    pub fn get(&self) -> i64 {
        self.0.load(Ordering::Relaxed)
    }
}

/// Fixed-bucket histogram for latency measurements.
///
/// Buckets (in microseconds): 50, 100, 250, 500, 1000, 2000, 4000, 8000, 16000, +Inf.
#[derive(Debug)]
pub struct Histogram {
    /// Bucket upper bounds (exclusive). Last bucket is +Inf.
    buckets: [AtomicU64; HISTOGRAM_BUCKET_COUNT],
    /// Sum of all observed values.
    sum: AtomicU64,
    /// Total number of observations.
    count: AtomicU64,
}

const HISTOGRAM_BUCKET_COUNT: usize = 10;
const HISTOGRAM_BOUNDS: [u64; HISTOGRAM_BUCKET_COUNT - 1] =
    [50, 100, 250, 500, 1_000, 2_000, 4_000, 8_000, 16_000];

impl Histogram {
    const fn new() -> Self {
        Self {
            buckets: [
                AtomicU64::new(0),
                AtomicU64::new(0),
                AtomicU64::new(0),
                AtomicU64::new(0),
                AtomicU64::new(0),
                AtomicU64::new(0),
                AtomicU64::new(0),
                AtomicU64::new(0),
                AtomicU64::new(0),
                AtomicU64::new(0),
            ],
            sum: AtomicU64::new(0),
            count: AtomicU64::new(0),
        }
    }

    /// Record a value. Increments the appropriate bucket, sum, and count.
    pub fn observe(&self, value: u64) {
        // Find the first bucket whose bound >= value
        let idx = HISTOGRAM_BOUNDS
            .iter()
            .position(|&bound| value <= bound)
            .unwrap_or(HISTOGRAM_BUCKET_COUNT - 1);
        self.buckets[idx].fetch_add(1, Ordering::Relaxed);
        self.sum.fetch_add(value, Ordering::Relaxed);
        self.count.fetch_add(1, Ordering::Relaxed);
    }

    /// Current count of observations.
    #[must_use]
    pub fn count(&self) -> u64 {
        self.count.load(Ordering::Relaxed)
    }

    /// Current sum of all observations.
    #[must_use]
    pub fn sum(&self) -> u64 {
        self.sum.load(Ordering::Relaxed)
    }

    /// Snapshot of cumulative bucket counts.
    #[must_use]
    pub fn bucket_counts(&self) -> [u64; HISTOGRAM_BUCKET_COUNT] {
        let mut counts = [0u64; HISTOGRAM_BUCKET_COUNT];
        let mut cumulative = 0u64;
        for (i, bucket) in self.buckets.iter().enumerate() {
            cumulative += bucket.load(Ordering::Relaxed);
            counts[i] = cumulative;
        }
        counts
    }
}

// ============================================================================
// Builtin metric enums
// ============================================================================

/// Builtin counter metrics.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum BuiltinCounter {
    /// Total render frames produced.
    RenderFramesTotal = 0,
    /// Total ANSI sequences parsed (all types).
    AnsiSequencesParsedTotal = 1,
    /// Malformed ANSI sequences encountered.
    AnsiMalformedTotal = 2,
    /// Runtime messages processed.
    RuntimeMessagesProcessedTotal = 3,
    /// Effects executed (commands).
    EffectsCommandTotal = 4,
    /// Effects executed (subscriptions).
    EffectsSubscriptionTotal = 5,
    /// SLO breaches detected.
    SloBreachesTotal = 6,
    /// Terminal resize events.
    TerminalResizeEventsTotal = 7,
    /// Incremental computation cache hits.
    IncrementalCacheHitsTotal = 8,
    /// Incremental computation cache misses.
    IncrementalCacheMissesTotal = 9,
    /// VOI samples taken.
    VoiSamplesTakenTotal = 10,
    /// VOI samples skipped.
    VoiSamplesSkippedTotal = 11,
    /// BOCPD change points detected.
    BocpdChangePointsTotal = 12,
    /// E-process rejections.
    EProcessRejectionsTotal = 13,
    /// Trace/evidence schema compatibility failures.
    TraceCompatFailuresTotal = 14,
}

impl BuiltinCounter {
    const COUNT: usize = 15;

    const ALL: [Self; Self::COUNT] = [
        Self::RenderFramesTotal,
        Self::AnsiSequencesParsedTotal,
        Self::AnsiMalformedTotal,
        Self::RuntimeMessagesProcessedTotal,
        Self::EffectsCommandTotal,
        Self::EffectsSubscriptionTotal,
        Self::SloBreachesTotal,
        Self::TerminalResizeEventsTotal,
        Self::IncrementalCacheHitsTotal,
        Self::IncrementalCacheMissesTotal,
        Self::VoiSamplesTakenTotal,
        Self::VoiSamplesSkippedTotal,
        Self::BocpdChangePointsTotal,
        Self::EProcessRejectionsTotal,
        Self::TraceCompatFailuresTotal,
    ];

    fn name(self) -> &'static str {
        match self {
            Self::RenderFramesTotal => "ftui_render_frames_total",
            Self::AnsiSequencesParsedTotal => "ftui_ansi_sequences_parsed_total",
            Self::AnsiMalformedTotal => "ftui_ansi_malformed_total",
            Self::RuntimeMessagesProcessedTotal => "ftui_runtime_messages_processed_total",
            Self::EffectsCommandTotal => "ftui_effects_command_total",
            Self::EffectsSubscriptionTotal => "ftui_effects_subscription_total",
            Self::SloBreachesTotal => "ftui_slo_breaches_total",
            Self::TerminalResizeEventsTotal => "ftui_terminal_resize_events_total",
            Self::IncrementalCacheHitsTotal => "ftui_incremental_cache_hits_total",
            Self::IncrementalCacheMissesTotal => "ftui_incremental_cache_misses_total",
            Self::VoiSamplesTakenTotal => "ftui_voi_samples_taken_total",
            Self::VoiSamplesSkippedTotal => "ftui_voi_samples_skipped_total",
            Self::BocpdChangePointsTotal => "ftui_bocpd_change_points_total",
            Self::EProcessRejectionsTotal => "ftui_eprocess_rejections_total",
            Self::TraceCompatFailuresTotal => "ftui_trace_compat_failures_total",
        }
    }

    fn help(self) -> &'static str {
        match self {
            Self::RenderFramesTotal => "Total render frames produced.",
            Self::AnsiSequencesParsedTotal => "Total ANSI sequences parsed.",
            Self::AnsiMalformedTotal => "Malformed ANSI sequences encountered.",
            Self::RuntimeMessagesProcessedTotal => "Runtime messages processed.",
            Self::EffectsCommandTotal => "Command effects executed.",
            Self::EffectsSubscriptionTotal => "Subscription effects started.",
            Self::SloBreachesTotal => "SLO breaches detected.",
            Self::TerminalResizeEventsTotal => "Terminal resize events received.",
            Self::IncrementalCacheHitsTotal => "Incremental computation cache hits.",
            Self::IncrementalCacheMissesTotal => "Incremental computation cache misses.",
            Self::VoiSamplesTakenTotal => "VOI samples taken.",
            Self::VoiSamplesSkippedTotal => "VOI samples skipped.",
            Self::BocpdChangePointsTotal => "BOCPD change points detected.",
            Self::EProcessRejectionsTotal => "E-process rejections triggered.",
            Self::TraceCompatFailuresTotal => "Trace/evidence schema compatibility failures.",
        }
    }
}

/// Builtin gauge metrics.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum BuiltinGauge {
    /// Currently active terminal instances.
    TerminalActive = 0,
    /// Current e-process wealth.
    EProcessWealth = 1,
    /// Current degradation level (0=Full, 4=Skeleton).
    DegradationLevel = 2,
}

impl BuiltinGauge {
    const COUNT: usize = 3;

    const ALL: [Self; Self::COUNT] = [
        Self::TerminalActive,
        Self::EProcessWealth,
        Self::DegradationLevel,
    ];

    fn name(self) -> &'static str {
        match self {
            Self::TerminalActive => "ftui_terminal_active",
            Self::EProcessWealth => "ftui_eprocess_wealth",
            Self::DegradationLevel => "ftui_degradation_level",
        }
    }

    fn help(self) -> &'static str {
        match self {
            Self::TerminalActive => "Currently active terminal instances.",
            Self::EProcessWealth => "Current e-process wealth value.",
            Self::DegradationLevel => "Current degradation level (0=Full, 4=Skeleton).",
        }
    }
}

/// Builtin histogram metrics.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum BuiltinHistogram {
    /// Render frame duration in microseconds.
    RenderFrameDurationUs = 0,
    /// Diff strategy computation duration in microseconds.
    DiffStrategyDurationUs = 1,
    /// Layout computation duration in microseconds.
    LayoutComputeDurationUs = 2,
    /// Widget render duration in microseconds.
    WidgetRenderDurationUs = 3,
    /// Conformal prediction interval width in microseconds.
    ConformalIntervalWidthUs = 4,
    /// Animation duration in milliseconds.
    AnimationDurationMs = 5,
}

impl BuiltinHistogram {
    const COUNT: usize = 6;

    const ALL: [Self; Self::COUNT] = [
        Self::RenderFrameDurationUs,
        Self::DiffStrategyDurationUs,
        Self::LayoutComputeDurationUs,
        Self::WidgetRenderDurationUs,
        Self::ConformalIntervalWidthUs,
        Self::AnimationDurationMs,
    ];

    fn name(self) -> &'static str {
        match self {
            Self::RenderFrameDurationUs => "ftui_render_frame_duration_us",
            Self::DiffStrategyDurationUs => "ftui_diff_strategy_duration_us",
            Self::LayoutComputeDurationUs => "ftui_layout_compute_duration_us",
            Self::WidgetRenderDurationUs => "ftui_widget_render_duration_us",
            Self::ConformalIntervalWidthUs => "ftui_conformal_interval_width_us",
            Self::AnimationDurationMs => "ftui_animation_duration_ms",
        }
    }

    fn help(self) -> &'static str {
        match self {
            Self::RenderFrameDurationUs => "Render frame duration in microseconds.",
            Self::DiffStrategyDurationUs => "Diff strategy computation duration in microseconds.",
            Self::LayoutComputeDurationUs => "Layout computation duration in microseconds.",
            Self::WidgetRenderDurationUs => "Widget render duration in microseconds.",
            Self::ConformalIntervalWidthUs => {
                "Conformal prediction interval width in microseconds."
            }
            Self::AnimationDurationMs => "Animation duration in milliseconds.",
        }
    }
}

// ============================================================================
// Metrics Registry
// ============================================================================

/// Central metrics registry with fixed-slot storage.
///
/// All metric access is lock-free (`Ordering::Relaxed` atomics).
/// Call [`render`](MetricsRegistry::render) to produce Prometheus text format.
pub struct MetricsRegistry {
    counters: [Counter; BuiltinCounter::COUNT],
    gauges: [Gauge; BuiltinGauge::COUNT],
    histograms: [Histogram; BuiltinHistogram::COUNT],
}

impl MetricsRegistry {
    #[allow(clippy::declare_interior_mutable_const)]
    const NEW_COUNTER: Counter = Counter::new();
    #[allow(clippy::declare_interior_mutable_const)]
    const NEW_GAUGE: Gauge = Gauge::new();
    #[allow(clippy::declare_interior_mutable_const)]
    const NEW_HISTOGRAM: Histogram = Histogram::new();

    const fn new() -> Self {
        Self {
            counters: [Self::NEW_COUNTER; BuiltinCounter::COUNT],
            gauges: [Self::NEW_GAUGE; BuiltinGauge::COUNT],
            histograms: [Self::NEW_HISTOGRAM; BuiltinHistogram::COUNT],
        }
    }

    /// Access a counter by its builtin enum.
    #[inline]
    pub fn counter(&self, c: BuiltinCounter) -> &Counter {
        &self.counters[c as usize]
    }

    /// Access a gauge by its builtin enum.
    #[inline]
    pub fn gauge(&self, g: BuiltinGauge) -> &Gauge {
        &self.gauges[g as usize]
    }

    /// Access a histogram by its builtin enum.
    #[inline]
    pub fn histogram(&self, h: BuiltinHistogram) -> &Histogram {
        &self.histograms[h as usize]
    }

    /// Render all metrics in Prometheus text exposition format.
    #[must_use]
    pub fn render(&self) -> String {
        let mut out = String::with_capacity(4096);
        self.render_to(&mut out);
        out
    }

    /// Render into an existing buffer (avoids allocation if reused).
    pub fn render_to(&self, out: &mut String) {
        // Counters
        for &variant in &BuiltinCounter::ALL {
            let val = self.counters[variant as usize].get();
            let name = variant.name();
            let help = variant.help();
            fmt::write(
                out,
                format_args!("# HELP {name} {help}\n# TYPE {name} counter\n{name} {val}\n",),
            )
            .ok();
        }

        // Gauges
        for &variant in &BuiltinGauge::ALL {
            let val = self.gauges[variant as usize].get();
            let name = variant.name();
            let help = variant.help();
            fmt::write(
                out,
                format_args!("# HELP {name} {help}\n# TYPE {name} gauge\n{name} {val}\n",),
            )
            .ok();
        }

        // Histograms
        for &variant in &BuiltinHistogram::ALL {
            let hist = &self.histograms[variant as usize];
            let counts = hist.bucket_counts();
            let name = variant.name();
            let help = variant.help();

            fmt::write(
                out,
                format_args!("# HELP {name} {help}\n# TYPE {name} histogram\n"),
            )
            .ok();

            // Cumulative bucket lines
            for (j, &bound) in HISTOGRAM_BOUNDS.iter().enumerate() {
                fmt::write(
                    out,
                    format_args!("{name}_bucket{{le=\"{bound}\"}} {}\n", counts[j]),
                )
                .ok();
            }
            // +Inf bucket
            fmt::write(
                out,
                format_args!(
                    "{name}_bucket{{le=\"+Inf\"}} {}\n",
                    counts[HISTOGRAM_BUCKET_COUNT - 1]
                ),
            )
            .ok();

            // Sum and count
            fmt::write(
                out,
                format_args!("{name}_sum {}\n{name}_count {}\n", hist.sum(), hist.count()),
            )
            .ok();
        }
    }

    /// Reset all metrics to zero. Useful for testing.
    pub fn reset(&self) {
        for c in &self.counters {
            c.0.store(0, Ordering::Relaxed);
        }
        for g in &self.gauges {
            g.0.store(0, Ordering::Relaxed);
        }
        for h in &self.histograms {
            for b in &h.buckets {
                b.store(0, Ordering::Relaxed);
            }
            h.sum.store(0, Ordering::Relaxed);
            h.count.store(0, Ordering::Relaxed);
        }
    }
}

impl fmt::Debug for MetricsRegistry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MetricsRegistry")
            .field("counters", &BuiltinCounter::COUNT)
            .field("gauges", &BuiltinGauge::COUNT)
            .field("histograms", &BuiltinHistogram::COUNT)
            .finish()
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counter_inc_and_get() {
        let c = Counter::new();
        assert_eq!(c.get(), 0);
        c.inc();
        assert_eq!(c.get(), 1);
        c.inc_by(5);
        assert_eq!(c.get(), 6);
    }

    #[test]
    fn gauge_set_inc_dec() {
        let g = Gauge::new();
        assert_eq!(g.get(), 0);
        g.set(42);
        assert_eq!(g.get(), 42);
        g.inc();
        assert_eq!(g.get(), 43);
        g.dec();
        assert_eq!(g.get(), 42);
        g.set(-10);
        assert_eq!(g.get(), -10);
    }

    #[test]
    fn histogram_observe_buckets() {
        let h = Histogram::new();
        h.observe(30); // bucket 0 (<=50)
        h.observe(75); // bucket 1 (<=100)
        h.observe(200); // bucket 2 (<=250)
        h.observe(20_000); // bucket 9 (+Inf)

        assert_eq!(h.count(), 4);
        assert_eq!(h.sum(), 30 + 75 + 200 + 20_000);

        let counts = h.bucket_counts();
        // Cumulative: [1, 2, 3, 3, 3, 3, 3, 3, 3, 4]
        assert_eq!(counts[0], 1); // <=50
        assert_eq!(counts[1], 2); // <=100
        assert_eq!(counts[2], 3); // <=250
        assert_eq!(counts[9], 4); // +Inf
    }

    #[test]
    fn histogram_boundary_values() {
        let h = Histogram::new();
        h.observe(50); // exactly on boundary — goes into <=50 bucket
        h.observe(100); // exactly on <=100
        h.observe(16_000); // exactly on <=16000

        let counts = h.bucket_counts();
        assert_eq!(counts[0], 1); // <=50
        assert_eq!(counts[1], 2); // <=100 (cumulative)
        assert_eq!(counts[8], 3); // <=16000 (cumulative)
    }

    #[test]
    fn registry_counter_access() {
        let reg = MetricsRegistry::new();
        reg.counter(BuiltinCounter::RenderFramesTotal).inc();
        reg.counter(BuiltinCounter::RenderFramesTotal).inc_by(4);
        assert_eq!(reg.counter(BuiltinCounter::RenderFramesTotal).get(), 5);
    }

    #[test]
    fn registry_gauge_access() {
        let reg = MetricsRegistry::new();
        reg.gauge(BuiltinGauge::TerminalActive).set(3);
        assert_eq!(reg.gauge(BuiltinGauge::TerminalActive).get(), 3);
        reg.gauge(BuiltinGauge::TerminalActive).dec();
        assert_eq!(reg.gauge(BuiltinGauge::TerminalActive).get(), 2);
    }

    #[test]
    fn registry_histogram_access() {
        let reg = MetricsRegistry::new();
        reg.histogram(BuiltinHistogram::RenderFrameDurationUs)
            .observe(1500);
        assert_eq!(
            reg.histogram(BuiltinHistogram::RenderFrameDurationUs)
                .count(),
            1
        );
        assert_eq!(
            reg.histogram(BuiltinHistogram::RenderFrameDurationUs).sum(),
            1500
        );
    }

    #[test]
    fn render_contains_all_metric_types() {
        // Use a fresh local registry to avoid test ordering issues
        let reg = MetricsRegistry::new();
        reg.counter(BuiltinCounter::RenderFramesTotal).inc();
        reg.gauge(BuiltinGauge::TerminalActive).set(1);
        reg.histogram(BuiltinHistogram::RenderFrameDurationUs)
            .observe(500);

        let output = reg.render();

        // Counters
        assert!(output.contains("# TYPE ftui_render_frames_total counter"));
        assert!(output.contains("ftui_render_frames_total 1"));

        // Gauges
        assert!(output.contains("# TYPE ftui_terminal_active gauge"));
        assert!(output.contains("ftui_terminal_active 1"));

        // Histograms
        assert!(output.contains("# TYPE ftui_render_frame_duration_us histogram"));
        assert!(output.contains("ftui_render_frame_duration_us_bucket{le=\"500\"} 1"));
        assert!(output.contains("ftui_render_frame_duration_us_count 1"));
        assert!(output.contains("ftui_render_frame_duration_us_sum 500"));
    }

    #[test]
    fn render_format_is_prometheus_compatible() {
        let reg = MetricsRegistry::new();
        let output = reg.render();

        // Every HELP line should be followed by TYPE then value
        for line in output.lines() {
            if line.starts_with('#') {
                assert!(
                    line.starts_with("# HELP ") || line.starts_with("# TYPE "),
                    "Comment lines must be HELP or TYPE: {line}"
                );
            }
        }
    }

    #[test]
    fn reset_clears_all() {
        let reg = MetricsRegistry::new();
        reg.counter(BuiltinCounter::AnsiMalformedTotal).inc();
        reg.gauge(BuiltinGauge::EProcessWealth).set(100);
        reg.histogram(BuiltinHistogram::AnimationDurationMs)
            .observe(50);

        reg.reset();

        assert_eq!(reg.counter(BuiltinCounter::AnsiMalformedTotal).get(), 0);
        assert_eq!(reg.gauge(BuiltinGauge::EProcessWealth).get(), 0);
        assert_eq!(
            reg.histogram(BuiltinHistogram::AnimationDurationMs).count(),
            0
        );
    }

    #[test]
    fn all_counter_names_unique() {
        let mut names = Vec::new();
        for &v in &BuiltinCounter::ALL {
            let n = v.name();
            assert!(!names.contains(&n), "Duplicate counter name: {n}");
            names.push(n);
        }
    }

    #[test]
    fn all_gauge_names_unique() {
        let mut names = Vec::new();
        for &v in &BuiltinGauge::ALL {
            let n = v.name();
            assert!(!names.contains(&n), "Duplicate gauge name: {n}");
            names.push(n);
        }
    }

    #[test]
    fn all_histogram_names_unique() {
        let mut names = Vec::new();
        for &v in &BuiltinHistogram::ALL {
            let n = v.name();
            assert!(!names.contains(&n), "Duplicate histogram name: {n}");
            names.push(n);
        }
    }

    #[test]
    fn histogram_empty_render() {
        let reg = MetricsRegistry::new();
        let output = reg.render();
        // Empty histograms should still render with 0 counts
        assert!(output.contains("ftui_render_frame_duration_us_count 0"));
        assert!(output.contains("ftui_render_frame_duration_us_sum 0"));
    }
}
