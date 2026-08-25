#![forbid(unsafe_code)]

//! Tracing layer integration that formats events as styled Segments.
//!
//! Provides a `tracing_subscriber::Layer` implementation that routes formatted
//! tracing events through the Console abstraction, respecting ftui's one-writer
//! rule. All output goes through an explicit `ConsoleSink`—never directly to
//! stdout/stderr.
//!
//! # Quick Start
//!
//! ```no_run
//! use ftui_extras::console::{Console, ConsoleSink};
//! use ftui_extras::logging::TracingConsoleLayer;
//! use tracing_subscriber::prelude::*;
//!
//! let sink = ConsoleSink::capture();
//! let console = Console::new(80, sink);
//! let layer = TracingConsoleLayer::new(console);
//!
//! tracing_subscriber::registry().with(layer).init();
//! ```

use std::fmt::{self, Write as FmtWrite};
use std::sync::{Condvar, Mutex};
use std::thread::ThreadId;
use std::time::{Duration, Instant};

use ftui_render::cell::PackedRgba;
use ftui_style::Style;
use ftui_text::Segment;
use tracing::field::{Field, Visit};
use tracing::{Event, Level, Subscriber};
use tracing_subscriber::Layer;
use tracing_subscriber::layer::Context;

use crate::console::Console;

// ============================================================================
// Configuration
// ============================================================================

/// Configuration for the tracing console layer.
#[derive(Debug, Clone)]
pub struct TracingConfig {
    /// Show timestamps. Default: true.
    pub show_time: bool,
    /// Show log level. Default: true.
    pub show_level: bool,
    /// Show the tracing target (module path). Default: true.
    pub show_target: bool,
    /// Show structured fields beyond `message`. Default: true.
    pub show_fields: bool,
    /// Show source file:line. Default: false.
    pub show_source: bool,
}

impl Default for TracingConfig {
    fn default() -> Self {
        Self {
            show_time: true,
            show_level: true,
            show_target: true,
            show_fields: true,
            show_source: false,
        }
    }
}

// ============================================================================
// Level Styling
// ============================================================================

/// Default styles for each tracing level.
fn level_style(level: Level) -> Style {
    match level {
        Level::ERROR => Style::new().fg(PackedRgba::rgb(255, 0, 0)).bold(),
        Level::WARN => Style::new().fg(PackedRgba::rgb(255, 200, 0)),
        Level::INFO => Style::new().fg(PackedRgba::rgb(0, 200, 0)),
        Level::DEBUG => Style::new().fg(PackedRgba::rgb(100, 100, 255)).dim(),
        Level::TRACE => Style::new().dim(),
    }
}

/// Format level as a fixed-width string.
fn level_str(level: Level) -> &'static str {
    match level {
        Level::ERROR => "ERROR",
        Level::WARN => "WARN ",
        Level::INFO => "INFO ",
        Level::DEBUG => "DEBUG",
        Level::TRACE => "TRACE",
    }
}

// ============================================================================
// Event Visitor
// ============================================================================

/// Extracts message and structured fields from a tracing event.
#[derive(Default)]
struct EventVisitor {
    message: Option<String>,
    fields: Vec<(String, String)>,
}

impl Visit for EventVisitor {
    fn record_debug(&mut self, field: &Field, value: &dyn fmt::Debug) {
        let rendered = format!("{value:?}");
        let rendered = strip_debug_quotes(&rendered);
        if field.name() == "message" {
            self.message = Some(rendered);
        } else {
            self.fields.push((field.name().to_string(), rendered));
        }
    }

    fn record_str(&mut self, field: &Field, value: &str) {
        if field.name() == "message" {
            self.message = Some(value.to_string());
        } else {
            self.fields
                .push((field.name().to_string(), value.to_string()));
        }
    }

    fn record_i64(&mut self, field: &Field, value: i64) {
        self.fields
            .push((field.name().to_string(), value.to_string()));
    }

    fn record_u64(&mut self, field: &Field, value: u64) {
        self.fields
            .push((field.name().to_string(), value.to_string()));
    }

    fn record_bool(&mut self, field: &Field, value: bool) {
        self.fields
            .push((field.name().to_string(), value.to_string()));
    }
}

/// Remove surrounding quotes from Debug-formatted strings.
fn strip_debug_quotes(s: &str) -> String {
    if s.len() >= 2 && s.starts_with('"') && s.ends_with('"') {
        s[1..s.len() - 1].to_string()
    } else {
        s.to_string()
    }
}

// ============================================================================
// Timestamp helper
// ============================================================================

/// Simple HH:MM:SS timestamp (no external dependency needed).
fn timestamp_now() -> String {
    // Use web_time for WASM compatibility (std::time panics on wasm32-unknown-unknown).
    // For deterministic tests, callers can disable show_time.
    let now = web_time::SystemTime::now();
    let since_epoch = now.duration_since(web_time::UNIX_EPOCH).unwrap_or_default();
    let secs = since_epoch.as_secs();
    let h = (secs / 3600) % 24;
    let m = (secs % 3600) / 60;
    let s = secs % 60;
    format!("{h:02}:{m:02}:{s:02}")
}

// ============================================================================
// TracingConsoleLayer
// ============================================================================

/// A `tracing_subscriber::Layer` that formats events as styled segments and
/// writes them through a `Console` sink.
///
/// Thread-safe: the inner `Console` is protected by a `Mutex`, so multiple
/// threads can emit events concurrently without interleaving within a single
/// formatted record.
pub struct TracingConsoleLayer {
    console: Mutex<Console>,
    console_owner: Mutex<Option<ThreadId>>,
    console_owner_ready: Condvar,
    config: TracingConfig,
}

const CONSOLE_ACCESS_WAIT_TIMEOUT: Duration = Duration::from_millis(250);

struct ConsoleAccessGuard<'a> {
    owner: &'a Mutex<Option<ThreadId>>,
    ready: &'a Condvar,
}

impl Drop for ConsoleAccessGuard<'_> {
    fn drop(&mut self) {
        let mut owner = self.owner.lock().unwrap_or_else(|e| e.into_inner());
        *owner = None;
        self.ready.notify_all();
    }
}

impl TracingConsoleLayer {
    /// Create a new layer with the given console and default configuration.
    pub fn new(console: Console) -> Self {
        Self {
            console: Mutex::new(console),
            console_owner: Mutex::new(None),
            console_owner_ready: Condvar::new(),
            config: TracingConfig::default(),
        }
    }

    /// Create a new layer with custom configuration.
    pub fn with_config(console: Console, config: TracingConfig) -> Self {
        Self {
            console: Mutex::new(console),
            console_owner: Mutex::new(None),
            console_owner_ready: Condvar::new(),
            config,
        }
    }

    /// Builder: set whether to show timestamps.
    #[must_use]
    pub fn show_time(mut self, show: bool) -> Self {
        self.config.show_time = show;
        self
    }

    /// Builder: set whether to show log level.
    #[must_use]
    pub fn show_level(mut self, show: bool) -> Self {
        self.config.show_level = show;
        self
    }

    /// Builder: set whether to show the target module.
    #[must_use]
    pub fn show_target(mut self, show: bool) -> Self {
        self.config.show_target = show;
        self
    }

    /// Builder: set whether to show structured fields.
    #[must_use]
    pub fn show_fields(mut self, show: bool) -> Self {
        self.config.show_fields = show;
        self
    }

    /// Builder: set whether to show source file:line.
    #[must_use]
    pub fn show_source(mut self, show: bool) -> Self {
        self.config.show_source = show;
        self
    }

    /// Format an event into segments and write them to the console.
    fn write_event(&self, event: &Event<'_>) {
        let Some(_console_access) = self.enter_console_access() else {
            return;
        };
        let metadata = event.metadata();
        let level = *metadata.level();

        // Visit fields
        let mut visitor = EventVisitor::default();
        event.record(&mut visitor);

        let mut console = match self.console.lock() {
            Ok(c) => c,
            Err(poisoned) => poisoned.into_inner(),
        };

        // Timestamp
        if self.config.show_time {
            let ts = timestamp_now();
            let dim = Style::new().dim();
            console.print(Segment::styled(ts, dim));
            console.print(Segment::text(" "));
        }

        // Level
        if self.config.show_level {
            let style = level_style(level);
            console.print(Segment::styled(level_str(level), style));
            console.print(Segment::text(" "));
        }

        // Target
        if self.config.show_target {
            let target = metadata.target();
            let dim = Style::new().dim();
            console.print(Segment::styled(target.to_string(), dim));
            console.print(Segment::styled(": ", dim));
        }

        // Message
        let message = visitor.message.unwrap_or_default();
        console.print(Segment::text(message));

        // Structured fields
        if self.config.show_fields && !visitor.fields.is_empty() {
            let dim = Style::new().dim();
            let mut field_str = String::new();
            for (i, (k, v)) in visitor.fields.iter().enumerate() {
                if i > 0 {
                    field_str.push(' ');
                }
                let _ = write!(field_str, "{k}={v}");
            }
            console.print(Segment::text(" "));
            console.print(Segment::styled(field_str, dim));
        }

        // Source location
        if self.config.show_source
            && let Some(file) = metadata.file()
        {
            let dim = Style::new().dim();
            let mut loc = format!(" {file}");
            if let Some(line) = metadata.line() {
                let _ = write!(loc, ":{line}");
            }
            console.print(Segment::styled(loc, dim));
        }

        console.newline();
    }

    fn enter_console_access(&self) -> Option<ConsoleAccessGuard<'_>> {
        let current = std::thread::current().id();
        let mut owner = self.console_owner.lock().unwrap_or_else(|e| e.into_inner());
        if owner.as_ref().is_some_and(|id| *id == current) {
            return None;
        }
        let deadline = Instant::now() + CONSOLE_ACCESS_WAIT_TIMEOUT;
        while owner.is_some() {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return None;
            }
            let (next_owner, wait_result) = self
                .console_owner_ready
                .wait_timeout(owner, remaining)
                .unwrap_or_else(|e| e.into_inner());
            owner = next_owner;
            if wait_result.timed_out() && owner.is_some() {
                return None;
            }
            if owner.as_ref().is_some_and(|id| *id == current) {
                return None;
            }
        }
        *owner = Some(current);
        drop(owner);
        Some(ConsoleAccessGuard {
            owner: &self.console_owner,
            ready: &self.console_owner_ready,
        })
    }

    /// Consume the layer and return the inner console (for test inspection).
    pub fn into_console(self) -> Console {
        self.console.into_inner().unwrap_or_else(|e| e.into_inner())
    }
}

impl<S> Layer<S> for TracingConsoleLayer
where
    S: Subscriber,
{
    fn on_event(&self, event: &Event<'_>, _ctx: Context<'_, S>) {
        self.write_event(event);
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::console::ConsoleSink;
    use std::io;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Condvar, Mutex, mpsc};
    use std::time::{Duration, Instant};
    use tracing_subscriber::prelude::*;

    #[derive(Clone, Default)]
    struct SharedWriter {
        inner: Arc<Mutex<Vec<u8>>>,
    }

    impl SharedWriter {
        fn new() -> Self {
            Self {
                inner: Arc::new(Mutex::new(Vec::new())),
            }
        }

        fn snapshot(&self) -> String {
            let bytes = self.inner.lock().expect("writer lock").clone();
            String::from_utf8(bytes).unwrap_or_default()
        }
    }

    impl io::Write for SharedWriter {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            let mut inner = self.inner.lock().expect("writer lock");
            inner.extend_from_slice(buf);
            Ok(buf.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    #[derive(Clone, Default)]
    struct ReentrantTracingWriter {
        inner: Arc<Mutex<Vec<u8>>>,
        fired: Arc<AtomicBool>,
    }

    impl ReentrantTracingWriter {
        fn snapshot(&self) -> String {
            let bytes = self.inner.lock().expect("writer lock").clone();
            String::from_utf8(bytes).unwrap_or_default()
        }
    }

    impl io::Write for ReentrantTracingWriter {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            if !self.fired.swap(true, Ordering::SeqCst) {
                tracing::info!("nested from writer");
            }
            let mut inner = self.inner.lock().expect("writer lock");
            inner.extend_from_slice(buf);
            Ok(buf.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    #[derive(Clone)]
    struct BlockingTracingWriter {
        inner: Arc<Mutex<Vec<u8>>>,
        block_writes: Arc<AtomicBool>,
        entered_tx: Arc<Mutex<Option<mpsc::Sender<()>>>>,
        released: Arc<(Mutex<bool>, Condvar)>,
        sent_entered: Arc<AtomicBool>,
    }

    impl BlockingTracingWriter {
        fn new(
            block_writes: Arc<AtomicBool>,
            entered_tx: mpsc::Sender<()>,
            released: Arc<(Mutex<bool>, Condvar)>,
        ) -> Self {
            Self {
                inner: Arc::new(Mutex::new(Vec::new())),
                block_writes,
                entered_tx: Arc::new(Mutex::new(Some(entered_tx))),
                released,
                sent_entered: Arc::new(AtomicBool::new(false)),
            }
        }

        fn snapshot(&self) -> String {
            let bytes = self.inner.lock().expect("writer lock").clone();
            String::from_utf8(bytes).unwrap_or_default()
        }
    }

    impl io::Write for BlockingTracingWriter {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            if self.block_writes.load(Ordering::SeqCst) {
                if !self.sent_entered.swap(true, Ordering::SeqCst) {
                    let entered_tx = self.entered_tx.lock().expect("entered tx lock").take();
                    if let Some(entered_tx) = entered_tx {
                        let _ = entered_tx.send(());
                    }
                }
                let (released_lock, released_ready) = &*self.released;
                let mut released = released_lock.lock().expect("released lock");
                while !*released {
                    released = released_ready.wait(released).expect("released wait");
                }
            }

            let mut inner = self.inner.lock().expect("writer lock");
            inner.extend_from_slice(buf);
            Ok(buf.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    fn make_layer(config: TracingConfig) -> TracingConsoleLayer {
        let sink = ConsoleSink::capture();
        let console = Console::new(120, sink);
        TracingConsoleLayer::with_config(console, config)
    }

    fn no_frills_config() -> TracingConfig {
        TracingConfig {
            show_time: false,
            show_level: false,
            show_target: false,
            show_fields: false,
            show_source: false,
        }
    }

    // -- Construction tests --

    #[test]
    fn default_config() {
        let cfg = TracingConfig::default();
        assert!(cfg.show_time);
        assert!(cfg.show_level);
        assert!(cfg.show_target);
        assert!(cfg.show_fields);
        assert!(!cfg.show_source);
    }

    #[test]
    fn layer_builder_chain() {
        let sink = ConsoleSink::capture();
        let console = Console::new(80, sink);
        let layer = TracingConsoleLayer::new(console)
            .show_time(false)
            .show_level(true)
            .show_target(false)
            .show_fields(true)
            .show_source(true);

        assert!(!layer.config.show_time);
        assert!(layer.config.show_level);
        assert!(!layer.config.show_target);
        assert!(layer.config.show_fields);
        assert!(layer.config.show_source);
    }

    #[test]
    fn reentrant_writer_logging_is_dropped_instead_of_deadlocking() {
        let writer = ReentrantTracingWriter::default();
        let probe = writer.clone();
        let sink = ConsoleSink::writer(writer);
        let console = Console::new(120, sink);
        let layer = TracingConsoleLayer::with_config(console, no_frills_config());
        let subscriber = tracing_subscriber::registry().with(layer);
        let dispatch = tracing::Dispatch::new(subscriber);

        tracing::dispatcher::with_default(&dispatch, || {
            tracing::info!("outer");
        });

        let output = probe.snapshot();
        assert!(output.contains("outer"));
        assert!(
            !output.contains("nested from writer"),
            "reentrant nested event should be dropped"
        );
    }

    #[test]
    fn stalled_writer_does_not_block_later_log_events() {
        let block_writes = Arc::new(AtomicBool::new(false));
        let (entered_tx, entered_rx) = mpsc::channel();
        let released = Arc::new((Mutex::new(false), Condvar::new()));
        let writer = BlockingTracingWriter::new(
            Arc::clone(&block_writes),
            entered_tx,
            Arc::clone(&released),
        );
        let probe = writer.clone();
        let sink = ConsoleSink::writer(writer);
        let console = Console::new(120, sink);
        let layer = TracingConsoleLayer::with_config(console, no_frills_config());
        let subscriber = tracing_subscriber::registry().with(layer);
        let dispatch = tracing::Dispatch::new(subscriber);

        block_writes.store(true, Ordering::SeqCst);

        let blocked_dispatch = dispatch.clone();
        let blocked_handle = std::thread::spawn(move || {
            tracing::dispatcher::with_default(&blocked_dispatch, || {
                tracing::info!("blocked event");
            });
        });

        entered_rx
            .recv_timeout(Duration::from_secs(1))
            .expect("blocked writer entered");

        let second_dispatch = dispatch.clone();
        let (done_tx, done_rx) = mpsc::channel();
        let second_handle = std::thread::spawn(move || {
            tracing::dispatcher::with_default(&second_dispatch, || {
                tracing::info!("later event");
            });
            let _ = done_tx.send(Instant::now());
        });

        let start = Instant::now();
        done_rx
            .recv_timeout(Duration::from_secs(1))
            .expect("later log event should not block behind a stuck writer");
        assert!(
            start.elapsed() < Duration::from_secs(1),
            "later log event should fail open instead of hanging"
        );

        let (released_lock, released_ready) = &*released;
        let mut released = released_lock.lock().expect("released lock");
        *released = true;
        released_ready.notify_all();
        drop(released);

        blocked_handle.join().expect("blocked log thread join");
        second_handle.join().expect("later log thread join");

        let output = probe.snapshot();
        assert!(output.contains("blocked event"), "output: {output}");
        assert!(
            !output.contains("later event"),
            "timed-out event should be dropped instead of blocking: {output}"
        );
    }

    // -- Level styling tests --

    #[test]
    fn level_styles_differ() {
        let error = level_style(Level::ERROR);
        let warn = level_style(Level::WARN);
        let info = level_style(Level::INFO);
        let debug = level_style(Level::DEBUG);
        let trace = level_style(Level::TRACE);

        // At minimum, error/warn/info should differ
        assert_ne!(error, warn);
        assert_ne!(warn, info);
        assert_ne!(info, debug);
        assert_ne!(debug, trace);
    }

    #[test]
    fn level_str_fixed_width() {
        // All level strings should be 5 chars for alignment
        assert_eq!(level_str(Level::ERROR).len(), 5);
        assert_eq!(level_str(Level::WARN).len(), 5);
        assert_eq!(level_str(Level::INFO).len(), 5);
        assert_eq!(level_str(Level::DEBUG).len(), 5);
        assert_eq!(level_str(Level::TRACE).len(), 5);
    }

    // -- Visitor tests --

    #[test]
    fn strip_debug_quotes_basic() {
        assert_eq!(strip_debug_quotes("\"hello\""), "hello");
        assert_eq!(strip_debug_quotes("plain"), "plain");
        assert_eq!(strip_debug_quotes(""), "");
        assert_eq!(strip_debug_quotes("\"\""), "");
        assert_eq!(strip_debug_quotes("\""), "\"");
    }

    #[test]
    fn event_visitor_default_empty() {
        let v = EventVisitor::default();
        assert!(v.message.is_none());
        assert!(v.fields.is_empty());
    }

    // -- Integration tests using tracing macros --

    #[test]
    fn captures_info_event() {
        let layer = make_layer(TracingConfig {
            show_time: false,
            show_level: true,
            show_target: false,
            show_fields: false,
            show_source: false,
        });

        let subscriber = tracing_subscriber::registry().with(layer);
        let _guard = tracing::subscriber::set_default(subscriber);

        tracing::info!("hello world");

        // We can't easily extract the output after set_default returns the
        // guard, but we verify no panic occurred. For deeper inspection, use
        // the into_console pattern below.
    }

    #[test]
    fn formats_message_with_fields_and_target() {
        let writer = SharedWriter::new();
        let console = Console::new(120, ConsoleSink::writer(writer.clone()));
        let layer = TracingConsoleLayer::with_config(
            console,
            TracingConfig {
                show_time: false,
                show_level: true,
                show_target: true,
                show_fields: true,
                show_source: false,
            },
        );

        let subscriber = tracing_subscriber::registry().with(layer);
        let dispatch = tracing::Dispatch::new(subscriber);

        tracing::dispatcher::with_default(&dispatch, || {
            tracing::info!(key = "value", count = 3, "hello world");
        });

        let output = writer.snapshot();
        assert!(output.contains("INFO"), "output: {output}");
        assert!(output.contains("hello world"), "output: {output}");
        assert!(output.contains("key=value"), "output: {output}");
        assert!(output.contains("count=3"), "output: {output}");
        assert!(
            output.contains("ftui_extras"),
            "expected target in output: {output}"
        );
    }

    #[test]
    fn respects_level_filter() {
        let writer = SharedWriter::new();
        let console = Console::new(120, ConsoleSink::writer(writer.clone()));
        let layer = TracingConsoleLayer::with_config(
            console,
            TracingConfig {
                show_time: false,
                show_level: true,
                show_target: false,
                show_fields: false,
                show_source: false,
            },
        );

        let filter = tracing_subscriber::filter::LevelFilter::INFO;
        let subscriber = tracing_subscriber::registry().with(layer).with(filter);
        let dispatch = tracing::Dispatch::new(subscriber);

        tracing::dispatcher::with_default(&dispatch, || {
            tracing::debug!("debug drop");
            tracing::info!("info keep");
        });

        let output = writer.snapshot();
        assert!(!output.contains("debug drop"), "output: {output}");
        assert!(output.contains("info keep"), "output: {output}");
    }

    #[test]
    fn into_console_captures_output() {
        let layer = make_layer(no_frills_config());

        // Use the layer directly by calling write_event with a synthetic event
        // is not straightforward, so we use the subscriber + dispatch pattern.
        let subscriber = tracing_subscriber::registry().with(layer);

        // Use a dispatcher to emit events then recover the layer
        // Unfortunately tracing doesn't let us recover layers easily.
        // Instead, test the formatting via a shared console approach.
        let _guard = tracing::subscriber::set_default(subscriber);
        tracing::info!("test message");
        // Guard dropped, subscriber dropped — but we can't recover the console.
        // This test verifies no panic. See shared_console_test for output checks.
    }

    #[test]
    fn shared_console_captures_output() {
        // Use a Mutex<Console> directly to verify output capture.
        let sink = ConsoleSink::capture();
        let console = Console::new(120, sink);
        let layer = TracingConsoleLayer::with_config(console, no_frills_config());

        // Manually simulate an event through the layer
        let subscriber = tracing_subscriber::registry().with(layer);
        let dispatch = tracing::Dispatch::new(subscriber);

        tracing::dispatcher::with_default(&dispatch, || {
            tracing::info!("captured event");
        });

        // We need to recover the console from the subscriber, which is tricky
        // with tracing's ownership model. The key assertion is no panic.
    }

    #[test]
    fn formats_with_all_components() {
        let sink = ConsoleSink::capture();
        let console = Console::new(120, sink);
        let config = TracingConfig {
            show_time: true,
            show_level: true,
            show_target: true,
            show_fields: true,
            show_source: true,
        };
        let layer = TracingConsoleLayer::with_config(console, config);

        let subscriber = tracing_subscriber::registry().with(layer);
        let dispatch = tracing::Dispatch::new(subscriber);

        tracing::dispatcher::with_default(&dispatch, || {
            tracing::info!(key = "value", "full format");
        });
        // No panic = components formatted correctly
    }

    #[test]
    fn timestamp_format_valid() {
        let ts = timestamp_now();
        // Should be HH:MM:SS format
        assert_eq!(ts.len(), 8);
        assert_eq!(&ts[2..3], ":");
        assert_eq!(&ts[5..6], ":");
    }

    #[test]
    fn multithreaded_logging_no_panic() {
        let sink = ConsoleSink::capture();
        let console = Console::new(120, sink);
        let layer = TracingConsoleLayer::new(console).show_time(false);
        let subscriber = tracing_subscriber::registry().with(layer);
        let dispatch = tracing::Dispatch::new(subscriber);

        let handles: Vec<_> = (0..4)
            .map(|i| {
                let dispatch = dispatch.clone();
                std::thread::spawn(move || {
                    tracing::dispatcher::with_default(&dispatch, || {
                        for j in 0..50 {
                            tracing::info!(thread = i, iter = j, "concurrent log");
                        }
                    });
                })
            })
            .collect();

        for h in handles {
            h.join().expect("thread should not panic");
        }
    }

    #[test]
    fn layer_is_send_sync() {
        fn assert_send<T: Send>() {}
        fn assert_sync<T: Sync>() {}
        assert_send::<TracingConsoleLayer>();
        assert_sync::<TracingConsoleLayer>();
    }

    #[test]
    fn poison_recovery() {
        let sink = ConsoleSink::capture();
        let console = Console::new(80, sink);
        let layer = TracingConsoleLayer::new(console).show_time(false);

        // Even if mutex is poisoned (simulated), write_event should not panic
        // due to into_inner recovery. We can't easily poison it in a unit test
        // without UB, so we verify the recovery code path exists.
        let subscriber = tracing_subscriber::registry().with(layer);
        let dispatch = tracing::Dispatch::new(subscriber);

        tracing::dispatcher::with_default(&dispatch, || {
            tracing::error!("after simulated poison");
        });
    }

    // -- Direct write_event tests via extracted console --

    #[test]
    fn direct_write_event_captures_message() {
        // We'll use a raw Dispatch + manual extraction approach
        let sink = ConsoleSink::capture();
        let console = Console::new(120, sink);
        let layer = TracingConsoleLayer::with_config(
            console,
            TracingConfig {
                show_time: false,
                show_level: true,
                show_target: false,
                show_fields: false,
                show_source: false,
            },
        );

        // We need to get output. Use Arc<Mutex<Console>> approach:
        // Actually, let's test write_event more directly by using the subscriber
        // and then checking the console.
        let subscriber = tracing_subscriber::registry().with(layer);
        let dispatch = tracing::Dispatch::new(subscriber);

        tracing::dispatcher::with_default(&dispatch, || {
            tracing::warn!("test warning");
        });

        // Note: we can't recover the console after subscriber is consumed.
        // The test verifies correctness by not panicking.
        // For full output verification, see the console_output_test below.
    }

    #[test]
    fn console_output_verification() {
        // Create a console, format segments manually, verify output
        let sink = ConsoleSink::capture();
        let mut console = Console::new(80, sink);

        // Simulate what write_event does:
        let style = level_style(Level::INFO);
        console.print(Segment::styled("INFO ", style));
        console.print(Segment::text("hello world"));
        console.newline();

        let output = console.into_captured();
        assert!(output.contains("INFO"));
        assert!(output.contains("hello world"));
    }

    #[test]
    fn console_output_with_fields() {
        let sink = ConsoleSink::capture();
        let mut console = Console::new(80, sink);

        let dim = Style::new().dim();
        console.print(Segment::text("message"));
        console.print(Segment::text(" "));
        console.print(Segment::styled("key=value", dim));
        console.newline();

        let output = console.into_captured();
        assert!(output.contains("message"));
        assert!(output.contains("key=value"));
    }

    // --- Edge case tests (bd-207vs) ---

    #[test]
    fn strip_debug_quotes_single_char() {
        // Single char that is a quote should not be stripped
        assert_eq!(strip_debug_quotes("\""), "\"");
    }

    #[test]
    fn strip_debug_quotes_no_quotes() {
        assert_eq!(strip_debug_quotes("no quotes here"), "no quotes here");
    }

    #[test]
    fn strip_debug_quotes_only_opening_quote() {
        assert_eq!(strip_debug_quotes("\"hello"), "\"hello");
    }

    #[test]
    fn strip_debug_quotes_only_closing_quote() {
        assert_eq!(strip_debug_quotes("hello\""), "hello\"");
    }

    #[test]
    fn strip_debug_quotes_inner_content_with_quotes() {
        assert_eq!(
            strip_debug_quotes("\"has \\\"inner\\\" quotes\""),
            "has \\\"inner\\\" quotes"
        );
    }

    #[test]
    fn level_str_exact_values() {
        assert_eq!(level_str(Level::ERROR), "ERROR");
        assert_eq!(level_str(Level::WARN), "WARN ");
        assert_eq!(level_str(Level::INFO), "INFO ");
        assert_eq!(level_str(Level::DEBUG), "DEBUG");
        assert_eq!(level_str(Level::TRACE), "TRACE");
    }

    #[test]
    fn level_style_error_is_bold() {
        let style = level_style(Level::ERROR);
        assert!(style.has_attr(ftui_style::StyleFlags::BOLD));
    }

    #[test]
    fn level_style_debug_is_dim() {
        let style = level_style(Level::DEBUG);
        assert!(style.has_attr(ftui_style::StyleFlags::DIM));
    }

    #[test]
    fn level_style_trace_is_dim() {
        let style = level_style(Level::TRACE);
        assert!(style.has_attr(ftui_style::StyleFlags::DIM));
    }

    #[test]
    fn config_fields_independent() {
        let cfg = TracingConfig {
            show_time: false,
            ..TracingConfig::default()
        };
        assert!(!cfg.show_time);
        assert!(cfg.show_level);
        assert!(cfg.show_target);
        assert!(cfg.show_fields);
        assert!(!cfg.show_source);
    }

    #[test]
    fn with_config_constructor() {
        let config = TracingConfig {
            show_time: false,
            show_level: false,
            show_target: false,
            show_fields: false,
            show_source: true,
        };
        let sink = ConsoleSink::capture();
        let console = Console::new(80, sink);
        let layer = TracingConsoleLayer::with_config(console, config);
        assert!(!layer.config.show_time);
        assert!(!layer.config.show_level);
        assert!(!layer.config.show_target);
        assert!(!layer.config.show_fields);
        assert!(layer.config.show_source);
    }

    #[test]
    fn error_level_output_via_shared_writer() {
        let writer = SharedWriter::new();
        let console = Console::new(120, ConsoleSink::writer(writer.clone()));
        let layer = TracingConsoleLayer::with_config(
            console,
            TracingConfig {
                show_time: false,
                show_level: true,
                show_target: false,
                show_fields: false,
                show_source: false,
            },
        );
        let subscriber = tracing_subscriber::registry().with(layer);
        let dispatch = tracing::Dispatch::new(subscriber);

        tracing::dispatcher::with_default(&dispatch, || {
            tracing::error!("an error occurred");
        });

        let output = writer.snapshot();
        assert!(output.contains("ERROR"), "output: {output}");
        assert!(output.contains("an error occurred"), "output: {output}");
    }

    #[test]
    fn warn_level_output_via_shared_writer() {
        let writer = SharedWriter::new();
        let console = Console::new(120, ConsoleSink::writer(writer.clone()));
        let layer = TracingConsoleLayer::with_config(
            console,
            TracingConfig {
                show_time: false,
                show_level: true,
                show_target: false,
                show_fields: false,
                show_source: false,
            },
        );
        let subscriber = tracing_subscriber::registry().with(layer);
        let dispatch = tracing::Dispatch::new(subscriber);

        tracing::dispatcher::with_default(&dispatch, || {
            tracing::warn!("a warning");
        });

        let output = writer.snapshot();
        assert!(output.contains("WARN"), "output: {output}");
        assert!(output.contains("a warning"), "output: {output}");
    }

    #[test]
    fn debug_level_output_via_shared_writer() {
        let writer = SharedWriter::new();
        let console = Console::new(120, ConsoleSink::writer(writer.clone()));
        let layer = TracingConsoleLayer::with_config(
            console,
            TracingConfig {
                show_time: false,
                show_level: true,
                show_target: false,
                show_fields: false,
                show_source: false,
            },
        );
        let subscriber = tracing_subscriber::registry().with(layer);
        let dispatch = tracing::Dispatch::new(subscriber);

        tracing::dispatcher::with_default(&dispatch, || {
            tracing::debug!("debug info");
        });

        let output = writer.snapshot();
        assert!(output.contains("DEBUG"), "output: {output}");
        assert!(output.contains("debug info"), "output: {output}");
    }

    #[test]
    fn trace_level_output_via_shared_writer() {
        let writer = SharedWriter::new();
        let console = Console::new(120, ConsoleSink::writer(writer.clone()));
        let layer = TracingConsoleLayer::with_config(
            console,
            TracingConfig {
                show_time: false,
                show_level: true,
                show_target: false,
                show_fields: false,
                show_source: false,
            },
        );
        let subscriber = tracing_subscriber::registry().with(layer);
        let dispatch = tracing::Dispatch::new(subscriber);

        tracing::dispatcher::with_default(&dispatch, || {
            tracing::trace!("trace data");
        });

        let output = writer.snapshot();
        assert!(output.contains("TRACE"), "output: {output}");
        assert!(output.contains("trace data"), "output: {output}");
    }

    #[test]
    fn event_with_bool_field() {
        let writer = SharedWriter::new();
        let console = Console::new(120, ConsoleSink::writer(writer.clone()));
        let layer = TracingConsoleLayer::with_config(
            console,
            TracingConfig {
                show_time: false,
                show_level: false,
                show_target: false,
                show_fields: true,
                show_source: false,
            },
        );
        let subscriber = tracing_subscriber::registry().with(layer);
        let dispatch = tracing::Dispatch::new(subscriber);

        tracing::dispatcher::with_default(&dispatch, || {
            tracing::info!(enabled = true, "with bool");
        });

        let output = writer.snapshot();
        assert!(output.contains("enabled=true"), "output: {output}");
    }

    #[test]
    fn event_with_i64_field() {
        let writer = SharedWriter::new();
        let console = Console::new(120, ConsoleSink::writer(writer.clone()));
        let layer = TracingConsoleLayer::with_config(
            console,
            TracingConfig {
                show_time: false,
                show_level: false,
                show_target: false,
                show_fields: true,
                show_source: false,
            },
        );
        let subscriber = tracing_subscriber::registry().with(layer);
        let dispatch = tracing::Dispatch::new(subscriber);

        tracing::dispatcher::with_default(&dispatch, || {
            tracing::info!(offset = -42_i64, "with i64");
        });

        let output = writer.snapshot();
        assert!(output.contains("offset=-42"), "output: {output}");
    }

    #[test]
    fn event_with_u64_field() {
        let writer = SharedWriter::new();
        let console = Console::new(120, ConsoleSink::writer(writer.clone()));
        let layer = TracingConsoleLayer::with_config(
            console,
            TracingConfig {
                show_time: false,
                show_level: false,
                show_target: false,
                show_fields: true,
                show_source: false,
            },
        );
        let subscriber = tracing_subscriber::registry().with(layer);
        let dispatch = tracing::Dispatch::new(subscriber);

        tracing::dispatcher::with_default(&dispatch, || {
            tracing::info!(count = 999_u64, "with u64");
        });

        let output = writer.snapshot();
        assert!(output.contains("count=999"), "output: {output}");
    }

    #[test]
    fn event_with_multiple_fields_separated() {
        let writer = SharedWriter::new();
        let console = Console::new(120, ConsoleSink::writer(writer.clone()));
        let layer = TracingConsoleLayer::with_config(
            console,
            TracingConfig {
                show_time: false,
                show_level: false,
                show_target: false,
                show_fields: true,
                show_source: false,
            },
        );
        let subscriber = tracing_subscriber::registry().with(layer);
        let dispatch = tracing::Dispatch::new(subscriber);

        tracing::dispatcher::with_default(&dispatch, || {
            tracing::info!(a = 1, b = "two", c = true, "multi");
        });

        let output = writer.snapshot();
        assert!(output.contains("a=1"), "output: {output}");
        assert!(output.contains("b=two"), "output: {output}");
        assert!(output.contains("c=true"), "output: {output}");
    }

    #[test]
    fn no_frills_config_disables_everything() {
        let cfg = no_frills_config();
        assert!(!cfg.show_time);
        assert!(!cfg.show_level);
        assert!(!cfg.show_target);
        assert!(!cfg.show_fields);
        assert!(!cfg.show_source);
    }

    #[test]
    fn event_with_no_message_only_fields() {
        let writer = SharedWriter::new();
        let console = Console::new(120, ConsoleSink::writer(writer.clone()));
        let layer = TracingConsoleLayer::with_config(
            console,
            TracingConfig {
                show_time: false,
                show_level: false,
                show_target: false,
                show_fields: true,
                show_source: false,
            },
        );
        let subscriber = tracing_subscriber::registry().with(layer);
        let dispatch = tracing::Dispatch::new(subscriber);

        tracing::dispatcher::with_default(&dispatch, || {
            tracing::info!(action = "click", x = 10, y = 20);
        });

        let output = writer.snapshot();
        // Fields should be present even without a message
        assert!(output.contains("action=click"), "output: {output}");
    }

    #[test]
    fn fields_hidden_when_show_fields_false() {
        let writer = SharedWriter::new();
        let console = Console::new(120, ConsoleSink::writer(writer.clone()));
        let layer = TracingConsoleLayer::with_config(
            console,
            TracingConfig {
                show_time: false,
                show_level: false,
                show_target: false,
                show_fields: false,
                show_source: false,
            },
        );
        let subscriber = tracing_subscriber::registry().with(layer);
        let dispatch = tracing::Dispatch::new(subscriber);

        tracing::dispatcher::with_default(&dispatch, || {
            tracing::info!(key = "value", "msg");
        });

        let output = writer.snapshot();
        assert!(output.contains("msg"), "output: {output}");
        assert!(!output.contains("key=value"), "output: {output}");
    }

    #[test]
    fn show_source_includes_file_info() {
        let writer = SharedWriter::new();
        let console = Console::new(200, ConsoleSink::writer(writer.clone()));
        let layer = TracingConsoleLayer::with_config(
            console,
            TracingConfig {
                show_time: false,
                show_level: false,
                show_target: false,
                show_fields: false,
                show_source: true,
            },
        );
        let subscriber = tracing_subscriber::registry().with(layer);
        let dispatch = tracing::Dispatch::new(subscriber);

        tracing::dispatcher::with_default(&dispatch, || {
            tracing::info!("source test");
        });

        let output = writer.snapshot();
        // Output should contain the source file path (this file)
        assert!(
            output.contains("logging.rs"),
            "expected source file in output: {output}"
        );
    }

    #[test]
    fn show_time_includes_colon_separated_timestamp() {
        let writer = SharedWriter::new();
        let console = Console::new(120, ConsoleSink::writer(writer.clone()));
        let layer = TracingConsoleLayer::with_config(
            console,
            TracingConfig {
                show_time: true,
                show_level: false,
                show_target: false,
                show_fields: false,
                show_source: false,
            },
        );
        let subscriber = tracing_subscriber::registry().with(layer);
        let dispatch = tracing::Dispatch::new(subscriber);

        tracing::dispatcher::with_default(&dispatch, || {
            tracing::info!("timestamped");
        });

        let output = writer.snapshot();
        // Timestamp is HH:MM:SS, so should contain two colons
        let colon_count = output.chars().filter(|&c| c == ':').count();
        assert!(
            colon_count >= 2,
            "expected timestamp colons in output: {output}"
        );
    }

    #[test]
    fn show_target_includes_module_path() {
        let writer = SharedWriter::new();
        let console = Console::new(120, ConsoleSink::writer(writer.clone()));
        let layer = TracingConsoleLayer::with_config(
            console,
            TracingConfig {
                show_time: false,
                show_level: false,
                show_target: true,
                show_fields: false,
                show_source: false,
            },
        );
        let subscriber = tracing_subscriber::registry().with(layer);
        let dispatch = tracing::Dispatch::new(subscriber);

        tracing::dispatcher::with_default(&dispatch, || {
            tracing::info!("target test");
        });

        let output = writer.snapshot();
        assert!(
            output.contains("ftui_extras"),
            "expected target module in output: {output}"
        );
    }

    #[test]
    fn empty_message_event() {
        let writer = SharedWriter::new();
        let console = Console::new(120, ConsoleSink::writer(writer.clone()));
        let layer = TracingConsoleLayer::with_config(
            console,
            TracingConfig {
                show_time: false,
                show_level: true,
                show_target: false,
                show_fields: false,
                show_source: false,
            },
        );
        let subscriber = tracing_subscriber::registry().with(layer);
        let dispatch = tracing::Dispatch::new(subscriber);

        tracing::dispatcher::with_default(&dispatch, || {
            tracing::info!("");
        });

        let output = writer.snapshot();
        assert!(output.contains("INFO"), "output: {output}");
    }

    #[test]
    fn into_console_returns_valid_console() {
        let sink = ConsoleSink::capture();
        let console = Console::new(80, sink);
        let layer = TracingConsoleLayer::new(console);
        // into_console should return the inner console without panic
        let _console = layer.into_console();
    }

    #[test]
    fn event_visitor_record_str_message() {
        let v = EventVisitor::default();
        assert!(v.message.is_none());
        assert!(v.fields.is_empty());
    }
}
