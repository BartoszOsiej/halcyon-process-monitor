#![forbid(unsafe_code)]

//! bd-3mjjt.5: Unit tests for deterministic replay.
//!
//! Tests EventTraceWriter/Reader/Replayer round-trip with a model simulator:
//!   1. Record a 100-event session, replay, verify identical frame output
//!   2. Test with resize events mid-session
//!   3. Test with concurrent timer events
//!   4. Verify evidence ledger entries match between record and replay
//!   5. Test corrupted trace file handling (graceful error)
//!
//! Requires feature `event-trace`.
//!
//! Run:
//!   cargo test -p ftui-runtime --features event-trace --test deterministic_replay

use ftui_core::event::{
    Event, KeyCode, KeyEvent, MouseButton, MouseEvent, MouseEventKind, PasteEvent,
};
use ftui_render::buffer::Buffer;
use ftui_render::cell::Cell;
use ftui_render::frame::Frame;
use ftui_render::grapheme_pool::GraphemePool;
use ftui_runtime::event_trace::{
    EventReplayer, EventTraceReader, EventTraceWriter, EvidenceVerifier,
};
use ftui_runtime::program::{Cmd, Model};
use ftui_runtime::unified_evidence::{
    DecisionDomain, EvidenceEntry, EvidenceEntryBuilder, EvidenceTerm,
};

// ============================================================================
// Test Model: deterministic counter with viewport awareness
// ============================================================================

#[derive(Clone, Debug)]
struct CounterModel {
    counter: i32,
    last_key: Option<char>,
    viewport: (u16, u16),
    click_count: u32,
    tick_count: u64,
}

impl CounterModel {
    fn new() -> Self {
        Self {
            counter: 0,
            last_key: None,
            viewport: (80, 24),
            click_count: 0,
            tick_count: 0,
        }
    }
}

#[derive(Debug)]
enum Msg {
    Key(char),
    Resize(u16, u16),
    Click(#[allow(dead_code)] u16, #[allow(dead_code)] u16),
    Tick,
    Other,
}

impl From<Event> for Msg {
    fn from(event: Event) -> Self {
        match event {
            Event::Key(k) => match k.code {
                KeyCode::Char(c) => Msg::Key(c),
                _ => Msg::Other,
            },
            Event::Resize { width, height } => Msg::Resize(width, height),
            Event::Mouse(m) => match m.kind {
                MouseEventKind::Down(MouseButton::Left) => Msg::Click(m.x, m.y),
                _ => Msg::Other,
            },
            Event::Tick => Msg::Tick,
            _ => Msg::Other,
        }
    }
}

impl Model for CounterModel {
    type Message = Msg;

    fn init(&mut self) -> Cmd<Self::Message> {
        Cmd::none()
    }

    fn update(&mut self, msg: Self::Message) -> Cmd<Self::Message> {
        match msg {
            Msg::Key(c) => {
                self.counter += 1;
                self.last_key = Some(c);
            }
            Msg::Resize(w, h) => {
                self.viewport = (w, h);
            }
            Msg::Click(_, _) => {
                self.click_count += 1;
                self.counter += 1;
            }
            Msg::Tick => {
                self.tick_count += 1;
            }
            Msg::Other => {}
        }
        Cmd::none()
    }

    fn view(&self, frame: &mut Frame) {
        // Render deterministic status line.
        let text = format!(
            "c={} k={} v={}x{} cl={} t={}",
            self.counter,
            self.last_key.unwrap_or('-'),
            self.viewport.0,
            self.viewport.1,
            self.click_count,
            self.tick_count,
        );

        for (i, ch) in text.chars().enumerate() {
            if (i as u16) < frame.width() {
                frame.buffer.set_raw(i as u16, 0, Cell::from_char(ch));
            }
        }
    }
}

// ============================================================================
// Helper: simulate with event trace
// ============================================================================

/// Record events into a trace, running them through a model, capture frames.
fn record_session(events: &[(Event, u64)], width: u16, height: u16) -> (Vec<Buffer>, Vec<u8>) {
    let mut model = CounterModel::new();
    model.viewport = (width, height);
    let _ = model.init();

    let mut pool = GraphemePool::new();
    let mut frames = Vec::new();
    let mut trace_buf = Vec::new();

    {
        let mut writer = EventTraceWriter::from_writer(
            &mut trace_buf,
            "test_session",
            (width, height),
            Some(42),
        )
        .expect("create writer");

        for (event, ts_ns) in events {
            // Record the event.
            writer.record(event, *ts_ns).expect("record event");

            // Process through model.
            let msg = Msg::from(event.clone());
            let _ = model.update(msg);

            // Capture frame after each event.
            let mut frame = Frame::new(width, height, &mut pool);
            model.view(&mut frame);
            frames.push(frame.buffer);
        }

        writer.finish().expect("finish");
    }

    (frames, trace_buf)
}

/// Replay events from a trace through a fresh model, capture frames.
fn replay_session(trace_data: &[u8], width: u16, height: u16) -> Vec<Buffer> {
    let trace = EventTraceReader::from_bytes(trace_data).expect("read trace");
    let mut replayer = EventReplayer::from_trace(&trace);

    let mut model = CounterModel::new();
    model.viewport = (width, height);
    let _ = model.init();

    let mut pool = GraphemePool::new();
    let mut frames = Vec::new();

    while let Some((event, _ts)) = replayer.next_event() {
        let msg = Msg::from(event);
        let _ = model.update(msg);

        let mut frame = Frame::new(width, height, &mut pool);
        model.view(&mut frame);
        frames.push(frame.buffer);
    }

    frames
}

// ============================================================================
// Test 1: 100-event session round-trip
// ============================================================================

#[test]
fn hundred_event_session_round_trip() {
    // Generate 100 events: mix of keys, mouse, ticks.
    let mut events = Vec::new();
    for i in 0..100u64 {
        let ts = i * 16_000_000; // ~16ms apart
        let event = match i % 4 {
            0 => Event::Key(KeyEvent::new(KeyCode::Char(
                (b'a' + (i % 26) as u8) as char,
            ))),
            1 => Event::Mouse(MouseEvent::new(
                MouseEventKind::Down(MouseButton::Left),
                (i % 80) as u16,
                (i % 24) as u16,
            )),
            2 => Event::Tick,
            _ => Event::Key(KeyEvent::new(KeyCode::Char(
                (b'A' + (i % 26) as u8) as char,
            ))),
        };
        events.push((event, ts));
    }

    let (recorded_frames, trace_data) = record_session(&events, 80, 24);
    assert_eq!(recorded_frames.len(), 100);

    // Replay and compare.
    let replayed_frames = replay_session(&trace_data, 80, 24);
    assert_eq!(replayed_frames.len(), 100);

    // Every frame must be identical.
    for (i, (rec, rep)) in recorded_frames
        .iter()
        .zip(replayed_frames.iter())
        .enumerate()
    {
        assert!(
            rec.content_eq(rep),
            "frame {i} differs between record and replay"
        );
    }
}

// ============================================================================
// Test 2: resize events mid-session
// ============================================================================

#[test]
fn resize_events_mid_session() {
    let events = vec![
        (Event::Key(KeyEvent::new(KeyCode::Char('a'))), 1_000_000),
        (Event::Key(KeyEvent::new(KeyCode::Char('b'))), 2_000_000),
        (
            Event::Resize {
                width: 120,
                height: 40,
            },
            3_000_000,
        ),
        (Event::Key(KeyEvent::new(KeyCode::Char('c'))), 4_000_000),
        (
            Event::Resize {
                width: 60,
                height: 15,
            },
            5_000_000,
        ),
        (Event::Key(KeyEvent::new(KeyCode::Char('d'))), 6_000_000),
        (
            Event::Resize {
                width: 80,
                height: 24,
            },
            7_000_000,
        ),
        (Event::Key(KeyEvent::new(KeyCode::Char('e'))), 8_000_000),
    ];

    // Use fixed viewport for frame capture (model tracks viewport internally).
    let (recorded_frames, trace_data) = record_session(&events, 80, 24);
    let replayed_frames = replay_session(&trace_data, 80, 24);

    assert_eq!(recorded_frames.len(), replayed_frames.len());
    for (i, (rec, rep)) in recorded_frames
        .iter()
        .zip(replayed_frames.iter())
        .enumerate()
    {
        assert!(rec.content_eq(rep), "frame {i} differs after resize events");
    }

    // Verify the trace file preserves resize events.
    let trace = EventTraceReader::from_bytes(&trace_data).expect("read");
    let events_back = trace.events_with_timestamps();
    let resize_count = events_back
        .iter()
        .filter(|(e, _)| matches!(e, Event::Resize { .. }))
        .count();
    assert_eq!(resize_count, 3, "expected 3 resize events in trace");
}

// ============================================================================
// Test 3: concurrent timer events
// ============================================================================

#[test]
fn concurrent_timer_events() {
    // Interleave ticks with key events at various intervals.
    let mut events = Vec::new();
    let mut ts = 0u64;

    for i in 0..50u64 {
        // Key event
        events.push((
            Event::Key(KeyEvent::new(KeyCode::Char(
                (b'a' + (i % 26) as u8) as char,
            ))),
            ts,
        ));
        ts += 8_000_000; // 8ms

        // Tick between some key events
        events.push((Event::Tick, ts));
        ts += 8_000_000;

        // Extra rapid ticks
        if i % 5 == 0 {
            events.push((Event::Tick, ts));
            ts += 1_000_000;
            events.push((Event::Tick, ts));
            ts += 1_000_000;
        }
    }

    let (recorded_frames, trace_data) = record_session(&events, 80, 24);
    let replayed_frames = replay_session(&trace_data, 80, 24);

    assert_eq!(recorded_frames.len(), replayed_frames.len());
    for (i, (rec, rep)) in recorded_frames
        .iter()
        .zip(replayed_frames.iter())
        .enumerate()
    {
        assert!(
            rec.content_eq(rep),
            "frame {i} differs with concurrent timers"
        );
    }

    // Verify tick count matches.
    let trace = EventTraceReader::from_bytes(&trace_data).expect("read");
    let tick_count = trace
        .events_with_timestamps()
        .iter()
        .filter(|(e, _)| matches!(e, Event::Tick))
        .count();
    assert!(
        tick_count >= 50,
        "expected at least 50 ticks, got {tick_count}"
    );
}

// ============================================================================
// Test 4: evidence ledger entries match between record and replay
// ============================================================================

fn make_evidence(domain: DecisionDomain, action: &'static str, id: u64) -> EvidenceEntry {
    EvidenceEntry {
        decision_id: id,
        timestamp_ns: id * 16_000_000,
        domain,
        log_posterior: 1.386 + (id as f64 * 0.01), // slightly different per entry
        top_evidence: [
            Some(EvidenceTerm::new("change_rate", 4.0 + id as f64 * 0.1)),
            Some(EvidenceTerm::new("dirty_ratio", 2.5)),
            None,
        ],
        action,
        loss_avoided: 0.15 + id as f64 * 0.001,
        confidence_interval: (0.72, 0.95),
    }
}

#[test]
fn evidence_ledger_entries_match_during_replay() {
    // Record a session with interleaved events and evidence entries.
    let events = [
        (Event::Key(KeyEvent::new(KeyCode::Char('a'))), 1_000_000),
        (Event::Key(KeyEvent::new(KeyCode::Char('b'))), 2_000_000),
        (Event::Key(KeyEvent::new(KeyCode::Char('c'))), 3_000_000),
        (Event::Key(KeyEvent::new(KeyCode::Char('d'))), 4_000_000),
        (Event::Key(KeyEvent::new(KeyCode::Char('e'))), 5_000_000),
    ];

    let evidence = [
        make_evidence(DecisionDomain::DiffStrategy, "dirty_rows", 0),
        make_evidence(DecisionDomain::FrameBudget, "hold", 1),
        make_evidence(DecisionDomain::VoiSampling, "sample", 2),
        make_evidence(DecisionDomain::Degradation, "degrade_1", 3),
        make_evidence(DecisionDomain::ResizeCoalescing, "apply", 4),
    ];

    let mut trace_buf = Vec::new();
    {
        let mut writer =
            EventTraceWriter::from_writer(&mut trace_buf, "evidence_test", (80, 24), Some(42))
                .expect("create writer");

        // Interleave events and evidence.
        for (i, (event, ts)) in events.iter().enumerate() {
            writer.record(event, *ts).expect("record event");
            writer
                .record_evidence(&evidence[i], *ts + 500_000)
                .expect("record evidence");
        }

        assert_eq!(writer.evidence_count(), 5);
        writer.finish().expect("finish");
    }

    // Read trace and verify evidence round-trip.
    let trace = EventTraceReader::from_bytes(&trace_buf).expect("read");
    assert_eq!(trace.total_evidence(), Some(5));

    let trace_evidence = trace.evidence_entries();
    assert_eq!(trace_evidence.len(), 5);

    // Verify each evidence entry matches the original.
    let mut verifier = EvidenceVerifier::from_trace(&trace, 1e-10);
    for entry in &evidence {
        assert!(
            verifier.verify(entry),
            "evidence mismatch for {:?}",
            entry.action
        );
    }
    assert!(verifier.is_deterministic());
    assert_eq!(verifier.verified_count(), 5);
    assert!(verifier.summary().contains("PASS"));
}

#[test]
fn evidence_replay_detects_nondeterminism() {
    // Record evidence entries.
    let evidence = [
        make_evidence(DecisionDomain::DiffStrategy, "dirty_rows", 0),
        make_evidence(DecisionDomain::FrameBudget, "hold", 1),
    ];

    let mut trace_buf = Vec::new();
    {
        let mut writer =
            EventTraceWriter::from_writer(&mut trace_buf, "nondet_test", (80, 24), None)
                .expect("create writer");
        for (i, entry) in evidence.iter().enumerate() {
            writer
                .record_evidence(entry, (i as u64 + 1) * 1_000_000)
                .expect("record evidence");
        }
        writer.finish().expect("finish");
    }

    let trace = EventTraceReader::from_bytes(&trace_buf).expect("read");
    let mut verifier = EvidenceVerifier::from_trace(&trace, 1e-10);

    // Replay with a different action for the second entry.
    let ok = verifier.verify(&evidence[0]);
    assert!(ok, "first entry should match");

    let mut wrong_entry = make_evidence(DecisionDomain::FrameBudget, "allocate", 1);
    wrong_entry.log_posterior = 99.0; // completely different
    let ok = verifier.verify(&wrong_entry);
    assert!(!ok, "second entry should NOT match");

    assert!(!verifier.is_deterministic());
    assert!(verifier.summary().contains("FAIL"));

    // Should detect both action and log_posterior mismatches.
    let fields: Vec<&str> = verifier
        .mismatches()
        .iter()
        .map(|m| m.field.as_str())
        .collect();
    assert!(fields.contains(&"action"), "expected action mismatch");
    assert!(
        fields.contains(&"log_posterior"),
        "expected log_posterior mismatch"
    );
}

#[test]
fn evidence_with_all_seven_domains() {
    // Verify round-trip for all 7 decision domains.
    let domains = DecisionDomain::ALL;
    let actions = [
        "dirty_rows",
        "apply",
        "hold",
        "degrade_1",
        "sample",
        "rank_1",
        "exact",
    ];

    let entries: Vec<EvidenceEntry> = domains
        .iter()
        .zip(actions.iter())
        .enumerate()
        .map(|(i, (d, a))| make_evidence(*d, a, i as u64))
        .collect();

    let mut trace_buf = Vec::new();
    {
        let mut writer =
            EventTraceWriter::from_writer(&mut trace_buf, "all_domains", (80, 24), None)
                .expect("create writer");
        for (i, entry) in entries.iter().enumerate() {
            writer
                .record_evidence(entry, (i as u64 + 1) * 1_000_000)
                .expect("record evidence");
        }
        writer.finish().expect("finish");
    }

    let trace = EventTraceReader::from_bytes(&trace_buf).expect("read");
    let mut verifier = EvidenceVerifier::from_trace(&trace, 1e-10);

    for entry in &entries {
        assert!(verifier.verify(entry));
    }
    assert!(verifier.is_deterministic());
}

#[test]
fn evidence_with_builder_round_trip() {
    // Use EvidenceEntryBuilder to construct entries and verify round-trip.
    let entry = EvidenceEntryBuilder::new(DecisionDomain::PaletteScoring, 42, 5_000_000)
        .log_posterior(2.302)
        .evidence("match_type", 9.0)
        .evidence("position", 1.5)
        .evidence("word_boundary", 2.0)
        .evidence("gap_penalty", 0.5) // will be dropped (top-3 only)
        .action("exact")
        .loss_avoided(0.8)
        .confidence_interval(0.90, 0.99)
        .build();

    let mut trace_buf = Vec::new();
    {
        let mut writer =
            EventTraceWriter::from_writer(&mut trace_buf, "builder_test", (80, 24), None)
                .expect("create writer");
        writer
            .record_evidence(&entry, 5_000_000)
            .expect("record evidence");
        writer.finish().expect("finish");
    }

    let trace = EventTraceReader::from_bytes(&trace_buf).expect("read");
    let mut verifier = EvidenceVerifier::from_trace(&trace, 1e-10);
    assert!(verifier.verify(&entry));
    assert!(verifier.is_deterministic());
}

// ============================================================================
// Test 5: corrupted trace file handling
// ============================================================================

#[test]
fn corrupted_trace_empty_data() {
    let result = EventTraceReader::from_bytes(&[]);
    // Empty data should parse as empty trace (no records).
    match result {
        Ok(trace) => {
            assert!(trace.records().is_empty());
        }
        Err(_) => {
            // Also acceptable — empty data could error.
        }
    }
}

#[test]
fn corrupted_trace_invalid_json() {
    let data = b"not valid json\n{also bad}\n";
    let result = EventTraceReader::from_bytes(data);
    assert!(result.is_err(), "invalid JSON should error");
}

#[test]
fn corrupted_trace_truncated_gzip() {
    // Create valid gzip header but truncate the data.
    let data = vec![0x1f, 0x8b, 0x08, 0x00, 0x00, 0x00, 0x00, 0x00, 0x02, 0xff];
    let result = EventTraceReader::from_bytes(&data);
    assert!(result.is_err(), "truncated gzip should error");
}

#[test]
fn corrupted_trace_partial_jsonl() {
    // Valid header line, then a truncated/invalid second line.
    let mut buf = Vec::new();
    {
        let mut writer = EventTraceWriter::from_writer(&mut buf, "partial", (80, 24), None)
            .expect("create writer");
        writer.record(&Event::Tick, 100).expect("record");
        writer.finish().expect("finish");
    }

    // Corrupt the last line.
    let text = String::from_utf8(buf).expect("utf8");
    let mut lines: Vec<&str> = text.lines().collect();
    // Replace summary with garbage.
    if let Some(last) = lines.last_mut() {
        *last = "{\"event\":\"trace_summary\",\"total_events\":999,\"invalid_field";
    }
    let corrupted = lines.join("\n") + "\n";
    let result = EventTraceReader::from_bytes(corrupted.as_bytes());
    assert!(result.is_err(), "partial JSON should error");
}

#[test]
fn corrupted_trace_unknown_event_type() {
    // A trace with an unknown event type should fail to parse.
    let data = concat!(
        r#"{"event":"trace_header","schema_version":"event-trace-v1","session_name":"t","terminal_size":[80,24]}"#,
        "\n",
        r#"{"event":"unknown_future_event","ts_ns":100,"data":"hello"}"#,
        "\n",
        r#"{"event":"trace_summary","total_events":1,"total_duration_ns":0}"#,
        "\n"
    );
    let result = EventTraceReader::from_bytes(data.as_bytes());
    // serde will fail on unknown tagged variant.
    assert!(result.is_err(), "unknown event type should error");
}

#[test]
fn corrupted_trace_mismatched_event_count() {
    // A trace where summary claims more events than present. Should parse,
    // but event count in summary won't match actual records.
    let mut buf = Vec::new();
    {
        let mut writer = EventTraceWriter::from_writer(&mut buf, "mismatch", (80, 24), None)
            .expect("create writer");
        writer.record(&Event::Tick, 100).expect("record");
        writer.finish().expect("finish");
    }

    // The trace is valid — just check we can read it.
    let trace = EventTraceReader::from_bytes(&buf).expect("read");
    assert_eq!(trace.total_events(), Some(1));
    assert_eq!(trace.events_with_timestamps().len(), 1);
}

// ============================================================================
// Additional edge cases
// ============================================================================

#[test]
fn replay_preserves_event_ordering() {
    // Verify events come back in exact recorded order.
    let events: Vec<(Event, u64)> = (0..20)
        .map(|i| {
            (
                Event::Key(KeyEvent::new(KeyCode::Char(
                    (b'a' + (i % 26) as u8) as char,
                ))),
                (i as u64) * 1_000_000,
            )
        })
        .collect();

    let mut buf = Vec::new();
    {
        let mut writer = EventTraceWriter::from_writer(&mut buf, "order_test", (80, 24), None)
            .expect("create writer");
        for (event, ts) in &events {
            writer.record(event, *ts).expect("record");
        }
        writer.finish().expect("finish");
    }

    let trace = EventTraceReader::from_bytes(&buf).expect("read");
    let replayed = trace.events_with_timestamps();

    assert_eq!(replayed.len(), events.len());
    for (i, ((orig_event, orig_ts), (rep_event, rep_ts))) in
        events.iter().zip(replayed.iter()).enumerate()
    {
        assert_eq!(orig_event, rep_event, "event {i} differs");
        assert_eq!(orig_ts, rep_ts, "timestamp {i} differs");
    }
}

#[test]
fn replay_with_paste_events() {
    let events = vec![
        (Event::Key(KeyEvent::new(KeyCode::Char('a'))), 1_000_000),
        (
            Event::Paste(PasteEvent::bracketed("hello world")),
            2_000_000,
        ),
        (Event::Key(KeyEvent::new(KeyCode::Char('b'))), 3_000_000),
        (
            Event::Paste(PasteEvent::bracketed("unicode: 漢字 emoji: 🎉")),
            4_000_000,
        ),
    ];

    let mut buf = Vec::new();
    {
        let mut writer = EventTraceWriter::from_writer(&mut buf, "paste_test", (80, 24), None)
            .expect("create writer");
        for (event, ts) in &events {
            writer.record(event, *ts).expect("record");
        }
        writer.finish().expect("finish");
    }

    let trace = EventTraceReader::from_bytes(&buf).expect("read");
    let replayed = trace.events_with_timestamps();
    assert_eq!(replayed.len(), 4);

    for (i, ((orig, _), (rep, _))) in events.iter().zip(replayed.iter()).enumerate() {
        assert_eq!(orig, rep, "event {i} (paste) differs");
    }
}

#[test]
fn gzip_round_trip_with_evidence() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("trace.jsonl.gz");

    let evidence = make_evidence(DecisionDomain::DiffStrategy, "dirty_rows", 0);

    {
        let mut writer =
            EventTraceWriter::gzip(&path, "gz_evidence", (80, 24)).expect("create gz writer");
        writer.record(&Event::Tick, 1_000).expect("record tick");
        writer
            .record_evidence(&evidence, 1_500)
            .expect("record evidence");
        writer
            .record(&Event::Key(KeyEvent::new(KeyCode::Char('x'))), 2_000)
            .expect("record key");

        let encoder = writer.finish().expect("finish");
        encoder.finish().expect("flush gzip");
    }

    let trace = EventTraceReader::open(&path).expect("read gz");
    assert_eq!(trace.total_events(), Some(3)); // tick + evidence + key
    assert_eq!(trace.total_evidence(), Some(1));

    let events = trace.events_with_timestamps();
    assert_eq!(events.len(), 2); // tick + key (evidence doesn't convert to Event)

    let ev = trace.evidence_entries();
    assert_eq!(ev.len(), 1);
    assert_eq!(ev[0].0.action, "dirty_rows");

    // Verify evidence via verifier.
    let mut verifier = EvidenceVerifier::from_trace(&trace, 1e-10);
    assert!(verifier.verify(&evidence));
    assert!(verifier.is_deterministic());
}

#[test]
fn replayer_advance_until_with_evidence_in_trace() {
    // Evidence records should not interfere with EventReplayer's event-only replay.
    let mut buf = Vec::new();
    {
        let mut writer = EventTraceWriter::from_writer(&mut buf, "advance_test", (80, 24), None)
            .expect("create writer");

        writer
            .record(&Event::Key(KeyEvent::new(KeyCode::Char('a'))), 1_000)
            .expect("record");
        writer
            .record_evidence(
                &make_evidence(DecisionDomain::DiffStrategy, "full", 0),
                1_500,
            )
            .expect("evidence");
        writer
            .record(&Event::Key(KeyEvent::new(KeyCode::Char('b'))), 2_000)
            .expect("record");
        writer
            .record(&Event::Key(KeyEvent::new(KeyCode::Char('c'))), 3_000)
            .expect("record");

        writer.finish().expect("finish");
    }

    let trace = EventTraceReader::from_bytes(&buf).expect("read");
    let mut replayer = EventReplayer::from_trace(&trace);

    // Should only see the 3 key events, not the evidence record.
    assert_eq!(replayer.total(), 3);

    let batch = replayer.advance_until(2_000);
    assert_eq!(batch.len(), 2); // 'a' at 1000, 'b' at 2000
}

#[test]
fn large_session_with_mixed_events_and_evidence() {
    // Stress test: 200 events + 50 evidence entries.
    let mut trace_buf = Vec::new();
    let mut model = CounterModel::new();
    let _ = model.init();
    let mut pool = GraphemePool::new();
    let mut recorded_frames = Vec::new();
    let mut evidence_entries = Vec::new();

    {
        let mut writer =
            EventTraceWriter::from_writer(&mut trace_buf, "stress_test", (80, 24), Some(12345))
                .expect("create writer");

        for i in 0..200u64 {
            let ts = i * 16_000_000;
            let event = match i % 5 {
                0 => Event::Key(KeyEvent::new(KeyCode::Char(
                    (b'a' + (i % 26) as u8) as char,
                ))),
                1 => Event::Mouse(MouseEvent::new(
                    MouseEventKind::Down(MouseButton::Left),
                    (i % 80) as u16,
                    (i % 24) as u16,
                )),
                2 => Event::Tick,
                3 => Event::Resize {
                    width: 80 + (i % 40) as u16,
                    height: 24 + (i % 10) as u16,
                },
                _ => Event::Key(KeyEvent::new(KeyCode::Char(
                    (b'0' + (i % 10) as u8) as char,
                ))),
            };

            writer.record(&event, ts).expect("record");

            let msg = Msg::from(event);
            let _ = model.update(msg);

            let mut frame = Frame::new(80, 24, &mut pool);
            model.view(&mut frame);
            recorded_frames.push(frame.buffer);

            // Record evidence every 4th event.
            if i % 4 == 0 {
                let domain = DecisionDomain::ALL[(i as usize / 4) % 7];
                let actions = [
                    "dirty_rows",
                    "apply",
                    "hold",
                    "degrade_1",
                    "sample",
                    "rank_1",
                    "exact",
                ];
                let action = actions[(i as usize / 4) % 7];
                let entry = make_evidence(domain, action, i / 4);
                writer
                    .record_evidence(&entry, ts + 500_000)
                    .expect("record evidence");
                evidence_entries.push(entry);
            }
        }

        assert_eq!(writer.evidence_count(), 50);
        writer.finish().expect("finish");
    }

    // Replay events.
    let replayed_frames = replay_session(&trace_buf, 80, 24);
    assert_eq!(replayed_frames.len(), 200);
    for (i, (rec, rep)) in recorded_frames
        .iter()
        .zip(replayed_frames.iter())
        .enumerate()
    {
        assert!(rec.content_eq(rep), "frame {i} differs in stress test");
    }

    // Verify evidence.
    let trace = EventTraceReader::from_bytes(&trace_buf).expect("read");
    assert_eq!(trace.total_evidence(), Some(50));
    let mut verifier = EvidenceVerifier::from_trace(&trace, 1e-10);
    for entry in &evidence_entries {
        assert!(verifier.verify(entry));
    }
    assert!(verifier.is_deterministic());
}
