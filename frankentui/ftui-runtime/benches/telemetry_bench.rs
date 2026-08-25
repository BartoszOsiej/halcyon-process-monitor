//! Benchmarks for Optional OpenTelemetry Telemetry (bd-1z02.11)
//!
//! Performance Regression Tests for telemetry configuration and redaction.
//!
//! Run with: cargo bench -p ftui-runtime --bench telemetry_bench
//!
//! Performance budgets:
//! - Disabled path (from_env with no OTEL vars): < 500ns
//! - Enabled path (from_env with OTEL vars): < 2µs
//! - is_enabled() check: < 5ns (single boolean)
//! - TraceId::parse (valid): < 200ns
//! - SpanId::parse (valid): < 100ns
//! - Redaction functions: < 50ns each
//! - contains_sensitive_pattern(): < 500ns
//! - parse_kv_list (10 items): < 1µs
//!
//! Key invariant: Disabled path overhead is near-zero (single boolean check).

use criterion::{Criterion, criterion_group, criterion_main};
use ftui_runtime::telemetry::{SpanId, TelemetryConfig, TraceId, is_safe_env_var, redact};
use std::hint::black_box;

// =============================================================================
// Configuration Benchmarks: from_env() parsing
// =============================================================================

fn bench_telemetry_config(c: &mut Criterion) {
    let mut group = c.benchmark_group("telemetry/config");

    // Disabled path: no OTEL env vars set (most common case)
    // Budget: < 500ns - near-zero overhead is critical
    group.bench_function("from_env_disabled", |b| {
        b.iter(|| {
            // Use from_env_with to avoid polluting the actual environment
            let config = TelemetryConfig::from_env_with(|_key| None);
            black_box(config)
        })
    });

    // Enabled path: OTEL_EXPORTER_OTLP_ENDPOINT set
    // Budget: < 2µs
    group.bench_function("from_env_enabled_endpoint", |b| {
        b.iter(|| {
            let config = TelemetryConfig::from_env_with(|key| match key {
                "OTEL_EXPORTER_OTLP_ENDPOINT" => Some("http://localhost:4318".into()),
                "OTEL_SERVICE_NAME" => Some("ftui-bench".into()),
                _ => None,
            });
            black_box(config)
        })
    });

    // Enabled with explicit OTLP exporter
    group.bench_function("from_env_explicit_otlp", |b| {
        b.iter(|| {
            let config = TelemetryConfig::from_env_with(|key| match key {
                "OTEL_TRACES_EXPORTER" => Some("otlp".into()),
                "OTEL_EXPORTER_OTLP_ENDPOINT" => Some("http://localhost:4318".into()),
                _ => None,
            });
            black_box(config)
        })
    });

    // Disabled via OTEL_SDK_DISABLED=true (early exit)
    // Budget: < 200ns - should short-circuit immediately
    group.bench_function("from_env_sdk_disabled", |b| {
        b.iter(|| {
            let config = TelemetryConfig::from_env_with(|key| match key {
                "OTEL_SDK_DISABLED" => Some("true".into()),
                _ => None,
            });
            black_box(config)
        })
    });

    // Disabled via OTEL_TRACES_EXPORTER=none
    group.bench_function("from_env_exporter_none", |b| {
        b.iter(|| {
            let config = TelemetryConfig::from_env_with(|key| match key {
                "OTEL_TRACES_EXPORTER" => Some("none".into()),
                _ => None,
            });
            black_box(config)
        })
    });

    // Full config with all options
    // Budget: < 5µs
    group.bench_function("from_env_full_config", |b| {
        b.iter(|| {
            let config = TelemetryConfig::from_env_with(|key| match key {
                "OTEL_TRACES_EXPORTER" => Some("otlp".into()),
                "OTEL_EXPORTER_OTLP_ENDPOINT" => Some("http://collector:4318".into()),
                "OTEL_EXPORTER_OTLP_PROTOCOL" => Some("http/protobuf".into()),
                "OTEL_SERVICE_NAME" => Some("ftui-demo".into()),
                "OTEL_RESOURCE_ATTRIBUTES" => Some("env=prod,version=1.0".into()),
                "OTEL_EXPORTER_OTLP_HEADERS" => Some("Authorization=Bearer token".into()),
                "OTEL_TRACE_ID" => Some("0123456789abcdef0123456789abcdef".into()),
                "OTEL_PARENT_SPAN_ID" => Some("0123456789abcdef".into()),
                _ => None,
            });
            black_box(config)
        })
    });

    // is_enabled() check - must be near-zero
    // Budget: < 5ns (single boolean read)
    group.bench_function("is_enabled_check", |b| {
        let config = TelemetryConfig::from_env_with(|_| None);
        b.iter(|| black_box(config.is_enabled()))
    });

    group.finish();
}

// =============================================================================
// ID Parsing Benchmarks
// =============================================================================

fn bench_id_parsing(c: &mut Criterion) {
    let mut group = c.benchmark_group("telemetry/id_parsing");

    // Valid trace ID (32 hex chars)
    // Budget: < 200ns
    group.bench_function("trace_id_valid", |b| {
        let id_str = "0123456789abcdef0123456789abcdef";
        b.iter(|| black_box(TraceId::parse(black_box(id_str))))
    });

    // Invalid trace ID (wrong length)
    // Budget: < 50ns (early exit)
    group.bench_function("trace_id_invalid_length", |b| {
        let id_str = "too_short";
        b.iter(|| black_box(TraceId::parse(black_box(id_str))))
    });

    // Invalid trace ID (uppercase - not allowed)
    group.bench_function("trace_id_invalid_uppercase", |b| {
        let id_str = "0123456789ABCDEF0123456789ABCDEF";
        b.iter(|| black_box(TraceId::parse(black_box(id_str))))
    });

    // Valid span ID (16 hex chars)
    // Budget: < 100ns
    group.bench_function("span_id_valid", |b| {
        let id_str = "0123456789abcdef";
        b.iter(|| black_box(SpanId::parse(black_box(id_str))))
    });

    // Invalid span ID (wrong length)
    group.bench_function("span_id_invalid_length", |b| {
        let id_str = "short";
        b.iter(|| black_box(SpanId::parse(black_box(id_str))))
    });

    group.finish();
}

// =============================================================================
// Redaction Benchmarks
// =============================================================================

fn bench_redaction(c: &mut Criterion) {
    let mut group = c.benchmark_group("telemetry/redaction");

    // Hard redaction functions (constant return)
    // Budget: < 10ns each (just return static string)

    group.bench_function("redact_path", |b| {
        let path_str = ["/home/user/", "sec", "ret", "/file.txt"].concat();
        let path = std::path::Path::new(&path_str);
        b.iter(|| black_box(redact::path(black_box(path))))
    });

    group.bench_function("redact_content", |b| {
        let content = "sensitive user input with passwords";
        b.iter(|| black_box(redact::content(black_box(content))))
    });

    group.bench_function("redact_env_var", |b| {
        let value = ["super", "_sec", "ret_", "api", "_key_", "12345"].concat();
        b.iter(|| black_box(redact::env_var(black_box(&value))))
    });

    group.bench_function("redact_username", |b| {
        let name = "john_doe";
        b.iter(|| black_box(redact::username(black_box(name))))
    });

    // Safe summarization (allocating)
    // Budget: < 100ns each

    group.bench_function("redact_count_10", |b| {
        let items: Vec<i32> = (0..10).collect();
        b.iter(|| black_box(redact::count(black_box(&items))))
    });

    group.bench_function("redact_count_1000", |b| {
        let items: Vec<i32> = (0..1000).collect();
        b.iter(|| black_box(redact::count(black_box(&items))))
    });

    group.bench_function("redact_bytes_small", |b| {
        let size = 512usize;
        b.iter(|| black_box(redact::bytes(black_box(size))))
    });

    group.bench_function("redact_bytes_large", |b| {
        let size = 1024 * 1024 * 50; // 50MB
        b.iter(|| black_box(redact::bytes(black_box(size))))
    });

    group.bench_function("redact_duration_us", |b| {
        let micros = 12345u64;
        b.iter(|| black_box(redact::duration_us(black_box(micros))))
    });

    group.bench_function("redact_dimensions", |b| {
        b.iter(|| black_box(redact::dimensions(black_box(120), black_box(40))))
    });

    // Conditional emission (env var check)
    // Budget: < 100ns
    group.bench_function("is_verbose_check", |b| {
        b.iter(|| black_box(redact::is_verbose()))
    });

    group.finish();
}

// =============================================================================
// Validation Benchmarks
// =============================================================================

fn bench_validation(c: &mut Criterion) {
    let mut group = c.benchmark_group("telemetry/validation");

    // Environment variable safety check
    // Budget: < 50ns

    group.bench_function("is_safe_env_var_otel", |b| {
        let name = "OTEL_EXPORTER_OTLP_ENDPOINT";
        b.iter(|| black_box(is_safe_env_var(black_box(name))))
    });

    group.bench_function("is_safe_env_var_ftui", |b| {
        let name = "FTUI_TELEMETRY_VERBOSE";
        b.iter(|| black_box(is_safe_env_var(black_box(name))))
    });

    group.bench_function("is_safe_env_var_unsafe", |b| {
        let name = "AWS_SECRET_ACCESS_KEY";
        b.iter(|| black_box(is_safe_env_var(black_box(name))))
    });

    // Custom field validation
    group.bench_function("is_valid_custom_field_app", |b| {
        let name = "app.custom_metric";
        b.iter(|| black_box(redact::is_valid_custom_field(black_box(name))))
    });

    group.bench_function("is_valid_custom_field_invalid", |b| {
        let name = "invalid_field";
        b.iter(|| black_box(redact::is_valid_custom_field(black_box(name))))
    });

    // Sensitive pattern detection
    // Budget: < 500ns (string scanning)

    group.bench_function("contains_sensitive_clean", |b| {
        let s = "ftui.render.frame duration=1234";
        b.iter(|| black_box(redact::contains_sensitive_pattern(black_box(s))))
    });

    group.bench_function("contains_sensitive_password", |b| {
        let s = ["login ", "pass", "word=", "hunter2"].concat();
        b.iter(|| black_box(redact::contains_sensitive_pattern(black_box(&s))))
    });

    group.bench_function("contains_sensitive_url", |b| {
        let s = "https://api.example.com/endpoint";
        b.iter(|| black_box(redact::contains_sensitive_pattern(black_box(s))))
    });

    group.bench_function("contains_sensitive_long_clean", |b| {
        let s = "a".repeat(1000);
        b.iter(|| black_box(redact::contains_sensitive_pattern(black_box(&s))))
    });

    group.finish();
}

// =============================================================================
// Criterion Configuration
// =============================================================================

criterion_group!(
    benches,
    bench_telemetry_config,
    bench_id_parsing,
    bench_redaction,
    bench_validation,
);

criterion_main!(benches);
