#![forbid(unsafe_code)]

//! bd-1q5.14: E2E test for flicker-free sync bracket round-trip.
//!
//! Captures actual presenter output, feeds it through the flicker detector,
//! and verifies:
//! 1. Sync bracket open/close always paired
//! 2. No partial frame visible between brackets
//! 3. Fallback to cursor-hiding when sync brackets unavailable
//! 4. Resize during render does not corrupt output
//! 5. render.sync_bracket tracing span emitted with correct fields
//! 6. No ERROR-level logs during normal operation
//! 7. WARN logged when fallback engaged
//!
//! Run:
//!   cargo test -p ftui-harness --test e2e_flicker_free_round_trip

use std::collections::HashMap;
use std::sync::{Arc, Mutex, Once};

use ftui_core::terminal_capabilities::TerminalCapabilities;
use ftui_harness::flicker_detection::{analyze_stream, assert_flicker_free};
use ftui_render::buffer::Buffer;
use ftui_render::cell::{Cell, PackedRgba};
use ftui_render::diff::BufferDiff;
use ftui_render::presenter::Presenter;

use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::registry::LookupSpan;

fn ensure_global_trace_level() {
    static INIT: Once = Once::new();
    INIT.call_once(|| {
        let subscriber =
            tracing_subscriber::registry().with(tracing_subscriber::filter::LevelFilter::TRACE);
        let _ = tracing::subscriber::set_global_default(subscriber);
    });
}

// ============================================================================
// Tracing capture infrastructure
// ============================================================================

#[derive(Debug, Clone)]
#[allow(dead_code)]
struct CapturedSpan {
    name: String,
    fields: HashMap<String, String>,
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
struct CapturedEvent {
    level: tracing::Level,
    target: String,
    fields: HashMap<String, String>,
}

struct SpanCapture {
    spans: Arc<Mutex<Vec<CapturedSpan>>>,
    events: Arc<Mutex<Vec<CapturedEvent>>>,
}

impl SpanCapture {
    fn new() -> (Self, CaptureHandle) {
        let spans = Arc::new(Mutex::new(Vec::new()));
        let events = Arc::new(Mutex::new(Vec::new()));
        let handle = CaptureHandle {
            spans: spans.clone(),
            events: events.clone(),
        };
        (Self { spans, events }, handle)
    }
}

struct CaptureHandle {
    spans: Arc<Mutex<Vec<CapturedSpan>>>,
    events: Arc<Mutex<Vec<CapturedEvent>>>,
}

impl CaptureHandle {
    fn spans(&self) -> Vec<CapturedSpan> {
        self.spans.lock().unwrap().clone()
    }

    fn events(&self) -> Vec<CapturedEvent> {
        self.events.lock().unwrap().clone()
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
    fn record_i64(&mut self, field: &tracing::field::Field, value: i64) {
        self.0.push((field.name().to_string(), value.to_string()));
    }
    fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
        self.0.push((field.name().to_string(), value.to_string()));
    }
    fn record_bool(&mut self, field: &tracing::field::Field, value: bool) {
        self.0.push((field.name().to_string(), value.to_string()));
    }
}

impl<S> tracing_subscriber::Layer<S> for SpanCapture
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
        let mut fields: HashMap<String, String> = visitor.0.into_iter().collect();
        for field in attrs.metadata().fields() {
            fields.entry(field.name().to_string()).or_default();
        }
        self.spans.lock().unwrap().push(CapturedSpan {
            name: attrs.metadata().name().to_string(),
            fields,
        });
    }

    fn on_event(
        &self,
        event: &tracing::Event<'_>,
        _ctx: tracing_subscriber::layer::Context<'_, S>,
    ) {
        let mut visitor = FieldVisitor(Vec::new());
        event.record(&mut visitor);
        let fields: HashMap<String, String> = visitor.0.into_iter().collect();
        self.events.lock().unwrap().push(CapturedEvent {
            level: *event.metadata().level(),
            target: event.metadata().target().to_string(),
            fields,
        });
    }
}

fn with_captured_tracing<F, R>(f: F) -> (R, CaptureHandle)
where
    F: FnOnce() -> R,
{
    ensure_global_trace_level();
    let (layer, handle) = SpanCapture::new();
    let subscriber = tracing_subscriber::registry().with(layer);
    let result = tracing::subscriber::with_default(subscriber, || {
        tracing::callsite::rebuild_interest_cache();
        f()
    });
    (result, handle)
}

// ============================================================================
// Helpers
// ============================================================================

fn caps_sync() -> TerminalCapabilities {
    let mut caps = TerminalCapabilities::basic();
    caps.sync_output = true;
    caps
}

fn caps_no_sync() -> TerminalCapabilities {
    let mut caps = TerminalCapabilities::basic();
    caps.sync_output = false;
    caps
}

fn present_frame(buffer: &Buffer, old: &Buffer, caps: TerminalCapabilities) -> Vec<u8> {
    let diff = BufferDiff::compute(old, buffer);
    let mut sink = Vec::new();
    let mut presenter = Presenter::new(&mut sink, caps);
    presenter.present(buffer, &diff).unwrap();
    drop(presenter);
    sink
}

/// Simple LCG for deterministic test data.
struct Lcg(u64);

impl Lcg {
    fn new(seed: u64) -> Self {
        Self(seed)
    }

    fn next_u64(&mut self) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1);
        self.0
    }

    fn next_u16(&mut self, max: u16) -> u16 {
        (self.next_u64() >> 16) as u16 % max
    }

    fn next_char(&mut self) -> char {
        char::from_u32('A' as u32 + (self.next_u64() % 26) as u32).unwrap()
    }
}

fn random_buffer(width: u16, height: u16, seed: u64, fill_fraction: f64) -> Buffer {
    let mut buf = Buffer::new(width, height);
    let mut rng = Lcg::new(seed);
    let total = (width as usize) * (height as usize);
    let fill_count = (total as f64 * fill_fraction) as usize;
    for _ in 0..fill_count {
        let x = rng.next_u16(width);
        let y = rng.next_u16(height);
        let ch = rng.next_char();
        let fg = PackedRgba::rgb(
            (rng.next_u64() % 256) as u8,
            (rng.next_u64() % 256) as u8,
            (rng.next_u64() % 256) as u8,
        );
        buf.set_raw(x, y, Cell::from_char(ch).with_fg(fg));
    }
    buf
}

// ============================================================================
// 1. Sync bracket round-trip: paired and flicker-free
// ============================================================================

#[test]
fn sync_bracket_round_trip_single_frame() {
    let old = Buffer::new(80, 24);
    let new = random_buffer(80, 24, 0xE2E0_0001, 0.6);
    let output = present_frame(&new, &old, caps_sync());

    // Verify brackets are paired and flicker-free
    assert_flicker_free(&output);

    let analysis = analyze_stream(&output);
    assert_eq!(analysis.stats.total_frames, 1);
    assert_eq!(analysis.stats.complete_frames, 1);
    assert_eq!(analysis.stats.sync_gaps, 0);
    assert_eq!(analysis.stats.partial_clears, 0);
    assert!(analysis.stats.is_flicker_free());
}

// ============================================================================
// 2. Multi-frame round-trip: each frame individually flicker-free
// ============================================================================

#[test]
fn multi_frame_round_trip_all_flicker_free() {
    let mut accumulated = Vec::new();
    let mut prev = Buffer::new(80, 24);
    let caps = caps_sync();

    for i in 0..10 {
        let next = random_buffer(80, 24, 0xE2E0_0100 + i, 0.5);
        let diff = BufferDiff::compute(&prev, &next);
        let mut sink = Vec::new();
        let mut presenter = Presenter::new(&mut sink, caps);
        presenter.present(&next, &diff).unwrap();
        drop(presenter);

        // Each individual frame should be flicker-free
        assert_flicker_free(&sink);

        accumulated.extend_from_slice(&sink);
        prev = next;
    }

    // Combined stream should also be flicker-free
    let combined = analyze_stream(&accumulated);
    assert_eq!(combined.stats.total_frames, 10);
    assert_eq!(combined.stats.complete_frames, 10);
    assert!(combined.stats.is_flicker_free());
}

// ============================================================================
// 3. Fallback: cursor-hiding when sync brackets unavailable
// ============================================================================

#[test]
fn fallback_uses_cursor_hide_show() {
    let old = Buffer::new(80, 24);
    let new = random_buffer(80, 24, 0xE2E0_0201, 0.5);
    let output = present_frame(&new, &old, caps_no_sync());

    // No sync brackets should appear
    let sync_begin = b"\x1b[?2026h";
    let sync_end = b"\x1b[?2026l";
    assert!(
        !output.windows(sync_begin.len()).any(|w| w == sync_begin),
        "sync_begin should not appear when sync unsupported"
    );
    assert!(
        !output.windows(sync_end.len()).any(|w| w == sync_end),
        "sync_end should not appear when sync unsupported"
    );

    // Cursor hide/show should appear instead
    let cursor_hide = b"\x1b[?25l";
    let cursor_show = b"\x1b[?25h";
    let hide_count = output
        .windows(cursor_hide.len())
        .filter(|w| *w == cursor_hide)
        .count();
    let show_count = output
        .windows(cursor_show.len())
        .filter(|w| *w == cursor_show)
        .count();

    assert_eq!(hide_count, 1, "expected exactly 1 cursor-hide");
    assert_eq!(show_count, 1, "expected exactly 1 cursor-show");
}

// ============================================================================
// 4. Resize during render does not corrupt output
// ============================================================================

#[test]
fn resize_between_frames_produces_valid_output() {
    let caps = caps_sync();

    // Frame 1: 80x24
    let old_80 = Buffer::new(80, 24);
    let new_80 = random_buffer(80, 24, 0xE2E0_0301, 0.5);
    let output_80 = present_frame(&new_80, &old_80, caps);
    assert_flicker_free(&output_80);

    // Resize to 120x40
    let old_120 = Buffer::new(120, 40);
    let new_120 = random_buffer(120, 40, 0xE2E0_0302, 0.6);
    let output_120 = present_frame(&new_120, &old_120, caps);
    assert_flicker_free(&output_120);

    // Resize back to 40x10 (smaller)
    let old_40 = Buffer::new(40, 10);
    let new_40 = random_buffer(40, 10, 0xE2E0_0303, 0.7);
    let output_40 = present_frame(&new_40, &old_40, caps);
    assert_flicker_free(&output_40);

    // Combined stream of all three frames
    let mut combined = Vec::new();
    combined.extend_from_slice(&output_80);
    combined.extend_from_slice(&output_120);
    combined.extend_from_slice(&output_40);

    let analysis = analyze_stream(&combined);
    assert_eq!(analysis.stats.total_frames, 3);
    assert_eq!(analysis.stats.complete_frames, 3);
    assert!(analysis.stats.is_flicker_free());
}

#[test]
fn resize_mid_sequence_no_orphan_brackets() {
    let caps = caps_sync();

    // Simulate rapid resize: frame at each size
    let sizes = [(80, 24), (120, 40), (60, 15), (200, 60), (40, 10)];
    let mut all_bytes = Vec::new();

    for (i, &(w, h)) in sizes.iter().enumerate() {
        let old = Buffer::new(w, h);
        let new = random_buffer(w, h, 0xE2E0_0400 + i as u64, 0.5);
        let output = present_frame(&new, &old, caps);
        assert_flicker_free(&output);
        all_bytes.extend_from_slice(&output);
    }

    let analysis = analyze_stream(&all_bytes);
    assert_eq!(analysis.stats.total_frames as usize, sizes.len());
    assert_eq!(analysis.stats.complete_frames as usize, sizes.len());
    assert!(analysis.stats.is_flicker_free());
}

// ============================================================================
// 5. Tracing: render.sync_bracket span emitted with correct fields
// ============================================================================

#[test]
fn sync_bracket_span_emitted_with_fields() {
    let old = Buffer::new(80, 24);
    let new = random_buffer(80, 24, 0xE2E0_0501, 0.5);

    let (output, handle) = with_captured_tracing(|| present_frame(&new, &old, caps_sync()));

    // Verify the output is still valid
    assert_flicker_free(&output);

    let spans = handle.spans();
    let sync_spans: Vec<_> = spans
        .iter()
        .filter(|s| s.name == "render.sync_bracket")
        .collect();

    assert!(!sync_spans.is_empty(), "expected render.sync_bracket span");

    let span = &sync_spans[0];
    assert!(
        span.fields.contains_key("bracket_supported"),
        "missing bracket_supported field"
    );
    assert!(
        span.fields.contains_key("fallback_used"),
        "missing fallback_used field"
    );
}

#[test]
fn sync_bracket_span_shows_fallback_when_no_sync() {
    let old = Buffer::new(80, 24);
    let new = random_buffer(80, 24, 0xE2E0_0502, 0.5);

    let (_output, handle) = with_captured_tracing(|| present_frame(&new, &old, caps_no_sync()));

    let spans = handle.spans();
    let sync_spans: Vec<_> = spans
        .iter()
        .filter(|s| s.name == "render.sync_bracket")
        .collect();

    assert!(!sync_spans.is_empty(), "expected render.sync_bracket span");

    let span = &sync_spans[0];
    assert_eq!(
        span.fields.get("fallback_used").map(String::as_str),
        Some("true"),
        "fallback_used should be true when sync unsupported"
    );
}

// ============================================================================
// 6. No ERROR-level logs during normal operation
// ============================================================================

#[test]
fn no_error_logs_during_normal_render() {
    let old = Buffer::new(80, 24);
    let new = random_buffer(80, 24, 0xE2E0_0601, 0.5);

    let (_output, handle) = with_captured_tracing(|| {
        // Render with sync brackets
        present_frame(&new, &old, caps_sync());

        // Render without sync brackets (fallback)
        present_frame(&new, &old, caps_no_sync());
    });

    let events = handle.events();
    let error_events: Vec<_> = events
        .iter()
        .filter(|e| e.level == tracing::Level::ERROR)
        .collect();

    assert!(
        error_events.is_empty(),
        "no ERROR-level logs expected during normal operation, got {}",
        error_events.len()
    );
}

// ============================================================================
// 7. WARN logged when fallback engaged
// ============================================================================

#[test]
fn warn_logged_when_fallback_engaged() {
    let old = Buffer::new(80, 24);
    let new = random_buffer(80, 24, 0xE2E0_0701, 0.5);

    let (_output, handle) = with_captured_tracing(|| present_frame(&new, &old, caps_no_sync()));

    let events = handle.events();
    let warn_events: Vec<_> = events
        .iter()
        .filter(|e| e.level == tracing::Level::WARN)
        .collect();

    assert!(
        !warn_events.is_empty(),
        "expected WARN event when sync brackets unsupported"
    );

    // Verify the WARN mentions fallback
    let fallback_warns: Vec<_> = warn_events
        .iter()
        .filter(|e| {
            e.fields
                .get("message")
                .is_some_and(|m| m.contains("fallback") || m.contains("cursor-hide"))
        })
        .collect();
    assert!(
        !fallback_warns.is_empty(),
        "WARN should mention fallback strategy"
    );
}

#[test]
fn no_warn_when_sync_brackets_supported() {
    let old = Buffer::new(80, 24);
    let new = random_buffer(80, 24, 0xE2E0_0702, 0.5);

    let (_output, handle) = with_captured_tracing(|| present_frame(&new, &old, caps_sync()));

    let events = handle.events();
    let warn_events: Vec<_> = events
        .iter()
        .filter(|e| e.level == tracing::Level::WARN)
        .collect();

    assert!(
        warn_events.is_empty(),
        "no WARN expected when sync brackets are supported, got {}",
        warn_events.len()
    );
}

// ============================================================================
// 8. Determinism: same input produces identical byte output
// ============================================================================

#[test]
fn deterministic_output_across_runs() {
    let caps = caps_sync();

    let run = || {
        let old = Buffer::new(80, 24);
        let new = random_buffer(80, 24, 0xE2E0_0801, 0.5);
        present_frame(&new, &old, caps)
    };

    let output_1 = run();
    let output_2 = run();

    assert_eq!(
        output_1, output_2,
        "identical inputs must produce identical byte output"
    );
}

#[test]
fn deterministic_flicker_analysis_across_runs() {
    let caps = caps_sync();

    let run = || {
        let old = Buffer::new(80, 24);
        let new = random_buffer(80, 24, 0xE2E0_0802, 0.5);
        let output = present_frame(&new, &old, caps);
        let analysis = analyze_stream(&output);
        (
            output.len(),
            analysis.stats.total_frames,
            analysis.stats.bytes_in_sync,
        )
    };

    let (len_1, frames_1, sync_1) = run();
    let (len_2, frames_2, sync_2) = run();

    assert_eq!(len_1, len_2, "output length must be deterministic");
    assert_eq!(frames_1, frames_2, "frame count must be deterministic");
    assert_eq!(sync_1, sync_2, "sync byte count must be deterministic");
}

// ============================================================================
// 9. JSONL analysis records correct stats
// ============================================================================

#[test]
fn jsonl_analysis_records_frame_stats() {
    let old = Buffer::new(80, 24);
    let new = random_buffer(80, 24, 0xE2E0_0901, 0.5);
    let output = present_frame(&new, &old, caps_sync());

    let analysis = analyze_stream(&output);

    // Verify JSONL is parseable and contains expected fields
    let jsonl = &analysis.jsonl;
    assert!(!jsonl.is_empty(), "JSONL analysis should produce output");

    // Should have at least frame_start and frame_end events
    let lines: Vec<&str> = jsonl.lines().collect();
    assert!(
        lines.len() >= 2,
        "expected at least 2 JSONL lines (frame_start + frame_end), got {}",
        lines.len()
    );
}

// ============================================================================
// 10. Full E2E lifecycle scenario
// ============================================================================

#[test]
fn full_e2e_lifecycle_sync_fallback_resize() {
    // Phase 1: Normal render with sync brackets
    let (sync_output, sync_handle) = with_captured_tracing(|| {
        let old = Buffer::new(80, 24);
        let new = random_buffer(80, 24, 0xE2E0_1001, 0.5);
        present_frame(&new, &old, caps_sync())
    });
    assert_flicker_free(&sync_output);

    // Phase 2: Fallback render
    let (_fallback_output, fallback_handle) = with_captured_tracing(|| {
        let old = Buffer::new(80, 24);
        let new = random_buffer(80, 24, 0xE2E0_1002, 0.5);
        present_frame(&new, &old, caps_no_sync())
    });

    // Phase 3: Resize and render
    let (resize_output, resize_handle) = with_captured_tracing(|| {
        let old = Buffer::new(120, 40);
        let new = random_buffer(120, 40, 0xE2E0_1003, 0.6);
        present_frame(&new, &old, caps_sync())
    });
    assert_flicker_free(&resize_output);

    // Assert: sync bracket spans in all phases
    for (name, handle) in [
        ("sync", &sync_handle),
        ("fallback", &fallback_handle),
        ("resize", &resize_handle),
    ] {
        let spans = handle.spans();
        let sync_spans: Vec<_> = spans
            .iter()
            .filter(|s| s.name == "render.sync_bracket")
            .collect();
        if sync_spans.is_empty() {
            println!(
                "Spans for phase {}: {:?}",
                name,
                spans.iter().map(|s| &s.name).collect::<Vec<_>>()
            );
        }
        assert!(
            !sync_spans.is_empty(),
            "phase '{name}' should emit render.sync_bracket span"
        );
    }

    // Assert: no ERROR events in any phase
    for (name, handle) in [
        ("sync", &sync_handle),
        ("fallback", &fallback_handle),
        ("resize", &resize_handle),
    ] {
        let events = handle.events();
        let errors: Vec<_> = events
            .iter()
            .filter(|e| e.level == tracing::Level::ERROR)
            .collect();
        assert!(
            errors.is_empty(),
            "phase '{name}' should have no ERROR events"
        );
    }

    // Assert: WARN only in fallback phase
    let fallback_warns: Vec<_> = fallback_handle
        .events()
        .iter()
        .filter(|e| e.level == tracing::Level::WARN)
        .cloned()
        .collect();
    assert!(
        !fallback_warns.is_empty(),
        "fallback phase should emit WARN"
    );

    let sync_warns: Vec<_> = sync_handle
        .events()
        .iter()
        .filter(|e| e.level == tracing::Level::WARN)
        .cloned()
        .collect();
    assert!(
        sync_warns.is_empty(),
        "sync phase should have no WARN events"
    );
}
